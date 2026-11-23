//! Позиция текстовой каретки через AT-SPI2.
//!
//! Работает только если в сессии включён `org.a11y.Status.IsEnabled` **и**
//! приложение запущено после включения (см. docs/spike-report.md, M0-05).
//! Если каретку получить не удалось — вызывающий код откатывается на курсор мыши.
//!
//! Все D-Bus вызовы выполняются в отдельном потоке с tokio-рантаймом, чтобы
//! зависшее приложение не блокировало демон.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use atspi::events::ObjectEvents;
use atspi::events::object::StateChangedEvent;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::{AccessibilityConnection, CoordType, State};
use futures_util::StreamExt;

/// Бюджет обхода дерева — защита от гигантских UI.
const NODE_BUDGET: usize = 8000;

/// Сколько ждать весь поиск.
const SEARCH_TIMEOUT: Duration = Duration::from_millis(800);

/// Прямоугольник в координатах экрана.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

enum Request {
    CaretRect(mpsc::Sender<Option<Rect>>),
}

/// Дескриптор фонового AT-SPI-воркера.
pub struct A11y {
    tx: mpsc::Sender<Request>,
}

impl A11y {
    /// Запустить воркер. Соединение устанавливается лениво, в фоне.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || worker(rx));
        Self { tx }
    }

    /// Прямоугольник каретки в координатах экрана, если удалось получить.
    pub fn caret_rect(&self, timeout: Duration) -> Option<Rect> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(Request::CaretRect(reply_tx)).ok()?;
        reply_rx.recv_timeout(timeout).ok().flatten()
    }
}

fn worker(rx: mpsc::Receiver<Request>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            crate::log::debug(&format!("atspi: не удалось создать рантайм: {err}"));
            return;
        }
    };

    let connection = match runtime.block_on(AccessibilityConnection::new()) {
        Ok(connection) => connection,
        Err(err) => {
            crate::log::debug(&format!("atspi: нет соединения — {err}"));
            for request in rx {
                let Request::CaretRect(reply) = request;
                let _ = reply.send(None);
            }
            return;
        }
    };

    for request in rx {
        match request {
            Request::CaretRect(reply) => {
                let rect = runtime.block_on(async {
                    tokio::time::timeout(SEARCH_TIMEOUT, find_caret(&connection))
                        .await
                        .ok()
                        .flatten()
                        .map(|(_, _, rect)| rect)
                });
                let _ = reply.send(rect);
            }
        }
    }
}

fn normalize(rect: &Rect) -> Rect {
    // На практике ширина в конце строки бывает 0 — это нормально,
    // высота строки при этом осмысленная (M0-05).
    if rect.w < 0 {
        Rect {
            x: rect.x + rect.w,
            w: -rect.w,
            ..*rect
        }
    } else {
        *rect
    }
}

async fn find_caret(connection: &AccessibilityConnection) -> Option<(String, String, Rect)> {
    let root = match connection.root_accessible_on_registry().await {
        Ok(root) => root,
        Err(err) => {
            crate::log::debug(&format!("atspi: корень реестра недоступен: {err}"));
            return None;
        }
    };
    let children = match root.get_children().await {
        Ok(children) => children,
        Err(err) => {
            crate::log::debug(&format!("atspi: get_children корня: {err}"));
            return None;
        }
    };
    crate::log::debug(&format!("atspi: приложений в реестре: {}", children.len()));

    let mut queue: VecDeque<_> = children.into();
    let mut visited = 0usize;
    let mut focused_seen = 0usize;

    while let Some(object) = queue.pop_front() {
        if visited >= NODE_BUDGET {
            crate::log::debug("atspi: исчерпан бюджет обхода");
            break;
        }
        visited += 1;

        let Ok(proxy) = object.as_accessible_proxy(connection.connection()).await else {
            continue;
        };

        if let Ok(state) = proxy.get_state().await {
            if state.contains(State::Focused) {
                focused_seen += 1;
                if let Some(rect) = caret_of(&proxy).await {
                    crate::log::debug(&format!(
                        "atspi: каретка найдена, узлов просмотрено {visited}"
                    ));
                    let inner = proxy.inner();
                    return Some((
                        inner.destination().to_string(),
                        inner.path().to_string(),
                        normalize(&rect),
                    ));
                }
            }
        }

        if let Ok(children) = proxy.get_children().await {
            queue.extend(children);
        }
    }
    crate::log::debug(&format!(
        "atspi: каретка не найдена (узлов {visited}, focused {focused_seen})"
    ));
    None
}

/// Ищет каретку у объекта и его потомков (Qt присылает фокус на контейнер,
/// а текстовый виджет лежит внутри). Бюджет узлов ограничен.
fn caret_in_subtree<'a>(
    proxy: &'a atspi::proxy::accessible::AccessibleProxy<'a>,
    budget: &'a mut usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Rect>> + 'a>> {
    Box::pin(async move {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;

        if let Some(rect) = caret_of(proxy).await {
            return Some(rect);
        }

        let Ok(children) = proxy.get_children().await else {
            return None;
        };
        for child in children {
            let Ok(child_proxy) = child.as_accessible_proxy(proxy.inner().connection()).await
            else {
                continue;
            };
            if let Some(rect) = caret_in_subtree(&child_proxy, budget).await {
                return Some(rect);
            }
            if *budget == 0 {
                break;
            }
        }
        None
    })
}

async fn caret_of(proxy: &atspi::proxy::accessible::AccessibleProxy<'_>) -> Option<Rect> {
    let inner = proxy.inner();
    caret_at(
        inner.connection(),
        &inner.destination().to_string(),
        &inner.path().to_string(),
    )
    .await
}

/// Каретка у объекта по его адресу на шине.
///
/// `TextProxy` строится напрямую: некоторые тулкиты (Qt) не отдают список
/// интерфейсов, и `proxies.text()` на них падает, хотя Text работает.
async fn caret_at(conn: &atspi::zbus::Connection, destination: &str, path: &str) -> Option<Rect> {
    let builder = atspi::proxy::text::TextProxy::builder(conn)
        .destination(destination.to_string())
        .ok()?
        .path(path.to_string())
        .ok()?;
    let text = builder.build().await.ok()?;

    let offset = text.caret_offset().await.ok()?;
    if offset < 0 {
        return None;
    }

    let (x, y, w, h) = text
        .get_character_extents(offset, CoordType::Screen)
        .await
        .ok()?;

    // Мусорные координаты отбрасываем: у GTK в конце строки ширина 0,
    // у Qt высота 0 — это нормально. Невалидно, когда оба размера неположительные
    // (встречалось как (0,0,0,0)).
    if w <= 0 && h <= 0 {
        return None;
    }
    Some(Rect { x, y, w, h })
}

/// Минимальный интервал между сигналами о фокусе (антидребезг).
const FOCUS_DEBOUNCE: Duration = Duration::from_millis(120);

/// Бюджет и таймаут полного обхода дерева (дорогой путь для Qt).
const DEEP_NODE_BUDGET: usize = 6000;
const DEEP_TIMEOUT: Duration = Duration::from_millis(6000);

/// Слушать AT-SPI-события фокуса.
///
/// На каждое событие «элемент получил фокус» сразу запрашивает каретку
/// у **этого** объекта (без обхода дерева — быстро и работает с Qt, где
/// общий поиск не успевает по таймауту). В `tx` уходит прямоугольник каретки,
/// если элемент действительно текстовый.
///
/// Требует включённой accessibility; при её отсутствии подписка не удастся,
/// и функция просто завершится.
pub fn spawn_focus_listener(tx: mpsc::Sender<Rect>, deep_search: bool) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                crate::log::debug(&format!("atspi-события: нет рантайма: {err}"));
                return;
            }
        };

        runtime.block_on(async move {
            let connection = match AccessibilityConnection::new().await {
                Ok(connection) => connection,
                Err(err) => {
                    crate::log::debug(&format!("atspi-события: нет соединения: {err}"));
                    return;
                }
            };

            if let Err(err) = connection.register_event::<ObjectEvents>().await {
                crate::log::debug(&format!(
                    "atspi-события: подписка не удалась (accessibility выключена?): {err}"
                ));
                return;
            }
            crate::log::debug("atspi-события: подписка на фокус активна");

            let stream = connection.event_stream();
            let mut events = std::pin::pin!(stream);
            let mut last = Instant::now() - FOCUS_DEBOUNCE;
            // Найденный дорогим обходом текстовый объект запоминаем по приложению.
            let mut cache: std::collections::HashMap<String, (String, String)> =
                std::collections::HashMap::new();

            while let Some(item) = events.next().await {
                let Ok(event) = item else {
                    continue;
                };
                let Ok(state_changed) = StateChangedEvent::try_from(event) else {
                    continue;
                };
                if state_changed.state != State::Focused || !state_changed.enabled {
                    continue;
                }
                if last.elapsed() < FOCUS_DEBOUNCE {
                    continue;
                }

                let app_key = state_changed
                    .item
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_default();

                // 1. Кэш: тот же текстовый объект в этом приложении.
                if let Some((destination, path)) = cache.get(&app_key).cloned() {
                    if let Some(rect) = caret_at(connection.connection(), &destination, &path).await
                    {
                        last = Instant::now();
                        if tx.send(rect).is_err() {
                            break;
                        }
                        continue;
                    }
                    cache.remove(&app_key);
                }

                // 2. Быстрый путь: объект из события и его поддерево.
                let Ok(proxy) = state_changed
                    .item
                    .as_accessible_proxy(connection.connection())
                    .await
                else {
                    crate::log::debug("atspi-событие: объект фокуса недоступен");
                    continue;
                };
                if let Some(rect) = caret_in_subtree(&proxy, &mut 400).await {
                    last = Instant::now();
                    if tx.send(rect).is_err() {
                        break;
                    }
                    continue;
                }

                // 3. Дорогой путь: полный обход дерева (нужен для Qt).
                if !deep_search {
                    crate::log::debug("atspi-событие: фокус не текстовый");
                    continue;
                }
                let deep = tokio::time::timeout(DEEP_TIMEOUT, find_caret(&connection)).await;
                match deep {
                    Ok(Some((destination, path, rect))) => {
                        cache.insert(app_key, (destination, path));
                        last = Instant::now();
                        if tx.send(rect).is_err() {
                            break;
                        }
                    }
                    _ => crate::log::debug("atspi-событие: фокус не текстовый"),
                }
            }
        });
    });
}

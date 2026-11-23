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
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{AccessibilityConnection, CoordType, State};
use futures_util::StreamExt;

/// Бюджет обхода дерева — защита от гигантских UI.
const NODE_BUDGET: usize = 3000;

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

async fn find_caret(connection: &AccessibilityConnection) -> Option<Rect> {
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
                    return Some(normalize(&rect));
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

async fn caret_of(proxy: &atspi::proxy::accessible::AccessibleProxy<'_>) -> Option<Rect> {
    let proxies = match proxy.proxies().await {
        Ok(proxies) => proxies,
        Err(err) => {
            crate::log::debug(&format!("atspi: proxies(): {err}"));
            return None;
        }
    };
    let text = match proxies.text().await {
        Ok(text) => text,
        Err(err) => {
            crate::log::debug(&format!("atspi: нет Text-интерфейса: {err}"));
            return None;
        }
    };
    let offset = match text.caret_offset().await {
        Ok(offset) => offset,
        Err(err) => {
            crate::log::debug(&format!("atspi: caret_offset: {err}"));
            return None;
        }
    };
    if offset < 0 {
        return None;
    }
    let (x, y, w, h) = match text.get_character_extents(offset, CoordType::Screen).await {
        Ok(extents) => extents,
        Err(err) => {
            crate::log::debug(&format!("atspi: character_extents: {err}"));
            return None;
        }
    };
    Some(Rect { x, y, w, h })
}

/// Минимальный интервал между сигналами о фокусе (антидребезг).
const FOCUS_DEBOUNCE: Duration = Duration::from_millis(120);

/// Слушать AT-SPI-события фокуса.
///
/// Сигналит в `tx`, когда **элемент** получает фокус (в том числе клик по полю
/// внутри уже активного окна). Фильтр по `STATE_FOCUSED` здесь только грубый:
/// что это действительно текстовое поле, проверяет вызывающий код через
/// [`A11y::caret_rect`].
///
/// Требует включённой accessibility в сессии; при её отсутствии подписка
/// не удастся, и функция просто завершится.
pub fn spawn_focus_listener(tx: mpsc::Sender<()>) {
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
                last = Instant::now();

                if tx.send(()).is_err() {
                    break;
                }
            }
        });
    });
}

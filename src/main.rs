//! layout-indicator — индикатор текущей раскладки клавиатуры рядом с точкой ввода.
//!
//! Слои (см. PRD §7.1):
//!   * [`backend`]   — откуда узнать про смену раскладки (платформенно-зависимый);
//!   * [`presenter`] — политика показа (тайминги, кросс-фейд, дедупликация);
//!   * [`renderer`]  — как нарисовать поверх других окон, не мешая им;
//!   * [`config`]    — пользовательская конфигурация (YAML);
//!   * [`layout`]    — индекс группы XKB → код раскладки (`0 → EN`);
//!   * [`screen`]    — геометрия мониторов;
//!   * [`theme`]     — системная тема и палитра;
//!   * [`instance`]  — защита от второго экземпляра.

// Часть конфигурации относится к ещё не реализованным режимам (caret/window, show_on_focus).
#![allow(dead_code)]

mod a11y;
mod backend;
mod click;
mod config;
mod focus;
mod instance;
mod layout;
mod log;
mod presenter;
mod renderer;
mod screen;
mod theme;
mod tray;

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use backend::BackendKind;
use config::{Config, Positioning};
use instance::InstanceGuard;
use presenter::Presenter;
use renderer::x11::X11Overlay;
use renderer::{Layer, render_badge};
use screen::Monitor;

/// Как часто перечитывать список раскладок (пользователь мог добавить новую).
const LAYOUTS_REFRESH: Duration = Duration::from_secs(5);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log::init(args.iter().any(|a| a == "-v" || a == "--verbose"));

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--doctor") {
        return doctor();
    }
    if args.iter().any(|a| a == "--print-config") {
        print!("{}", Config::default_yaml());
        return Ok(());
    }
    if args.iter().any(|a| a == "--install-autostart") {
        return install_autostart();
    }
    if args.iter().any(|a| a == "--install-icons") {
        return install_icons();
    }

    if args.iter().any(|a| a == "--watch") {
        return watch();
    }
    if args.iter().any(|a| a == "--caret-probe") {
        return caret_probe();
    }
    if let Some(text) = arg_value(&args, "--demo") {
        return demo(&text);
    }

    // Один демон на сессию; блокировка снимается при выходе процесса.
    let _guard = InstanceGuard::acquire()?;
    run()
}

fn print_help() {
    println!(
        "layout-indicator {} — индикатор раскладки",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Использование: layout-indicator [опция]");
    println!();
    println!("  (без опций)          запустить демон");
    println!("  --demo <TEXT>        показать бейдж один раз, например --demo EN");
    println!("  --watch              печатать смену раскладки (отладка)");
    println!("  --caret-probe        попробовать получить позицию каретки через AT-SPI");
    println!("  --doctor             диагностика окружения");
    println!("  --print-config       напечатать YAML с настройками по умолчанию");
    println!("  --install-autostart  добавить автозапуск в ~/.config/autostart");
    println!("  --install-icons      установить иконки трея в ~/.local/share/icons");
    println!("  -v, --verbose        подробный лог в stderr");
    println!("  --help, -h           эта справка");
    println!();
    println!(
        "Конфигурация: {}",
        Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).cloned()
}

/// Всё, что пересоздаётся при изменении конфигурации.
struct Runtime {
    cfg: Config,
    font: ab_glyph::FontVec,
    overlay: X11Overlay,
    config_mtime: Option<SystemTime>,
}

impl Runtime {
    fn new() -> Result<Self> {
        let mut cfg = Config::load()?;
        let font = renderer::load_font(&cfg.appearance.font_path)?;
        let overlay = X11Overlay::new(cfg.appearance.width, cfg.appearance.height)?;
        apply_environment(&mut cfg, &overlay);
        Ok(Self {
            cfg,
            font,
            overlay,
            config_mtime: config_mtime(),
        })
    }

    /// Перечитать конфиг, если файл изменился.
    fn maybe_reload(&mut self) -> Result<()> {
        if !self.cfg.behavior.hot_reload {
            return Ok(());
        }
        let Some(path) = Config::path() else {
            return Ok(());
        };
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        if mtime.is_none() || mtime == self.config_mtime {
            return Ok(());
        }
        self.config_mtime = mtime;

        match Config::load_from(&path) {
            Ok(mut cfg) => {
                let font = renderer::load_font(&cfg.appearance.font_path)?;
                let overlay = X11Overlay::new(cfg.appearance.width, cfg.appearance.height)?;
                apply_environment(&mut cfg, &overlay);
                self.cfg = cfg;
                self.font = font;
                self.overlay = overlay;
                log::info("конфигурация перезагружена");
            }
            Err(err) => log::info(&format!("конфигурация не применена: {err}")),
        }
        Ok(())
    }
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()?).ok()?.modified().ok()
}

/// Точка привязки бейджа: каретка → активное окно → курсор мыши.
/// Точка привязки бейджа. Цепочка зависит от режима:
/// * `auto`    — каретка из AT-SPI-событий → клик → мышь (без обхода дерева);
/// * `caret`   — запрос каретки (может быть дорогим) → мышь;
/// * `click`   — последний клик → мышь;
/// * `window`  — угол активного окна → мышь;
/// * `pointer` — всегда мышь.
fn resolve_anchor(
    runtime: &Runtime,
    a11y: &mut Option<a11y::A11y>,
    click: &click::ClickState,
    last_caret: Option<(i16, i16)>,
) -> Result<(i16, i16)> {
    use Positioning::*;

    let mode = runtime.cfg.positioning;

    if matches!(mode, Auto | Caret) {
        if let Some(position) = last_caret {
            log::debug(&format!("каретка (из события): {position:?}"));
            return Ok(position);
        }
        if mode == Caret {
            let worker = a11y.get_or_insert_with(a11y::A11y::spawn);
            if let Some(rect) = worker.caret_rect(Duration::from_millis(900)) {
                log::debug(&format!(
                    "каретка: ({}, {}) {}x{}",
                    rect.x, rect.y, rect.w, rect.h
                ));
                return Ok((clamp_i16(rect.x), clamp_i16(rect.y)));
            }
            log::debug("каретка недоступна — откат к курсору мыши");
        }
    }

    if matches!(mode, Auto | Click) {
        if let Some(position) = click.recent() {
            log::debug(&format!("точка клика: {position:?}"));
            return Ok(position);
        }
    }

    if mode == Window {
        if let Some((x, y, w, _h)) = runtime.overlay.active_window_rect() {
            return Ok((clamp_i16(x + w), clamp_i16(y)));
        }
        log::debug("активное окно неизвестно — откат к курсору мыши");
    }

    runtime.overlay.pointer_position()
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Проверка получения позиции каретки (`--caret-probe`).
fn caret_probe() -> Result<()> {
    // Диагностический режим — всегда подробно.
    log::init(true);
    let worker = a11y::A11y::spawn();
    match worker.caret_rect(Duration::from_secs(2)) {
        Some(rect) => println!("каретка: ({}, {}) {}x{}", rect.x, rect.y, rect.w, rect.h),
        None => println!(
            "каретка недоступна: a11y выключена, нет текстового поля \
             или приложение не перезапущено после включения a11y"
        ),
    }
    Ok(())
}

/// Общее состояние, нужное для показа бейджа.
struct BadgeContext<'a> {
    runtime: &'a Runtime,
    a11y: &'a mut Option<a11y::A11y>,
    tray: &'a tray::TrayState,
    codes: &'a [String],
    position: &'a mut (i16, i16),
    click: &'a click::ClickState,
    last_caret: Option<(i16, i16)>,
}

/// Показать бейдж с учётом подавления и пересчёта позиции.
fn show_badge(
    ctx: &mut BadgeContext<'_>,
    presenter: &mut Presenter,
    group: u8,
    anchor_override: Option<(i16, i16)>,
    reposition: bool,
) -> Result<()> {
    if ctx.tray.is_paused() {
        log::debug("показ отключён из трея");
        return Ok(());
    }
    if ctx.runtime.cfg.behavior.suppress_fullscreen
        && ctx.runtime.overlay.active_window_is_fullscreen()
    {
        log::debug("показ подавлен: активно полноэкранное окно");
        return Ok(());
    }
    // Позицию пересчитываем на первом кадре и при смене окна/поля —
    // иначе бейдж остаётся там, где было предыдущее окно.
    if presenter.is_hidden() || reposition {
        let monitors = screen::monitors();
        let anchor = match anchor_override {
            Some(anchor) => anchor,
            None => resolve_anchor(ctx.runtime, ctx.a11y, ctx.click, ctx.last_caret)?,
        };
        *ctx.position = position_for(&ctx.runtime.cfg, anchor, &ctx.runtime.overlay, &monitors);
    }
    let code = layout::code_for(group, ctx.codes);
    log::debug(&format!("показ: группа {group} -> {code}"));
    ctx.tray.set_layout(&code);
    presenter.show(code);
    Ok(())
}

/// Подстроить конфигурацию под окружение.
fn apply_environment(cfg: &mut Config, overlay: &X11Overlay) {
    // Без композитора альфа-канал не смешивается — рисуем непрозрачно.
    if !overlay.compositor_present() {
        log::debug("композитор не найден: прозрачность отключена");
        cfg.appearance.background_alpha = 1.0;
    }
}

/// События, приходящие в главный цикл.
enum Signal {
    /// Сменилась раскладка (новый индекс группы).
    Layout(u8),
    /// Сменилось активное окно (для `show_on_focus`).
    Focus,
    /// Текстовое поле получило фокус (AT-SPI); внутри — позиция каретки.
    FieldFocus(a11y::Rect),
}

/// Демон: следит за раскладкой и показывает бейдж.
fn run() -> Result<()> {
    let mut runtime = Runtime::new()?;
    let mut codes = backend::layout_codes();
    log::info(&format!("раскладки: {codes:?}"));

    // Текущая группа нужна и для показа по фокусу окна.
    let mut last_group = backend::open(BackendKind::Auto)
        .and_then(|backend| backend.current_group())
        .unwrap_or(0);

    let polling_ms = runtime.cfg.backend.polling_ms;
    let backend_kind = runtime.cfg.backend.kind;
    let (tx, rx) = mpsc::channel::<Result<Signal>>();

    // События смены раскладки.
    let layout_tx = tx.clone();
    thread::spawn(move || backend_thread(layout_tx, backend_kind, polling_ms));

    // Смена активного окна — только для показа по фокусу.
    let focus_tx = tx.clone();
    thread::spawn(move || {
        let (inner_tx, inner_rx) = mpsc::channel();
        thread::spawn(move || {
            if let Err(err) = focus::watch(inner_tx) {
                log::debug(&format!("слежение за фокусом остановлено: {err}"));
            }
        });
        while inner_rx.recv().is_ok() {
            if focus_tx.send(Ok(Signal::Focus)).is_err() {
                break;
            }
        }
    });
    // Фокус элемента через AT-SPI — ловит клик по полю в уже активном окне.
    // Поднимаем всегда: при hot reload опции show_on_focus обработчик сам
    // решит, реагировать или нет, а повторно поток уже не создать.
    {
        let field_tx = tx.clone();
        let deep_search = runtime.cfg.behavior.deep_search;
        thread::spawn(move || {
            let (inner_tx, inner_rx) = mpsc::channel();
            a11y::spawn_focus_listener(inner_tx, deep_search);
            while let Ok(rect) = inner_rx.recv() {
                if field_tx.send(Ok(Signal::FieldFocus(rect))).is_err() {
                    break;
                }
            }
        });
    }

    drop(tx);
    let mut presenter = Presenter::new(&runtime.cfg);
    let mut position = (0i16, 0i16);
    // AT-SPI-воркер создаётся только если он реально нужен.
    let mut a11y: Option<a11y::A11y> = None;
    // Точка последнего клика — «простой» движок позиционирования.
    let click_state = click::ClickState::new();
    {
        let state = std::sync::Arc::clone(&click_state);
        thread::spawn(move || {
            if let Err(err) = click::watch(state) {
                log::debug(&format!("слежение за кликами остановлено: {err}"));
            }
        });
    }
    // Последняя известная каретка (из AT-SPI-событий) — для режима auto.
    let mut last_caret: Option<(i16, i16)> = None;
    // Трей необязателен: без StatusNotifierWatcher демон работает как обычно.
    let tray_state = tray::TrayState::new();
    let _tray = tray::spawn(&tray_state);
    let mut layouts_refreshed = Instant::now();
    let mut strict_warned = false;

    loop {
        match rx.recv_timeout(presenter.frame_delay()) {
            Ok(Ok(Signal::Layout(group))) => {
                last_group = group;
                if layouts_refreshed.elapsed() > LAYOUTS_REFRESH {
                    codes = layout::layout_codes();
                    layouts_refreshed = Instant::now();
                }
                let mut ctx = BadgeContext {
                    runtime: &runtime,
                    a11y: &mut a11y,
                    tray: &tray_state,
                    codes: &codes,
                    position: &mut position,
                    click: click_state.as_ref(),
                    last_caret,
                };
                show_badge(&mut ctx, &mut presenter, group, None, false)?;
            }
            Ok(Ok(Signal::Focus)) => {
                if runtime.cfg.behavior.show_on_focus {
                    let allowed = if runtime.cfg.behavior.focus_requires_text_field {
                        let worker = a11y.get_or_insert_with(a11y::A11y::spawn);
                        worker.caret_rect(Duration::from_millis(900)).is_some()
                    } else {
                        true
                    };

                    if allowed {
                        let mut ctx = BadgeContext {
                            runtime: &runtime,
                            a11y: &mut a11y,
                            tray: &tray_state,
                            codes: &codes,
                            position: &mut position,
                            click: click_state.as_ref(),
                            last_caret,
                        };
                        show_badge(&mut ctx, &mut presenter, last_group, None, true)?;
                    } else {
                        log::debug("фокус: текстового поля нет, показ пропущен");
                        if !strict_warned {
                            strict_warned = true;
                            log::info(
                                "show_on_focus: текстовое поле не определяется. \
                                 Скорее всего выключена accessibility — \
                                 см. README, раздел «Позиция каретки»",
                            );
                        }
                    }
                }
            }
            Ok(Ok(Signal::FieldFocus(rect))) => {
                if runtime.cfg.behavior.show_on_focus {
                    let caret = (clamp_i16(rect.x), clamp_i16(rect.y));
                    last_caret = Some(caret);
                    let anchor = if matches!(
                        runtime.cfg.positioning,
                        Positioning::Caret | Positioning::Auto
                    ) {
                        Some(caret)
                    } else {
                        None
                    };
                    log::debug(&format!("фокус поля: каретка ({}, {})", rect.x, rect.y));

                    let mut ctx = BadgeContext {
                        runtime: &runtime,
                        a11y: &mut a11y,
                        tray: &tray_state,
                        codes: &codes,
                        position: &mut position,
                        click: click_state.as_ref(),
                        last_caret,
                    };
                    show_badge(&mut ctx, &mut presenter, last_group, anchor, true)?;
                }
            }
            Ok(Err(err)) => return Err(err),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                runtime.maybe_reload()?;
            }
        }

        if tray_state.should_quit() {
            log::info("выход по команде из трея");
            break;
        }

        match presenter.tick() {
            Some(opacity) => {
                let layers = presenter.layers();
                let refs: Vec<Layer<'_>> = layers.iter().map(|(t, a)| Layer::new(t, *a)).collect();
                let pixmap = render_badge(&runtime.cfg.appearance, &runtime.font, &refs, opacity)
                    .context("не удалось отрисовать бейдж")?;
                runtime.overlay.draw_at(&pixmap, position.0, position.1)?;
            }
            None => runtime.overlay.hide()?,
        }
    }

    Ok(())
}

fn backend_thread(tx: mpsc::Sender<Result<Signal>>, kind: BackendKind, polling_ms: Option<u64>) {
    let mut backend = match backend::open(kind) {
        Ok(backend) => backend,
        Err(err) => {
            let _ = tx.send(Err(err));
            return;
        }
    };

    // Резервный режим: опрос вместо событий (экзотические X-серверы).
    if let Some(ms) = polling_ms {
        log::debug(&format!("режим опроса каждые {ms} мс"));
        let mut last = backend.current_group().ok();
        loop {
            thread::sleep(Duration::from_millis(ms));
            match backend.current_group() {
                Ok(group) => {
                    if Some(group) != last {
                        last = Some(group);
                        if tx.send(Ok(Signal::Layout(group))).is_err() {
                            break;
                        }
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                    break;
                }
            }
        }
        return;
    }

    loop {
        match backend.next_change() {
            Ok(group) => {
                if tx.send(Ok(Signal::Layout(group))).is_err() {
                    break;
                }
            }
            Err(err) => {
                let _ = tx.send(Err(err));
                break;
            }
        }
    }
}

/// Однократный показ бейджа — для проверки внешнего вида.
fn demo(text: &str) -> Result<()> {
    let cfg = Config::load()?;
    let font = renderer::load_font(&cfg.appearance.font_path)?;
    let mut overlay = X11Overlay::new(cfg.appearance.width, cfg.appearance.height)?;

    let anchor = overlay.pointer_position()?;
    let position = position_for(&cfg, anchor, &overlay, &screen::monitors());
    println!(
        "окно 0x{:x} в ({}, {})",
        overlay.window_id(),
        position.0,
        position.1
    );

    let mut presenter = Presenter::new(&cfg);
    presenter.show(text.to_string());

    loop {
        match presenter.tick() {
            Some(opacity) => {
                let layers = presenter.layers();
                let refs: Vec<Layer<'_>> = layers.iter().map(|(t, a)| Layer::new(t, *a)).collect();
                let pixmap = render_badge(&cfg.appearance, &font, &refs, opacity)
                    .context("не удалось отрисовать бейдж")?;
                overlay.draw_at(&pixmap, position.0, position.1)?;
                thread::sleep(presenter.frame_delay());
            }
            None => {
                overlay.hide()?;
                return Ok(());
            }
        }
    }
}

/// Отладочный режим: печатает каждое переключение раскладки.
fn watch() -> Result<()> {
    let mut backend = backend::open(BackendKind::Auto)?;
    let codes = layout::layout_codes();

    println!("раскладки: {:?}", codes);
    let group = backend.current_group()?;
    println!(
        "старт: группа {} ({})",
        group,
        layout::code_for(group, &codes)
    );

    loop {
        let group = backend.next_change()?;
        println!("-> группа {} ({})", group, layout::code_for(group, &codes));
    }
}

/// Позиция бейджа относительно точки привязки с учётом границ монитора.
fn position_for(
    cfg: &Config,
    anchor: (i16, i16),
    overlay: &X11Overlay,
    monitors: &[Monitor],
) -> (i16, i16) {
    let w = cfg.appearance.width as i32;
    let h = cfg.appearance.height as i32;
    let (px, py) = (i32::from(anchor.0), i32::from(anchor.1));

    // Границы: монитор под точкой привязки, иначе весь корневой экран.
    let bounds = screen::monitor_at(monitors, px, py)
        .map(|m| (m.x, m.y, m.right(), m.bottom()))
        .unwrap_or_else(|| {
            let (sw, sh) = overlay.root_size();
            (0, 0, i32::from(sw), i32::from(sh))
        });
    let (left, top, right, bottom) = bounds;

    let mut x = px + i32::from(cfg.offset.dx);
    let mut y = py + i32::from(cfg.offset.dy);

    // Не влезает справа — показываем слева.
    if x + w > right {
        x = px - w - i32::from(cfg.offset.dx);
    }
    // Не влезает сверху — показываем снизу.
    if y < top {
        y = py + 24;
    }

    x = x.clamp(left, (right - w).max(left));
    y = y.clamp(top, (bottom - h).max(top));
    (x as i16, y as i16)
}

/// Диагностика окружения.
fn doctor() -> Result<()> {
    println!(
        "layout-indicator {} — диагностика",
        env!("CARGO_PKG_VERSION")
    );
    for key in [
        "DISPLAY",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "KDE_SESSION_VERSION",
    ] {
        println!(
            "{key}: {}",
            std::env::var(key).unwrap_or_else(|_| "—".into())
        );
    }

    if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
        println!("ВНИМАНИЕ: сессия Wayland — поддержка не реализована (этап M3)");
    }

    let cfg = Config::load()?;
    match Config::path() {
        Some(path) if path.exists() => println!("конфиг: {} (найден)", path.display()),
        Some(path) => println!("конфиг: {} (нет, используются дефолты)", path.display()),
        None => println!("конфиг: не удалось определить каталог"),
    }
    println!(
        "шрифт: {} ({})",
        cfg.appearance.font_path.display(),
        if cfg.appearance.font_path.exists() {
            "найден"
        } else {
            "НЕ НАЙДЕН"
        }
    );

    let monitors = screen::monitors();
    println!("мониторы: {}", monitors.len());
    for m in &monitors {
        println!("  {}x{}+{}+{}", m.w, m.h, m.x, m.y);
    }

    let codes = layout::layout_codes();
    println!("раскладки: {codes:?}");

    match X11Overlay::new(cfg.appearance.width, cfg.appearance.height) {
        Ok(overlay) => {
            let (w, h) = overlay.root_size();
            println!("X11: ок, корневое окно {w}x{h}");
            println!(
                "композитор: {}",
                if overlay.compositor_present() {
                    "есть"
                } else {
                    "НЕТ (прозрачность не будет работать)"
                }
            );
            println!(
                "активное окно fullscreen: {}",
                overlay.active_window_is_fullscreen()
            );
        }
        Err(err) => println!("X11: ОШИБКА — {err}"),
    }

    match backend::open(BackendKind::Auto) {
        Ok(backend) => match backend.current_group() {
            Ok(group) => println!("XKB: ок, текущая группа {group}"),
            Err(err) => println!("XKB: ошибка чтения группы — {err}"),
        },
        Err(err) => println!("XKB: ОШИБКА — {err}"),
    }

    println!(
        "тема: {:?} (режим {:?})",
        theme::resolve(cfg.appearance.theme),
        cfg.appearance.theme
    );
    println!("позиционирование: {:?}", cfg.positioning);

    let worker = a11y::A11y::spawn();
    match worker.caret_rect(Duration::from_millis(900)) {
        Some(rect) => println!(
            "каретка (AT-SPI): ({}, {}) {}x{}",
            rect.x, rect.y, rect.w, rect.h
        ),
        None => println!("каретка (AT-SPI): недоступна"),
    }

    Ok(())
}

/// Поставить иконки трея в hicolor-тему текущего пользователя.
fn install_icons() -> Result<()> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("не удалось определить каталог данных")?
        .join("icons/hicolor");

    for (size, png) in tray::ICONS {
        let dir = base.join(format!("{size}x{size}/apps"));
        std::fs::create_dir_all(&dir).with_context(|| format!("создание {}", dir.display()))?;
        std::fs::write(dir.join("layout-indicator.png"), png)
            .with_context(|| format!("запись иконки {size}px"))?;
    }

    println!("иконки установлены в {}", base.display());
    Ok(())
}

/// Записать `.desktop` в `~/.config/autostart` и поставить иконки трея.
fn install_autostart() -> Result<()> {
    install_icons()?;
    let exe = std::env::current_exe().context("не удалось определить путь к бинарнику")?;
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("не удалось определить каталог конфигурации")?
        .join("autostart");

    std::fs::create_dir_all(&dir).context("создание каталога autostart")?;
    let path = dir.join("layout-indicator.desktop");
    let body = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Layout Indicator\n\
         Comment=Индикатор раскладки клавиатуры\n\
         Exec={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    std::fs::write(&path, body).context("запись .desktop")?;
    println!("создан {}", path.display());
    Ok(())
}

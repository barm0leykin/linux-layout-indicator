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

mod backend;
mod config;
mod instance;
mod layout;
mod log;
mod presenter;
mod renderer;
mod screen;
mod theme;

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use backend::Backend;
use backend::x11::X11Backend;
use config::Config;
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

    if args.iter().any(|a| a == "--watch") {
        return watch();
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
    println!("  --doctor             диагностика окружения");
    println!("  --print-config       напечатать YAML с настройками по умолчанию");
    println!("  --install-autostart  добавить автозапуск в ~/.config/autostart");
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

/// Подстроить конфигурацию под окружение.
fn apply_environment(cfg: &mut Config, overlay: &X11Overlay) {
    // Без композитора альфа-канал не смешивается — рисуем непрозрачно.
    if !overlay.compositor_present() {
        log::debug("композитор не найден: прозрачность отключена");
        cfg.appearance.background_alpha = 1.0;
    }
}

/// Демон: следит за раскладкой и показывает бейдж.
fn run() -> Result<()> {
    let mut runtime = Runtime::new()?;
    let mut codes = layout::layout_codes();
    log::info(&format!("раскладки: {codes:?}"));

    let polling_ms = runtime.cfg.backend.polling_ms;
    let (tx, rx) = mpsc::channel::<Result<u8>>();
    thread::spawn(move || backend_thread(tx, polling_ms));

    let mut presenter = Presenter::new(&runtime.cfg);
    let mut position = (0i16, 0i16);
    let mut layouts_refreshed = Instant::now();

    loop {
        match rx.recv_timeout(presenter.frame_delay()) {
            Ok(Ok(group)) => {
                if layouts_refreshed.elapsed() > LAYOUTS_REFRESH {
                    codes = layout::layout_codes();
                    layouts_refreshed = Instant::now();
                }

                let suppressed = runtime.cfg.behavior.suppress_fullscreen
                    && runtime.overlay.active_window_is_fullscreen();
                if suppressed {
                    log::debug("показ подавлен: активно полноэкранное окно");
                } else {
                    if presenter.is_hidden() {
                        // Геометрию перечитываем только перед показом.
                        let monitors = screen::monitors();
                        let anchor = runtime.overlay.pointer_position()?;
                        position = position_for(&runtime.cfg, anchor, &runtime.overlay, &monitors);
                    }
                    let code = layout::code_for(group, &codes);
                    log::debug(&format!("раскладка {group} -> {code}"));
                    presenter.show(code);
                }
            }
            Ok(Err(err)) => return Err(err),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                runtime.maybe_reload()?;
            }
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

fn backend_thread(tx: mpsc::Sender<Result<u8>>, polling_ms: Option<u64>) {
    let mut backend = match X11Backend::new() {
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
                        if tx.send(Ok(group)).is_err() {
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
                if tx.send(Ok(group)).is_err() {
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
    let mut backend = X11Backend::new()?;
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

    match X11Backend::new() {
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

    Ok(())
}

/// Записать `.desktop` в `~/.config/autostart`.
fn install_autostart() -> Result<()> {
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

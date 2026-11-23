//! layout-indicator — индикатор текущей раскладки клавиатуры рядом с точкой ввода.
//!
//! Слои (см. PRD §7.1):
//!   * [`backend`]   — откуда узнать про смену раскладки (платформенно-зависимый);
//!   * [`presenter`] — политика показа (тайминги, подавление, дедупликация);
//!   * [`renderer`]  — как нарисовать поверх других окон, не мешая им;
//!   * [`config`]    — пользовательская конфигурация (YAML);
//!   * [`layout`]    — индекс группы XKB → код раскладки (`0 → EN`).

// Часть конфигурации относится к ещё не реализованным режимам (caret/window, show_on_focus).
#![allow(dead_code)]

mod backend;
mod config;
mod layout;
mod presenter;
mod renderer;

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;

use anyhow::{Context, Result};

use backend::Backend;
use backend::x11::X11Backend;
use config::Config;
use presenter::Presenter;
use renderer::render_badge;
use renderer::x11::X11Overlay;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--print-config") {
        print!("{}", Config::default_yaml());
        return Ok(());
    }
    if args.iter().any(|a| a == "--watch") {
        return watch();
    }
    if let Some(text) = arg_value(&args, "--demo") {
        return demo(&text);
    }

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
    println!("  (без опций)        запустить демон");
    println!("  --watch            печатать смену раскладки (отладка)");
    println!("  --demo <TEXT>      показать бейдж один раз, например --demo EN");
    println!("  --print-config     напечатать YAML с настройками по умолчанию");
    println!("  --help, -h         эта справка");
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

/// Демон: следит за раскладкой и показывает бейдж.
fn run() -> Result<()> {
    let cfg = Config::load()?;
    let font = renderer::load_font(&cfg.appearance.font_path)?;
    let mut overlay = X11Overlay::new(cfg.appearance.width, cfg.appearance.height)?;
    let codes = layout::layout_codes();

    // Бэкенд — в отдельном потоке: ожидание X-событий блокирующее,
    // анимация же требует регулярных тиков.
    let (tx, rx) = mpsc::channel::<Result<u8>>();
    thread::spawn(move || {
        let mut backend = match X11Backend::new() {
            Ok(backend) => backend,
            Err(err) => {
                let _ = tx.send(Err(err));
                return;
            }
        };
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
    });

    let mut presenter = Presenter::new(&cfg);
    let mut position = (0i16, 0i16);

    loop {
        match rx.recv_timeout(presenter.frame_delay()) {
            Ok(Ok(group)) => {
                let suppressed =
                    cfg.behavior.suppress_fullscreen && overlay.active_window_is_fullscreen();
                if !suppressed {
                    if presenter.is_hidden() {
                        let (px, py) = overlay.pointer_position()?;
                        position = position_for(&cfg, px, py, &overlay);
                    }
                    presenter.show(layout::code_for(group, &codes));
                }
            }
            Ok(Err(err)) => return Err(err),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        match presenter.tick() {
            Some(opacity) => {
                let pixmap = render_badge(&cfg.appearance, &font, presenter.text(), opacity)
                    .context("не удалось отрисовать бейдж")?;
                overlay.draw_at(&pixmap, position.0, position.1)?;
            }
            None => overlay.hide()?,
        }
    }

    Ok(())
}

/// Однократный показ бейджа — для проверки внешнего вида.
fn demo(text: &str) -> Result<()> {
    let cfg = Config::load()?;
    let font = renderer::load_font(&cfg.appearance.font_path)?;
    let mut overlay = X11Overlay::new(cfg.appearance.width, cfg.appearance.height)?;

    let (px, py) = overlay.pointer_position()?;
    let position = position_for(&cfg, px, py, &overlay);
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
                let pixmap = render_badge(&cfg.appearance, &font, presenter.text(), opacity)
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

/// Позиция бейджа относительно точки привязки с учётом краёв экрана.
fn position_for(cfg: &Config, px: i16, py: i16, overlay: &X11Overlay) -> (i16, i16) {
    let w = cfg.appearance.width as i32;
    let h = cfg.appearance.height as i32;
    let (sw, sh) = overlay.root_size();
    let (sw, sh) = (sw as i32, sh as i32);

    let mut x = px as i32 + cfg.offset.dx as i32;
    let mut y = py as i32 + cfg.offset.dy as i32;

    // Не влезает справа — показываем слева.
    if x + w > sw {
        x = px as i32 - w - cfg.offset.dx as i32;
    }
    // Не влезает сверху — показываем снизу.
    if y < 0 {
        y = py as i32 + 24;
    }

    x = x.clamp(0, (sw - w).max(0));
    y = y.clamp(0, (sh - h).max(0));
    (x as i16, y as i16)
}

//! Иконка в системном трее (StatusNotifierItem).
//!
//! Даёт минимум управления: пауза/включение, открыть конфиг, выход.
//! Состояние разделяется с главным циклом через [`TrayState`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Category, Icon, Status, ToolTip, Tray};
use tiny_skia::Pixmap;

use crate::config::Config;

/// Иконки трея по размерам: чёрный квадрат с белым глифом (из `img/ico.png`).
pub const ICONS: [(u32, &[u8]); 4] = [
    (22, include_bytes!("../assets/icon-22.png")),
    (24, include_bytes!("../assets/icon-24.png")),
    (32, include_bytes!("../assets/icon-32.png")),
    (48, include_bytes!("../assets/icon-48.png")),
];

/// PNG → SNI `Icon` (ARGB32, сетевой порядок байт, без premultiply).
fn decode_icon(png: &[u8]) -> Option<Icon> {
    let pixmap = Pixmap::decode_png(png).ok()?;
    let (width, height) = (pixmap.width(), pixmap.height());

    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for px in pixmap.data().chunks_exact(4) {
        // tiny-skia отдаёт premultiplied RGBA — снимаем premultiply.
        let alpha = px[3];
        let (r, g, b) = if alpha == 0 {
            (0, 0, 0)
        } else {
            let un = |c: u8| {
                ((u16::from(c) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
            };
            (un(px[0]), un(px[1]), un(px[2]))
        };
        data.extend_from_slice(&[alpha, r, g, b]);
    }

    Some(Icon {
        width: width as i32,
        height: height as i32,
        data,
    })
}

/// Набор иконок трея.
fn icons() -> Vec<Icon> {
    ICONS
        .iter()
        .filter_map(|(_, png)| decode_icon(png))
        .collect()
}

/// Состояние, которое трей и демон делят между собой.
pub struct TrayState {
    paused: AtomicBool,
    quit: AtomicBool,
    layout: Mutex<String>,
    version: AtomicU64,
}

impl TrayState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            paused: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            layout: Mutex::new(String::new()),
            version: AtomicU64::new(0),
        })
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn should_quit(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    /// Обновить текущую раскладку (для тултипа).
    pub fn set_layout(&self, code: &str) {
        if let Ok(mut guard) = self.layout.lock() {
            if guard.as_str() == code {
                return;
            }
            *guard = code.to_string();
        }
        self.bump();
    }

    fn layout_text(&self) -> String {
        self.layout
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn toggle_pause(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
        self.bump();
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            paused: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            layout: Mutex::new(String::new()),
            version: AtomicU64::new(0),
        }
    }
}

pub struct LayoutTray {
    state: Arc<TrayState>,
}

impl Tray for LayoutTray {
    fn id(&self) -> String {
        "layout-indicator".into()
    }

    fn title(&self) -> String {
        "Layout Indicator".into()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    // Имя из темы иконок: Plasma 5.27 не показывает IconPixmap без IconName.
    // Файлы ставятся командой `--install-icons` в hicolor.
    fn icon_name(&self) -> String {
        "layout-indicator".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icons()
    }

    // Всегда Active: Passive в некоторых оболочках скрывает иконку.
    fn status(&self) -> Status {
        Status::Active
    }

    fn tool_tip(&self) -> ToolTip {
        let layout = self.state.layout_text();
        let description = if self.state.is_paused() {
            "Пауза — индикатор не показывается".to_string()
        } else if layout.is_empty() {
            "Следит за раскладкой".to_string()
        } else {
            format!("Текущая раскладка: {layout}")
        };
        ToolTip {
            icon_name: "layout-indicator".into(),
            icon_pixmap: icons(),
            title: "Layout Indicator".into(),
            description,
        }
    }

    // Клик по иконке — пауза/включение.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.state.toggle_pause();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let toggle_label = if self.state.is_paused() {
            "Включить индикатор"
        } else {
            "Приостановить индикатор"
        };

        vec![
            StandardItem {
                label: toggle_label.into(),
                activate: Box::new(|this: &mut Self| this.state.toggle_pause()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Открыть конфиг".into(),
                activate: Box::new(|_: &mut Self| open_config()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Выход".into(),
                activate: Box::new(|this: &mut Self| {
                    this.state.quit.store(true, Ordering::Relaxed);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Открыть файл конфигурации; если его нет — создать с дефолтами.
fn open_config() {
    let Some(path) = Config::path() else {
        return;
    };
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, Config::default_yaml());
    }
    // stderr/stdout дочернего процесса не нужны: KDE-приложения, открывающие
    // файл, сыплют своими предупреждениями в наш лог.
    let _ = std::process::Command::new("xdg-open")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Запустить трей. `None` — если StatusNotifierWatcher недоступен
/// (не KDE/GNOME-оболочка, нет трея) — демон продолжает работать без иконки.
pub fn spawn(state: &Arc<TrayState>) -> Option<Handle<LayoutTray>> {
    let tray = LayoutTray {
        state: Arc::clone(state),
    };

    let handle = match tray.spawn() {
        Ok(handle) => handle,
        Err(err) => {
            crate::log::debug(&format!("трей недоступен: {err}"));
            return None;
        }
    };

    // ksni не замечает изменений в общем состоянии сам — периодически
    // просим его перечитать свойства, если версия изменилась.
    let refresh = handle.clone();
    let shared = Arc::clone(state);
    thread::spawn(move || {
        let mut seen = shared.version();
        while !refresh.is_closed() {
            thread::sleep(Duration::from_millis(250));
            let version = shared.version();
            if version != seen {
                seen = version;
                let _ = refresh.update(|_| {});
            }
            if shared.should_quit() {
                break;
            }
        }
    });

    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_toggles() {
        let state = TrayState::default();
        assert!(!state.is_paused());
        state.toggle_pause();
        assert!(state.is_paused());
        state.toggle_pause();
        assert!(!state.is_paused());
    }

    #[test]
    fn layout_update_bumps_version() {
        let state = TrayState::default();
        let v0 = state.version();
        state.set_layout("RU");
        assert!(state.version() > v0);
        assert_eq!(state.layout_text(), "RU");
        // Повтор того же значения не должен дёргать обновление.
        let v1 = state.version();
        state.set_layout("RU");
        assert_eq!(state.version(), v1);
    }
}

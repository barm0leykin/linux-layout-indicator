//! Пользовательская конфигурация: `~/.config/layout-indicator/config.yaml`.
//!
//! Любой ключ можно опустить — недостающее берётся из [`Config::default`].
//! Полный список значений с комментариями: `layout-indicator --print-config`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Режим позиционирования индикатора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Positioning {
    /// Автоматически: каретка из AT-SPI-событий → точка клика → курсор мыши.
    #[default]
    Auto,
    /// У курсора мыши — работает всегда (T0).
    Pointer,
    /// В точке последнего левого клика (XInput2, без accessibility).
    Click,
    /// У текстовой каретки (T2). Требует включённой accessibility.
    Caret,
    /// У активного окна (T1).
    Window,
}

/// Как выбирать палитру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Следовать системной теме (KDE `kdeglobals`), иначе тёмная.
    #[default]
    Auto,
    Dark,
    Light,
}

/// Цвет в формате `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for Color {
    fn default() -> Self {
        Self::white()
    }
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn white() -> Self {
        Self::new(255, 255, 255)
    }
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
        if hex.len() != 6 {
            return Err(format!("ожидался цвет вида #rrggbb, получено {value:?}"));
        }
        let parse = |s: &str| u8::from_str_radix(s, 16).map_err(|e| format!("{value:?}: {e}"));
        Ok(Self {
            r: parse(&hex[0..2])?,
            g: parse(&hex[2..4])?,
            b: parse(&hex[4..6])?,
        })
    }
}

impl From<Color> for String {
    fn from(c: Color) -> Self {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    }
}

/// Внешний вид бейджа.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Ширина окна, px.
    pub width: u32,
    /// Высота окна, px.
    pub height: u32,
    /// Радиус скругления углов, px.
    pub corner_radius: f32,
    /// Путь к TTF/OTF-шрифту.
    pub font_path: PathBuf,
    /// Размер шрифта, px.
    pub font_size: f32,
    /// Палитра: auto/dark/light. Цвета ниже переопределяют её.
    pub theme: ThemeMode,
    /// Цвет текста; отсутствует — из палитры темы.
    pub text_color: Option<Color>,
    /// Цвет фона; отсутствует — из палитры темы.
    pub background_color: Option<Color>,
    /// Цвет рамки; отсутствует — из палитры темы.
    pub border_color: Option<Color>,
    /// Толщина рамки в px (0 — без рамки).
    pub border_width: f32,
    /// Непрозрачность фона, 0.0–1.0.
    pub background_alpha: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            width: 88,
            height: 44,
            corner_radius: 8.0,
            font_path: PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
            font_size: 20.0,
            theme: ThemeMode::Auto,
            text_color: None,
            background_color: None,
            border_color: None,
            border_width: 1.0,
            background_alpha: 0.88,
        }
    }
}

impl Appearance {
    /// Итоговые цвета: явные из конфига либо из палитры темы.
    pub fn resolved_colors(&self) -> (Color, Color) {
        let (bg, fg, _) = crate::theme::palette(crate::theme::resolve(self.theme));
        (
            self.background_color.unwrap_or(bg),
            self.text_color.unwrap_or(fg),
        )
    }

    /// Итоговая рамка: (цвет, толщина).
    pub fn resolved_border(&self) -> (Color, f32) {
        let (_, _, border) = crate::theme::palette(crate::theme::resolve(self.theme));
        (
            self.border_color.unwrap_or(border),
            self.border_width.max(0.0),
        )
    }
}

/// Тайминги показа.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Timing {
    pub fade_in_ms: u64,
    pub hold_ms: u64,
    pub fade_out_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            fade_in_ms: 120,
            hold_ms: 700,
            fade_out_ms: 200,
        }
    }
}

impl Timing {
    pub fn total_ms(&self) -> u64 {
        self.fade_in_ms + self.hold_ms + self.fade_out_ms
    }
}

/// Смещение бейджа относительно точки привязки.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Offset {
    pub dx: i16,
    pub dy: i16,
}

impl Default for Offset {
    fn default() -> Self {
        Self { dx: 14, dy: -24 }
    }
}

/// Поведение.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Behavior {
    /// Не показывать поверх полноэкранных окон.
    pub suppress_fullscreen: bool,
    /// Показывать раскладку при переходе в другое окно.
    pub show_on_focus: bool,
    /// Требовать, чтобы в окне было текстовое поле (нужна accessibility).
    /// По умолчанию выключено: без a11y проверка всегда даёт «поля нет»,
    /// и показ по фокусу не работает вовсе.
    pub focus_requires_text_field: bool,
    /// Искать каретку полным обходом дерева доступности, если быстрый путь
    /// не дал результата. Нужно для Qt-приложений; первый раз может быть
    /// медленным (секунды), дальше объект кэшируется по приложению.
    pub deep_search: bool,
    /// Перечитывать конфиг при изменении файла.
    pub hot_reload: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            suppress_fullscreen: true,
            show_on_focus: false,
            focus_requires_text_field: false,
            deep_search: true,
            hot_reload: true,
        }
    }
}

/// Настройки источника событий.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Backend {
    /// Откуда брать раскладку: `auto` — KDE D-Bus, иначе XKB.
    pub kind: crate::backend::BackendKind,
    /// Опрашивать состояние каждые N мс вместо ожидания XKB-событий.
    /// Нужно на экзотических X-серверах, где события не приходят.
    pub polling_ms: Option<u64>,
}

/// Полная конфигурация.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub positioning: Positioning,
    pub appearance: Appearance,
    pub timing: Timing,
    pub offset: Offset,
    pub behavior: Behavior,
    pub backend: Backend,
}

impl Config {
    /// Путь к файлу конфигурации.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("layout-indicator").join("config.yaml"))
    }

    /// Загрузить конфигурацию. Нет файла — используются дефолты.
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from(&path)
    }

    /// Загрузить из конкретного файла (используется и в hot reload).
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("чтение {}", path.display()))?;
        let cfg: Self = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("разбор YAML {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Проверка значений.
    pub fn validate(&self) -> Result<()> {
        let a = &self.appearance;
        if a.width == 0 || a.width > 1024 {
            bail!("appearance.width должен быть в диапазоне 1..=1024");
        }
        if a.height == 0 || a.height > 1024 {
            bail!("appearance.height должен быть в диапазоне 1..=1024");
        }
        if !(0.0..=1.0).contains(&a.background_alpha) {
            bail!("appearance.background_alpha должен быть в диапазоне 0.0..=1.0");
        }
        if !(0.0..=16.0).contains(&a.border_width) {
            bail!("appearance.border_width должен быть в диапазоне 0..=16");
        }
        if a.font_size <= 0.0 || a.font_size > 512.0 {
            bail!("appearance.font_size должен быть в диапазоне (0, 512]");
        }
        if !a.font_path.exists() {
            bail!(
                "шрифт не найден: {} (укажите appearance.font_path)",
                a.font_path.display()
            );
        }
        if self.timing.hold_ms == 0 && self.timing.fade_in_ms + self.timing.fade_out_ms == 0 {
            bail!("timing: индикатор не будет виден ни одного кадра");
        }
        if let Some(ms) = self.backend.polling_ms {
            if ms < 20 {
                bail!("backend.polling_ms должен быть >= 20");
            }
        }
        Ok(())
    }

    /// YAML с дефолтными значениями (для `--print-config`).
    pub fn default_yaml() -> String {
        serde_yaml_ng::to_string(&Self::default())
            .unwrap_or_else(|_| String::from("# не удалось сериализовать дефолты"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_color() {
        let c = Color::try_from("#1b2c3d".to_string()).unwrap();
        assert_eq!(c, Color::new(0x1b, 0x2c, 0x3d));
    }

    #[test]
    fn rejects_bad_color() {
        assert!(Color::try_from("zzz".to_string()).is_err());
        assert!(Color::try_from("#12345".to_string()).is_err());
    }

    #[test]
    fn partial_yaml_uses_defaults() {
        let cfg: Config = serde_yaml_ng::from_str("timing:\n  hold_ms: 1500\n").unwrap();
        assert_eq!(cfg.timing.hold_ms, 1500);
        assert_eq!(cfg.timing.fade_in_ms, 120);
        assert_eq!(cfg.appearance.height, 44);
        assert!(cfg.behavior.hot_reload);
    }

    #[test]
    fn rejects_zero_size() {
        let mut cfg = Config::default();
        cfg.appearance.width = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_too_fast_polling() {
        let mut cfg = Config::default();
        cfg.backend.polling_ms = Some(1);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn explicit_colors_override_theme() {
        let mut cfg = Config::default();
        cfg.appearance.background_color = Some(Color::new(1, 2, 3));
        cfg.appearance.text_color = Some(Color::new(4, 5, 6));
        assert_eq!(
            cfg.appearance.resolved_colors(),
            (Color::new(1, 2, 3), Color::new(4, 5, 6))
        );
    }
}

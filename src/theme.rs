//! Определение системной темы (тёмная/светлая) для режима `theme: auto`.
//!
//! Пока поддерживается KDE (`~/.config/kdeglobals`). Если определить не удалось,
//! используется тёмная палитра — она читается на любом фоне.

use std::path::PathBuf;

use crate::config::{Color, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

/// Цвета бейджа по умолчанию для темы.
pub fn palette(theme: Theme) -> (Color, Color) {
    match theme {
        Theme::Dark => (Color::new(0x1b, 0x1c, 0x1d), Color::new(0xff, 0xff, 0xff)),
        Theme::Light => (Color::new(0xfc, 0xfc, 0xfc), Color::new(0x1b, 0x1c, 0x1d)),
    }
}

/// Тема с учётом режима из конфига.
pub fn resolve(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::Dark => Theme::Dark,
        ThemeMode::Light => Theme::Light,
        ThemeMode::Auto => detect().unwrap_or(Theme::Dark),
    }
}

/// Системная тема, если её удалось определить.
pub fn detect() -> Option<Theme> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;

    // KDE: [General] ColorScheme=... или [Colors:Window] BackgroundNormal=...
    if let Ok(text) = std::fs::read_to_string(base.join("kdeglobals")) {
        if let Some(theme) = parse_kdeglobals(&text) {
            return Some(theme);
        }
    }
    None
}

fn parse_kdeglobals(text: &str) -> Option<Theme> {
    let mut section = String::new();
    let mut scheme: Option<String> = None;
    let mut window_bg: Option<Color> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (section.as_str(), key.trim()) {
            ("General", "ColorScheme") => scheme = Some(value.trim().to_string()),
            ("Colors:Window", "BackgroundNormal") => {
                window_bg = parse_rgb_triplet(value.trim());
            }
            _ => {}
        }
    }

    if let Some(name) = scheme {
        let lower = name.to_ascii_lowercase();
        if lower.contains("dark") {
            return Some(Theme::Dark);
        }
        if lower.contains("light") {
            return Some(Theme::Light);
        }
    }
    window_bg.map(|c| {
        // Воспринимаемая яркость: тёмный фон → тёмная тема.
        let luma = 0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32;
        if luma < 128.0 {
            Theme::Dark
        } else {
            Theme::Light
        }
    })
}

fn parse_rgb_triplet(value: &str) -> Option<Color> {
    let mut it = value.split(',').map(|s| s.trim().parse::<u8>().ok());
    Some(Color::new(it.next()??, it.next()??, it.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dark_from_scheme_name() {
        let text = "[General]\nColorScheme=BreezeDark\n";
        assert_eq!(parse_kdeglobals(text), Some(Theme::Dark));
    }

    #[test]
    fn detects_light_from_scheme_name() {
        let text = "[General]\nColorScheme=BreezeLight\n";
        assert_eq!(parse_kdeglobals(text), Some(Theme::Light));
    }

    #[test]
    fn falls_back_to_background_luminance() {
        let dark = "[Colors:Window]\nBackgroundNormal=35,38,41\n";
        assert_eq!(parse_kdeglobals(dark), Some(Theme::Dark));
        let light = "[Colors:Window]\nBackgroundNormal=252,252,252\n";
        assert_eq!(parse_kdeglobals(light), Some(Theme::Light));
    }

    #[test]
    fn unknown_scheme_gives_none() {
        assert_eq!(parse_kdeglobals("[General]\nColorScheme=Foo\n"), None);
    }
}

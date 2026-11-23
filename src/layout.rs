//! Разрешение «индекс группы XKB → код раскладки» (`0 → EN`, `1 → RU`).
//!
//! Источник — `LayoutList` из `~/.config/kxkbrc` (KDE), с фолбэком на
//! `setxkbmap -query`. `XkbGetNames` для этой цели не годится: он отдаёт
//! локализованные названия («English (US)», «Russian»), а не коды.
//! См. docs/spike-report.md, M0-02.

use std::path::PathBuf;
use std::process::Command;

/// Список кодов раскладок в порядке групп, например `["us", "ru"]`.
pub fn layout_codes() -> Vec<String> {
    from_kxkbrc()
        .or_else(from_setxkbmap)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Код раскладки для индекса группы, либо `?<N>` если конфигурация неизвестна.
pub fn code_for(group: u8, codes: &[String]) -> String {
    codes
        .get(group as usize)
        .cloned()
        .unwrap_or_else(|| format!("?{}", group))
}

fn kxkbrc_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("kxkbrc"))
}

/// Парсит `LayoutList=us,ru` из конфига KDE.
fn from_kxkbrc() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(kxkbrc_path()?).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("LayoutList=") {
            let codes: Vec<String> = value
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !codes.is_empty() {
                return Some(codes);
            }
        }
    }
    None
}

/// Фолбэк: `setxkbmap -query` → строка `layout: us,ru`.
///
/// Вызывается только на старте, не на каждое переключение.
fn from_setxkbmap() -> Option<Vec<String>> {
    let out = Command::new("setxkbmap").arg("-query").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("layout:") {
            let codes: Vec<String> = value
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !codes.is_empty() {
                return Some(codes);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_for_existing_group() {
        let codes = vec!["US".to_string(), "RU".to_string()];
        assert_eq!(code_for(0, &codes), "US");
        assert_eq!(code_for(1, &codes), "RU");
    }

    #[test]
    fn code_for_missing_group() {
        let codes = vec!["US".to_string()];
        assert_eq!(code_for(3, &codes), "?3");
    }
}

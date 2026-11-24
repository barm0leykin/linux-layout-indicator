//! Платформенные источники события смены раскладки и точки привязки.
//!
//! Реализации: X11 (M1), KWin/Wayland (M3), Windows/macOS (M5, бэклог).

pub mod kde;
pub mod x11;

use anyhow::Result;

/// Какой источник раскладки использовать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// KDE D-Bus, если доступен, иначе XKB.
    #[default]
    Auto,
    /// Только `org.kde.KeyboardLayouts` (KDE X11 и Wayland).
    Kde,
    /// Только XKB-события (прочие X11).
    Xkb,
}

/// Открыть бэкенд по настройке.
pub fn open(kind: BackendKind) -> Result<Box<dyn Backend>> {
    match kind {
        BackendKind::Kde => Ok(Box::new(kde::KdeBackend::new()?)),
        BackendKind::Xkb => Ok(Box::new(x11::X11Backend::new()?)),
        BackendKind::Auto => match kde::KdeBackend::new() {
            Ok(backend) => Ok(Box::new(backend)),
            Err(err) => {
                crate::log::debug(&format!("KDE-бэкенд недоступен: {err}"));
                Ok(Box::new(x11::X11Backend::new()?))
            }
        },
    }
}

/// Коды раскладок: сначала спрашиваем KDE, иначе — из конфигурации XKB.
pub fn layout_codes() -> Vec<String> {
    kde::layout_codes().unwrap_or_else(crate::layout::layout_codes)
}

/// Смена раскладки: новый индекс группы.
///
/// В X11 это `XkbStateNotify.group`. Одно реальное переключение даёт
/// ровно одно событие (проверено в спайке, см. docs/spike-report.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutChanged {
    /// Индекс группы XKB (0-based).
    pub group: u8,
}

/// Контракт платформенного бэкенда.
pub trait Backend {
    /// Текущий индекс группы раскладки.
    fn current_group(&self) -> Result<u8>;

    /// Блокирующее ожидание следующей смены раскладки.
    ///
    /// Возвращает новый индекс группы. Не перехватывает ввод: подписка
    /// на `XkbStateNotify` пассивна.
    fn next_change(&mut self) -> Result<u8>;
}

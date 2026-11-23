//! Платформенные источники события смены раскладки и точки привязки.
//!
//! Реализации: X11 (M1), KWin/Wayland (M3), Windows/macOS (M5, бэклог).

pub mod x11;

use anyhow::Result;

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

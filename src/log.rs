//! Минимальное логирование в stderr.
//!
//! Под systemd/journald stderr уходит в журнал автоматически. Содержимое полей
//! и нажатия не логируются никогда (PRD §9).

use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn init(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Сообщение, которое полезно видеть всегда.
pub fn info(msg: &str) {
    eprintln!("{msg}");
}

/// Сообщение только в режиме `--verbose` / `-v`.
pub fn debug(msg: &str) {
    if is_verbose() {
        eprintln!("[debug] {msg}");
    }
}

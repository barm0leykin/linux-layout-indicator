//! Политика показа индикатора: фазы, тайминги, дедупликация текста.
//!
//! Состояние: Hidden → FadeIn → Hold → FadeOut → Hidden.
//! Повторное переключение во время показа не пересоздаёт окно, а меняет текст
//! и сбрасывает таймер удержания (PRD §6.2).

use std::time::{Duration, Instant};

use crate::config::Config;

/// Целевая частота кадров анимации.
const FRAME: Duration = Duration::from_millis(16);

/// Скрытый индикатор «спит» до следующего события раскладки.
const IDLE: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Hidden,
    FadeIn,
    Hold,
    FadeOut,
}

pub struct Presenter {
    fade_in_ms: u64,
    hold_ms: u64,
    fade_out_ms: u64,
    phase: Phase,
    since: Instant,
    text: String,
    opacity: f32,
}

impl Presenter {
    pub fn new(cfg: &Config) -> Self {
        Self {
            fade_in_ms: cfg.timing.fade_in_ms,
            hold_ms: cfg.timing.hold_ms,
            fade_out_ms: cfg.timing.fade_out_ms,
            phase: Phase::Hidden,
            since: Instant::now(),
            text: String::new(),
            opacity: 0.0,
        }
    }

    pub fn is_hidden(&self) -> bool {
        self.phase == Phase::Hidden
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Показать индикатор с новым текстом.
    pub fn show(&mut self, text: String) {
        let was_visible = self.phase != Phase::Hidden;
        self.text = text;
        self.since = Instant::now();
        if was_visible {
            // Уже на экране: текст меняется на месте, таймер удержания — заново.
            self.phase = Phase::Hold;
            self.opacity = 1.0;
        } else {
            self.phase = Phase::FadeIn;
            self.opacity = 0.0;
        }
    }

    /// Продвинуть анимацию. `Some(opacity)` — нужно перерисовать, `None` — скрыть.
    pub fn tick(&mut self) -> Option<f32> {
        let elapsed = self.since.elapsed().as_millis() as u64;
        match self.phase {
            Phase::Hidden => None,
            Phase::FadeIn => {
                if self.fade_in_ms == 0 || elapsed >= self.fade_in_ms {
                    self.phase = Phase::Hold;
                    self.since = Instant::now();
                    self.opacity = 1.0;
                } else {
                    self.opacity = elapsed as f32 / self.fade_in_ms as f32;
                }
                Some(self.opacity)
            }
            Phase::Hold => {
                if elapsed >= self.hold_ms {
                    self.phase = Phase::FadeOut;
                    self.since = Instant::now();
                }
                Some(1.0)
            }
            Phase::FadeOut => {
                if self.fade_out_ms == 0 || elapsed >= self.fade_out_ms {
                    self.phase = Phase::Hidden;
                    self.opacity = 0.0;
                    None
                } else {
                    self.opacity = 1.0 - elapsed as f32 / self.fade_out_ms as f32;
                    Some(self.opacity)
                }
            }
        }
    }

    /// Сколько ждать до следующего кадра (или до события, если скрыт).
    pub fn frame_delay(&self) -> Duration {
        if self.phase == Phase::Hidden {
            IDLE
        } else {
            FRAME
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.timing.fade_in_ms = 0;
        c.timing.hold_ms = 0;
        c.timing.fade_out_ms = 0;
        c
    }

    #[test]
    fn starts_hidden() {
        let p = Presenter::new(&cfg());
        assert!(p.is_hidden());
    }

    #[test]
    fn zero_timings_hide_immediately() {
        let mut p = Presenter::new(&cfg());
        p.show("EN".into());
        assert_eq!(p.tick(), Some(1.0)); // fade_in = 0 → сразу Hold
        assert_eq!(p.tick(), Some(1.0)); // hold = 0 → переход в FadeOut
        assert_eq!(p.tick(), None); // fade_out = 0 → Hidden
        assert!(p.is_hidden());
    }

    #[test]
    fn text_updates_in_place() {
        let mut p = Presenter::new(&cfg());
        p.show("EN".into());
        assert_eq!(p.text(), "EN");
        p.show("RU".into());
        assert_eq!(p.text(), "RU");
        assert!(!p.is_hidden());
    }
}

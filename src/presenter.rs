//! Политика показа индикатора: фазы, тайминги, кросс-фейд текста.
//!
//! Состояние: Hidden → FadeIn → Hold → FadeOut → Hidden.
//! Повторное переключение во время показа не пересоздаёт окно: старый текст
//! гаснет, новый проявляется за [`CROSS_FADE`], таймер удержания сбрасывается
//! (PRD §6.2).

use std::time::{Duration, Instant};

use crate::config::Config;

/// Частота кадров анимации.
const FRAME: Duration = Duration::from_millis(16);

/// Скрытый индикатор просыпается раз в секунду — чтобы проверить конфиг
/// и список раскладок.
const IDLE: Duration = Duration::from_secs(1);

/// Длительность кросс-фейда при смене текста.
const CROSS_FADE: Duration = Duration::from_millis(100);

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
    previous: Option<String>,
    crossfade_since: Instant,
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
            previous: None,
            crossfade_since: Instant::now(),
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

        if was_visible && text != self.text {
            // Старый текст уже на экране — запускаем кросс-фейд.
            self.previous = Some(std::mem::replace(&mut self.text, text));
            self.crossfade_since = Instant::now();
        } else {
            self.text = text;
            self.previous = None;
        }

        self.since = Instant::now();
        if was_visible {
            self.phase = Phase::Hold;
            self.opacity = 1.0;
        } else {
            self.phase = Phase::FadeIn;
            self.opacity = 0.0;
        }
    }

    /// Слои текста для отрисовки: обычно один, при кросс-фейде — два.
    pub fn layers(&self) -> Vec<(String, f32)> {
        if let Some(previous) = &self.previous {
            let total = CROSS_FADE.as_secs_f32();
            let t = if total <= 0.0 {
                1.0
            } else {
                (self.crossfade_since.elapsed().as_secs_f32() / total).clamp(0.0, 1.0)
            };
            if t < 1.0 {
                return vec![(previous.clone(), 1.0 - t), (self.text.clone(), t)];
            }
        }
        vec![(self.text.clone(), 1.0)]
    }

    /// Продвинуть анимацию. `Some(opacity)` — перерисовать, `None` — скрыть.
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
                    self.previous = None;
                }
                Some(1.0)
            }
            Phase::FadeOut => {
                if self.fade_out_ms == 0 || elapsed >= self.fade_out_ms {
                    self.phase = Phase::Hidden;
                    self.opacity = 0.0;
                    self.previous = None;
                    None
                } else {
                    self.opacity = 1.0 - elapsed as f32 / self.fade_out_ms as f32;
                    Some(self.opacity)
                }
            }
        }
    }

    /// Сколько ждать до следующего кадра (или до проверки состояния, если скрыт).
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

    #[test]
    fn switch_while_visible_starts_crossfade() {
        let mut p = Presenter::new(&cfg());
        p.show("EN".into());
        p.show("RU".into());
        let layers = p.layers();
        assert_eq!(layers.len(), 2, "ожидались два слоя кросс-фейда");
        assert_eq!(layers[0].0, "EN");
        assert_eq!(layers[1].0, "RU");
        assert!(layers[0].1 + layers[1].1 <= 1.0001);
    }

    #[test]
    fn same_text_does_not_crossfade() {
        let mut p = Presenter::new(&cfg());
        p.show("EN".into());
        p.show("EN".into());
        assert_eq!(p.layers().len(), 1);
    }
}

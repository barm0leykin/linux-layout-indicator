//! Геометрия мониторов через `xrandr --query`.
//!
//! `_NET_WORKAREA` в X11 — один прямоугольник на весь десктоп, поэтому
//! для мультимониторных конфигураций границы считаем по отдельным мониторам
//! (см. docs/spike-report.md, M0-07).

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Monitor {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Список подключённых мониторов. Пустой — если `xrandr` недоступен.
pub fn monitors() -> Vec<Monitor> {
    let Ok(output) = Command::new("xrandr").arg("--query").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

fn parse(text: &str) -> Vec<Monitor> {
    let mut result = Vec::new();
    for line in text.lines() {
        if !line.contains(" connected") {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(monitor) = parse_geometry(token) {
                result.push(monitor);
                break;
            }
        }
    }
    result
}

/// Разбирает `1920x1080+1920+0`.
fn parse_geometry(token: &str) -> Option<Monitor> {
    let (size, position) = token.split_once('+')?;
    let mut pos = position.split('+');
    let x: i32 = pos.next()?.parse().ok()?;
    let y: i32 = pos.next()?.parse().ok()?;
    let (w, h) = size.split_once('x')?;
    Some(Monitor {
        x,
        y,
        w: w.parse().ok()?,
        h: h.parse().ok()?,
    })
}

/// Монитор, содержащий точку; иначе ближайший по расстоянию до центра.
pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<Monitor> {
    if let Some(m) = monitors.iter().find(|m| m.contains(x, y)) {
        return Some(*m);
    }
    monitors
        .iter()
        .min_by_key(|m| {
            let cx = m.x + m.w / 2;
            let cy = m.y + m.h / 2;
            (cx - x).abs() + (cy - y).abs()
        })
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_geometry() {
        assert_eq!(
            parse_geometry("2560x1440+1920+0"),
            Some(Monitor {
                x: 1920,
                y: 0,
                w: 2560,
                h: 1440
            })
        );
    }

    #[test]
    fn ignores_non_geometry_tokens() {
        assert_eq!(parse_geometry("connected"), None);
        assert_eq!(parse_geometry("primary"), None);
    }

    #[test]
    fn picks_monitor_containing_point() {
        let ms = vec![
            Monitor {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            Monitor {
                x: 1920,
                y: 0,
                w: 2560,
                h: 1440,
            },
        ];
        assert_eq!(monitor_at(&ms, 100, 100).unwrap().x, 0);
        assert_eq!(monitor_at(&ms, 2000, 100).unwrap().x, 1920);
    }

    #[test]
    fn falls_back_to_nearest() {
        let ms = vec![Monitor {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        }];
        assert_eq!(monitor_at(&ms, 5000, 5000).unwrap().x, 0);
        assert!(monitor_at(&[], 0, 0).is_none());
    }
}

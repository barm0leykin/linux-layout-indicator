//! Отрисовка бейджа.
//!
//! Рисуем в собственный RGBA-буфер (предумноженная альфа, как ожидает X ARGB32):
//! скруглённый прямоугольник — через `tiny-skia`, текст — через `ab_glyph`.
//! Затем буфер целиком уходит в окно одним `PutImage`.

pub mod x11;

use ab_glyph::{Font, FontVec, Glyph, PxScale, ScaleFont, point};
use anyhow::{Context, Result};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::config::{Appearance, Color};

/// Загрузить шрифт из файла.
pub fn load_font(path: &std::path::Path) -> Result<FontVec> {
    let data = std::fs::read(path).with_context(|| format!("чтение шрифта {}", path.display()))?;
    FontVec::try_from_vec(data).map_err(|e| anyhow::anyhow!("некорректный шрифт: {e}"))
}

/// Отрисовать бейдж с заданной общей непрозрачностью (0.0–1.0).
pub fn render_badge(app: &Appearance, font: &FontVec, text: &str, opacity: f32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(app.width, app.height)?;

    let path = rounded_rect(app.width as f32, app.height as f32, app.corner_radius)?;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(sk_color(app.background_color, app.background_alpha));
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    draw_text(&mut pixmap, font, text, app);

    let opacity = opacity.clamp(0.0, 1.0);
    if opacity < 1.0 {
        // Буфер предумноженный: умножаем все каналы — это корректное затухание.
        for b in pixmap.data_mut().iter_mut() {
            *b = (*b as f32 * opacity).round() as u8;
        }
    }

    Some(pixmap)
}

fn sk_color(c: Color, alpha: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Путь скруглённого прямоугольника (в tiny-skia нет `push_round_rect`).
fn rounded_rect(w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.quad_to(w, 0.0, w, r);
    pb.line_to(w, h - r);
    pb.quad_to(w, h, w - r, h);
    pb.line_to(r, h);
    pb.quad_to(0.0, h, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.quad_to(0.0, 0.0, r, 0.0);
    pb.close();
    pb.finish()
}

fn draw_text(pixmap: &mut Pixmap, font: &FontVec, text: &str, app: &Appearance) {
    let scale = PxScale::from(app.font_size);
    let scaled = font.as_scaled(scale);

    let mut glyphs = Vec::new();
    let mut total_width = 0.0f32;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        let advance = scaled.h_advance(id);
        glyphs.push((id, advance));
        total_width += advance;
    }

    let ascent = scaled.ascent();
    let descent = scaled.descent();
    let text_height = ascent - descent;

    let mut pen_x = (app.width as f32 - total_width) / 2.0;
    let baseline = (app.height as f32 - text_height) / 2.0 + ascent;

    for (id, advance) in glyphs {
        let glyph = Glyph {
            id,
            scale,
            position: point(pen_x, baseline),
        };
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let x = bounds.min.x as i32 + gx as i32;
                let y = bounds.min.y as i32 + gy as i32;
                blend_pixel(pixmap, x, y, app.text_color, coverage);
            });
        }
        pen_x += advance;
    }
}

/// Наложить цвет с покрытием поверх предумноженного буфера (source-over).
fn blend_pixel(pixmap: &mut Pixmap, x: i32, y: i32, color: Color, coverage: f32) {
    let alpha = coverage.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }

    let idx = ((y as u32 * pixmap.width() + x as u32) * 4) as usize;
    let data = pixmap.data_mut();
    let inv = 1.0 - alpha;

    let src = [
        color.r as f32 * alpha,
        color.g as f32 * alpha,
        color.b as f32 * alpha,
        255.0 * alpha,
    ];
    for (i, s) in src.iter().enumerate() {
        let dst = data[idx + i] as f32;
        data[idx + i] = (s + dst * inv).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_has_requested_size() {
        let app = Appearance {
            width: 40,
            height: 20,
            ..Default::default()
        };
        let font = load_font(&app.font_path).expect("шрифт по умолчанию должен быть доступен");
        let pixmap = render_badge(&app, &font, "EN", 1.0).expect("пиксмап");
        assert_eq!(pixmap.width(), 40);
        assert_eq!(pixmap.height(), 20);
    }

    #[test]
    fn fully_transparent_badge_is_empty() {
        let app = Appearance::default();
        let font = load_font(&app.font_path).expect("шрифт по умолчанию должен быть доступен");
        let pixmap = render_badge(&app, &font, "RU", 0.0).expect("пиксмап");
        assert!(pixmap.data().iter().all(|&b| b == 0));
    }
}

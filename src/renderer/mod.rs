//! Отрисовка бейджа.
//!
//! Рисуем в собственный RGBA-буфер (предумноженная альфа, как ожидает X ARGB32):
//! скруглённый прямоугольник — через `tiny-skia`, текст — через `ab_glyph`.
//! Затем буфер целиком уходит в окно одним `PutImage`.
//!
//! Слоёв текста может быть несколько — так делается кросс-фейд при быстром
//! переключении (старый текст гаснет, новый проявляется).

pub mod x11;

use ab_glyph::{Font, FontVec, Glyph, PxScale, ScaleFont, point};
use anyhow::{Context, Result};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::config::{Appearance, Color};

/// Слой текста: что рисовать и с какой непрозрачностью.
pub struct Layer<'a> {
    pub text: &'a str,
    pub alpha: f32,
}

impl<'a> Layer<'a> {
    pub fn new(text: &'a str, alpha: f32) -> Self {
        Self { text, alpha }
    }
}

/// Загрузить шрифт из файла.
pub fn load_font(path: &std::path::Path) -> Result<FontVec> {
    let data = std::fs::read(path).with_context(|| format!("чтение шрифта {}", path.display()))?;
    FontVec::try_from_vec(data).map_err(|e| anyhow::anyhow!("некорректный шрифт: {e}"))
}

/// Отрисовать бейдж с заданной общей непрозрачностью (0.0–1.0).
pub fn render_badge(
    app: &Appearance,
    font: &FontVec,
    layers: &[Layer<'_>],
    opacity: f32,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(app.width, app.height)?;
    let (background, foreground) = app.resolved_colors();

    let path = rounded_rect(app.width as f32, app.height as f32, app.corner_radius)?;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(sk_color(background, app.background_alpha));
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // Рамка поверх фона — для контраста на любом фоне рабочего стола.
    let (border, border_width) = app.resolved_border();
    if border_width > 0.0 {
        let mut border_paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        border_paint.set_color(sk_color(border, 1.0));
        let stroke = Stroke {
            width: border_width,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &border_paint, &stroke, Transform::identity(), None);
    }

    for layer in layers {
        if layer.alpha > 0.0 {
            draw_text(&mut pixmap, font, layer.text, app, foreground, layer.alpha);
        }
    }

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

fn draw_text(
    pixmap: &mut Pixmap,
    font: &FontVec,
    text: &str,
    app: &Appearance,
    color: Color,
    alpha: f32,
) {
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
                blend_pixel(pixmap, x, y, color, coverage * alpha);
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

    fn font() -> FontVec {
        load_font(&Appearance::default().font_path)
            .expect("шрифт по умолчанию должен быть доступен")
    }

    #[test]
    fn badge_has_requested_size() {
        let app = Appearance {
            width: 40,
            height: 20,
            ..Default::default()
        };
        let pixmap = render_badge(&app, &font(), &[Layer::new("EN", 1.0)], 1.0).expect("пиксмап");
        assert_eq!(pixmap.width(), 40);
        assert_eq!(pixmap.height(), 20);
    }

    #[test]
    fn fully_transparent_badge_is_empty() {
        let app = Appearance::default();
        let pixmap = render_badge(&app, &font(), &[Layer::new("RU", 1.0)], 0.0).expect("пиксмап");
        assert!(pixmap.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn zero_alpha_layer_leaves_no_text() {
        let app = Appearance::default();
        let empty = render_badge(&app, &font(), &[], 1.0).expect("пиксмап");
        let faded = render_badge(&app, &font(), &[Layer::new("RU", 0.0)], 1.0).expect("пиксмап");
        assert_eq!(empty.data(), faded.data());
    }

    #[test]
    fn border_is_drawn_with_configured_color() {
        let mut app = Appearance {
            width: 60,
            height: 30,
            ..Default::default()
        };
        app.background_alpha = 1.0;
        app.border_color = Some(Color::new(0xff, 0x00, 0x00));
        app.border_width = 2.0;

        let pixmap = render_badge(&app, &font(), &[], 1.0).expect("пиксмап");
        let has_red = pixmap
            .data()
            .chunks_exact(4)
            .any(|px| px[0] > 150 && px[1] < 100 && px[2] < 100);
        assert!(has_red, "рамка заданного цвета не отрисована");
    }

    #[test]
    fn zero_border_width_draws_nothing_red() {
        let mut app = Appearance {
            width: 60,
            height: 30,
            ..Default::default()
        };
        app.background_alpha = 1.0;
        app.border_color = Some(Color::new(0xff, 0x00, 0x00));
        app.border_width = 0.0;

        let pixmap = render_badge(&app, &font(), &[], 1.0).expect("пиксмап");
        let has_red = pixmap
            .data()
            .chunks_exact(4)
            .any(|px| px[0] > 150 && px[1] < 100 && px[2] < 100);
        assert!(!has_red, "при нулевой толщине рамки быть не должно");
    }

    #[test]
    fn text_changes_pixels() {
        let app = Appearance::default();
        let empty = render_badge(&app, &font(), &[], 1.0).expect("пиксмап");
        let with_text =
            render_badge(&app, &font(), &[Layer::new("EN", 1.0)], 1.0).expect("пиксмап");
        assert_ne!(empty.data(), with_text.data());
    }
}

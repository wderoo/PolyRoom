//! Icons drawn in code (original work; see AGENTS.md on assets).

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use polyroom_engine::ColorLabel;

/// A five-pointed star. Drawn as a convex pentagon plus five tip triangles, since egui only
/// fills convex polygons reliably.
pub fn star(painter: &Painter, center: Pos2, radius: f32, fill: Color32) {
    let inner = radius * 0.42;
    let point = |i: usize, r: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        center + vec2(a.cos(), a.sin()) * r
    };
    let pentagon: Vec<Pos2> = (0..5).map(|i| point(i * 2 + 1, inner)).collect();
    painter.add(Shape::convex_polygon(pentagon, fill, Stroke::NONE));
    for i in 0..5 {
        let tip = point(i * 2, radius);
        let left = point((i * 2 + 9) % 10, inner);
        let right = point(i * 2 + 1, inner);
        painter.add(Shape::convex_polygon(
            vec![tip, right, left],
            fill,
            Stroke::NONE,
        ));
    }
}

/// A row of five stars, `rating` of them filled. Returns the rect of each star for hit-testing.
pub fn stars(
    painter: &Painter,
    left_center: Pos2,
    size: f32,
    rating: u8,
    on: Color32,
    off: Color32,
) -> [Rect; 5] {
    let step = size * 1.15;
    std::array::from_fn(|i| {
        let c = left_center + vec2(size * 0.5 + i as f32 * step, 0.0);
        let filled = (i as u8) < rating;
        star(painter, c, size * 0.5, if filled { on } else { off });
        Rect::from_center_size(c, vec2(step, size))
    })
}

/// A small pennant flag (for "pick").
pub fn pick_flag(painter: &Painter, rect: Rect, color: Color32) {
    let pole_x = rect.left() + rect.width() * 0.25;
    painter.line_segment(
        [pos2(pole_x, rect.top()), pos2(pole_x, rect.bottom())],
        Stroke::new(1.5, color),
    );
    painter.add(Shape::convex_polygon(
        vec![
            pos2(pole_x, rect.top()),
            pos2(rect.right(), rect.top() + rect.height() * 0.28),
            pos2(pole_x, rect.top() + rect.height() * 0.56),
        ],
        color,
        Stroke::NONE,
    ));
}

/// A crossed circle (for "rejected").
pub fn reject_mark(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.45;
    painter.circle_stroke(c, r, Stroke::new(1.5, color));
    let d = r * 0.5;
    painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], Stroke::new(1.5, color));
    painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], Stroke::new(1.5, color));
}

/// Three horizontal dots (a "more actions" button).
pub fn dots(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let gap = rect.width().min(rect.height()) * 0.26;
    for dx in [-gap, 0.0, gap] {
        painter.circle_filled(c + vec2(dx, 0.0), 1.6, color);
    }
}

/// An upward arrow (for "up one folder").
pub fn up_arrow(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let h = rect.height() * 0.32;
    let stroke = Stroke::new(1.8, color);
    painter.line_segment([c + vec2(0.0, h), c + vec2(0.0, -h)], stroke);
    painter.line_segment([c + vec2(-h * 0.8, -h * 0.1), c + vec2(0.0, -h)], stroke);
    painter.line_segment([c + vec2(h * 0.8, -h * 0.1), c + vec2(0.0, -h)], stroke);
}

pub fn label_color(label: ColorLabel) -> Color32 {
    match label {
        ColorLabel::Red => Color32::from_rgb(0xd9, 0x4c, 0x4c),
        ColorLabel::Yellow => Color32::from_rgb(0xe0, 0xc2, 0x3c),
        ColorLabel::Green => Color32::from_rgb(0x5c, 0xb8, 0x5c),
        ColorLabel::Blue => Color32::from_rgb(0x4f, 0x8c, 0xe0),
        ColorLabel::Purple => Color32::from_rgb(0xa0, 0x6c, 0xd5),
    }
}

//! The thumbnail grid and the filmstrip.

use egui::{
    Align2, Color32, CornerRadius, FontId, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use polyroom_engine::{Flag, ModelId, Session};
use serde_json::json;

use crate::layout::Actions;
use crate::thumbs::ThumbTextures;
use crate::{icons, theme};

const GAP: f32 = 8.0;
/// Space under the thumbnail for the name and stars.
const CAPTION: f32 = 40.0;

/// Scroll bookkeeping so keyboard navigation keeps the current model in view.
#[derive(Default)]
pub struct GridState {
    last_current: Option<ModelId>,
    offset: f32,
    viewport_height: f32,
}

fn select_mode(modifiers: Modifiers) -> &'static str {
    if modifiers.shift {
        "extend"
    } else if modifiers.command {
        "toggle"
    } else {
        "replace"
    }
}

/// Handles clicks on a model cell; double-click opens the loupe.
fn cell_clicks(ui: &Ui, response: &egui::Response, id: ModelId, actions: &mut Actions) {
    if response.double_clicked() {
        actions.push_args("select.model", json!({ "id": id }));
        actions.push("view.loupe");
    } else if response.clicked() {
        let mode = select_mode(ui.input(|i| i.modifiers));
        actions.push_args("select.model", json!({ "id": id, "mode": mode }));
    }
}

/// Draws the thumbnail (or a placeholder) into `rect`, requesting it if needed.
fn thumbnail(ui: &Ui, rect: Rect, id: ModelId, session: &mut Session, thumbs: &mut ThumbTextures) {
    if let Some(tex) = thumbs.get(id) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(tex, rect, uv, Color32::WHITE);
    } else if session.thumbnail_failed(id) {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "⚠",
            FontId::proportional(rect.height() * 0.2),
            theme::TEXT_DIM,
        );
    } else {
        session.want_thumbnail(id);
        // A soft pulsing dot while loading.
        let t = ui.input(|i| i.time) as f32;
        let alpha = 0.35 + 0.25 * (t * 3.0).sin();
        ui.painter().circle_filled(
            rect.center(),
            rect.width() * 0.04,
            theme::TEXT_DIM.gamma_multiply(alpha),
        );
    }
}

fn cell_fill(selected: bool, hovered: bool) -> Color32 {
    match (selected, hovered) {
        (true, _) => Color32::from_gray(0x4a),
        (false, true) => Color32::from_gray(0x3a),
        (false, false) => Color32::from_gray(0x30),
    }
}

pub fn grid(
    ui: &mut Ui,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    state: &mut GridState,
    actions: &mut Actions,
) {
    let size = session.thumb_size;
    let cell = vec2(size, size + CAPTION);
    let width = ui.available_width() - GAP;
    let cols = ((width + GAP) / (cell.x + GAP)).floor().max(1.0) as usize;
    session.grid_columns = cols;
    let ids: Vec<ModelId> = session.visible().to_vec();
    let rows = ids.len().div_ceil(cols);
    let row_height = cell.y + GAP;

    // Centre the columns horizontally.
    let used = cols as f32 * (cell.x + GAP) - GAP;
    let margin = ((width - used) / 2.0).max(0.0) + GAP / 2.0;

    let mut area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("grid");
    // Only once the viewport size is known (not on the very first frame).
    if session.current() != state.last_current && state.viewport_height > 0.0 {
        state.last_current = session.current();
        if let Some(pos) = session
            .current()
            .and_then(|c| ids.iter().position(|&i| i == c))
        {
            let top = (pos / cols) as f32 * row_height;
            if top < state.offset {
                area = area.vertical_scroll_offset(top);
            } else if top + row_height > state.offset + state.viewport_height {
                area = area.vertical_scroll_offset(top + row_height - state.viewport_height);
            }
        }
    }

    let output = area.show_rows(ui, row_height, rows, |ui, range| {
        for row in range {
            ui.add_space(GAP / 2.0);
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.spacing_mut().item_spacing.x = GAP;
                for &id in ids.iter().skip(row * cols).take(cols) {
                    grid_cell(ui, id, cell, session, thumbs, actions);
                }
            });
            ui.add_space(GAP / 2.0);
        }
    });
    state.offset = output.state.clamped_offset().y;
    state.viewport_height = output.inner_rect.height();
}

fn grid_cell(
    ui: &mut Ui,
    id: ModelId,
    size: egui::Vec2,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    actions: &mut Actions,
) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    cell_clicks(ui, &response, id, actions);
    let Some(model) = session.model(id).cloned() else {
        return;
    };
    let selected = session.is_selected(id);
    let current = session.current() == Some(id);
    let painter = ui.painter_at(rect);

    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        cell_fill(selected, response.hovered()),
    );
    if current {
        painter.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(3),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }

    let pad = 6.0;
    let img = Rect::from_min_size(
        rect.min + vec2(pad, pad),
        vec2(rect.width() - 2.0 * pad, rect.width() - 2.0 * pad),
    );
    thumbnail(ui, img, id, session, thumbs);
    if model.flag == Flag::Reject {
        // Lightroom dims rejected photos; do the same.
        painter.rect_filled(img, CornerRadius::ZERO, Color32::from_black_alpha(140));
    }

    // Flag badge, top-left.
    let badge = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(12.0, 14.0));
    match model.flag {
        Flag::Pick => icons::pick_flag(&painter, badge, Color32::WHITE),
        Flag::Reject => icons::reject_mark(&painter, badge, Color32::from_rgb(0xe0, 0x60, 0x60)),
        Flag::None => {}
    }

    // Name and stars.
    let name_pos = pos2(rect.left() + pad, rect.top() + rect.width() - 2.0);
    let name = model.name();
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), theme::TEXT);
    let clip = Rect::from_min_size(name_pos, vec2(rect.width() - 2.0 * pad, 16.0));
    painter
        .with_clip_rect(clip)
        .galley(name_pos, galley, theme::TEXT);
    if model.rating > 0 || response.hovered() || selected {
        icons::stars(
            &painter,
            pos2(rect.left() + pad, rect.bottom() - 11.0),
            10.0,
            model.rating,
            theme::TEXT,
            Color32::from_gray(0x5a),
        );
    }

    // Colour label: a strip along the bottom edge.
    if let Some(label) = model.label {
        let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 3.0), rect.max);
        painter.rect_filled(strip, CornerRadius::same(1), icons::label_color(label));
    }

    if let Some(stats) = model.stats {
        response.on_hover_text(format!(
            "{}\n{} triangles · {}",
            model.path.display(),
            crate::format::count(stats.triangles),
            crate::format::dimensions(&stats),
        ));
    } else {
        response.on_hover_text(model.path.display().to_string());
    }
}

/// A horizontal strip of small thumbnails along the bottom, like Lightroom's filmstrip.
pub fn filmstrip(
    ui: &mut Ui,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    state: &mut GridState,
    actions: &mut Actions,
) {
    let ids: Vec<ModelId> = session.visible().to_vec();
    let h = ui.available_height();
    let size = vec2(h, h);
    let step = size.x + 4.0;

    let mut area = egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .id_salt("filmstrip");
    if session.current() != state.last_current && state.viewport_height > 0.0 {
        state.last_current = session.current();
        if let Some(pos) = session
            .current()
            .and_then(|c| ids.iter().position(|&i| i == c))
        {
            let left = pos as f32 * step;
            if left < state.offset || left + step > state.offset + state.viewport_height {
                area = area.horizontal_scroll_offset((left - state.viewport_height / 2.0).max(0.0));
            }
        }
    }
    let output = area.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(vec2(ids.len() as f32 * step, h));
        let origin = ui.min_rect().min;
        let first = (viewport.min.x / step).floor().max(0.0) as usize;
        let count = (viewport.width() / step).ceil() as usize + 1;
        for (i, &id) in ids.iter().enumerate().skip(first).take(count) {
            let rect = Rect::from_min_size(origin + vec2(i as f32 * step, 0.0), size);
            let response = ui.interact(rect, ui.scope_id().with(("film", id)), Sense::click());
            cell_clicks(ui, &response, id, actions);
            let selected = session.is_selected(id);
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(2),
                cell_fill(selected, response.hovered()),
            );
            thumbnail(ui, rect.shrink(4.0), id, session, thumbs);
            if session.current() == Some(id) {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(2),
                    Stroke::new(1.5, theme::ACCENT),
                    StrokeKind::Inside,
                );
            }
            if let Some(label) = session.model(id).and_then(|m| m.label) {
                let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 3.0), rect.max);
                ui.painter()
                    .rect_filled(strip, CornerRadius::ZERO, icons::label_color(label));
            }
        }
    });
    state.offset = output.state.clamped_offset().x;
    // For the horizontal strip, "viewport_height" holds the visible width.
    state.viewport_height = output.inner_rect.width();
}

//! The thumbnail grid.

use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, FontId, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use polyroom_engine::{Flag, Format, ModelId, Session};
use serde_json::json;

use crate::layout::Actions;
use crate::thumbs::ThumbTextures;
use crate::{icons, meta_ui, theme};

const GAP: f32 = 8.0;
/// Space kept free at the right of the grid, so the scroll bar does not sit on the cards.
const RIGHT_MARGIN: f32 = 14.0;
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

/// Handles clicks on a model cell: a click selects, a double-click opens the file, and a
/// right-click selects the cell first unless it is part of the selection already.
fn cell_clicks(
    ui: &Ui,
    response: &egui::Response,
    id: ModelId,
    selected: bool,
    actions: &mut Actions,
) {
    if response.double_clicked() {
        actions.push_args("select.model", json!({ "id": id }));
        actions.push("file.open");
    } else if response.clicked() {
        let mode = select_mode(ui.input(|i| i.modifiers));
        actions.push_args("select.model", json!({ "id": id, "mode": mode }));
    } else if response.secondary_clicked() && !selected {
        actions.push_args("select.model", json!({ "id": id }));
    }
}

/// The right-click menu of a model cell. It acts on the selection.
fn model_menu(ui: &mut Ui, session: &Session, id: ModelId, actions: &mut Actions) {
    let Some(model) = session.model(id) else {
        return;
    };
    if ui.button("Open").clicked() {
        actions.push("file.open");
    }
    if ui.button("Reveal in File Explorer").clicked() {
        actions.push("file.reveal");
    }
    ui.separator();
    let favorite = if model.meta.favorite {
        "Remove from Favorites"
    } else {
        "Add to Favorites"
    };
    if ui.button(favorite).clicked() {
        actions.push("meta.favorite.toggle");
    }
    ui.menu_button("Category", |ui| {
        meta_ui::category_items(ui, session, model.meta.category, None, actions);
    });
    ui.menu_button("Add Tag", |ui| {
        ui.set_min_width(180.0);
        let key = ui.make_persistent_id("cell-menu-tag");
        let mut text: String = ui.data_mut(|d| d.get_temp(key)).unwrap_or_default();
        meta_ui::add_tag_box(ui, &mut text, None, actions);
        ui.data_mut(|d| d.insert_temp(key, text));
    });
    if ui.button("Regenerate Thumbnail").clicked() {
        actions.push("preview.regenerate");
    }
    ui.separator();
    let delete = egui::RichText::new("Delete…").color(Color32::from_rgb(0xe0, 0x60, 0x60));
    if ui.button(delete).clicked() {
        actions.push("file.delete");
    }
}

/// Draws the thumbnail of `id` (its cover photo, if it has one) or a placeholder into `rect`,
/// requesting it if needed.
pub(crate) fn thumbnail(
    ui: &Ui,
    rect: Rect,
    id: ModelId,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
) {
    // A photo that can't be shown falls back to the model's own render.
    let id = Some(session.cover(id))
        .filter(|&c| !session.thumbnail_failed(c))
        .unwrap_or(id);
    let is_zip = session.model(id).is_some_and(|m| m.format.is_archive());
    if session
        .model(id)
        .is_some_and(|m| !m.format.is_previewable())
    {
        icons::file(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(rect.width() * 0.3, rect.width() * 0.38)),
            theme::TEXT_DIM,
        );
    } else if let Some(tex) = thumbs.get(id) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(tex, rect, uv, Color32::WHITE);
    } else if is_zip
        && session.thumbnail_failed(id)
        && session.model(id).is_some_and(|m| m.error.is_none())
    {
        // A fine archive with nothing inside to preview.
        icons::archive(
            ui.painter(),
            Rect::from_center_size(rect.center(), rect.size() * 0.4),
            theme::TEXT_DIM,
        );
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
    let width = ui.available_width() - GAP - RIGHT_MARGIN;
    let cols = ((width + GAP) / (cell.x + GAP)).floor().max(1.0) as usize;
    session.grid_columns = cols;
    let ids: Vec<ModelId> = session.visible().to_vec();
    // Subfolders come first, like in a file manager.
    let folders: Vec<PathBuf> = session.visible_folders().to_vec();
    let total = folders.len() + ids.len();
    let rows = total.div_ceil(cols);
    let row_height = cell.y + GAP;

    // Centre the columns horizontally.
    let used = cols as f32 * (cell.x + GAP) - GAP;
    let margin = ((width - used) / 2.0).max(0.0) + GAP / 2.0;

    // Each folder keeps its own scroll position.
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt(("grid", session.collection().clone()));
    // Only once the viewport size is known (not on the very first frame).
    if session.current() != state.last_current && state.viewport_height > 0.0 {
        state.last_current = session.current();
        if let Some(pos) = session
            .current()
            .and_then(|c| ids.iter().position(|&i| i == c))
        {
            let top = ((pos + folders.len()) / cols) as f32 * row_height;
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
                for i in (row * cols)..((row + 1) * cols).min(total) {
                    match folders.get(i) {
                        Some(path) => folder_cell(ui, path, cell, session, thumbs, actions),
                        None => {
                            if let Some(&id) = ids.get(i - folders.len()) {
                                grid_cell(ui, id, cell, session, thumbs, actions);
                            }
                        }
                    }
                }
            });
            ui.add_space(GAP / 2.0);
        }
    });
    state.offset = output.state.clamped_offset().y;
    state.viewport_height = output.inner_rect.height();
}

/// A subfolder: a folder shape holding up to four thumbnails from inside it.
/// Double-click opens it.
fn folder_cell(
    ui: &mut Ui,
    path: &Path,
    size: egui::Vec2,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    actions: &mut Actions,
) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.double_clicked() {
        actions.push_args("library.show_folder", json!({ "path": path }));
    } else if response.clicked() || response.secondary_clicked() {
        actions.push_args("select.folder", json!({ "path": path }));
    }
    let focused = session.focused_folder() == Some(path);
    let Some(node) = session.folder(path).cloned() else {
        return;
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} folder", node.name))
    });
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        cell_fill(focused, response.hovered()),
    );
    if focused {
        painter.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(3),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }

    // Folder shape: a back panel with a tab, and a front panel.
    let pad = 10.0;
    let w = rect.width() - 2.0 * pad;
    let body = Rect::from_min_size(rect.min + vec2(pad, pad + w * 0.12), vec2(w, w * 0.78));
    let tab = Rect::from_min_size(
        rect.min + vec2(pad, pad + w * 0.04),
        vec2(w * 0.38, w * 0.12),
    );
    let back = Color32::from_rgb(0x8a, 0x6a, 0x3c);
    let front = Color32::from_rgb(0xc8, 0x9a, 0x55);
    painter.rect_filled(tab, CornerRadius::same(4), back);
    painter.rect_filled(body, CornerRadius::same(4), back);
    let face = Rect::from_min_max(body.min + vec2(0.0, w * 0.08), body.max);
    painter.rect_filled(face, CornerRadius::same(4), front);

    // Up to four sample thumbnails on the front, in a 2×2 grid.
    let inner = face.shrink(w * 0.06);
    let half = (inner.size() - vec2(4.0, 4.0)) / 2.0;
    for (k, id) in node.samples.iter().enumerate() {
        let col = (k % 2) as f32;
        let row = (k / 2) as f32;
        let slot = Rect::from_min_size(
            inner.min + vec2(col * (half.x + 4.0), row * (half.y + 4.0)),
            half,
        );
        let side = slot.width().min(slot.height());
        let square = Rect::from_center_size(slot.center(), vec2(side, side));
        painter.rect_filled(slot, CornerRadius::same(2), Color32::from_black_alpha(40));
        thumbnail(ui, square, *id, session, thumbs);
    }

    let caption = pos2(rect.left() + pad - 4.0, rect.top() + rect.width() - 2.0);
    let galley = painter.layout_no_wrap(node.name.clone(), FontId::proportional(12.0), theme::TEXT);
    painter
        .with_clip_rect(Rect::from_min_size(caption, vec2(rect.width() - pad, 16.0)))
        .galley(caption, galley, theme::TEXT);
    let count = crate::format::contents(
        node.total
            .saturating_sub(node.images + node.archives + node.others) as u64,
        node.images as u64,
        node.archives as u64,
        node.others as u64,
    );
    painter.text(
        pos2(caption.x, rect.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        count,
        FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
    response.context_menu(|ui| crate::tree::folder_menu(ui, path, session, actions));
    // "⋯" in the top-right corner, also opening the folder menu.
    let dots = Rect::from_min_size(
        pos2(rect.right() - 30.0, rect.top() + 4.0),
        vec2(26.0, 18.0),
    );
    let hovered = ui.rect_contains_pointer(rect);
    let dots_id = ui.make_persistent_id(("cell-dots", path));
    crate::tree::dots_button(ui, dots, dots_id, path, hovered, session, actions);
    response.on_hover_text(format!("{}\nDouble-click to open", node.path.display()));
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
    let selected = session.is_selected(id);
    cell_clicks(ui, &response, id, selected, actions);
    let Some(model) = session.model(id).cloned() else {
        return;
    };
    response.context_menu(|ui| model_menu(ui, session, id, actions));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, model.name()));
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
    kind_badge(&painter, img, &model, session.stack(id).len());

    // Flag badge, top-left.
    let badge = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(12.0, 14.0));
    match model.flag {
        Flag::Pick => icons::pick_flag(&painter, badge, Color32::WHITE),
        Flag::Reject => icons::reject_mark(&painter, badge, Color32::from_rgb(0xe0, 0x60, 0x60)),
        Flag::None => {}
    }

    // Favorite heart, top-right.
    if model.meta.favorite {
        let heart = Rect::from_min_size(pair_top_right(img, vec2(18.0, 16.0)), vec2(14.0, 13.0));
        icons::heart(&painter, heart.expand(1.0), Color32::from_black_alpha(160));
        icons::heart(&painter, heart, icons::HEART);
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

    category_pill(&painter, rect, session, &model);

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

/// Where a `size` box goes in the top-right corner of `rect`, inset by 6.
fn pair_top_right(rect: Rect, size: egui::Vec2) -> egui::Pos2 {
    pos2(rect.right() - size.x + 4.0, rect.top() + 6.0)
}

/// The model's category (its own or its folder's) as a small pill at the bottom right.
fn category_pill(
    painter: &egui::Painter,
    cell: Rect,
    session: &Session,
    model: &polyroom_engine::Model,
) {
    let Some(id) = session.catalog().effective_category(model) else {
        return;
    };
    let Some(category) = session.catalog().categories().iter().find(|c| c.id == id) else {
        return;
    };
    let galley = painter.layout_no_wrap(
        category.name.clone(),
        FontId::proportional(10.0),
        Color32::WHITE,
    );
    let width = (galley.size().x + 10.0).min(cell.width() * 0.55);
    let pill = Rect::from_min_size(
        pos2(cell.right() - width - 6.0, cell.bottom() - 20.0),
        vec2(width, 15.0),
    );
    painter.rect_filled(
        pill,
        CornerRadius::same(7),
        meta_ui::category_color(category),
    );
    painter.with_clip_rect(pill.shrink2(vec2(3.0, 0.0))).galley(
        pos2(pill.left() + 5.0, pill.center().y - galley.size().y / 2.0),
        galley,
        Color32::WHITE,
    );
}

/// What the badge of a file says: its type, e.g. "STL", "ZIP", or the extension of an image or
/// other file.
pub(crate) fn badge_text(model: &polyroom_engine::Model) -> String {
    match model.format {
        Format::Image | Format::Other => model
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .filter(|e| !e.is_empty())
            .map_or_else(
                || "FILE".to_owned(),
                |e| e.chars().take(5).collect::<String>(),
            ),
        format => format.label().to_uppercase(),
    }
}

/// Pills on the thumbnail: the file type at the bottom right, and the photo count of a stacked
/// model at the bottom left.
fn kind_badge(painter: &egui::Painter, img: Rect, model: &polyroom_engine::Model, photos: usize) {
    let font = FontId::proportional(10.0);
    let galley = painter.layout_no_wrap(badge_text(model), font.clone(), Color32::WHITE);
    let size = vec2(galley.size().x + 10.0, 16.0);
    let pill = Rect::from_min_size(img.max - size - vec2(4.0, 4.0), size);
    painter.rect_filled(pill, CornerRadius::same(8), Color32::from_black_alpha(170));
    let pos = pos2(pill.left() + 5.0, pill.center().y - galley.size().y / 2.0);
    painter.galley(pos, galley, Color32::WHITE);

    if photos > 0 {
        let galley = painter.layout_no_wrap(photos.to_string(), font, Color32::WHITE);
        let size = vec2(galley.size().x + 24.0, 16.0);
        let pill = Rect::from_min_size(pos2(img.left() + 4.0, img.bottom() - size.y - 4.0), size);
        painter.rect_filled(pill, CornerRadius::same(8), Color32::from_black_alpha(170));
        icons::photo(
            painter,
            Rect::from_min_size(pill.min + vec2(5.0, 4.0), vec2(11.0, 8.0)),
            Color32::WHITE,
        );
        painter.galley(
            pos2(pill.left() + 19.0, pill.center().y - galley.size().y / 2.0),
            galley,
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use polyroom_engine::{CommandRegistry, Request, default_registry};

    use super::*;
    use crate::test_support::imported;

    struct App {
        session: Session,
        registry: CommandRegistry,
        thumbs: ThumbTextures,
        grid: GridState,
    }

    impl App {
        /// Dispatches what the frame asked for, like `PolyRoomApp::dispatch` does.
        fn run(&mut self, actions: Actions) {
            for (id, args) in actions.list {
                self.registry.run(id, &mut self.session, &args).unwrap();
            }
            self.session.poll();
        }

        fn current_name(&self) -> Option<String> {
            let id = self.session.current()?;
            Some(self.session.model(id)?.name())
        }
    }

    /// 30 models in the root and one subfolder, so the grid has a folder cell before them.
    fn files() -> Vec<String> {
        let mut files: Vec<String> = (0..30).map(|i| format!("m{i:02}.stl")).collect();
        files.push("extra/e0.stl".into());
        files
    }

    fn app(session: Session) -> App {
        App {
            session,
            registry: default_registry(),
            thumbs: ThumbTextures::default(),
            grid: GridState::default(),
        }
    }

    fn grid_harness(session: Session) -> Harness<'static, App> {
        Harness::builder()
            .with_size(vec2(1000.0, 420.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    allow_slow_double_clicks(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    grid(
                        ui,
                        &mut app.session,
                        &mut app.thumbs,
                        &mut app.grid,
                        &mut actions,
                    );
                    app.run(actions);
                },
                app(session),
            )
    }

    /// The harness advances time in big steps per frame, so two quick clicks would otherwise
    /// not count as a double click.
    fn allow_slow_double_clicks(ctx: &egui::Context) {
        ctx.options_mut(|o| o.input_options.max_double_click_delay = 60.0);
    }

    /// Runs frames until scroll animations and the like have finished.
    fn settle(harness: &mut Harness<'_, App>) {
        harness.run_steps(20);
    }

    /// The on-screen area the cells can be clicked in.
    fn screen(harness: &Harness<'_, App>) -> egui::Rect {
        harness.ctx.content_rect()
    }

    /// Clicks the visible part of the cell labelled `label` `times` times in quick succession.
    /// (Cells at the edge are only partly on screen, so their centre may be out of reach.)
    fn click_cell(harness: &mut Harness<'_, App>, label: &str, times: usize, modifiers: Modifiers) {
        let rect = harness.get_by_label(label).rect();
        let pos = rect.intersect(screen(harness)).center();
        harness.event(egui::Event::PointerMoved(pos));
        harness.event(egui::Event::ModifiersChanged(modifiers));
        for _ in 0..times {
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
            }
            // One click per frame, as with a real mouse, so egui sees a double click.
            harness.step();
        }
        harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        settle(harness);
    }

    /// Labels of the cells currently on screen.
    fn visible_cells(harness: &Harness<'_, App>) -> Vec<String> {
        harness
            .query_all_by_role(Role::Button)
            .filter_map(|n| n.accesskit_node().label())
            .collect()
    }

    /// Clicks every cell, scrolling until all were reached; each click must select the model
    /// whose name is on that cell.
    fn click_every_cell(harness: &mut Harness<'_, App>, expected_models: usize) {
        let mut checked = std::collections::BTreeSet::new();
        for _ in 0..40 {
            settle(harness);
            for label in visible_cells(harness) {
                if label.ends_with(" folder") || !checked.insert(label.clone()) {
                    continue;
                }
                click_cell(harness, &label, 1, Modifiers::NONE);
                assert_eq!(
                    harness.state().current_name().as_deref(),
                    Some(label.as_str()),
                    "clicked {label}"
                );
            }
            if checked.len() == expected_models {
                return;
            }
            // Bring the next rows into view.
            let Some(last) = visible_cells(harness).pop() else {
                break;
            };
            harness.get_by_label(&last).scroll_down();
            settle(harness);
        }
        panic!("only reached {} of {expected_models} models", checked.len());
    }

    #[test]
    fn clicking_a_grid_cell_selects_that_model_wherever_the_grid_is_scrolled() {
        let (_dir, session) = imported(&files());
        let mut harness = grid_harness(session);
        assert!(!visible_cells(&harness).is_empty() || harness.run() > 0);
        click_every_cell(&mut harness, 30);
    }

    #[test]
    fn double_clicking_opens_that_file() {
        // One cell in the first row, and the last one, which sits in a partly visible row.
        for last in [false, true] {
            let (_dir, session) = imported(&files());
            let mut harness = grid_harness(session);
            harness.run();
            let mut models = visible_cells(&harness)
                .into_iter()
                .filter(|l| !l.ends_with(" folder"));
            let target = if last {
                models.next_back()
            } else {
                models.nth(1)
            }
            .unwrap();
            click_cell(&mut harness, &target, 2, Modifiers::NONE);
            let app = harness.state_mut();
            assert_eq!(app.current_name().as_deref(), Some(target.as_str()));
            let opened: Vec<String> = app
                .session
                .drain_requests()
                .into_iter()
                .filter_map(|r| match r {
                    Request::OpenFile(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                opened,
                std::slice::from_ref(&target),
                "double-clicked {target}"
            );
        }
    }

    /// Right-clicks the cell labelled `label`.
    fn right_click_cell(harness: &mut Harness<'_, App>, label: &str) {
        let rect = harness.get_by_label(label).rect();
        let pos = rect.intersect(screen(harness)).center();
        harness.event(egui::Event::PointerMoved(pos));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.run_steps(5);
    }

    #[test]
    fn right_clicking_a_model_selects_it_and_offers_reveal_and_delete() {
        let (_dir, session) = imported(&files());
        let mut harness = grid_harness(session);
        harness.run();
        let target = visible_cells(&harness)
            .into_iter()
            .find(|l| !l.ends_with(" folder") && l != "m00.stl")
            .unwrap();
        right_click_cell(&mut harness, &target);
        assert_eq!(
            harness.state().current_name().as_deref(),
            Some(target.as_str())
        );
        harness.get_by_label("Reveal in File Explorer");
        harness.get_by_label_contains("Delete");
        harness.get_by_label("Add to Favorites");

        harness.get_by_label("Reveal in File Explorer").click();
        harness.run_steps(3);
        let requests = harness.state_mut().session.drain_requests();
        assert!(
            matches!(requests.as_slice(), [Request::RevealFile(p)] if p.ends_with(&target)),
            "{requests:?}"
        );
    }

    #[test]
    fn a_folder_cell_is_focused_by_a_click_and_has_a_context_menu() {
        let (_dir, session) = imported(&files());
        let mut harness = grid_harness(session);
        harness.run();
        click_cell(&mut harness, "extra folder", 1, Modifiers::NONE);
        assert!(harness.state().session.focused_folder().is_some());

        right_click_cell(&mut harness, "extra folder");
        harness.get_by_label("Remove from Library…");
        harness.get_by_label("Reveal in File Explorer");
        harness.get_by_label("Add to Favorites");
    }

    #[test]
    fn a_photo_named_like_a_model_is_stacked_on_it_not_shown_alone() {
        let files: Vec<String> = ["wolf.stl", "wolf.png", "boat.stl", "other.jpg", "kit.zip"]
            .map(String::from)
            .to_vec();
        let (_dir, session) = imported(&files);
        let mut harness = grid_harness(session);
        harness.run();
        let mut cells = visible_cells(&harness);
        cells.sort();
        assert_eq!(cells, ["boat.stl", "kit.zip", "other.jpg", "wolf.stl"]);
    }

    #[test]
    fn shift_click_selects_the_range_between_two_cells() {
        let (_dir, session) = imported(&files());
        let mut harness = grid_harness(session);
        harness.run();
        click_cell(&mut harness, "m01.stl", 1, Modifiers::NONE);
        click_cell(&mut harness, "m04.stl", 1, Modifiers::SHIFT);
        let app = harness.state();
        let mut names: Vec<String> = app
            .session
            .visible()
            .iter()
            .filter(|id| app.session.is_selected(**id))
            .filter_map(|id| Some(app.session.model(*id)?.name()))
            .collect();
        names.sort();
        assert_eq!(names, ["m01.stl", "m02.stl", "m03.stl", "m04.stl"]);
    }

    #[test]
    fn badges_name_the_type_of_every_kind_of_file() {
        let model = |name: &str, format| polyroom_engine::Model {
            id: 0,
            path: std::path::PathBuf::from("C:/m").join(name),
            folder: "C:/m".into(),
            format,
            size: 1,
            modified: None,
            added: 0,
            import: 1,
            rating: 0,
            flag: Flag::None,
            label: None,
            stats: None,
            meta: polyroom_engine::Meta::default(),
            error: None,
        };
        assert_eq!(badge_text(&model("a.stl", Format::Stl)), "STL");
        assert_eq!(badge_text(&model("a.3mf", Format::ThreeMf)), "3MF");
        assert_eq!(badge_text(&model("a.zip", Format::Zip)), "ZIP");
        assert_eq!(badge_text(&model("a.jpeg", Format::Image)), "JPEG");
        assert_eq!(badge_text(&model("notes.pdf", Format::Other)), "PDF");
        assert_eq!(badge_text(&model("README", Format::Other)), "FILE");
        assert_eq!(badge_text(&model("a.longext", Format::Other)), "LONGE");
    }
}

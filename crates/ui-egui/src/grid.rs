//! The thumbnail grid and the filmstrip.

use std::path::{Path, PathBuf};

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
    }
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
        cell_fill(false, response.hovered()),
    );

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
    let count = crate::format::models(node.total as u64);
    painter.text(
        pos2(caption.x, rect.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        count,
        FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
    response.context_menu(|ui| crate::tree::folder_menu(ui, path, actions));
    // "⋯" in the top-right corner, also opening the folder menu.
    let dots = Rect::from_min_size(
        pos2(rect.right() - 30.0, rect.top() + 4.0),
        vec2(26.0, 18.0),
    );
    let hovered = ui.rect_contains_pointer(rect);
    let dots_id = ui.make_persistent_id(("cell-dots", path));
    crate::tree::dots_button(ui, dots, dots_id, path, hovered, actions);
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
    cell_clicks(ui, &response, id, actions);
    let Some(model) = session.model(id).cloned() else {
        return;
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, model.name()));
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
            if let Some(model) = session.model(id) {
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, model.name())
                });
            }
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

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use polyroom_engine::{CommandRegistry, ViewMode, default_registry};

    use super::*;
    use crate::test_support::imported;

    struct App {
        session: Session,
        registry: CommandRegistry,
        thumbs: ThumbTextures,
        grid: GridState,
        film: GridState,
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
            film: GridState::default(),
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

    fn filmstrip_harness(session: Session) -> Harness<'static, App> {
        Harness::builder()
            .with_size(vec2(700.0, 96.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    filmstrip(
                        ui,
                        &mut app.session,
                        &mut app.thumbs,
                        &mut app.film,
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
    fn clicking_a_filmstrip_cell_selects_that_model() {
        let (_dir, mut session) = imported(&files());
        let root = session.catalog().folders()[0].clone();
        default_registry()
            .run(
                "library.show_folder",
                &mut session,
                &serde_json::json!({ "path": root }),
            )
            .unwrap();
        session.poll();
        let mut harness = filmstrip_harness(session);
        click_every_cell(&mut harness, 30);
    }

    #[test]
    fn double_clicking_opens_that_model_in_the_loupe() {
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
            let app = harness.state();
            assert_eq!(app.session.view, ViewMode::Loupe, "double-clicked {target}");
            assert_eq!(app.current_name().as_deref(), Some(target.as_str()));
            assert_eq!(
                app.session.loupe().id,
                app.session.current(),
                "the loupe loads the clicked model"
            );
        }
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
}

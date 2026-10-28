//! The preview pane on the right: a live preview of the selected file (3D view, photo or ZIP
//! contents) with its details, category, tags and favorite. A clicked folder shows its own.

use std::path::Path;

use egui::{Color32, CornerRadius, Frame, Margin, Rect, RichText, Sense, Ui, vec2};
use polyroom_engine::{Flag, Format, Model, Session};

use crate::layout::{Actions, LABELS, RATE_IDS, section};
use crate::loupe::LoupeView;
use crate::thumbs::ThumbTextures;
use crate::{format, icons, meta_ui, theme};

const DELETE_RED: Color32 = Color32::from_rgb(0xc0, 0x39, 0x2b);

/// What the pane remembers between frames.
#[derive(Default)]
pub struct PreviewState {
    pub tag_input: String,
}

pub fn right_panel(
    ui: &mut Ui,
    session: &mut Session,
    loupe: &mut LoupeView,
    render_state: Option<&egui_wgpu::RenderState>,
    thumbs: &mut ThumbTextures,
    state: &mut PreviewState,
    actions: &mut Actions,
) {
    let mut open = session.panels.right;
    egui::Panel::right("right_panel")
        .default_size(340.0)
        .size_range(260.0..=f32::INFINITY)
        .show_collapsible(ui, &mut open, |ui| {
            if let Some(folder) = session.focused_folder().map(Path::to_path_buf) {
                folder_preview(ui, session, thumbs, state, &folder, actions);
            } else if let Some(model) = session.current().and_then(|id| session.model(id)).cloned()
            {
                model_preview(
                    ui,
                    session,
                    loupe,
                    render_state,
                    thumbs,
                    state,
                    &model,
                    actions,
                );
            } else {
                placeholder(ui);
            }
        });
    if open != session.panels.right {
        actions.push("view.toggle_right_panel");
    }
}

fn placeholder(ui: &mut Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.35).max(0.0));
        ui.label(RichText::new("Select a file to view details").color(theme::TEXT_DIM));
    });
}

/// A value over a dim caption, in a rounded box.
fn tile(ui: &mut Ui, width: f32, value: &str, caption: &str) {
    Frame::new()
        .fill(Color32::from_gray(0x24))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(width - 16.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(value).strong().size(15.0).color(theme::TEXT));
                ui.label(RichText::new(caption).small().color(theme::TEXT_DIM));
            });
        });
}

fn tiles(ui: &mut Ui, first: (&str, &str), second: (&str, &str)) {
    let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
    ui.horizontal(|ui| {
        tile(ui, width, first.0, first.1);
        tile(ui, width, second.0, second.1);
    });
}

/// Footer buttons of equal width; returns the index of the one clicked.
fn footer(ui: &mut Ui, buttons: &[(&str, Option<Color32>)]) -> Option<usize> {
    let gap = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - gap * (buttons.len() as f32 - 1.0)) / buttons.len() as f32;
    let mut clicked = None;
    ui.horizontal(|ui| {
        for (i, (label, fill)) in buttons.iter().enumerate() {
            let mut button = egui::Button::new(RichText::new(*label).color(match fill {
                Some(_) => Color32::WHITE,
                None => theme::TEXT,
            }));
            if let Some(fill) = fill {
                button = button.fill(*fill);
            }
            if ui.add_sized([width, 30.0], button).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

fn footer_panel(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Panel::bottom("preview_footer")
        .frame(Frame::new().inner_margin(Margin::symmetric(0, 8)))
        .show(ui, add);
}

#[allow(clippy::too_many_arguments)]
fn model_preview(
    ui: &mut Ui,
    session: &mut Session,
    loupe: &mut LoupeView,
    render_state: Option<&egui_wgpu::RenderState>,
    thumbs: &mut ThumbTextures,
    state: &mut PreviewState,
    model: &Model,
    actions: &mut Actions,
) {
    footer_panel(ui, |ui| {
        let buttons = [
            ("Open", Some(theme::ACCENT.gamma_multiply(0.8))),
            ("Reveal", None),
            ("Delete", Some(DELETE_RED)),
        ];
        match footer(ui, &buttons) {
            Some(0) => actions.push("file.open"),
            Some(1) => actions.push("file.reveal"),
            Some(2) => actions.push("file.delete"),
            _ => {}
        }
    });

    let selected = session.selection_len();
    let photos = session.stack(model.id).len();
    let contents = archive_contents(session, model);
    let inherited = session.catalog().folder_category(&model.path);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            if model.format.is_archive() {
                loupe.show_archive(ui, session, thumbs);
            } else {
                let height = (width * 0.85).max(180.0);
                ui.allocate_ui(vec2(width, height), |ui| {
                    loupe.show(ui, render_state, session, thumbs, actions);
                });
            }
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(model.name()).strong().size(15.0)).wrap());
            if selected > 1 {
                ui.label(
                    RichText::new(format!("{selected} models selected; edits apply to all"))
                        .color(theme::ACCENT)
                        .small(),
                );
            }
            ui.add_space(6.0);
            tiles(
                ui,
                (&format::bytes(model.size), "File Size"),
                (model.format.label(), "Type"),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                meta_ui::category_combo(
                    ui,
                    "preview-category",
                    session,
                    model.meta.category,
                    inherited,
                    None,
                    actions,
                );
                meta_ui::favorite_button(ui, model.meta.favorite, None, actions);
            });
            ui.add_space(6.0);
            section(ui, "Tags", |ui| {
                meta_ui::tag_editor(ui, &model.meta.tags, &mut state.tag_input, None, actions);
            });
            section(ui, "Rating & Flags", |ui| {
                rating_and_flags(ui, model, actions);
            });
            section(ui, "Metadata", |ui| {
                metadata(ui, model, photos, contents.as_deref());
            });
        });
}

/// What a ZIP holds, once the viewer has listed it.
fn archive_contents(session: &Session, model: &Model) -> Option<String> {
    let loupe = session.loupe();
    let entries = loupe
        .archive
        .as_ref()
        .filter(|_| loupe.id == Some(model.id))?;
    let of =
        |f: fn(Format) -> bool| entries.iter().filter(|e| e.format.is_some_and(f)).count() as u64;
    Some(format!(
        "{} files · {}",
        format::count(entries.len() as u64),
        format::contents(of(Format::is_mesh), of(|f| f == Format::Image), 0, 0)
    ))
}

fn rating_and_flags(ui: &mut Ui, model: &Model, actions: &mut Actions) {
    // Clickable stars; clicking the current rating clears it, as in Lightroom.
    let (rect, response) = ui.allocate_exact_size(vec2(120.0, 20.0), Sense::click());
    let hover = response.hover_pos();
    let rects = icons::stars(
        ui.painter(),
        rect.left_center(),
        16.0,
        model.rating,
        theme::TEXT,
        Color32::from_gray(0x55),
    );
    if response.clicked()
        && let Some(i) = hover.and_then(|p| rects.iter().position(|r| r.contains(p)))
    {
        let n = i + 1;
        let rating = if usize::from(model.rating) == n { 0 } else { n };
        actions.push(RATE_IDS.get(rating).copied().unwrap_or("model.rate.0"));
    }
    ui.horizontal(|ui| {
        for (flag, set, text) in [
            (Flag::Pick, "model.flag.pick", "Pick"),
            (Flag::Reject, "model.flag.reject", "Reject"),
        ] {
            if ui.selectable_label(model.flag == flag, text).clicked() {
                actions.push(if model.flag == flag {
                    "model.flag.none"
                } else {
                    set
                });
            }
        }
    });
    ui.horizontal(|ui| {
        for (label, id) in LABELS {
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            let color = icons::label_color(label);
            ui.painter()
                .rect_filled(rect.shrink(2.0), CornerRadius::same(3), color);
            if model.label == Some(label) {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(3),
                    egui::Stroke::new(1.5, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
            if response.on_hover_text(label.name()).clicked() {
                actions.push(id);
            }
        }
    });
}

fn metadata(ui: &mut Ui, model: &Model, photos: usize, contents: Option<&str>) {
    egui::Grid::new("metadata")
        .num_columns(2)
        .spacing(vec2(10.0, 4.0))
        .show(ui, |ui| {
            let mut row = |key: &str, value: String| {
                ui.label(RichText::new(key).color(theme::TEXT_DIM));
                ui.add(egui::Label::new(value).truncate());
                ui.end_row();
            };
            row("File", model.name());
            // Relative to the imported folder; the full path is in the tooltip.
            let root = model
                .folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let rel = model
                .path
                .parent()
                .and_then(|p| p.strip_prefix(&model.folder).ok())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let folder = if rel.is_empty() {
                root
            } else {
                format!("{root}\\{rel}")
            };
            ui.label(RichText::new("Folder").color(theme::TEXT_DIM));
            ui.add(egui::Label::new(folder).truncate())
                .on_hover_text(model.path.display().to_string());
            ui.end_row();
            let mut row = |key: &str, value: String| {
                ui.label(RichText::new(key).color(theme::TEXT_DIM));
                ui.add(egui::Label::new(value).truncate());
                ui.end_row();
            };
            row("Added", format::date(model.added));
            if let Some(m) = model.modified {
                row("Modified", format::date(m));
            }
            if photos > 0 {
                row("Photos", format::count(photos as u64));
            }
            if let Some(contents) = contents {
                row("Contents", contents.to_owned());
            }
            match &model.stats {
                Some(s) => {
                    row("Dimensions", format::dimensions(s));
                    row("Triangles", format::count(s.triangles));
                    row("Volume", format::volume(s.volume));
                    row("Surface", format::area(s.area));
                }
                None if model.error.is_none() && model.format.is_mesh() => {
                    row("Geometry", "loading…".into());
                }
                None => {}
            }
        });
    if let Some(error) = &model.error {
        ui.add_space(4.0);
        ui.colored_label(
            Color32::from_rgb(0xe0, 0x80, 0x60),
            format!("Can't load: {error}"),
        );
    }
}

fn folder_preview(
    ui: &mut Ui,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    state: &mut PreviewState,
    folder: &Path,
    actions: &mut Actions,
) {
    footer_panel(ui, |ui| {
        let buttons = [
            ("Reveal", None),
            ("Remove", None),
            ("Delete", Some(DELETE_RED)),
        ];
        match footer(ui, &buttons) {
            Some(0) => actions.push("file.reveal"),
            Some(1) => actions.push("library.remove_folder"),
            Some(2) => actions.push("file.delete"),
            _ => {}
        }
    });
    let Some(node) = session.folder(folder).cloned() else {
        placeholder(ui);
        return;
    };
    let meta = session
        .catalog()
        .folder_meta(folder)
        .cloned()
        .unwrap_or_default();
    let inherited = session.catalog().folder_category(folder);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(width, width * 0.6), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), Color32::from_gray(0x1c));
            if let Some(&id) = node.samples.first() {
                let side = rect.height().min(rect.width());
                let square = Rect::from_center_size(rect.center(), vec2(side, side));
                crate::grid::thumbnail(ui, square, id, session, thumbs);
            }
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(&node.name).strong().size(15.0)).wrap());
            ui.label(
                RichText::new(folder.display().to_string())
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(6.0);
            tiles(
                ui,
                (&format::count(node.total as u64), "Files"),
                (&format::bytes(node.bytes), "Size"),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                meta_ui::category_combo(
                    ui,
                    "preview-folder-category",
                    session,
                    meta.category,
                    inherited,
                    Some(folder),
                    actions,
                );
                meta_ui::favorite_button(ui, meta.favorite, Some(folder), actions);
            });
            ui.add_space(6.0);
            section(ui, "Tags", |ui| {
                meta_ui::tag_editor(ui, &meta.tags, &mut state.tag_input, Some(folder), actions);
            });
            ui.label(
                RichText::new(format::contents(
                    node.total
                        .saturating_sub(node.images + node.archives + node.others)
                        as u64,
                    node.images as u64,
                    node.archives as u64,
                    node.others as u64,
                ))
                .color(theme::TEXT_DIM),
            );
        });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use polyroom_engine::{CommandRegistry, default_registry};
    use serde_json::{Value, json};

    use super::*;
    use crate::layout::{LeftState, left_panel};
    use crate::test_support::imported;
    use crate::tree::TreeState;

    struct App {
        session: Session,
        registry: CommandRegistry,
        loupe: LoupeView,
        thumbs: ThumbTextures,
        state: PreviewState,
        /// Where the right panel's left edge is, i.e. what the centre view's right edge is.
        right_panel_x: f32,
        left_panel_x: f32,
    }

    fn harness() -> Harness<'static, App> {
        let long = format!("{}.stl", "a_very_long_model_name_".repeat(6));
        let (dir, session) = imported(&[
            "short.stl".to_owned(),
            long,
            format!("deeply/nested/{}/x.stl", "folder_".repeat(12)),
        ]);
        std::mem::forget(dir);
        let mut harness = Harness::builder()
            .with_size(vec2(1300.0, 700.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    let mut tree = TreeState::default();
                    let mut left = LeftState::default();
                    left_panel(ui, &app.session, &mut tree, &mut left, &mut actions);
                    app.left_panel_x = ui.available_rect_before_wrap().left();
                    right_panel(
                        ui,
                        &mut app.session,
                        &mut app.loupe,
                        None,
                        &mut app.thumbs,
                        &mut app.state,
                        &mut actions,
                    );
                    app.right_panel_x = ui.available_rect_before_wrap().right();
                    for (id, args) in actions.list {
                        app.registry.run(id, &mut app.session, &args).unwrap();
                    }
                },
                App {
                    session,
                    registry: default_registry(),
                    loupe: LoupeView::default(),
                    thumbs: ThumbTextures::default(),
                    state: PreviewState::default(),
                    right_panel_x: 0.0,
                    left_panel_x: 0.0,
                },
            );
        let app = harness.state_mut();
        app.registry
            .run("library.show_all", &mut app.session, &Value::Null)
            .unwrap();
        harness.run_steps(5);
        harness
    }

    fn run(harness: &mut Harness<'_, App>, id: &str, args: Value) {
        let app = harness.state_mut();
        app.registry.run(id, &mut app.session, &args).unwrap();
        harness.run_steps(5);
    }

    /// Drags a panel edge from `from` to `to` (x positions) with the mouse.
    fn drag_edge(harness: &mut Harness<'_, App>, from: f32, to: f32) {
        let y = 300.0;
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(if pressed { from } else { to }, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        harness.event(egui::Event::PointerMoved(egui::pos2(from, y)));
        harness.step();
        harness.event(button(true));
        harness.step();
        for i in 1..=4 {
            let x = from + (to - from) * i as f32 / 4.0;
            harness.event(egui::Event::PointerMoved(egui::pos2(x, y)));
            harness.step();
        }
        harness.event(button(false));
        harness.run_steps(5);
    }

    /// Selects every model in turn, then all at once; neither side panel may move.
    fn assert_selecting_keeps_panels_put(harness: &mut Harness<'_, App>, when: &str) {
        let (left, right) = (harness.state().left_panel_x, harness.state().right_panel_x);
        let models: Vec<_> = harness
            .state()
            .session
            .catalog()
            .models()
            .iter()
            .map(|m| (m.id, m.name()))
            .collect();
        for (id, name) in models {
            run(harness, "select.model", json!({ "id": id }));
            let app = harness.state();
            assert_eq!(
                (app.left_panel_x, app.right_panel_x),
                (left, right),
                "{when}: a side panel moved after selecting {name}"
            );
        }
        run(harness, "select.all", Value::Null);
        let app = harness.state();
        assert_eq!(
            (app.left_panel_x, app.right_panel_x),
            (left, right),
            "{when}: a side panel moved after select all"
        );
    }

    /// Regression: the side panels resized themselves to whatever the selected model's details
    /// needed, so clicking another model moved the panel edge the user had just dragged.
    #[test]
    fn side_panels_keep_their_width_when_the_selection_changes() {
        let mut harness = harness();
        assert_selecting_keeps_panels_put(&mut harness, "default widths");

        for (target, name) in [(900.0, "dragged wider"), (1000.0, "dragged narrower")] {
            let edge = harness.state().right_panel_x;
            drag_edge(&mut harness, edge, target);
            let moved = harness.state().right_panel_x;
            assert!(
                (moved - target).abs() < 2.0,
                "{name}: the drag should have put the edge at {target}, but it is at {moved}"
            );
            assert_selecting_keeps_panels_put(&mut harness, name);
        }
    }

    #[test]
    fn the_pane_shows_the_selected_model_and_its_buttons_ask_before_deleting() {
        let mut harness = harness();
        let id = harness.state().session.visible()[0];
        run(&mut harness, "select.model", json!({ "id": id }));
        let name = harness.state().session.model(id).unwrap().name();
        assert!(harness.query_all_by_label(&name).count() >= 1);
        harness.get_by_label("Open");
        harness.get_by_label("Reveal");

        harness.get_by_label("Delete").click();
        harness.run_steps(3);
        let requests = harness.state_mut().session.drain_requests();
        assert!(
            matches!(
                requests.as_slice(),
                [polyroom_engine::Request::ConfirmDelete(_)]
            ),
            "{requests:?}"
        );
    }

    #[test]
    fn a_clicked_folder_shows_its_own_details() {
        let mut harness = harness();
        let root = harness.state().session.catalog().folders()[0].clone();
        run(&mut harness, "library.show_folder", json!({ "path": root }));
        let sub = harness.state().session.visible_folders()[0].clone();
        run(&mut harness, "select.folder", json!({ "path": sub }));
        harness.get_by_label("Files");
        harness.get_by_label("Remove");
        assert!(
            harness.query_by_label("Type").is_none(),
            "a folder has no type"
        );
    }
}

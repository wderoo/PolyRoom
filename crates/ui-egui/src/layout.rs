//! The Lightroom-style window layout: module bar, side panels, centre view and filmstrip.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Ui, vec2};
use polyroom_engine::{
    AttributeFilter, Collection, ColorLabel, Flag, Format, Module, Session, ViewMode,
};
use serde_json::{Value, json};

use crate::tree::{self, TreeState};
use crate::{format, icons, theme};

/// Commands requested while drawing; dispatched by the app after the frame.
#[derive(Default)]
pub struct Actions {
    pub list: Vec<(&'static str, Value)>,
}

impl Actions {
    pub fn push(&mut self, id: &'static str) {
        self.list.push((id, Value::Null));
    }

    pub fn push_args(&mut self, id: &'static str, args: Value) {
        self.list.push((id, args));
    }
}

/// Top bar: app name on the left, module switcher on the right (like Lightroom's module picker).
pub fn module_bar(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    egui::Panel::top("module_bar")
        .frame(
            Frame::new()
                .fill(theme::HEADER)
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("PolyRoom")
                        .size(20.0)
                        .strong()
                        .color(theme::TEXT),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled(false, module_tab("Print Prep", false));
                    ui.label(RichText::new("|").color(theme::TEXT_DIM));
                    if ui
                        .add(module_tab("Viewer", session.module == Module::Viewer))
                        .clicked()
                    {
                        actions.push("module.viewer");
                    }
                    ui.label(RichText::new("|").color(theme::TEXT_DIM));
                    if ui
                        .add(module_tab("Library", session.module == Module::Library))
                        .clicked()
                    {
                        actions.push("module.library");
                    }
                });
            });
        });
}

fn module_tab(label: &str, active: bool) -> egui::Button<'static> {
    let color = if active { theme::TEXT } else { theme::TEXT_DIM };
    egui::Button::new(RichText::new(label.to_owned()).size(16.0).color(color)).frame(false)
}

/// A row with a label on the left and a dim count on the right.
fn counted_row(ui: &mut Ui, selected: bool, label: &str, count: usize) -> egui::Response {
    ui.add(
        egui::Button::selectable(selected, label)
            .right_text(RichText::new(count.to_string()).color(theme::TEXT_DIM))
            .truncate()
            .min_size(vec2(ui.available_width(), 0.0)),
    )
}

pub fn left_panel(
    ui: &mut Ui,
    session: &Session,
    tree_state: &mut TreeState,
    actions: &mut Actions,
) {
    // Counts are cheap to compute even for tens of thousands of models.
    let models = session.catalog().models();
    let last_import = session.catalog().last_import();
    let total = models.len();
    let previous = models.iter().filter(|m| m.import == last_import).count();
    let collection = session.collection().clone();
    let current_folder = match &collection {
        Collection::Folder(p) => Some(p.as_path()),
        _ => None,
    };
    tree_state.track(&collection);

    let mut open = session.panels.left;
    egui::Panel::left("left_panel")
        .default_size(260.0)
        .size_range(180.0..=480.0)
        .show_collapsible(ui, &mut open, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    section(ui, "Catalog", |ui| {
                        if counted_row(ui, collection == Collection::All, "All Models", total)
                            .clicked()
                        {
                            actions.push("library.show_all");
                        }
                        if counted_row(
                            ui,
                            collection == Collection::PreviousImport,
                            "Previous Import",
                            previous,
                        )
                        .clicked()
                        {
                            actions.push("library.show_previous_import");
                        }
                    });
                    section(ui, "Folders", |ui| {
                        if session.folder_tree().is_empty() {
                            ui.label(RichText::new("No folders imported").color(theme::TEXT_DIM));
                        }
                        tree::folder_tree(
                            ui,
                            session.folder_tree(),
                            current_folder,
                            tree_state,
                            actions,
                        );
                        ui.add_space(4.0);
                        if ui.button("+ Import Folder…").clicked() {
                            actions.push("library.import_folder");
                        }
                    });
                });
        });
    if open != session.panels.left {
        actions.push("view.toggle_left_panel");
    }
}

/// Clickable path segments for the folder being browsed: `3D › Halloween › skeleton`.
pub fn breadcrumbs(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let Collection::Folder(current) = session.collection() else {
        return;
    };
    let Some(root) = session.folder_root(current) else {
        return;
    };
    let mut crumbs: Vec<&std::path::Path> = current
        .ancestors()
        .take_while(|p| p.starts_with(root))
        .collect();
    crumbs.reverse();
    Frame::new()
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, up) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                let enabled = crumbs.len() > 1;
                let color = match (enabled, up.hovered()) {
                    (false, _) => Color32::from_gray(0x50),
                    (true, true) => theme::TEXT,
                    (true, false) => theme::TEXT_DIM,
                };
                icons::up_arrow(ui.painter(), rect, color);
                if enabled && up.on_hover_text("Up one folder (Backspace)").clicked() {
                    actions.push("library.folder_up");
                }
                for (i, path) in crumbs.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("›").color(theme::TEXT_DIM));
                    }
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    let last = i + 1 == crumbs.len();
                    let text = if last {
                        RichText::new(name).strong().color(theme::TEXT)
                    } else {
                        RichText::new(name).color(theme::TEXT_DIM)
                    };
                    let crumb = ui.add(egui::Button::new(text).frame(false));
                    if crumb.clicked() && !last {
                        actions.push_args("library.show_folder", json!({ "path": path }));
                    }
                    crumb.context_menu(|ui| tree::folder_menu(ui, path, actions));
                }
                // Actions for the folder being browsed.
                let (dots, _) = ui.allocate_exact_size(vec2(22.0, 18.0), Sense::hover());
                let id = ui.make_persistent_id("crumb-dots");
                tree::dots_button(ui, dots, id, current, true, actions);
            });
        });
}

pub fn right_panel(ui: &mut Ui, session: &mut Session, actions: &mut Actions) {
    let model = session.current().and_then(|id| session.model(id)).cloned();
    let selected = session.selection_len();
    let photos = model.as_ref().map_or(0, |m| session.stack(m.id).len());
    // What a ZIP holds, once the loupe has listed it.
    let loupe = session.loupe();
    let contents = loupe
        .archive
        .as_ref()
        .filter(|_| model.as_ref().is_some_and(|m| loupe.id == Some(m.id)))
        .map(|entries| {
            let of = |f: fn(Format) -> bool| {
                entries.iter().filter(|e| e.format.is_some_and(f)).count() as u64
            };
            format!(
                "{} files · {}",
                format::count(entries.len() as u64),
                format::contents(of(Format::is_mesh), of(|f| f == Format::Image), 0)
            )
        });
    egui::Panel::right("right_panel")
        .default_size(270.0)
        .size_range(220.0..=440.0)
        .show_collapsible(ui, &mut session.panels.right, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let Some(model) = model else {
                        section(ui, "Metadata", |ui| {
                            ui.label(
                                RichText::new("Select a model to see its details")
                                    .color(theme::TEXT_DIM),
                            );
                        });
                        return;
                    };
                    if selected > 1 {
                        ui.label(
                            RichText::new(format!(
                                "{selected} models selected; edits apply to all"
                            ))
                            .color(theme::ACCENT)
                            .small(),
                        );
                        ui.add_space(4.0);
                    }
                    section(ui, "Rating & Flags", |ui| {
                        // Clickable stars; clicking the current rating clears it, as in Lightroom.
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(120.0, 20.0), Sense::click());
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
                            && let Some(i) =
                                hover.and_then(|p| rects.iter().position(|r| r.contains(p)))
                        {
                            let n = i + 1;
                            let rating = if usize::from(model.rating) == n { 0 } else { n };
                            actions.push(RATE_IDS.get(rating).copied().unwrap_or("model.rate.0"));
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(model.flag == Flag::Pick, "Pick")
                                .clicked()
                            {
                                actions.push(if model.flag == Flag::Pick {
                                    "model.flag.none"
                                } else {
                                    "model.flag.pick"
                                });
                            }
                            if ui
                                .selectable_label(model.flag == Flag::Reject, "Reject")
                                .clicked()
                            {
                                actions.push(if model.flag == Flag::Reject {
                                    "model.flag.none"
                                } else {
                                    "model.flag.reject"
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            for (label, id) in LABELS {
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                let color = icons::label_color(label);
                                ui.painter().rect_filled(
                                    rect.shrink(2.0),
                                    CornerRadius::same(3),
                                    color,
                                );
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
                    });
                    section(ui, "Metadata", |ui| {
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
                                row("Format", model.format.label().to_owned());
                                row("File size", format::bytes(model.size));
                                if let Some(m) = model.modified {
                                    row("Modified", format::date(m));
                                }
                                if photos > 0 {
                                    row("Photos", format::count(photos as u64));
                                }
                                if let Some(contents) = &contents {
                                    row("Contents", contents.clone());
                                }
                                match &model.stats {
                                    Some(s) => {
                                        row("Dimensions", format::dimensions(s));
                                        row("Triangles", format::count(s.triangles));
                                        row("Volume", format::volume(s.volume));
                                        row("Surface", format::area(s.area));
                                    }
                                    None if model.error.is_none() && model.format.is_mesh() => {
                                        row("Geometry", "loading…".into())
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
                    });
                });
        });
}

const RATE_IDS: [&str; 6] = [
    "model.rate.0",
    "model.rate.1",
    "model.rate.2",
    "model.rate.3",
    "model.rate.4",
    "model.rate.5",
];

const LABELS: [(ColorLabel, &str); 5] = [
    (ColorLabel::Red, "model.label.red"),
    (ColorLabel::Yellow, "model.label.yellow"),
    (ColorLabel::Green, "model.label.green"),
    (ColorLabel::Blue, "model.label.blue"),
    (ColorLabel::Purple, "model.label.purple"),
];

/// The toolbar under the centre view: view switcher, filter box, counts and thumbnail size.
pub fn toolbar(ui: &mut Ui, session: &Session, filter: &mut String, actions: &mut Actions) {
    egui::Panel::bottom("toolbar")
        .frame(
            Frame::new()
                .fill(theme::PANEL)
                .inner_margin(Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(session.view == ViewMode::Grid, "Grid")
                    .on_hover_text("G")
                    .clicked()
                {
                    actions.push("view.grid");
                }
                if ui
                    .selectable_label(session.view == ViewMode::Loupe, "Loupe")
                    .on_hover_text("E")
                    .clicked()
                {
                    actions.push("view.loupe");
                }
                ui.separator();
                let edit = egui::TextEdit::singleline(filter)
                    .hint_text("Filter by name")
                    .desired_width(180.0);
                if ui.add(edit).changed() {
                    actions.push_args("library.filter", json!({ "text": filter.as_str() }));
                }
                let total = session.visible().len();
                let selected = session.selection_len();
                let models = format::models(total as u64);
                let counts = if selected > 1 {
                    format!("{models} · {selected} selected")
                } else {
                    models
                };
                ui.label(RichText::new(counts).color(theme::TEXT_DIM));

                if session.view == ViewMode::Grid {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut size = session.thumb_size;
                        let slider =
                            egui::Slider::new(&mut size, Session::THUMB_MIN..=Session::THUMB_MAX)
                                .show_value(false);
                        if ui.add(slider).changed() {
                            actions.push_args("view.thumb_size", json!({ "size": size }));
                        }
                        ui.label(RichText::new("Thumbnails").color(theme::TEXT_DIM));
                    });
                }
            });
            ui.add_space(4.0);
            filter_row(ui, session.attributes(), filter, actions);
        });
}

/// Metadata filter under the toolbar: minimum stars, flags, colour labels and formats.
/// Each control toggles one choice of the engine's attribute filter.
pub fn filter_row(
    ui: &mut Ui,
    attrs: &AttributeFilter,
    text_filter: &mut String,
    actions: &mut Actions,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Filter").color(theme::TEXT_DIM));

        // Stars: "this many or more". Clicking the chosen one again clears it.
        let (rect, response) = ui.allocate_exact_size(vec2(88.0, 20.0), Sense::click());
        let hover = response.hover_pos();
        let rects = icons::stars(
            ui.painter(),
            rect.left_center(),
            14.0,
            attrs.min_rating,
            theme::TEXT,
            Color32::from_gray(0x55),
        );
        let hovered = hover.and_then(|p| rects.iter().position(|r| r.contains(p)));
        if let Some(i) = hovered {
            response.clone().on_hover_text(match i {
                0 => "1 star or more".to_owned(),
                n => format!("{} stars or more", n + 1),
            });
        }
        if response.clicked()
            && let Some(i) = hovered
        {
            actions.push_args("filter.rating", json!({ "min": i + 1 }));
        }
        ui.separator();

        for (flag, key, text) in [
            (Flag::Pick, "pick", "Pick"),
            (Flag::None, "none", "Unflagged"),
            (Flag::Reject, "reject", "Rejected"),
        ] {
            if ui
                .selectable_label(attrs.flags.contains(&flag), text)
                .clicked()
            {
                actions.push_args("filter.flag", json!({ "flag": key }));
            }
        }
        ui.separator();

        for (label, key) in LABELS_BY_NAME {
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            let chosen = attrs.labels.contains(&label);
            let color = icons::label_color(label);
            ui.painter().rect_filled(
                rect.shrink(2.0),
                CornerRadius::same(3),
                if chosen {
                    color
                } else {
                    color.gamma_multiply(0.45)
                },
            );
            if chosen {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(3),
                    egui::Stroke::new(1.5, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
            if response.on_hover_text(label.name()).clicked() {
                actions.push_args("filter.label", json!({ "label": key }));
            }
        }
        ui.separator();

        for format in Format::ALL {
            if ui
                .selectable_label(attrs.formats.contains(&format), format.label())
                .clicked()
            {
                actions.push_args("filter.format", json!({ "format": format.label() }));
            }
        }

        if (attrs.is_active() || !text_filter.is_empty())
            && ui
                .button("Clear")
                .on_hover_text("Remove all filters")
                .clicked()
        {
            if attrs.is_active() {
                actions.push("filter.clear");
            }
            text_filter.clear();
            actions.push_args("library.filter", json!({ "text": "" }));
        }
    });
}

const LABELS_BY_NAME: [(ColorLabel, &str); 5] = [
    (ColorLabel::Red, "red"),
    (ColorLabel::Yellow, "yellow"),
    (ColorLabel::Green, "green"),
    (ColorLabel::Blue, "blue"),
    (ColorLabel::Purple, "purple"),
];
pub fn empty_library(ui: &mut Ui, actions: &mut Actions) {
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() * 0.35).max(0.0));
        ui.label(RichText::new("Your library is empty").size(22.0));
        ui.add_space(6.0);
        ui.label(
            RichText::new("Import a folder of STL, 3MF or OBJ files to get started.")
                .color(theme::TEXT_DIM),
        );
        ui.add_space(14.0);
        let button = egui::Button::new(RichText::new("Import Folder…").size(16.0))
            .min_size(vec2(160.0, 36.0));
        if ui.add(button).clicked() {
            actions.push("library.import_folder");
        }
    });
}

pub fn no_matches(ui: &mut Ui) {
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new("No models match").color(theme::TEXT_DIM));
    });
}

/// A Lightroom-style panel section: a collapsible header strip.
fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, body);
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn active_filter() -> AttributeFilter {
        let mut f = AttributeFilter::default();
        f.toggle_min_rating(3);
        f.toggle_flag(Flag::Pick);
        f.toggle_label(ColorLabel::Green);
        f.toggle_format(Format::Stl);
        f
    }

    #[test]
    fn clear_appears_only_while_filtering_and_the_row_fits() {
        let _gpu = crate::GPU_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (i, attrs) in [AttributeFilter::default(), active_filter()]
            .into_iter()
            .enumerate()
        {
            let mut harness = egui_kittest::Harness::builder()
                .with_size(vec2(420.0, 70.0))
                .build_ui(|ui| {
                    crate::theme::apply(ui.ctx());
                    let mut text = String::new();
                    filter_row(ui, &attrs, &mut text, &mut Actions::default());
                });
            harness.run();
            assert_eq!(harness.query_by_label("Clear").is_some(), i == 1);
            assert!(harness.query_by_label("Unflagged").is_some());
            if let Ok(image) = harness.render() {
                let out = std::env::temp_dir().join(format!("polyroom_filter_row_{i}.png"));
                image.save(&out).unwrap();
                eprintln!("rendered {}", out.display());
            }
        }
    }
}

#[cfg(test)]
mod panel_tests {
    use egui_kittest::Harness;
    use polyroom_engine::{CommandRegistry, default_registry};
    use serde_json::json;

    use super::*;
    use crate::test_support::imported;

    struct App {
        session: Session,
        registry: CommandRegistry,
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
        // Keep the folder alive for the whole test; the catalog only needs the paths, but
        // this avoids surprises if that changes.
        std::mem::forget(dir);
        let mut harness = Harness::builder()
            .with_size(vec2(1300.0, 600.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    let mut tree = TreeState::default();
                    left_panel(ui, &app.session, &mut tree, &mut actions);
                    app.left_panel_x = ui.available_rect_before_wrap().left();
                    right_panel(ui, &mut app.session, &mut actions);
                    app.right_panel_x = ui.available_rect_before_wrap().right();
                    for (id, args) in actions.list {
                        app.registry.run(id, &mut app.session, &args).unwrap();
                    }
                },
                App {
                    session,
                    registry: default_registry(),
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
}

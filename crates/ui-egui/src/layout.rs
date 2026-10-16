//! The Lightroom-style window layout: module bar, side panels, centre view and filmstrip.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Ui, vec2};
use polyroom_engine::{Collection, ColorLabel, Flag, Module, Session, ViewMode};
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
            egui::ScrollArea::vertical().show(ui, |ui| {
                section(ui, "Catalog", |ui| {
                    if counted_row(ui, collection == Collection::All, "All Models", total).clicked()
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
    egui::Panel::right("right_panel")
        .default_size(270.0)
        .size_range(220.0..=440.0)
        .show_collapsible(ui, &mut session.panels.right, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
                        RichText::new(format!("{selected} models selected; edits apply to all"))
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
                            match &model.stats {
                                Some(s) => {
                                    row("Dimensions", format::dimensions(s));
                                    row("Triangles", format::count(s.triangles));
                                    row("Volume", format::volume(s.volume));
                                    row("Surface", format::area(s.area));
                                }
                                None if model.error.is_none() => row("Geometry", "loading…".into()),
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
        });
}

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

//! The library navigation in the left panel, and the pieces the other panels share.

use egui::{Align, Color32, Layout, Rect, RichText, Sense, Ui, pos2, vec2};
use polyroom_engine::{Collection, ColorLabel, Session};
use serde_json::{Value, json};

use crate::meta_ui::category_color;
use crate::tree::{self, TreeState};
use crate::{icons, theme};

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

pub const RATE_IDS: [&str; 6] = [
    "model.rate.0",
    "model.rate.1",
    "model.rate.2",
    "model.rate.3",
    "model.rate.4",
    "model.rate.5",
];

pub const LABELS: [(ColorLabel, &str); 5] = [
    (ColorLabel::Red, "model.label.red"),
    (ColorLabel::Yellow, "model.label.yellow"),
    (ColorLabel::Green, "model.label.green"),
    (ColorLabel::Blue, "model.label.blue"),
    (ColorLabel::Purple, "model.label.purple"),
];

pub const LABELS_BY_NAME: [(ColorLabel, &str); 5] = [
    (ColorLabel::Red, "red"),
    (ColorLabel::Yellow, "yellow"),
    (ColorLabel::Green, "green"),
    (ColorLabel::Blue, "blue"),
    (ColorLabel::Purple, "purple"),
];

/// What the left panel remembers between frames.
#[derive(Default)]
pub struct LeftState {
    adding_category: bool,
    focus_new_category: bool,
    new_category: String,
}

/// What a navigation row shows in front of its label.
#[derive(Clone, Copy)]
enum Mark {
    None,
    Dot(Color32),
    Heart,
}

/// A row of the navigation: mark, label and a dim count on the right.
fn nav_row(ui: &mut Ui, selected: bool, mark: Mark, label: &str, count: usize) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let mut x = rect.left() + 8.0;
        let mid = rect.center().y;
        match mark {
            Mark::None => {}
            Mark::Dot(color) => {
                painter.circle_filled(pos2(x + 4.0, mid), 4.0, color);
                x += 16.0;
            }
            Mark::Heart => {
                let heart = Rect::from_min_size(pos2(x, mid - 6.0), vec2(13.0, 12.0));
                icons::heart(painter, heart, icons::HEART);
                x += 20.0;
            }
        }
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text_color = if selected {
            Color32::WHITE
        } else {
            theme::TEXT
        };
        let count_color = if selected {
            Color32::from_white_alpha(200)
        } else {
            theme::TEXT_DIM
        };
        let count = painter.layout_no_wrap(count.to_string(), font.clone(), count_color);
        let count_pos = pos2(
            rect.right() - 8.0 - count.size().x,
            mid - count.size().y / 2.0,
        );
        let text = painter.layout_no_wrap(label.to_owned(), font, text_color);
        let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(count_pos.x - 4.0, rect.bottom()));
        painter
            .with_clip_rect(clip)
            .galley(pos2(x, mid - text.size().y / 2.0), text, text_color);
        painter.galley(count_pos, count, count_color);
    }
    response
}

pub fn left_panel(
    ui: &mut Ui,
    session: &Session,
    tree_state: &mut TreeState,
    state: &mut LeftState,
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
                    library_section(ui, &collection, total, previous, actions);
                    favorites_row(ui, session, actions);
                    categories_section(ui, session, state, actions);
                    tags_section(ui, session, actions);
                    section(ui, "Folders", |ui| {
                        if session.folder_tree().is_empty() {
                            ui.label(RichText::new("No folders imported").color(theme::TEXT_DIM));
                        }
                        tree::folder_tree(
                            ui,
                            session.folder_tree(),
                            current_folder,
                            session,
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

fn library_section(
    ui: &mut Ui,
    collection: &Collection,
    total: usize,
    previous: usize,
    actions: &mut Actions,
) {
    section(ui, "Library", |ui| {
        let all = nav_row(
            ui,
            *collection == Collection::All,
            Mark::None,
            "All Models",
            total,
        );
        if all.clicked() {
            actions.push("library.show_all");
        }
        let recent = nav_row(
            ui,
            *collection == Collection::PreviousImport,
            Mark::None,
            "Previous Import",
            previous,
        );
        if recent.clicked() {
            actions.push("library.show_previous_import");
        }
    });
}

/// Filters the grid to favorites; the choice is toggled, like the other filters.
fn favorites_row(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let chosen = session.attributes().favorites;
    let row = nav_row(
        ui,
        chosen,
        Mark::Heart,
        "Favorites",
        session.favorite_count(),
    );
    if row.on_hover_text("Show only favorites").clicked() {
        actions.push("filter.favorites");
    }
    ui.add_space(4.0);
}

fn categories_section(
    ui: &mut Ui,
    session: &Session,
    state: &mut LeftState,
    actions: &mut Actions,
) {
    section(ui, "Categories", |ui| {
        let chosen = &session.attributes().categories;
        for category in session.catalog().categories() {
            let row = nav_row(
                ui,
                chosen.contains(&Some(category.id)),
                Mark::Dot(category_color(category)),
                &category.name,
                session.category_count(Some(category.id)),
            );
            if row.clicked() {
                actions.push_args("filter.category", json!({ "id": category.id }));
            }
            row.context_menu(|ui| category_menu(ui, category.id, &category.name, actions));
        }
        let uncategorized = nav_row(
            ui,
            chosen.contains(&None),
            Mark::Dot(Color32::from_gray(0x70)),
            "Uncategorized",
            session.category_count(None),
        );
        if uncategorized.clicked() {
            actions.push_args("filter.category", json!({ "id": null }));
        }
        add_category_row(ui, state, actions);
    });
}

/// Rename and delete, in the right-click menu of a category.
fn category_menu(ui: &mut Ui, id: u32, name: &str, actions: &mut Actions) {
    let key = ui.make_persistent_id(("rename-category", id));
    let mut text: String = ui
        .data_mut(|d| d.get_temp(key))
        .unwrap_or_else(|| name.to_owned());
    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(140.0));
    ui.data_mut(|d| d.insert_temp(key, text.clone()));
    let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if (ui.button("Rename").clicked() || entered) && !text.trim().is_empty() {
        actions.push_args("category.rename", json!({ "id": id, "name": text.trim() }));
        ui.data_mut(|d| d.remove_temp::<String>(key));
        ui.close();
    }
    if ui.button("Delete Category").clicked() {
        actions.push_args("category.remove", json!({ "id": id }));
        ui.data_mut(|d| d.remove_temp::<String>(key));
        ui.close();
    }
}

fn add_category_row(ui: &mut Ui, state: &mut LeftState, actions: &mut Actions) {
    if !state.adding_category {
        if ui
            .add(
                egui::Button::new(RichText::new("+ Add Category").color(theme::TEXT_DIM))
                    .frame(false),
            )
            .clicked()
        {
            state.adding_category = true;
            state.focus_new_category = true;
        }
        return;
    }
    let edit = ui.add(
        egui::TextEdit::singleline(&mut state.new_category)
            .hint_text("Category name")
            .desired_width(ui.available_width()),
    );
    if std::mem::take(&mut state.focus_new_category) {
        edit.request_focus();
    }
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if enter && edit.lost_focus() && !state.new_category.trim().is_empty() {
        actions.push_args("category.add", json!({ "name": state.new_category.trim() }));
        state.new_category.clear();
        state.adding_category = false;
    } else if escape || edit.lost_focus() && state.new_category.trim().is_empty() {
        state.new_category.clear();
        state.adding_category = false;
    }
}

fn tags_section(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let tags = session.tag_counts();
    if tags.is_empty() {
        return;
    }
    section(ui, "Tags", |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tag, count) in tags {
                let chosen = session.attributes().tags.contains(tag);
                let text = RichText::new(format!("{tag} ({count})")).small();
                if ui.selectable_label(chosen, text).clicked() {
                    actions.push_args("filter.tag", json!({ "tag": tag }));
                }
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

/// A panel section: a collapsible header strip.
pub fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, body);
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use polyroom_engine::{CommandRegistry, default_registry};

    use super::*;
    use crate::test_support::imported;

    struct App {
        session: Session,
        registry: CommandRegistry,
        left: LeftState,
    }

    fn harness() -> Harness<'static, App> {
        let (dir, session) = imported(&["a.stl".to_owned(), "minis/b.stl".to_owned()]);
        std::mem::forget(dir);
        let mut harness = Harness::builder()
            .with_size(vec2(320.0, 700.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    let mut tree = TreeState::default();
                    left_panel(ui, &app.session, &mut tree, &mut app.left, &mut actions);
                    for (id, args) in actions.list {
                        app.registry.run(id, &mut app.session, &args).unwrap();
                    }
                },
                App {
                    session,
                    registry: default_registry(),
                    left: LeftState::default(),
                },
            );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn category_rows_show_counts_and_filter_the_grid_when_clicked() {
        let mut harness = harness();
        {
            let app = harness.state_mut();
            let root = app.session.catalog().folders()[0].clone();
            app.registry
                .run("library.show_all", &mut app.session, &Value::Null)
                .unwrap();
            app.registry
                .run(
                    "category.add",
                    &mut app.session,
                    &json!({ "name": "Miniatures" }),
                )
                .unwrap();
            let id = app.session.catalog().categories()[0].id;
            app.registry
                .run(
                    "meta.category",
                    &mut app.session,
                    &json!({ "id": id, "path": root.join("minis") }),
                )
                .unwrap();
        }
        harness.run_steps(3);
        assert_eq!(harness.state().session.visible().len(), 2);

        harness.get_by_label_contains("Miniatures").click();
        harness.run_steps(3);
        let app = harness.state_mut();
        app.session.poll();
        let shown: Vec<String> = app
            .session
            .visible()
            .iter()
            .filter_map(|&id| Some(app.session.model(id)?.name()))
            .collect();
        assert_eq!(shown, ["b.stl"], "only the model in the categorised folder");
        assert!(app.session.attributes().categories.len() == 1);
    }

    #[test]
    fn the_favorites_row_toggles_the_favorites_filter() {
        let mut harness = harness();
        harness.get_by_label_contains("Favorites").click();
        harness.run_steps(3);
        assert!(harness.state().session.attributes().favorites);
        harness.get_by_label_contains("Favorites").click();
        harness.run_steps(3);
        assert!(!harness.state().session.attributes().favorites);
    }

    #[test]
    fn a_new_category_is_added_from_the_panel() {
        let mut harness = harness();
        let before = harness.state().session.catalog().categories().len();
        harness.get_by_label("+ Add Category").click();
        harness.run_steps(3);
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .type_text("Dragons");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        let cats = harness.state().session.catalog().categories();
        assert_eq!(cats.len(), before + 1);
        assert_eq!(cats.last().map(|c| c.name.as_str()), Some("Dragons"));
    }
}

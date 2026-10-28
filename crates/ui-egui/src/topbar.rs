//! The bar above the grid: breadcrumb, sorting and thumbnail size, filters, and the action bar
//! that appears while something is selected.

use std::path::Path;

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, PopupCloseBehavior, RichText, Sense, Ui,
    vec2,
};
use polyroom_engine::{AttributeFilter, Collection, Flag, Format, Session, SortDir};
use serde_json::json;

use crate::layout::{Actions, LABELS_BY_NAME};
use crate::{format, icons, meta_ui, theme, tree};

/// Text typed into the bar's boxes (the engine holds what is applied).
#[derive(Default)]
pub struct TopbarState {
    pub filter: String,
    pub tag_input: String,
}

const DELETE_RED: Color32 = Color32::from_rgb(0xc0, 0x39, 0x2b);

pub fn topbar(ui: &mut Ui, session: &Session, state: &mut TopbarState, actions: &mut Actions) {
    egui::Panel::top("grid_topbar")
        .frame(
            Frame::new()
                .fill(theme::HEADER)
                .inner_margin(Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            path_and_sort_row(ui, session, actions);
            ui.add_space(6.0);
            filter_row(ui, session, state, actions);
            if session.selection_len() > 1 || session.focused_folder().is_some() {
                ui.add_space(6.0);
                action_bar(ui, session, state, actions);
            }
        });
}

fn path_and_sort_row(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    ui.horizontal(|ui| {
        breadcrumbs(ui, session, actions);
        let items = session.visible().len() + session.visible_folders().len();
        let noun = if items == 1 { "item" } else { "items" };
        ui.label(
            RichText::new(format!("{} {noun}", format::count(items as u64))).color(theme::TEXT_DIM),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut size = session.thumb_size;
            let slider = egui::Slider::new(&mut size, Session::THUMB_MIN..=Session::THUMB_MAX)
                .show_value(false);
            let response = ui.add(slider);
            let label = ui.label(RichText::new("Thumbnails").color(theme::TEXT_DIM));
            if response.labelled_by(label.id).changed() {
                actions.push_args("view.thumb_size", json!({ "size": size }));
            }
            sort_menu(ui, session, actions);
        });
    });
}

/// Clickable path segments for the folder being browsed: `3D › Halloween › skeleton`.
fn breadcrumbs(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let current = match session.collection() {
        Collection::Folder(current) => current,
        Collection::All => {
            ui.label(RichText::new("All Models").strong().color(theme::TEXT));
            return;
        }
        Collection::PreviousImport => {
            ui.label(RichText::new("Previous Import").strong().color(theme::TEXT));
            return;
        }
    };
    let Some(root) = session.folder_root(current) else {
        return;
    };
    let mut crumbs: Vec<&Path> = current
        .ancestors()
        .take_while(|p| p.starts_with(root))
        .collect();
    crumbs.reverse();

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
        crumb.context_menu(|ui| tree::folder_menu(ui, path, session, actions));
    }
    // Actions for the folder being browsed.
    let (dots, _) = ui.allocate_exact_size(vec2(22.0, 18.0), Sense::hover());
    let id = ui.make_persistent_id("crumb-dots");
    tree::dots_button(ui, dots, id, current, true, session, actions);
}

fn sort_menu(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let sort = session.sort();
    let order = match sort.dir {
        SortDir::Ascending => "asc",
        SortDir::Descending => "desc",
    };
    MenuButton::new(format!("Sort: {} ({order})", sort.key.label())).ui(ui, |ui| {
        for key in polyroom_engine::SortKey::ALL {
            if ui.selectable_label(sort.key == key, key.label()).clicked() {
                actions.push_args("sort.by", json!({ "key": key.id() }));
            }
        }
        ui.separator();
        if ui.button("Reverse order").clicked() {
            actions.push("sort.direction");
        }
    });
}

/// A button that opens a menu staying open while choices are toggled.
fn filter_menu(ui: &mut Ui, label: &str, active: usize, content: impl FnOnce(&mut Ui)) {
    let text = if active > 0 {
        RichText::new(format!("{label} ({active})")).color(theme::ACCENT)
    } else {
        RichText::new(label)
    };
    MenuButton::new(text)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, content);
}

fn filter_row(ui: &mut Ui, session: &Session, state: &mut TopbarState, actions: &mut Actions) {
    let attrs = session.attributes();
    ui.horizontal_wrapped(|ui| {
        let edit = egui::TextEdit::singleline(&mut state.filter)
            .hint_text("Search names…")
            .desired_width(200.0);
        if ui.add(edit).changed() {
            actions.push_args("library.filter", json!({ "text": state.filter.as_str() }));
        }

        filter_menu(ui, "Categories", attrs.categories.len(), |ui| {
            categories_menu(ui, session, actions);
        });
        filter_menu(ui, "Tags", attrs.tags.len(), |ui| {
            tags_menu(ui, session, actions);
        });
        filter_menu(ui, "Types", attrs.formats.len(), |ui| {
            for format in Format::ALL {
                if ui
                    .selectable_label(attrs.formats.contains(&format), format.label())
                    .clicked()
                {
                    actions.push_args("filter.format", json!({ "format": format.label() }));
                }
            }
        });
        let rated = usize::from(attrs.min_rating > 0) + attrs.flags.len() + attrs.labels.len();
        filter_menu(ui, "Rating & Flags", rated, |ui| {
            rating_flag_label_controls(ui, attrs, actions);
        });
        if ui
            .selectable_label(attrs.favorites, "Favorites")
            .on_hover_text("Show only favorites")
            .clicked()
        {
            actions.push("filter.favorites");
        }

        if (attrs.is_active() || !state.filter.is_empty())
            && ui
                .button("Clear")
                .on_hover_text("Remove all filters")
                .clicked()
        {
            if attrs.is_active() {
                actions.push("filter.clear");
            }
            state.filter.clear();
            actions.push_args("library.filter", json!({ "text": "" }));
        }
    });
}

fn categories_menu(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let chosen = &session.attributes().categories;
    for category in session.catalog().categories() {
        let text = RichText::new(category.name.clone()).color(meta_ui::category_color(category));
        if ui
            .selectable_label(chosen.contains(&Some(category.id)), text)
            .clicked()
        {
            actions.push_args("filter.category", json!({ "id": category.id }));
        }
    }
    if ui
        .selectable_label(chosen.contains(&None), "Uncategorized")
        .clicked()
    {
        actions.push_args("filter.category", json!({ "id": null }));
    }
}

fn tags_menu(ui: &mut Ui, session: &Session, actions: &mut Actions) {
    let tags = session.tag_counts();
    if tags.is_empty() {
        ui.label(RichText::new("No tags yet").color(theme::TEXT_DIM));
    }
    for (tag, count) in tags {
        let chosen = session.attributes().tags.contains(tag);
        if ui
            .selectable_label(chosen, format!("{tag} ({count})"))
            .clicked()
        {
            actions.push_args("filter.tag", json!({ "tag": tag }));
        }
    }
}

/// Minimum stars, flags and colour labels; each control toggles one choice of the engine's
/// attribute filter.
fn rating_flag_label_controls(ui: &mut Ui, attrs: &AttributeFilter, actions: &mut Actions) {
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

    ui.horizontal(|ui| {
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
    });
    ui.separator();

    ui.horizontal(|ui| {
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
    });
}

/// What the buttons of the action bar act on: the clicked folder, or the selected models.
fn action_bar(ui: &mut Ui, session: &Session, state: &mut TopbarState, actions: &mut Actions) {
    let selected = session.selection_len();
    let focused = session.focused_folder();
    let current_category = match focused {
        Some(folder) => session
            .catalog()
            .folder_meta(folder)
            .and_then(|m| m.category),
        None => session
            .current()
            .and_then(|id| session.model(id))
            .and_then(|m| m.meta.category),
    };
    let favorite = match focused {
        Some(folder) => session
            .catalog()
            .folder_meta(folder)
            .is_some_and(|m| m.favorite),
        None => session
            .current()
            .and_then(|id| session.model(id))
            .is_some_and(|m| m.meta.favorite),
    };
    ui.horizontal_wrapped(|ui| {
        match focused {
            Some(folder) => {
                let name = folder.file_name().map_or_else(
                    || folder.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.label(
                    RichText::new(format!("Folder {name}"))
                        .strong()
                        .color(theme::TEXT),
                );
            }
            None => {
                ui.label(
                    RichText::new(format!("{selected} selected"))
                        .strong()
                        .color(theme::TEXT),
                );
                if ui.button("Select All").clicked() {
                    actions.push("select.all");
                }
                if ui.button("Clear").clicked() {
                    actions.push("select.none");
                }
            }
        }
        ui.separator();
        MenuButton::new("Category").ui(ui, |ui| {
            meta_ui::category_items(ui, session, current_category, None, actions);
        });
        MenuButton::new("Add Tag")
            .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
                ui.set_min_width(180.0);
                meta_ui::add_tag_box(ui, &mut state.tag_input, None, actions);
            });
        meta_ui::favorite_button(ui, favorite, None, actions);
        if ui.button("Regenerate Thumbs").clicked() {
            actions.push("preview.regenerate");
        }
        if ui.button("Reveal").clicked() {
            actions.push("file.reveal");
        }
        let label = match focused {
            Some(_) => "Delete".to_owned(),
            None => format!("Delete ({selected})"),
        };
        let delete = egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(DELETE_RED);
        if ui
            .add(delete)
            .on_hover_text("Move to the Recycle Bin")
            .clicked()
        {
            actions.push("file.delete");
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use polyroom_engine::{CommandRegistry, default_registry};
    use serde_json::Value;

    use super::*;
    use crate::test_support::imported;

    struct App {
        session: Session,
        registry: CommandRegistry,
        state: TopbarState,
    }

    fn harness() -> Harness<'static, App> {
        let files = ["a.stl", "b.stl", "sub/c.stl"].map(String::from);
        let (dir, session) = imported(&files);
        std::mem::forget(dir);
        let mut harness = Harness::builder()
            .with_size(vec2(1100.0, 200.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    crate::theme::apply(ui.ctx());
                    app.session.poll();
                    let mut actions = Actions::default();
                    topbar(ui, &app.session, &mut app.state, &mut actions);
                    for (id, args) in actions.list {
                        app.registry.run(id, &mut app.session, &args).unwrap();
                    }
                },
                App {
                    session,
                    registry: default_registry(),
                    state: TopbarState::default(),
                },
            );
        harness.run_steps(3);
        harness
    }

    fn run(harness: &mut Harness<'_, App>, id: &str, args: Value) {
        let app = harness.state_mut();
        app.registry.run(id, &mut app.session, &args).unwrap();
        app.session.poll();
        harness.run_steps(3);
    }

    #[test]
    fn the_action_bar_appears_for_several_models_or_a_folder() {
        let mut harness = harness();
        // The import selected one model; that needs no bar.
        assert!(harness.query_by_label("Select All").is_none());

        run(&mut harness, "select.all", Value::Null);
        harness.get_by_label_contains("selected");
        harness.get_by_label("Select All");
        harness.get_by_label_contains("Delete (");

        // A clicked folder gets the bar too, and its Delete has no count.
        let sub = harness.state().session.visible_folders()[0].clone();
        run(&mut harness, "select.folder", json!({ "path": sub }));
        assert!(harness.query_by_label("Select All").is_none());
        harness.get_by_label("Delete");
    }

    #[test]
    fn the_breadcrumb_names_the_folder_and_the_sort_button_the_order() {
        let mut harness = harness();
        let root = harness.state().session.catalog().folders()[0].clone();
        let name = root.file_name().unwrap().to_string_lossy().into_owned();
        harness.get_by_label(&name);
        harness.get_by_label("Sort: Name (asc)");

        run(&mut harness, "sort.by", json!({ "key": "size" }));
        harness.get_by_label("Sort: Size (desc)");
        run(&mut harness, "library.show_all", Value::Null);
        harness.get_by_label("All Models");
    }

    #[test]
    fn clear_appears_only_while_filtering() {
        let mut harness = harness();
        assert!(harness.query_by_label("Clear").is_none());
        run(&mut harness, "filter.favorites", Value::Null);
        harness.get_by_label("Clear");
        harness.get_by_label("Favorites");
        run(&mut harness, "filter.clear", Value::Null);
        // Favorites filter is cleared with the rest.
        assert!(harness.query_by_label("Clear").is_none());
    }

    #[test]
    fn filter_buttons_show_how_many_choices_are_active() {
        let mut harness = harness();
        run(&mut harness, "filter.format", json!({ "format": "STL" }));
        harness.get_by_label("Types (1)");
        run(&mut harness, "category.add", json!({ "name": "Art" }));
        let id = harness.state().session.catalog().categories()[0].id;
        run(&mut harness, "filter.category", json!({ "id": id }));
        harness.get_by_label("Categories (1)");
    }
}

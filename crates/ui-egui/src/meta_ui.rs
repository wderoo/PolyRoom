//! Widgets that edit the category, tags and favorite of models and folders. Each one only
//! pushes commands; with a `path` they apply to that folder, otherwise to the selection.

use std::collections::BTreeSet;
use std::path::Path;

use egui::{Color32, Key, RichText, Sense, Ui, vec2};
use polyroom_engine::{Category, CategoryId, Session};
use serde_json::{Value, json};

use crate::layout::Actions;
use crate::{icons, theme};

/// Adds the folder a command applies to, if there is one.
pub fn with_path(mut args: Value, path: Option<&Path>) -> Value {
    if let (Some(path), Some(map)) = (path, args.as_object_mut()) {
        map.insert("path".into(), json!(path));
    }
    args
}

pub fn category_color(category: &Category) -> Color32 {
    let [r, g, b] = category.color;
    Color32::from_rgb(r, g, b)
}

/// The choices of a "set category" menu: Uncategorized, then every category. `current` is
/// marked.
pub fn category_items(
    ui: &mut Ui,
    session: &Session,
    current: Option<CategoryId>,
    path: Option<&Path>,
    actions: &mut Actions,
) {
    if ui
        .selectable_label(current.is_none(), "Uncategorized")
        .clicked()
    {
        actions.push_args("meta.category", with_path(json!({ "id": null }), path));
        ui.close();
    }
    for category in session.catalog().categories() {
        let text = RichText::new(category.name.clone()).color(category_color(category));
        if ui
            .selectable_label(current == Some(category.id), text)
            .clicked()
        {
            actions.push_args(
                "meta.category",
                with_path(json!({ "id": category.id }), path),
            );
            ui.close();
        }
    }
}

/// A drop-down showing a category, for the preview pane. A model without a category of its
/// own shows the one it inherits from its folder, if any.
pub fn category_combo(
    ui: &mut Ui,
    salt: &str,
    session: &Session,
    own: Option<CategoryId>,
    inherited: Option<CategoryId>,
    path: Option<&Path>,
    actions: &mut Actions,
) {
    let name_of = |id: CategoryId| {
        session
            .catalog()
            .categories()
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
    };
    let text = match (own.and_then(name_of), inherited.and_then(name_of)) {
        (Some(name), _) => name,
        (None, Some(name)) => format!("{name} (from folder)"),
        (None, None) => "Uncategorized".to_owned(),
    };
    egui::ComboBox::from_id_salt(salt)
        .width((ui.available_width() - 44.0).max(80.0))
        .selected_text(text)
        .show_ui(ui, |ui| category_items(ui, session, own, path, actions))
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Category"));
}

/// A heart that toggles the favorite.
pub fn favorite_button(ui: &mut Ui, favorite: bool, path: Option<&Path>, actions: &mut Actions) {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    let color = match (favorite, response.hovered()) {
        (true, _) => icons::HEART,
        (false, true) => theme::TEXT,
        (false, false) => Color32::from_gray(0x5a),
    };
    icons::heart(ui.painter(), rect.shrink2(vec2(5.0, 5.0)), color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Favorite"));
    if response.on_hover_text("Favorite (H)").clicked() {
        actions.push_args("meta.favorite.toggle", with_path(json!({}), path));
    }
}

/// Tag chips (click one to remove it) and a box to add another.
pub fn tag_editor(
    ui: &mut Ui,
    tags: &BTreeSet<String>,
    input: &mut String,
    path: Option<&Path>,
    actions: &mut Actions,
) {
    ui.horizontal_wrapped(|ui| {
        for tag in tags {
            let chip = egui::Button::new(RichText::new(format!("{tag}  ×")).small())
                .corner_radius(8.0)
                .fill(Color32::from_gray(0x38));
            if ui.add(chip).on_hover_text("Remove tag").clicked() {
                actions.push_args("meta.tag.remove", with_path(json!({ "tag": tag }), path));
            }
        }
    });
    add_tag_box(ui, input, path, actions);
}

/// A text box that adds the typed tag on Enter.
pub fn add_tag_box(ui: &mut Ui, input: &mut String, path: Option<&Path>, actions: &mut Actions) {
    let response = ui.add(
        egui::TextEdit::singleline(input)
            .hint_text("Type to add a tag…")
            .desired_width(ui.available_width()),
    );
    let entered = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if entered && !input.trim().is_empty() {
        actions.push_args(
            "meta.tag.add",
            with_path(json!({ "tag": input.trim() }), path),
        );
        input.clear();
        response.request_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_path_is_added_to_the_arguments() {
        let args = with_path(json!({ "id": 3 }), Some(Path::new("C:/m")));
        assert_eq!(args, json!({ "id": 3, "path": "C:/m" }));
        assert_eq!(with_path(json!({ "id": 3 }), None), json!({ "id": 3 }));
    }
}

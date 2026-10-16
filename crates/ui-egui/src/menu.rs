//! Builds the menu bar from the command registry's menu paths.

use polyroom_engine::{Command, CommandRegistry, Session};

/// Top-level menus in display order. Commands under other roots are appended after these.
const ROOT_ORDER: &[&str] = &["File", "Edit", "Library", "Model", "View", "Window", "Help"];

/// Draws the menu bar. Returns the id of the command the user clicked, if any.
pub fn menu_bar(
    ui: &mut egui::Ui,
    registry: &CommandRegistry,
    session: &Session,
) -> Option<&'static str> {
    let entries: Vec<(Vec<&'static str>, &Command)> = registry
        .iter()
        .filter(|c| !c.menu.is_empty())
        .map(|c| (c.menu.split('/').collect(), c))
        .collect();

    let mut roots: Vec<&'static str> = ROOT_ORDER.to_vec();
    for (path, _) in &entries {
        if let Some(root) = path.first()
            && !roots.contains(root)
        {
            roots.push(root);
        }
    }

    let mut clicked = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for root in roots {
            let items: Vec<_> = entries
                .iter()
                .filter(|(p, _)| p.first() == Some(&root))
                .map(|(p, c)| (p.get(1..).unwrap_or_default(), *c))
                .collect();
            if items.is_empty() {
                continue;
            }
            ui.menu_button(root, |ui| {
                if let Some(id) = submenu(ui, &items, session) {
                    clicked = Some(id);
                }
            });
        }
    });
    clicked
}

/// Draws `items` (paths relative to this menu), nesting deeper path segments into submenus in
/// first-seen order.
fn submenu(
    ui: &mut egui::Ui,
    items: &[(&[&'static str], &Command)],
    session: &Session,
) -> Option<&'static str> {
    let mut clicked = None;
    let mut seen_groups: Vec<&str> = Vec::new();
    for (path, command) in items {
        match path {
            [_leaf] => {
                let mut button = egui::Button::new(command.label);
                if let Some(s) = command.shortcut {
                    button = button.shortcut_text(s.label());
                }
                if ui.add_enabled((command.enabled)(session), button).clicked() {
                    clicked = Some(command.id);
                    ui.close();
                }
            }
            [group, ..] => {
                if seen_groups.contains(group) {
                    continue;
                }
                seen_groups.push(group);
                let children: Vec<_> = items
                    .iter()
                    .filter(|(p, _)| p.len() > 1 && p.first() == Some(group))
                    .map(|(p, c)| (p.get(1..).unwrap_or_default(), *c))
                    .collect();
                ui.menu_button(*group, |ui| {
                    if let Some(id) = submenu(ui, &children, session) {
                        clicked = Some(id);
                    }
                });
            }
            [] => {}
        }
    }
    clicked
}

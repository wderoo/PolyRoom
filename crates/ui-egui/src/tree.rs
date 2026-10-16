//! The folder tree in the left panel, and the folder actions menu shared with the grid.

use std::path::{Path, PathBuf};

use egui::collapsing_header::CollapsingState;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Rect, Sense, Ui, pos2, vec2};
use polyroom_engine::{Collection, FolderNode};
use serde_json::json;

use crate::layout::Actions;
use crate::{icons, theme};

/// Remembers which folder to reveal (expand the ancestors of) after navigating to it from
/// somewhere other than the tree, e.g. by double-clicking a folder in the grid.
#[derive(Default)]
pub struct TreeState {
    last: Option<Collection>,
    reveal: Option<PathBuf>,
}

impl TreeState {
    /// Call once per frame with the current collection.
    pub fn track(&mut self, collection: &Collection) {
        if self.last.as_ref() != Some(collection) {
            if let Collection::Folder(path) = collection {
                self.reveal = Some(path.clone());
            }
            self.last = Some(collection.clone());
        }
    }
}

/// The actions for a folder, used by the right-click menu and the "⋯" button alike.
pub fn folder_menu(ui: &mut Ui, path: &Path, actions: &mut Actions) {
    if ui
        .button("Refresh Folder")
        .on_hover_text("Rescan for new, changed and deleted files")
        .clicked()
    {
        actions.push_args("library.refresh_folder", json!({ "path": path }));
    }
    if ui.button("Show in Explorer").clicked() {
        actions.push_args("library.show_in_explorer", json!({ "path": path }));
    }
}

/// A small "⋯" button at `rect` that opens the folder menu. Dim unless hovered, open or
/// `emphasize`d (e.g. its row is hovered).
pub fn dots_button(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    path: &Path,
    emphasize: bool,
    actions: &mut Actions,
) {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_text("Folder actions");
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let active = response.hovered() || open;
    if active {
        ui.painter()
            .rect_filled(rect, 3.0, egui::Color32::from_white_alpha(18));
    }
    let color = match (active, emphasize) {
        (true, _) => theme::TEXT,
        (false, true) => theme::TEXT_DIM,
        (false, false) => theme::TEXT_DIM.gamma_multiply(0.45),
    };
    icons::dots(ui.painter(), rect, color);
    egui::Popup::menu(&response).show(|ui| folder_menu(ui, path, actions));
}

pub fn folder_tree(
    ui: &mut Ui,
    roots: &[FolderNode],
    current: Option<&Path>,
    state: &mut TreeState,
    actions: &mut Actions,
) {
    for root in roots {
        node(ui, root, current, state, actions, true);
    }
    state.reveal = None;
}

/// One tree row: `name …        count  ⋯`. Laid out by hand so that the count and the dots
/// line up at the panel's right edge at every depth, and long names are truncated instead of
/// widening the panel.
fn folder_row(ui: &mut Ui, n: &FolderNode, selected: bool, is_root: bool, actions: &mut Actions) {
    const DOTS: f32 = 20.0;
    const PAD: f32 = 6.0;
    let height = ui.spacing().interact_size.y;
    let width = ui.available_width().max(DOTS + 40.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let main = Rect::from_min_max(rect.min, pos2(rect.right() - DOTS - 2.0, rect.bottom()));
    let dots = Rect::from_min_max(pos2(rect.right() - DOTS, rect.top()), rect.max);

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let visuals = ui.visuals();
        if selected {
            painter.rect_filled(main, 3.0, visuals.selection.bg_fill);
        } else if response.hovered() {
            painter.rect_filled(main, 3.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text_color = if selected {
            egui::Color32::WHITE
        } else {
            theme::TEXT
        };
        let count_color = if selected {
            egui::Color32::from_white_alpha(200)
        } else {
            theme::TEXT_DIM
        };

        let count = painter.layout_no_wrap(n.total.to_string(), font.clone(), count_color);
        let count_pos = pos2(
            main.right() - PAD - count.size().x,
            main.center().y - count.size().y / 2.0,
        );

        let name_width = (count_pos.x - main.left() - 2.0 * PAD).max(0.0);
        let mut job = LayoutJob::single_section(
            n.name.clone(),
            TextFormat {
                font_id: font,
                color: text_color,
                ..Default::default()
            },
        );
        job.wrap = TextWrapping::truncate_at_width(name_width);
        let name = painter.layout_job(job);
        let name_pos = pos2(main.left() + PAD, main.center().y - name.size().y / 2.0);

        painter.galley(name_pos, name, text_color);
        painter.galley(count_pos, count, count_color);
        if is_root {
            // Imported folders get a faint underline to set them apart.
            let y = main.bottom() - 0.5;
            painter.line_segment(
                [pos2(main.left() + PAD, y), pos2(main.right() - PAD, y)],
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(12)),
            );
        }
    }

    if response.clicked() {
        actions.push_args("library.show_folder", json!({ "path": n.path }));
    }
    response.context_menu(|ui| folder_menu(ui, &n.path, actions));
    let row_hovered = ui.rect_contains_pointer(rect);
    dots_button(
        ui,
        dots,
        ui.make_persistent_id(("dots", &n.path)),
        &n.path,
        row_hovered || selected,
        actions,
    );
    response.on_hover_text(n.path.display().to_string());
}

fn node(
    ui: &mut Ui,
    n: &FolderNode,
    current: Option<&Path>,
    state: &TreeState,
    actions: &mut Actions,
    is_root: bool,
) {
    let selected = current == Some(n.path.as_path());
    let mut row = |ui: &mut Ui| folder_row(ui, n, selected, is_root, actions);

    if n.children.is_empty() {
        // Line leaves up with the labels of expandable siblings.
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            row(ui);
        });
        return;
    }

    let id = ui.make_persistent_id(("folder", &n.path));
    let mut collapsing = CollapsingState::load_with_default_open(ui.ctx(), id, is_root);
    let reveal = state
        .reveal
        .as_deref()
        .is_some_and(|r| r != n.path && r.starts_with(&n.path));
    if reveal {
        collapsing.set_open(true);
    }
    collapsing.show_header(ui, row).body(|ui| {
        for child in &n.children {
            node(ui, child, current, state, actions, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn folder(path: &str, total: usize, children: Vec<FolderNode>) -> FolderNode {
        let path = PathBuf::from(path);
        FolderNode {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            children,
            direct: total,
            total,
            samples: Vec::new(),
        }
    }

    /// A deep tree with long names, like a real library.
    fn library() -> Vec<FolderNode> {
        let leaf = |p: &str, n| folder(p, n, Vec::new());
        vec![folder(
            "C:/3D",
            388,
            vec![
                folder(
                    "C:/3D/Haloween",
                    20,
                    vec![
                        leaf("C:/3D/Haloween/lego+halloween+head_stls", 1),
                        leaf(
                            "C:/3D/Haloween/skeleton+body+with+Head+LEG0+at+90%_stls",
                            11,
                        ),
                    ],
                ),
                leaf("C:/3D/lightbar", 15),
                folder(
                    "C:/3D/PuppetsWar",
                    210,
                    vec![folder(
                        "C:/3D/PuppetsWar/Orc - Ninja Van",
                        30,
                        vec![leaf("C:/3D/PuppetsWar/Orc - Ninja Van/Orc - Ninja Van", 30)],
                    )],
                ),
                folder(
                    "C:/3D/ToPrint",
                    47,
                    vec![leaf(
                        "C:/3D/ToPrint/ESP32-C6-Touch-LCD-1_47_without_headers_stls",
                        3,
                    )],
                ),
            ],
        )]
    }

    #[test]
    fn deep_rows_stay_inside_the_panel() {
        const WIDTH: f32 = 260.0;
        let roots = library();
        let selections = [
            "C:/3D/PuppetsWar/Orc - Ninja Van/Orc - Ninja Van",
            "C:/3D/Haloween/skeleton+body+with+Head+LEG0+at+90%_stls",
        ];
        for (i, selected) in selections.into_iter().enumerate() {
            let current = PathBuf::from(selected);
            let used = Cell::new(0.0f32);
            let mut state = TreeState::default();
            state.track(&Collection::Folder(current.clone()));
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(WIDTH, 300.0))
                .build_ui(|ui| {
                    crate::theme::apply(ui.ctx());
                    ui.set_max_width(WIDTH - 16.0);
                    let mut actions = Actions::default();
                    folder_tree(
                        ui,
                        &roots,
                        Some(current.as_path()),
                        &mut state,
                        &mut actions,
                    );
                    used.set(ui.min_rect().width());
                });
            harness.run();
            assert!(
                used.get() <= WIDTH - 16.0 + 0.5,
                "{selected}: tree is {} wide",
                used.get()
            );

            // Save a picture for eyeballing; skipped without a GPU.
            if let Ok(image) = harness.render() {
                let out = std::env::temp_dir().join(format!("polyroom_folder_tree_{i}.png"));
                image.save(&out).unwrap();
                eprintln!("rendered {}", out.display());
            }
        }
    }
}

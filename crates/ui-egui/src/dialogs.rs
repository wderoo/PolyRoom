//! Modal windows: the settings, and the questions asked before anything is deleted.

use std::path::{Path, PathBuf};

use egui::{Color32, Frame, Margin, RichText};
use polyroom_engine::{PreviewSettings, THUMB_SIZES};
use serde_json::{Value, json};

use crate::layout::Actions;
use crate::theme;

const DELETE_RED: Color32 = Color32::from_rgb(0xc0, 0x39, 0x2b);
/// The most paths a question lists before summing up the rest.
const LISTED: usize = 10;
/// The background offered when the transparent one is turned off.
const DEFAULT_BACKGROUND: [u8; 3] = [28, 28, 30];

/// A question waiting for the user's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Delete(Vec<PathBuf>),
    RemoveFolder(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Confirm,
    Cancel,
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Shows the question; returns the answer once the user gave one.
pub fn confirm(ctx: &egui::Context, dialog: &Dialog) -> Option<Answer> {
    let modal = egui::Modal::new(egui::Id::unique("confirm-dialog")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        let confirm_label = match dialog {
            Dialog::Delete(paths) => {
                let n = paths.len();
                ui.heading(format!("Delete {n} item{}?", if n == 1 { "" } else { "s" }));
                ui.add_space(6.0);
                for path in paths.iter().take(LISTED) {
                    ui.label(RichText::new(name_of(path)).color(theme::TEXT));
                }
                if n > LISTED {
                    ui.label(
                        RichText::new(format!("…and {} more", n - LISTED)).color(theme::TEXT_DIM),
                    );
                }
                ui.add_space(6.0);
                ui.label("They are moved to the Recycle Bin and removed from the library.");
                "Move to Recycle Bin"
            }
            Dialog::RemoveFolder(path) => {
                ui.heading(format!("Remove {} from the library?", name_of(path)));
                ui.add_space(6.0);
                ui.label("The files stay on disk; only PolyRoom forgets about them.");
                "Remove from Library"
            }
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let confirm = egui::Button::new(RichText::new(confirm_label).color(Color32::WHITE))
                .fill(DELETE_RED);
            if ui.add(confirm).clicked() {
                return Some(Answer::Confirm);
            }
            ui.button("Cancel").clicked().then_some(Answer::Cancel)
        })
        .inner
    });
    match modal.inner {
        Some(answer) => Some(answer),
        None if modal.should_close() => Some(Answer::Cancel),
        None => None,
    }
}

/// The part of the settings that decides how a preview looks.
fn same_look(a: &PreviewSettings, b: &PreviewSettings) -> bool {
    (a.size, a.background, a.model_color) == (b.size, b.background, b.model_color)
}

/// The settings window. `draft` is what the controls edit; it is applied once the mouse is
/// released, so dragging in a colour picker does not redraw every thumbnail on the way.
/// Returns true when the window should close.
pub fn settings(
    ctx: &egui::Context,
    current: PreviewSettings,
    draft: &mut PreviewSettings,
    actions: &mut Actions,
) -> bool {
    let modal = egui::Modal::new(egui::Id::unique("settings-dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Settings");
        ui.add_space(8.0);
        Frame::group(ui.style())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Preview Images").strong());
                ui.add_space(6.0);
                egui::Grid::new("preview-settings")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| preview_rows(ui, draft, actions));
            });
        ui.add_space(10.0);
        ui.button("Close").clicked()
    });
    if !same_look(draft, &current) && !ctx.input(|i| i.pointer.any_down()) {
        actions.push_args(
            "preview.settings",
            json!({
                "size": draft.size,
                "background": draft.background.map_or(Value::Null, |c| json!(c)),
                "model_color": draft.model_color,
            }),
        );
    }
    modal.inner || modal.should_close()
}

fn preview_rows(ui: &mut egui::Ui, draft: &mut PreviewSettings, actions: &mut Actions) {
    ui.vertical(|ui| {
        ui.label("Image Quality");
        ui.label(
            RichText::new("Higher quality uses more disk space")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
    egui::ComboBox::from_id_salt("preview-size")
        .selected_text(format!("{} px", draft.size))
        .show_ui(ui, |ui| {
            for size in THUMB_SIZES {
                ui.selectable_value(&mut draft.size, size, format!("{size} px"));
            }
        });
    ui.end_row();

    ui.vertical(|ui| {
        ui.label("Background Color");
        ui.label(
            RichText::new("Behind the model in previews")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
    ui.horizontal(|ui| {
        let mut transparent = draft.background.is_none();
        if ui.checkbox(&mut transparent, "Transparent").changed() {
            draft.background = (!transparent).then_some(DEFAULT_BACKGROUND);
        }
        if let Some(color) = &mut draft.background {
            ui.color_edit_button_srgb(color);
        }
    });
    ui.end_row();

    ui.vertical(|ui| {
        ui.label("Model Color");
        ui.label(
            RichText::new("Of models in previews and the 3D view")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
    ui.color_edit_button_srgb(&mut draft.model_color);
    ui.end_row();

    ui.vertical(|ui| {
        ui.label("Refresh All Previews");
        ui.label(
            RichText::new("Make every preview image again")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
    if ui.button("Regenerate").clicked() {
        actions.push("preview.regenerate_all");
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    struct State {
        dialog: Dialog,
        answer: Option<Answer>,
    }

    fn harness(dialog: Dialog) -> Harness<'static, State> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                |ui, state: &mut State| {
                    crate::theme::apply(ui.ctx());
                    if state.answer.is_none() {
                        state.answer = confirm(ui.ctx(), &state.dialog);
                    }
                },
                State {
                    dialog,
                    answer: None,
                },
            );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn deleting_lists_the_files_and_waits_for_an_answer() {
        let paths: Vec<PathBuf> = (0..12)
            .map(|i| PathBuf::from(format!("C:/m/{i}.stl")))
            .collect();
        let mut harness = harness(Dialog::Delete(paths));
        harness.get_by_label("Delete 12 items?");
        harness.get_by_label("0.stl");
        harness.get_by_label("…and 2 more");
        assert_eq!(
            harness.state().answer,
            None,
            "no answer until a button is used"
        );

        harness.get_by_label("Move to Recycle Bin").click();
        harness.run_steps(2);
        assert_eq!(harness.state().answer, Some(Answer::Confirm));
    }

    #[test]
    fn cancelling_answers_cancel() {
        let mut harness = harness(Dialog::RemoveFolder(PathBuf::from("C:/m/minis")));
        harness.get_by_label("Remove minis from the library?");
        harness.get_by_label("Cancel").click();
        harness.run_steps(2);
        assert_eq!(harness.state().answer, Some(Answer::Cancel));
    }

    #[test]
    fn only_the_look_decides_whether_settings_need_applying() {
        let a = PreviewSettings::default();
        let regenerated = PreviewSettings { generation: 9, ..a };
        assert!(same_look(&a, &regenerated));
        assert!(!same_look(&a, &PreviewSettings { size: 512, ..a }));
        assert!(!same_look(
            &a,
            &PreviewSettings {
                background: Some([1, 2, 3]),
                ..a
            }
        ));
    }
}

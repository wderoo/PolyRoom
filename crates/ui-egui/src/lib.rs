//! PolyRoom egui front-end. Draws engine state and dispatches commands; holds no behaviour of
//! its own beyond what only a UI can do (dialogs, window control, GPU textures).

#![forbid(unsafe_code)]

mod dialogs;
mod format;
mod gizmo;
mod grid;
mod icons;
mod keys;
mod layout;
mod loupe;
mod menu;
mod meta_ui;
mod preview;
mod theme;
mod thumbs;
mod topbar;
mod tree;

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Frame, Margin, RichText};
use polyroom_engine::{
    CommandRegistry, Paths, PreviewSettings, Request, Session, default_registry,
};
use serde_json::{Value, json};

use dialogs::{Answer, Dialog};

pub struct PolyRoomApp {
    session: Session,
    registry: CommandRegistry,
    thumbs: thumbs::ThumbTextures,
    grid: grid::GridState,
    loupe: loupe::LoupeView,
    tree: tree::TreeState,
    left: layout::LeftState,
    topbar: topbar::TopbarState,
    preview: preview::PreviewState,
    show_shortcuts: bool,
    /// A question waiting for an answer before something is deleted or removed.
    dialog: Option<Dialog>,
    /// The settings window, while it is open, with the values its controls edit.
    settings_draft: Option<PreviewSettings>,
    /// A command ran since the last frame was drawn, so what's on screen is stale.
    dirty: bool,
}

impl PolyRoomApp {
    /// Creates the app; `import` is a folder to import at startup (from `--import`).
    pub fn new(cc: &eframe::CreationContext<'_>, import: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let session = match Paths::for_user().map(|p| Session::open(&p)) {
            Some(Ok(session)) => session,
            Some(Err(e)) => {
                log::error!("{e}");
                let mut s = Session::new();
                s.set_status(format!(
                    "Could not open the catalog ({e}); changes won't be saved"
                ));
                s
            }
            None => {
                let mut s = Session::new();
                s.set_status("No home folder found; changes won't be saved");
                s
            }
        };
        let mut app = Self {
            topbar: topbar::TopbarState {
                filter: session.filter().to_owned(),
                ..topbar::TopbarState::default()
            },
            session,
            registry: default_registry(),
            thumbs: thumbs::ThumbTextures::default(),
            grid: grid::GridState::default(),
            loupe: loupe::LoupeView::default(),
            tree: tree::TreeState::default(),
            left: layout::LeftState::default(),
            preview: preview::PreviewState::default(),
            show_shortcuts: false,
            dialog: None,
            settings_draft: None,
            dirty: false,
        };
        if let Some(dir) = import {
            app.dispatch("library.import", &json!({ "path": dir }));
        }
        app
    }

    fn dispatch(&mut self, id: &str, args: &Value) {
        self.dirty = true;
        if let Err(e) = self.registry.run(id, &mut self.session, args) {
            log::warn!("{e}");
            self.session.set_status(e.to_string());
        }
    }

    fn modal_open(&self) -> bool {
        self.show_shortcuts || self.dialog.is_some() || self.settings_draft.is_some()
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Let text fields keep their keystrokes, and keep keys away from what is behind a modal.
        if ctx.egui_wants_keyboard_input() || self.modal_open() {
            return;
        }
        for shortcut in keys::pressed_shortcuts(ctx) {
            let Some(id) = self.registry.find_shortcut(shortcut).map(|c| c.id) else {
                continue;
            };
            // A disabled command just ignores its shortcut, as in Lightroom.
            if self.registry.is_enabled(id, &self.session) {
                self.dispatch(id, &Value::Null);
            }
        }
    }

    fn handle_requests(&mut self, ctx: &egui::Context) {
        for request in self.session.drain_requests() {
            match request {
                Request::PickImportFolder => {
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_title("Import folder")
                        .pick_folder()
                    {
                        self.dispatch("library.import", &json!({ "path": dir }));
                    }
                }
                Request::ShowShortcuts => self.show_shortcuts = true,
                Request::ShowInExplorer(path) | Request::OpenFile(path) => {
                    open_in_file_manager(&path, &mut self.session);
                }
                Request::RevealFile(path) => reveal_in_file_manager(&path, &mut self.session),
                Request::ConfirmDelete(paths) => self.dialog = Some(Dialog::Delete(paths)),
                Request::ConfirmRemoveFolder(path) => {
                    self.dialog = Some(Dialog::RemoveFolder(path));
                }
                Request::ShowSettings => {
                    self.settings_draft = Some(self.session.preview_settings());
                }
                Request::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let modal = egui::Modal::new(egui::Id::unique("shortcuts")).show(ctx, |ui| {
            ui.heading("Keyboard Shortcuts");
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    egui::Grid::new("shortcut_grid")
                        .striped(true)
                        .show(ui, |ui| {
                            for c in self.registry.iter() {
                                if let Some(s) = c.shortcut {
                                    ui.label(c.label);
                                    ui.monospace(s.label());
                                    ui.end_row();
                                }
                            }
                        });
                });
            ui.add_space(8.0);
            ui.button("Close").clicked()
        });
        if modal.inner || modal.should_close() {
            self.show_shortcuts = false;
        }
    }

    /// The questions and the settings; what they decide is added to `actions`.
    fn dialogs(&mut self, ctx: &egui::Context, actions: &mut layout::Actions) {
        self.shortcuts_window(ctx);
        if let Some(dialog) = self.dialog.clone()
            && let Some(answer) = dialogs::confirm(ctx, &dialog)
        {
            self.dialog = None;
            if answer == Answer::Confirm {
                match dialog {
                    Dialog::Delete(paths) => {
                        actions.push_args("file.delete_confirmed", json!({ "paths": paths }));
                    }
                    Dialog::RemoveFolder(path) => {
                        actions
                            .push_args("library.remove_folder_confirmed", json!({ "path": path }));
                    }
                }
            }
        }
        if let Some(mut draft) = self.settings_draft.take() {
            let close =
                dialogs::settings(ctx, self.session.preview_settings(), &mut draft, actions);
            if !close {
                self.settings_draft = Some(draft);
            }
        }
    }

    fn center(&mut self, ui: &mut egui::Ui, actions: &mut layout::Actions) {
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(theme::CENTER))
            .show(ui, |ui| {
                if self.session.catalog().models().is_empty() {
                    layout::empty_library(ui, actions);
                } else if self.session.visible().is_empty()
                    && self.session.visible_folders().is_empty()
                {
                    layout::no_matches(ui);
                } else {
                    grid::grid(
                        ui,
                        &mut self.session,
                        &mut self.thumbs,
                        &mut self.grid,
                        actions,
                    );
                }
            });
    }
}

/// Opens `path` with the platform's default handler: a folder in the file manager, a file in
/// the application for its type.
fn open_in_file_manager(path: &Path, session: &mut Session) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        session.set_status(format!("Could not open {}: {e}", path.display()));
    }
}

/// Shows `path` selected in the platform's file manager.
fn reveal_in_file_manager(path: &Path, session: &mut Session) {
    if let Err(e) = reveal_command(path) {
        session.set_status(format!("Could not show {}: {e}", path.display()));
    }
}

#[cfg(windows)]
fn reveal_command(path: &Path) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    // Explorer wants backslashes, and the whole argument quoted by hand.
    let path = path.display().to_string().replace('/', "\\");
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn reveal_command(path: &Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn()
}

impl eframe::App for PolyRoomApp {
    fn on_exit(&mut self) {
        self.session.save_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.session.poll();
        self.thumbs.begin_frame();
        for id in self.session.take_stale_thumbnails() {
            self.thumbs.remove(id);
        }
        for (id, image) in self.session.take_thumbnails() {
            self.thumbs.insert(&ctx, id, &image);
        }
        self.handle_shortcuts(&ctx);

        let mut actions = layout::Actions::default();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            if let Some(id) = menu::menu_bar(ui, &self.registry, &self.session) {
                actions.push(id);
            }
        });
        egui::Panel::bottom("status_bar")
            .frame(
                Frame::new()
                    .fill(theme::HEADER)
                    .inner_margin(Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| {
                status_bar(
                    ui,
                    &library_counts(&self.session),
                    self.session.status(),
                    self.session.progress().as_ref(),
                );
            });
        layout::left_panel(
            ui,
            &self.session,
            &mut self.tree,
            &mut self.left,
            &mut actions,
        );
        preview::right_panel(
            ui,
            &mut self.session,
            &mut self.loupe,
            frame.wgpu_render_state(),
            &mut self.thumbs,
            &mut self.preview,
            &mut actions,
        );
        if !self.session.catalog().models().is_empty() {
            topbar::topbar(ui, &self.session, &mut self.topbar, &mut actions);
        }
        self.center(ui, &mut actions);
        self.dialogs(&ctx, &mut actions);

        for (id, args) in actions.list {
            self.dispatch(id, &args);
        }
        self.handle_requests(&ctx);

        // Commands change state after this frame was drawn (clicks are only known while
        // drawing). Start their follow-up work now, e.g. loading the newly selected model,
        // and draw again so the screen shows the result instead of waiting for the next input.
        if std::mem::take(&mut self.dirty) {
            self.session.poll();
            ctx.request_repaint();
        }

        if self.session.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

/// `401 folders · 1,240 files · 40.2 GB`, plus the selection when there are several models.
fn library_counts(session: &Session) -> String {
    let stats = session.library_stats();
    let plural = |n: usize, word: &str| {
        format!(
            "{} {word}{}",
            format::count(n as u64),
            if n == 1 { "" } else { "s" }
        )
    };
    let mut text = format!(
        "{} · {} · {}",
        plural(stats.folders, "folder"),
        plural(stats.files, "file"),
        format::bytes(stats.bytes)
    );
    let (selected, bytes) = session.selection_stats();
    if selected > 1 {
        text += &format!(" · {selected} selected ({})", format::bytes(bytes));
    }
    text
}

/// The library totals and status line, with a progress bar (or spinner while the amount of work
/// is unknown) at the right edge while background work runs.
fn status_bar(
    ui: &mut egui::Ui,
    counts: &str,
    status: &str,
    progress: Option<&polyroom_engine::Progress>,
) {
    const BAR: f32 = 260.0;
    ui.horizontal(|ui| {
        ui.label(RichText::new(counts).color(theme::TEXT).small());
        if !status.is_empty() {
            ui.label(RichText::new("·").color(theme::TEXT_DIM).small());
            ui.label(RichText::new(status).color(theme::TEXT_DIM).small());
        }
        let Some(progress) = progress else {
            return;
        };
        match progress.fraction {
            Some(f) => {
                let gap = ui.available_width() - BAR;
                if gap > 0.0 {
                    ui.add_space(gap);
                }
                // The unfilled track uses this colour; the default vanishes against the header.
                ui.visuals_mut().extreme_bg_color = egui::Color32::from_gray(58);
                ui.add(
                    egui::ProgressBar::new(f)
                        .desired_width(BAR.min(ui.available_width()))
                        .desired_height(14.0)
                        .text(RichText::new(&progress.text).small()),
                );
            }
            None => {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(&progress.text).color(theme::TEXT_DIM).small());
            }
        }
    });
}

/// Tests that render with the GPU take this lock: creating several wgpu devices at once crashes
/// some drivers.
#[cfg(test)]
pub(crate) static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use polyroom_engine::Progress;

    use super::*;

    #[test]
    fn progress_bar_is_drawn_at_the_right_edge_of_the_status_bar() {
        let _gpu = crate::GPU_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let progress = Progress {
            text: "Generating previews 120 / 400".into(),
            fraction: Some(0.3),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 30.0))
            .build_ui(|ui| {
                theme::apply(ui.ctx());
                Frame::new()
                    .fill(theme::HEADER)
                    .inner_margin(Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        status_bar(
                            ui,
                            "401 folders · 1,240 files · 40.2 GB",
                            "",
                            Some(&progress),
                        );
                    });
            });
        harness.run();
        // The bar's text is exposed to accessibility, so it is on screen.
        harness.get_by_label_contains("Generating previews 120 / 400");
        harness.get_by_label("401 folders · 1,240 files · 40.2 GB");
        if let Ok(image) = harness.render() {
            let out = std::env::temp_dir().join("polyroom_status_bar.png");
            image.save(&out).unwrap();
            eprintln!("rendered {}", out.display());
        }
    }

    #[test]
    fn the_status_bar_counts_folders_files_and_size_and_the_selection() {
        let files = ["a.stl", "b.stl", "sub/c.stl"].map(String::from);
        let (_dir, mut session) = test_support::imported(&files);
        assert_eq!(library_counts(&session), "2 folders · 3 files · 3 B");

        default_registry()
            .run("select.all", &mut session, &Value::Null)
            .unwrap();
        assert_eq!(
            library_counts(&session),
            "2 folders · 3 files · 3 B · 2 selected (2 B)",
            "the root folder shows two files"
        );
    }
    /// Every panel together, as the app lays them out; the picture goes to the temp folder.
    #[test]
    fn the_library_layout_draws_every_panel() {
        let _gpu = crate::GPU_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Parts {
            session: Session,
            registry: CommandRegistry,
            thumbs: thumbs::ThumbTextures,
            grid: grid::GridState,
            loupe: loupe::LoupeView,
            tree: tree::TreeState,
            left: layout::LeftState,
            topbar: topbar::TopbarState,
            preview: preview::PreviewState,
        }
        let files: Vec<String> = [
            "dragon.stl",
            "knight.stl",
            "orc.stl",
            "minis/elf.stl",
            "terrain/hill.stl",
        ]
        .map(String::from)
        .to_vec();
        let (dir, mut session) = test_support::imported(&files);
        std::mem::forget(dir);
        let registry = default_registry();
        let root = session.catalog().folders()[0].clone();
        for (id, args) in [
            ("category.add", json!({ "name": "Miniatures" })),
            ("category.add", json!({ "name": "Terrain" })),
            (
                "meta.category",
                json!({ "id": 1, "path": root.join("minis") }),
            ),
            (
                "meta.tag.add",
                json!({ "tag": "fantasy", "path": root.join("minis") }),
            ),
            (
                "meta.favorite.toggle",
                json!({ "path": root.join("terrain") }),
            ),
        ] {
            registry.run(id, &mut session, &args).unwrap();
        }
        session.poll();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 800.0))
            .build_ui_state(
                |ui, p: &mut Parts| {
                    theme::apply(ui.ctx());
                    p.session.poll();
                    let mut actions = layout::Actions::default();
                    egui::Panel::bottom("status").show(ui, |ui| {
                        status_bar(ui, &library_counts(&p.session), p.session.status(), None);
                    });
                    layout::left_panel(ui, &p.session, &mut p.tree, &mut p.left, &mut actions);
                    preview::right_panel(
                        ui,
                        &mut p.session,
                        &mut p.loupe,
                        None,
                        &mut p.thumbs,
                        &mut p.preview,
                        &mut actions,
                    );
                    topbar::topbar(ui, &p.session, &mut p.topbar, &mut actions);
                    egui::CentralPanel::no_frame().show(ui, |ui| {
                        grid::grid(ui, &mut p.session, &mut p.thumbs, &mut p.grid, &mut actions);
                    });
                    for (id, args) in actions.list {
                        p.registry.run(id, &mut p.session, &args).unwrap();
                    }
                },
                Parts {
                    session,
                    registry: default_registry(),
                    thumbs: thumbs::ThumbTextures::default(),
                    grid: grid::GridState::default(),
                    loupe: loupe::LoupeView::default(),
                    tree: tree::TreeState::default(),
                    left: layout::LeftState::default(),
                    topbar: topbar::TopbarState::default(),
                    preview: preview::PreviewState::default(),
                },
            );
        harness.run_steps(5);
        let parts = harness.state_mut();
        parts
            .registry
            .run("select.all", &mut parts.session, &Value::Null)
            .unwrap();
        harness.run_steps(5);
        for label in ["Categories", "Tags", "Favorites"] {
            assert!(harness.query_all_by_label(label).count() >= 1, "{label}");
        }
        assert!(harness.query_all_by_label_contains("selected").count() >= 1);
        if let Ok(image) = harness.render() {
            let out = std::env::temp_dir().join("polyroom_layout.png");
            image.save(&out).unwrap();
            eprintln!("rendered {}", out.display());
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::time::{Duration, Instant};

    use polyroom_engine::{Session, default_registry};
    use serde_json::json;

    /// A session that has imported `files` (paths relative to a temp folder, which is returned
    /// too so that it outlives the test). No preview workers run, so no GPU is needed.
    pub fn imported(files: &[String]) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        let mut session = Session::new();
        default_registry()
            .run("library.import", &mut session, &json!({"path": dir.path()}))
            .unwrap();
        let start = Instant::now();
        while session.is_busy() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
            session.poll();
        }
        session.poll();
        (dir, session)
    }
}

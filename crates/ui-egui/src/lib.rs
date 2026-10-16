//! PolyRoom egui front-end. Draws engine state and dispatches commands; holds no behaviour of
//! its own beyond what only a UI can do (dialogs, window control, GPU textures).

#![forbid(unsafe_code)]

mod format;
mod grid;
mod icons;
mod keys;
mod layout;
mod loupe;
mod menu;
mod theme;
mod thumbs;

use std::time::Duration;

use egui::{Frame, Margin, RichText};
use polyroom_engine::{
    CommandRegistry, Module, Paths, Request, Session, ViewMode, default_registry,
};
use serde_json::{Value, json};

pub struct PolyRoomApp {
    session: Session,
    registry: CommandRegistry,
    thumbs: thumbs::ThumbTextures,
    grid: grid::GridState,
    filmstrip: grid::GridState,
    loupe: loupe::LoupeView,
    /// Text in the filter box (the engine holds the applied filter).
    filter: String,
    show_shortcuts: bool,
}

impl PolyRoomApp {
    /// Creates the app; `import` is a folder to import at startup (from `--import`).
    pub fn new(cc: &eframe::CreationContext<'_>, import: Option<std::path::PathBuf>) -> Self {
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
            filter: session.filter().to_owned(),
            session,
            registry: default_registry(),
            thumbs: thumbs::ThumbTextures::default(),
            grid: grid::GridState::default(),
            filmstrip: grid::GridState::default(),
            loupe: loupe::LoupeView::default(),
            show_shortcuts: false,
        };
        if let Some(dir) = import {
            app.dispatch("library.import", &json!({ "path": dir }));
        }
        app
    }

    fn dispatch(&mut self, id: &str, args: &Value) {
        if let Err(e) = self.registry.run(id, &mut self.session, args) {
            log::warn!("{e}");
            self.session.set_status(e.to_string());
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Let text fields keep their keystrokes.
        if ctx.egui_wants_keyboard_input() {
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

    fn center(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, actions: &mut layout::Actions) {
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(theme::CENTER))
            .show(ui, |ui| {
                let show_loupe =
                    self.session.module == Module::Viewer || self.session.view == ViewMode::Loupe;
                if self.session.catalog().models().is_empty() {
                    layout::empty_library(ui, actions);
                } else if show_loupe {
                    self.loupe.show(ui, frame, &self.session);
                } else if self.session.visible().is_empty() {
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

impl eframe::App for PolyRoomApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.session.poll();
        self.thumbs.begin_frame();
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
        layout::module_bar(ui, &self.session, &mut actions);
        egui::Panel::bottom("status_bar")
            .frame(
                Frame::new()
                    .fill(theme::HEADER)
                    .inner_margin(Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.session.status())
                        .color(theme::TEXT_DIM)
                        .small(),
                );
            });
        if self.session.module == Module::Library {
            let film_open = self.session.panels.filmstrip;
            let mut open = film_open;
            egui::Panel::bottom("filmstrip")
                .exact_size(96.0)
                .frame(
                    Frame::new()
                        .fill(theme::HEADER)
                        .inner_margin(Margin::same(6)),
                )
                .show_collapsible(ui, &mut open, |ui| {
                    grid::filmstrip(
                        ui,
                        &mut self.session,
                        &mut self.thumbs,
                        &mut self.filmstrip,
                        &mut actions,
                    );
                });
            if open != film_open {
                actions.push("view.toggle_filmstrip");
            }
            layout::toolbar(ui, &self.session, &mut self.filter, &mut actions);
            layout::left_panel(ui, &mut self.session, &mut actions);
        }
        layout::right_panel(ui, &mut self.session, &mut actions);
        self.center(ui, frame, &mut actions);
        self.shortcuts_window(&ctx);

        for (id, args) in actions.list {
            self.dispatch(id, &args);
        }
        self.handle_requests(&ctx);

        if self.session.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

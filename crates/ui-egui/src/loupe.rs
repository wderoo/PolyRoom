//! The 3D viewer of the preview pane: renders the current model with wgpu into a texture shown
//! by egui.
//! Left-drag orbits, right/middle-drag pans, the wheel zooms, double-click resets the view.

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use polyroom_engine::{Format, ModelId, Session};
use polyroom_render::{Camera, GpuMesh, Renderer, Style, Target};
use serde_json::json;

use crate::layout::Actions;
use crate::thumbs::ThumbTextures;
use crate::{gizmo, theme};

/// Edge length of a chip in the photo strip.
const CHIP: f32 = 56.0;
const CHIP_GAP: f32 = 6.0;

/// The default background colour (#1c1c1e), matching [`theme::CENTER`].
const BACKGROUND: [f32; 4] = [28.0 / 255.0, 28.0 / 255.0, 30.0 / 255.0, 1.0];

/// The background and model colour (sRGB) the view is drawn in.
type Look = (Option<[u8; 3]>, [u8; 3]);

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn look_style(look: Look, show_grid: bool) -> Style {
    let (background, [r, g, b]) = look;
    Style {
        background: background.map_or(BACKGROUND, |[r, g, b]| {
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        }),
        base_color: [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)],
        show_grid,
    }
}

#[derive(Default)]
pub struct LoupeView {
    renderer: Option<Renderer>,
    target: Option<Target>,
    texture: Option<egui::TextureId>,
    mesh: Option<(ModelId, GpuMesh)>,
    camera: Option<Camera>,
    /// The camera/size/mesh the texture was last rendered with.
    rendered: Option<(Camera, u32, u32, ModelId, bool, Look)>,
    upload_error: Option<String>,
    /// The image shown, as a texture, and which file it is of.
    picture: Option<(ModelId, egui::TextureHandle)>,
}

impl LoupeView {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        render_state: Option<&egui_wgpu::RenderState>,
        session: &mut Session,
        thumbs: &mut ThumbTextures,
        actions: &mut Actions,
    ) {
        let rect = ui.available_rect_before_wrap();
        let Some(current) = session.current() else {
            ui.allocate_rect(rect, Sense::hover());
            ui.painter().rect_filled(rect, 0.0, theme::CENTER);
            centered(ui.painter(), rect, "No model selected");
            return;
        };
        let format = session.model(current).map(|m| m.format);
        let photo = session.loupe().photo;
        if format == Some(Format::Zip) {
            show_archive(ui, rect, session);
            return;
        }
        if format == Some(Format::Image) || photo.is_some() {
            self.show_picture(ui, rect, session);
        } else {
            self.show_model(ui, rect, render_state, session);
        }
        let stack = session.stack(current).to_vec();
        if !stack.is_empty() {
            photo_strip(ui, rect, &stack, photo, session, thumbs, actions);
        }
    }

    fn show_model(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        render_state: Option<&egui_wgpu::RenderState>,
        session: &Session,
    ) {
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::CENTER);

        let Some(render_state) = render_state else {
            centered(&painter, rect, "3D view needs the wgpu renderer");
            return;
        };
        let device = &render_state.device;
        let renderer = self.renderer.get_or_insert_with(|| Renderer::new(device));

        let loupe = session.loupe();
        let Some(current) = session.current() else {
            centered(&painter, rect, "No model selected");
            return;
        };

        // Upload a newly loaded mesh.
        if let (Some(id), Some(mesh)) = (loupe.id, &loupe.mesh)
            && self.mesh.as_ref().map(|(m, _)| *m) != Some(id)
        {
            match renderer.upload(device, mesh) {
                Ok(gpu) => {
                    let aspect = rect.width() / rect.height().max(1.0);
                    self.camera = Some(Camera::fit(&gpu.bounds, aspect));
                    self.mesh = Some((id, gpu));
                    self.upload_error = None;
                }
                Err(e) => self.upload_error = Some(e.to_string()),
            }
        }

        // Camera interaction.
        if let (Some(camera), true) = (&mut self.camera, response.hovered() || response.dragged()) {
            let (drag, wheel, pointer) = ui.input(|i| {
                (
                    i.pointer.delta(),
                    i.smooth_scroll_delta.y,
                    (
                        i.pointer.primary_down(),
                        i.pointer.secondary_down() || i.pointer.middle_down(),
                    ),
                )
            });
            if response.dragged() {
                if pointer.1 {
                    camera.pan(drag.x, drag.y, rect.height());
                } else if pointer.0 {
                    camera.orbit(drag.x, drag.y);
                }
            }
            if wheel != 0.0 {
                camera.zoom((-wheel * 0.0015).exp());
            }
            if response.double_clicked()
                && let Some((_, gpu)) = &self.mesh
            {
                *camera = Camera::fit(&gpu.bounds, rect.width() / rect.height().max(1.0));
            }
        }

        // Render when something changed.
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = (
            (rect.width() * ppp).round() as u32,
            (rect.height() * ppp).round() as u32,
        );
        if w == 0 || h == 0 {
            return;
        }
        let resized = self
            .target
            .as_ref()
            .is_none_or(|t| t.width != w || t.height != h);
        if resized {
            let target = Target::new(device, w, h);
            let view = target.display_view();
            let mut egui_renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &view,
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.texture = Some(egui_renderer.register_native_texture(
                        device,
                        &view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
            self.target = Some(target);
            self.rendered = None;
        }
        if let (Some(target), Some(camera), Some((id, gpu))) =
            (&self.target, self.camera, &self.mesh)
        {
            let preview = session.preview_settings();
            let look = (preview.background, preview.model_color);
            let state = Some((camera, w, h, *id, session.show_bed_grid, look));
            if self.rendered != state {
                let style = look_style(look, session.show_bed_grid);
                renderer.render(
                    device,
                    &render_state.queue,
                    target,
                    Some(gpu),
                    &camera,
                    &style,
                );
                self.rendered = state;
            }
        }

        let showing_current = self.mesh.as_ref().is_some_and(|(id, _)| *id == current);
        if let Some(texture) = self.texture
            && self.mesh.is_some()
        {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            let tint = if showing_current {
                Color32::WHITE
            } else {
                Color32::from_gray(110)
            };
            painter.image(texture, rect, uv, tint);
        }

        // Axis gizmo: shows the orientation and snaps the view to an axis when clicked.
        if showing_current && let Some(camera) = &mut self.camera {
            let center = rect.right_top() + vec2(-(gizmo::ARM + 34.0), gizmo::ARM + 34.0);
            if let Some(axis) = gizmo::show(ui, center, camera) {
                camera.look_along(axis);
            }
        }

        // Overlays.
        if loupe.loading || (!showing_current && loupe.error.is_none()) {
            centered(&painter, rect, "Loading model…");
            ui.ctx().request_repaint();
        } else if let Some(error) = loupe.error.as_ref().or(self.upload_error.as_ref()) {
            centered(&painter, rect, &format!("Can't show this model\n{error}"));
        }
        if let Some(model) = session.model(current) {
            let mut info = model.name();
            if let Some(stats) = model.stats {
                info += &format!("\n{}", crate::format::dimensions(&stats));
            }
            painter.text(
                rect.left_top() + vec2(14.0, 12.0),
                Align2::LEFT_TOP,
                info,
                FontId::proportional(14.0),
                theme::TEXT,
            );
        }
        if rect.width() >= 520.0 {
            painter.text(
                rect.left_bottom() + vec2(14.0, -10.0),
                Align2::LEFT_BOTTOM,
                "Drag to orbit · Right-drag to pan · Scroll to zoom · Double-click to reset · Click an axis to align",
                FontId::proportional(11.0),
                theme::TEXT_DIM,
            );
        }
        if showing_current {
            self.reset_button(ui, rect);
        }
    }
}

impl LoupeView {
    /// A "Reset" button in the top-right corner that fits the model back into view.
    fn reset_button(&mut self, ui: &mut Ui, rect: Rect) {
        let button = Rect::from_min_size(rect.right_top() + vec2(-62.0, 8.0), vec2(54.0, 22.0));
        let response = ui.interact(button, ui.scope_id().with("reset-view"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Reset view"));
        let fill = if response.hovered() {
            Color32::from_black_alpha(190)
        } else {
            Color32::from_black_alpha(140)
        };
        ui.painter()
            .rect_filled(button, CornerRadius::same(4), fill);
        ui.painter().text(
            button.center(),
            Align2::CENTER_CENTER,
            "Reset",
            FontId::proportional(12.0),
            theme::TEXT,
        );
        if response.on_hover_text("Fit the model into view").clicked()
            && let (Some(camera), Some((_, gpu))) = (&mut self.camera, &self.mesh)
        {
            *camera = Camera::fit(&gpu.bounds, rect.width() / rect.height().max(1.0));
        }
    }

    /// An image file, or a photo from the current model's stack, fitted into the view.
    fn show_picture(&mut self, ui: &mut Ui, rect: Rect, session: &Session) {
        ui.allocate_rect(rect, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::CENTER);
        let loupe = session.loupe();
        let Some(of) = loupe.picture_of else {
            return;
        };
        if let Some(image) = &loupe.picture
            && self.picture.as_ref().map(|(id, _)| *id) != Some(of)
        {
            let size = [image.width as usize, image.height as usize];
            if size[0] * size[1] * 4 == image.rgba.len() {
                let color = egui::ColorImage::from_rgba_premultiplied(size, &image.rgba);
                let texture =
                    ui.ctx()
                        .load_texture("loupe-picture", color, egui::TextureOptions::LINEAR);
                self.picture = Some((of, texture));
            }
        }
        let model = session.model(of);
        match self.picture.as_ref().filter(|(id, _)| *id == of) {
            Some((_, texture)) => {
                let area = rect.shrink(24.0);
                let size = texture.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y).min(1.0);
                let shown = Rect::from_center_size(area.center(), size * scale);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(texture.id(), shown, uv, Color32::WHITE);
            }
            None => match model.and_then(|m| m.error.as_ref()) {
                Some(error) => centered(&painter, rect, &format!("Can't show this image\n{error}")),
                None if session.thumbnail_failed(of) => {
                    centered(&painter, rect, "Can't show this image");
                }
                None => {
                    centered(&painter, rect, "Loading image…");
                    ui.ctx().request_repaint();
                }
            },
        }
        if let Some(model) = model {
            painter.text(
                rect.left_top() + vec2(14.0, 12.0),
                Align2::LEFT_TOP,
                model.name(),
                FontId::proportional(14.0),
                theme::TEXT,
            );
        }
    }
}

/// The files inside a ZIP, as a list.
fn show_archive(ui: &mut Ui, rect: Rect, session: &Session) {
    ui.painter().rect_filled(rect, 0.0, theme::CENTER);
    let loupe = session.loupe();
    let name = session
        .current()
        .and_then(|id| session.model(id))
        .map(|m| m.name())
        .unwrap_or_default();
    let Some(entries) = &loupe.archive else {
        ui.allocate_rect(rect, Sense::hover());
        match &loupe.error {
            Some(error) => centered(ui.painter(), rect, &format!("Can't open this ZIP\n{error}")),
            None => {
                centered(ui.painter(), rect, "Reading archive…");
                ui.ctx().request_repaint();
            }
        }
        return;
    };
    let models = entries
        .iter()
        .filter(|e| e.format.is_some_and(Format::is_mesh))
        .count();
    let images = entries
        .iter()
        .filter(|e| e.format == Some(Format::Image))
        .count();
    let width = rect.width().min(720.0) - 32.0;
    let inner = Rect::from_center_size(rect.center(), vec2(width.max(100.0), rect.height() - 32.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.label(RichText::new(name).size(16.0).color(theme::TEXT));
        ui.label(
            RichText::new(format!(
                "{} files · {} · {} inside",
                crate::format::count(entries.len() as u64),
                crate::format::contents(models as u64, images as u64, 0),
                crate::format::bytes(entries.iter().map(|e| e.size).fold(0, u64::saturating_add)),
            ))
            .color(theme::TEXT_DIM),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("archive-entries")
                    .striped(true)
                    .num_columns(3)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for e in entries {
                            let known = e.format.is_some();
                            let color = if known { theme::TEXT } else { theme::TEXT_DIM };
                            ui.label(RichText::new(&e.name).color(color));
                            ui.label(
                                RichText::new(e.format.map_or("", Format::label))
                                    .color(theme::TEXT_DIM),
                            );
                            ui.label(
                                RichText::new(crate::format::bytes(e.size)).color(theme::TEXT_DIM),
                            );
                            ui.end_row();
                        }
                    });
            });
    });
}

/// A row of chips along the bottom of the loupe: the 3D view, then each stacked photo.
fn photo_strip(
    ui: &mut Ui,
    rect: Rect,
    stack: &[ModelId],
    photo: Option<ModelId>,
    session: &mut Session,
    thumbs: &mut ThumbTextures,
    actions: &mut Actions,
) {
    let fits = ((rect.width() - 40.0) / (CHIP + CHIP_GAP)).floor().max(1.0) as usize;
    let items: Vec<Option<ModelId>> = std::iter::once(None)
        .chain(stack.iter().copied().map(Some))
        .take(fits)
        .collect();
    let width = items.len() as f32 * (CHIP + CHIP_GAP) - CHIP_GAP;
    let top = rect.bottom() - 34.0 - CHIP;
    let left = rect.center().x - width / 2.0;
    for (i, item) in items.into_iter().enumerate() {
        let chip = Rect::from_min_size(
            pos2(left + i as f32 * (CHIP + CHIP_GAP), top),
            vec2(CHIP, CHIP),
        );
        let response = ui.interact(chip, ui.scope_id().with(("photo-chip", i)), Sense::click());
        let label = match item {
            None => "3D model".to_owned(),
            Some(id) => session.model(id).map(|m| m.name()).unwrap_or_default(),
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        let fill = if response.hovered() {
            Color32::from_gray(0x3a)
        } else {
            Color32::from_gray(0x2a)
        };
        ui.painter().rect_filled(chip, CornerRadius::same(3), fill);
        match item {
            None => {
                ui.painter().text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    "3D",
                    FontId::proportional(16.0),
                    theme::TEXT,
                );
            }
            Some(id) => crate::grid::thumbnail(ui, chip.shrink(3.0), id, session, thumbs),
        }
        if item == photo {
            ui.painter().rect_stroke(
                chip,
                CornerRadius::same(3),
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Inside,
            );
        }
        if response.clicked() {
            match item {
                None => actions.push("loupe.show_3d"),
                Some(id) => actions.push_args("loupe.show_photo", json!({ "id": id })),
            }
        }
        response.on_hover_text(label);
    }
}

fn centered(painter: &egui::Painter, rect: Rect, text: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(16.0),
        theme::TEXT_DIM,
    );
}

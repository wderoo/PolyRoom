//! The 3D loupe: renders the current model with wgpu into a texture shown by egui.
//! Left-drag orbits, right/middle-drag pans, the wheel zooms, double-click resets the view.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
use polyroom_engine::{ModelId, Session};
use polyroom_render::{Camera, GpuMesh, Renderer, Style, Target};

use crate::theme;

/// The centre background colour (#1c1c1e), matching [`theme::CENTER`].
const BACKGROUND: [f32; 4] = [28.0 / 255.0, 28.0 / 255.0, 30.0 / 255.0, 1.0];

#[derive(Default)]
pub struct LoupeView {
    renderer: Option<Renderer>,
    target: Option<Target>,
    texture: Option<egui::TextureId>,
    mesh: Option<(ModelId, GpuMesh)>,
    camera: Option<Camera>,
    /// The camera/size/mesh the texture was last rendered with.
    rendered: Option<(Camera, u32, u32, ModelId)>,
    upload_error: Option<String>,
}

impl LoupeView {
    pub fn show(&mut self, ui: &mut Ui, frame: &eframe::Frame, session: &Session) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::CENTER);

        let Some(render_state) = frame.wgpu_render_state() else {
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
            let state = Some((camera, w, h, *id));
            if self.rendered != state {
                let style = Style {
                    background: BACKGROUND,
                    base_color: [0.62, 0.60, 0.57],
                    show_grid: true,
                };
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
        painter.text(
            rect.left_bottom() + vec2(14.0, -10.0),
            Align2::LEFT_BOTTOM,
            "Drag to orbit · Right-drag to pan · Scroll to zoom · Double-click to reset",
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
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

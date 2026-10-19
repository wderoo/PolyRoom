//! The axis gizmo in a corner of the 3D view. It shows how the world axes are oriented and
//! snaps the camera to look along an axis when one of its handles is clicked.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, vec2};
use polyroom_render::Camera;

/// Distance from the gizmo's centre to a handle, in logical pixels.
pub const ARM: f32 = 40.0;
const COLORS: [Color32; 3] = [
    Color32::from_rgb(226, 82, 72),
    Color32::from_rgb(104, 188, 84),
    Color32::from_rgb(72, 120, 232),
];
const NAMES: [&str; 3] = ["X", "Y", "Z"];

/// One end of an axis: where it is on screen relative to the centre, and how far from the viewer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    /// 0, 1, 2 for X, Y, Z.
    pub axis: usize,
    pub positive: bool,
    pub offset: Vec2,
    /// Larger is further from the viewer.
    pub depth: f32,
}

impl Handle {
    /// The world direction this handle points along.
    pub fn direction(&self) -> [f32; 3] {
        let mut d = [0.0; 3];
        if let Some(c) = d.get_mut(self.axis) {
            *c = if self.positive { 1.0 } else { -1.0 };
        }
        d
    }

    fn name(&self) -> &'static str {
        NAMES.get(self.axis).copied().unwrap_or("")
    }

    fn color(&self) -> Color32 {
        COLORS.get(self.axis).copied().unwrap_or(Color32::GRAY)
    }
}

/// The six handles as seen by `camera`, furthest first so that drawing in order puts the
/// nearest on top.
pub fn handles(camera: &Camera) -> Vec<Handle> {
    let mut out = Vec::with_capacity(6);
    for axis in 0..3 {
        for positive in [true, false] {
            let mut handle = Handle {
                axis,
                positive,
                offset: Vec2::ZERO,
                depth: 0.0,
            };
            let [right, up, away] = camera.view_dir(handle.direction());
            handle.offset = vec2(right, -up) * ARM;
            handle.depth = away;
            out.push(handle);
        }
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// Draws the gizmo centred at `center` and returns the direction to look along if a handle was
/// clicked.
pub fn show(ui: &mut Ui, center: Pos2, camera: &Camera) -> Option<[f32; 3]> {
    let mut clicked = None;
    let near = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| p.distance(center) < ARM + 16.0);
    if near {
        ui.painter()
            .circle_filled(center, ARM + 14.0, Color32::from_white_alpha(14));
    }

    for handle in handles(camera) {
        let pos = center + handle.offset;
        let color = handle.color();
        // Handles pointing away fade, so the front ones read first.
        let fade = (0.75 - handle.depth * 0.25).clamp(0.45, 1.0);
        let radius = if handle.positive { 9.0 } else { 6.5 };
        let hit = Rect::from_center_size(pos, Vec2::splat(radius * 2.0 + 4.0));
        let id = ui.make_persistent_id(("axis_gizmo", handle.axis, handle.positive));
        let response = ui.interact(hit, id, Sense::click());
        let hovered = response.hovered();
        if response.clicked() {
            clicked = Some(handle.direction());
        }
        let painter = ui.painter();
        if handle.positive {
            painter.line_segment(
                [center, pos],
                Stroke::new(2.5, color.gamma_multiply(fade * 0.9)),
            );
            painter.circle_filled(pos, radius, color.gamma_multiply(fade));
            if hovered {
                painter.circle_stroke(pos, radius + 1.5, Stroke::new(1.5, Color32::WHITE));
            }
            painter.text(
                pos,
                Align2::CENTER_CENTER,
                handle.name(),
                FontId::proportional(11.0),
                Color32::from_black_alpha(200),
            );
        } else {
            painter.circle_filled(pos, radius, color.gamma_multiply(fade * 0.35));
            painter.circle_stroke(
                pos,
                radius,
                Stroke::new(1.5, color.gamma_multiply(fade * 0.8)),
            );
            if hovered {
                painter.circle_filled(pos, radius, color.gamma_multiply(0.8));
                painter.text(
                    pos,
                    Align2::CENTER_CENTER,
                    format!("-{}", handle.name()),
                    FontId::proportional(10.0),
                    Color32::from_black_alpha(200),
                );
            }
        }
        if hovered {
            let sign = if handle.positive { "+" } else { "-" };
            response.on_hover_text(format!("Look from {sign}{}", handle.name()));
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use polyroom_mesh::Aabb;

    use super::*;

    fn camera() -> Camera {
        Camera::fit(
            &Aabb {
                min: [0.0; 3],
                max: [10.0; 3],
            },
            1.0,
        )
    }

    #[test]
    fn up_axis_points_up_on_screen_and_nearest_handle_is_last() {
        let hs = handles(&camera());
        assert_eq!(hs.len(), 6);
        let z = hs.iter().find(|h| h.axis == 2 && h.positive).unwrap();
        assert!(z.offset.y < 0.0, "+Z should be above the centre");
        assert!(hs.windows(2).all(|w| w[0].depth >= w[1].depth));
    }

    #[test]
    fn snapping_to_an_axis_makes_it_the_nearest_handle() {
        let mut cam = camera();
        for (axis, positive) in [(0, true), (1, false), (2, true), (0, false)] {
            let h = Handle {
                axis,
                positive,
                offset: Vec2::ZERO,
                depth: 0.0,
            };
            cam.look_along(h.direction());
            let nearest = handles(&cam).pop().unwrap();
            assert_eq!((nearest.axis, nearest.positive), (axis, positive));
        }
    }
}

#[cfg(test)]
mod render_tests {
    use polyroom_mesh::Aabb;

    use super::*;

    /// Draws the gizmo for the default view and a top view and saves pictures; skipped without a GPU.
    #[test]
    fn saves_a_picture_for_eyeballing() {
        let _gpu = crate::GPU_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut camera = Camera::fit(
            &Aabb {
                min: [0.0; 3],
                max: [10.0; 3],
            },
            1.0,
        );
        for (i, axis) in [None, Some([0.0, 0.0, 1.0])].into_iter().enumerate() {
            if let Some(axis) = axis {
                camera.look_along(axis);
            }
            let mut harness = egui_kittest::Harness::builder()
                .with_size(vec2(140.0, 140.0))
                .build_ui(|ui| {
                    crate::theme::apply(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, crate::theme::CENTER);
                    show(ui, ui.max_rect().center(), &camera);
                });
            harness.run();
            if let Ok(image) = harness.render() {
                let out = std::env::temp_dir().join(format!("polyroom_gizmo_{i}.png"));
                image.save(&out).unwrap();
                eprintln!("rendered {}", out.display());
            }
        }
    }
}

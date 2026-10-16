//! wgpu mesh rendering for the loupe (into a texture the UI displays) and for thumbnails
//! (headless, read back to RGBA).

#![forbid(unsafe_code)]

mod camera;

use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use polyroom_mesh::{Aabb, Mesh};
use thiserror::Error;
use wgpu::util::DeviceExt;

pub use camera::Camera;

/// Storage format of render targets; egui samples native textures in this format.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Format we render in. The shader encodes to sRGB itself: resolving into an sRGB view of a
/// UNORM texture is not applied consistently by drivers.
const VIEW_FORMAT: wgpu::TextureFormat = TARGET_FORMAT;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("no usable GPU adapter: {0}")]
    NoAdapter(String),
    #[error("could not open GPU device: {0}")]
    Device(String),
    #[error("GPU readback failed: {0}")]
    Readback(String),
    #[error("model has no triangles")]
    EmptyMesh,
    #[error("model is too large for this GPU ({triangles} triangles)")]
    TooLarge { triangles: usize },
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    base_color: [f32; 4],
    line_color: [f32; 4],
}

/// How to draw a frame.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Clear colour, sRGB-encoded RGBA (premultiplied).
    pub background: [f32; 4],
    /// Model colour in linear RGB.
    pub base_color: [f32; 3],
    pub show_grid: bool,
}

impl Style {
    /// Neutral light grey on a transparent background, for thumbnails.
    pub const THUMBNAIL: Self = Self {
        background: [0.0; 4],
        base_color: [0.62, 0.60, 0.57],
        show_grid: false,
    };
}

/// A mesh uploaded to the GPU.
pub struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    grid: Option<(wgpu::Buffer, u32)>,
    pub bounds: Aabb,
}

/// Colour + depth attachments of one size. Recreate when the viewport size changes.
pub struct Target {
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Target {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = |label, samples, format, view_formats: &[wgpu::TextureFormat], usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = texture(
            "polyroom msaa",
            SAMPLES,
            VIEW_FORMAT,
            &[],
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth = texture(
            "polyroom depth",
            SAMPLES,
            DEPTH_FORMAT,
            &[],
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolve = texture(
            "polyroom resolve",
            1,
            TARGET_FORMAT,
            &[],
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let resolve_view = resolve.create_view(&wgpu::TextureViewDescriptor {
            format: Some(VIEW_FORMAT),
            ..Default::default()
        });
        Self {
            msaa: msaa.create_view(&Default::default()),
            depth: depth.create_view(&Default::default()),
            resolve,
            resolve_view,
            width,
            height,
        }
    }

    /// A view of the final image in [`TARGET_FORMAT`], for display.
    pub fn display_view(&self) -> wgpu::TextureView {
        self.resolve.create_view(&Default::default())
    }

    fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

/// Pipelines and uniforms; one per device.
pub struct Renderer {
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("polyroom uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("polyroom uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("polyroom uniforms"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("polyroom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = |label, fs: &str, topology, blend, depth_write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: VIEW_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            mesh_pipeline: pipeline(
                "polyroom mesh",
                "fs_mesh",
                wgpu::PrimitiveTopology::TriangleList,
                None,
                true,
            ),
            line_pipeline: pipeline(
                "polyroom grid",
                "fs_line",
                wgpu::PrimitiveTopology::LineList,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                false,
            ),
            uniforms,
            bind_group,
        }
    }

    pub fn upload(&self, device: &wgpu::Device, mesh: &Mesh) -> Result<GpuMesh, RenderError> {
        let bounds = mesh.bounds().ok_or(RenderError::EmptyMesh)?;
        let index_count =
            u32::try_from(mesh.indices().len()).map_err(|_| RenderError::EmptyMesh)?;
        // Exceeding the device's buffer limit is a wgpu validation error, which panics by
        // default; refuse up front instead.
        let max = device.limits().max_buffer_size;
        let bytes = |n: usize, each: u64| (n as u64).saturating_mul(each);
        if bytes(mesh.positions().len(), 12) > max || bytes(mesh.indices().len(), 4) > max {
            return Err(RenderError::TooLarge {
                triangles: mesh.triangle_count(),
            });
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("polyroom vertices"),
            contents: bytemuck::cast_slice(mesh.positions()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("polyroom indices"),
            contents: bytemuck::cast_slice(mesh.indices()),
            usage: wgpu::BufferUsages::INDEX,
        });
        let lines = grid_lines(&bounds);
        let grid = u32::try_from(lines.len()).ok().map(|n| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("polyroom grid"),
                contents: bytemuck::cast_slice(&lines),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, n)
        });
        Ok(GpuMesh {
            vertices,
            indices,
            index_count,
            grid,
            bounds,
        })
    }

    /// Renders `mesh` into `target`. Submits its own command buffer.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &Target,
        mesh: Option<&GpuMesh>,
        camera: &Camera,
        style: &Style,
    ) {
        let aspect = target.aspect();
        let u = Uniforms {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            eye: camera.eye().extend(1.0).into(),
            right: camera.right().extend(0.0).into(),
            up: camera.up().extend(0.0).into(),
            base_color: Vec3::from(style.base_color).extend(1.0).into(),
            line_color: [0.5, 0.5, 0.52, 0.35],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = style.background.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("polyroom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(mesh) = mesh {
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                if let (true, Some((grid, n))) = (style.show_grid, &mesh.grid) {
                    pass.set_pipeline(&self.line_pipeline);
                    pass.set_vertex_buffer(0, grid.slice(..));
                    pass.draw(0..*n, 0..1);
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
}

/// The smallest 1/2/5 × 10ⁿ step that is at least `x` (and at least 1 mm).
fn nice_step(x: f32) -> f32 {
    let x = x.max(1.0);
    let p = 10f32.powf(x.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * p)
        .find(|s| *s >= x)
        .unwrap_or(10.0 * p)
}

/// Build-plate grid lines under the model, at a round spacing giving at most ~40 lines.
fn grid_lines(bounds: &Aabb) -> Vec<[f32; 3]> {
    let size = bounds.size();
    let extent = size.x.max(size.y) * 1.6 + 1.0;
    if !extent.is_finite() {
        return Vec::new();
    }
    let spacing = nice_step(extent / 40.0);
    let c = bounds.center();
    let half = (extent / spacing / 2.0).ceil() * spacing;
    let (cx, cy) = (
        (c.x / spacing).round() * spacing,
        (c.y / spacing).round() * spacing,
    );
    // Slightly below the model so the grid doesn't fight with a flat bottom face.
    let z = bounds.min[2] - size.z.max(1.0) * 1e-3;
    // Bounded no matter what the bounds say: a corrupt model once asked for billions of lines.
    let steps = ((2.0 * half / spacing).round() as i32).clamp(0, 100);
    let mut out = Vec::with_capacity(4 * (steps as usize + 1));
    for i in 0..=steps {
        let t = -half + i as f32 * spacing;
        out.extend_from_slice(&[
            [cx + t, cy - half, z],
            [cx + t, cy + half, z],
            [cx - half, cy + t, z],
            [cx + half, cy + t, z],
        ]);
    }
    out
}

/// An RGBA8 image with premultiplied alpha, sRGB-encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A GPU device of its own, for rendering thumbnails off the UI thread.
pub struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
}

impl Headless {
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("polyroom thumbnails"),
            // As much as the GPU allows, so big models still get thumbnails.
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        // wgpu panics on validation errors by default; log them instead.
        device.on_uncaptured_error(std::sync::Arc::new(|e| {
            log::error!("GPU error while rendering a thumbnail: {e}");
        }));
        let renderer = Renderer::new(&device);
        Ok(Self {
            device,
            queue,
            renderer,
        })
    }

    /// Renders a square three-quarter view of `mesh`.
    pub fn thumbnail(&self, mesh: &Mesh, size: u32) -> Result<Image, RenderError> {
        let gpu = self.renderer.upload(&self.device, mesh)?;
        let target = Target::new(&self.device, size, size);
        let camera = Camera::fit(&gpu.bounds, 1.0);
        self.renderer.render(
            &self.device,
            &self.queue,
            &target,
            Some(&gpu),
            &camera,
            &Style::THUMBNAIL,
        );
        self.read_back(&target)
    }

    fn read_back(&self, target: &Target) -> Result<Image, RenderError> {
        let (w, h) = (target.width, target.height);
        let row = w * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("polyroom readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.resolve,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let (tx, rx) = mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;

        let data = buffer
            .get_mapped_range(..)
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut rgba = Vec::with_capacity((row * h) as usize);
        for r in data.chunks(padded as usize).take(h as usize) {
            rgba.extend_from_slice(r.get(..row as usize).unwrap_or_default());
        }
        drop(data);
        buffer.unmap();
        Ok(Image {
            width: w,
            height: h,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_bounded_and_under_the_model() {
        let b = Aabb {
            min: [0.0, 0.0, 5.0],
            max: [220.0, 220.0, 50.0],
        };
        let lines = grid_lines(&b);
        assert!(lines.len() <= 4 * 42);
        assert!(lines.iter().all(|p| p[2] < 5.0));
    }

    #[test]
    fn grid_stays_small_for_absurd_bounds() {
        for v in [1e6, 3.4e38, f32::INFINITY] {
            let b = Aabb {
                min: [-v, -v, -v],
                max: [v, v, v],
            };
            assert!(grid_lines(&b).len() <= 4 * 101, "{v}");
        }
    }

    #[test]
    fn clear_color_is_stored_as_given() {
        let Ok(gpu) = Headless::new() else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let target = Target::new(&gpu.device, 4, 4);
        let style = Style {
            background: [28.0 / 255.0, 28.0 / 255.0, 28.0 / 255.0, 1.0],
            ..Style::THUMBNAIL
        };
        let cam = Camera::fit(&polyroom_mesh::test_cube().bounds().unwrap(), 1.0);
        gpu.renderer
            .render(&gpu.device, &gpu.queue, &target, None, &cam, &style);
        let img = gpu.read_back(&target).unwrap();
        assert!((26..=29).contains(&img.rgba[0]), "{}", img.rgba[0]);
    }

    /// Needs a GPU (or a software adapter); skipped when none is available.
    #[test]
    fn headless_cube_thumbnail_has_coverage() {
        let Ok(gpu) = Headless::new() else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let img = gpu.thumbnail(&polyroom_mesh::test_cube(), 64).unwrap();
        assert_eq!(img.rgba.len(), 64 * 64 * 4);
        let opaque = img.rgba.chunks(4).filter(|p| p[3] == 255).count();
        // The cube should cover a good chunk of the frame but not all of it.
        assert!(opaque > 64 * 64 / 5 && opaque < 64 * 64, "{opaque}");
    }
}

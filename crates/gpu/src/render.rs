//! The wgpu side: adapter and device, one instanced pipeline, offscreen target, readback.
//!
//! The renderer knows nothing about documents (see [`crate::scene`]); it draws a [`Scene`] into an
//! `Rgba8Unorm` texture with **premultiplied** blending (`One` / `OneMinusSrcAlpha`), then reads the
//! pixels back row by row (every row padded to wgpu's 256-byte copy alignment).
//!
//! A scene that lists an unsupported feature is refused here, not drawn "best effort": the caller
//! (canvas or export) then renders the same frame with `figcraft-render` on the CPU.

use std::sync::{Mutex, OnceLock};

use bytemuck::{Pod, Zeroable};
use figcraft_geom::Affine;

use crate::scene::{Paint, Scene, ShapeKind, ShapeNode, Unsupported};

/// Why a frame did not render on the GPU.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderError {
    /// The frame uses features this milestone cannot draw; the CPU must take it.
    Unsupported(Vec<Unsupported>),
    /// The frame has no drawable size.
    Size(String),
    /// The device was lost or a submission failed.
    Device(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(items) => {
                let features: Vec<&str> = items.iter().map(|u| u.feature).collect();
                write!(f, "GPU cannot draw this frame: {}", features.join(", "))
            }
            Self::Size(s) | Self::Device(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for RenderError {}

/// The instance as the shader sees it. 256 bytes, every member 16-byte aligned, no padding.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuInstance {
    m: [f32; 4],
    off: [f32; 4],
    size: [f32; 4],
    radii: [f32; 4],
    params: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    clip: [f32; 4],
    stop_pos: [f32; 8],
    stop_col: [[f32; 4]; 6],
}

impl GpuInstance {
    fn of(node: &ShapeNode) -> Self {
        let (shape_kind, radii, params) = match node.shape {
            ShapeKind::RoundedRect { radii } => (0.0, radii, [0.0; 4]),
            ShapeKind::Ellipse => (1.0, [0.0; 4], [0.0; 4]),
            ShapeKind::Polygon { count } => (2.0, [0.0; 4], [count as f32, 0.0, 0.0, 0.0]),
            ShapeKind::Star { count, inner_ratio } => (3.0, [0.0; 4], [count as f32, inner_ratio, 0.0, 0.0]),
        };
        let mut stop_pos = [0.0f32; 8];
        let mut stop_col = [[0.0f32; 4]; 6];
        let mut p0 = [0.0f32; 4];
        let mut p1 = [0.0f32; 4];
        let (paint_kind, stop_count) = match &node.paint {
            Paint::Solid(c) => {
                stop_col[0] = c.premultiplied();
                (0.0, 1.0)
            }
            Paint::Gradient { kind, from, to, width, stops } => {
                let kind_code = match kind {
                    crate::scene::GradientKind::Linear => 1.0,
                    crate::scene::GradientKind::Radial => 2.0,
                    crate::scene::GradientKind::Angular => 3.0,
                    crate::scene::GradientKind::Diamond => 4.0,
                };
                for (i, s) in stops.iter().enumerate().take(6) {
                    stop_pos[i] = s.at;
                    stop_col[i] = s.color.premultiplied();
                }
                p0 = [from[0], from[1], to[0], to[1]];
                p1 = [width[0], width[1], 0.0, 0.0];
                (kind_code, stops.len().min(6) as f32)
            }
        };
        Self {
            m: [node.mat[0], node.mat[1], node.mat[2], node.mat[3]],
            off: [node.mat[4], node.mat[5], node.opacity, shape_kind],
            size: [node.size[0], node.size[1], paint_kind, stop_count],
            radii,
            params,
            p0,
            p1,
            clip: node.clip.0,
            stop_pos,
            stop_col,
        }
    }
}

struct Target {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// Offscreen GPU renderer. One per process (the canvas and the exporter share it through
/// [`global`]), reused across frames.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_globals: wgpu::BindGroup,
    instances_layout: wgpu::BindGroupLayout,
    instances: Option<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    target: Option<Target>,
    adapter: String,
    max_side: u32,
}

impl Renderer {
    /// Build a device and the pipeline. `None` when the machine has no adapter that meets the limits
    /// (see [`crate::backend`]); the caller falls back to the CPU.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let info = adapter.get_info();
        let mut limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        limits.max_storage_buffers_per_shader_stage = adapter.limits().max_storage_buffers_per_shader_stage.min(8);
        limits.max_storage_buffer_binding_size = adapter.limits().max_storage_buffer_binding_size;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("figcraft-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-shape"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shape.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let instances_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape"),
            bind_group_layouts: &[Some(&globals_layout), Some(&instances_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });

        let max_side = device.limits().max_texture_dimension_2d;
        Some(Self {
            device,
            queue,
            pipeline,
            globals,
            bind_globals,
            instances_layout,
            instances: None,
            target: None,
            adapter: format!("{} ({:?})", info.name, info.backend),
            max_side,
        })
    }

    /// The adapter this renderer draws on, for the preferences row and logs.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Draw `scene` and read it back as premultiplied RGBA8, `width * height * 4` bytes.
    pub fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, RenderError> {
        if !scene.is_supported() {
            return Err(RenderError::Unsupported(scene.unsupported_features().to_vec()));
        }
        let (w, h) = (scene.width, scene.height);
        if w == 0 || h == 0 {
            return Err(RenderError::Size("the frame has no drawable size".into()));
        }
        if w > self.max_side || h > self.max_side {
            return Err(RenderError::Size(format!("{w}×{h} is past the device's {}-pixel side", self.max_side)));
        }

        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let globals = [w as f32, h as f32, 0.0, 0.0];
        self.queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));

        let instances: Vec<GpuInstance> = scene.nodes.iter().map(GpuInstance::of).collect();
        let count = instances.len().max(1) as u32;
        let bytes: Vec<u8> = if instances.is_empty() {
            // An empty scene still needs a binding; one zero instance is never drawn.
            bytemuck::bytes_of(&GpuInstance::zeroed()).to_vec()
        } else {
            bytemuck::cast_slice(&instances).to_vec()
        };
        let need = bytes.len() as u64;
        if self.instances.as_ref().is_none_or(|(_, _, cap)| *cap < need) {
            let size = need.next_power_of_two().max(256);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instances"),
                layout: &self.instances_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            });
            self.instances = Some((buffer, bind, size));
        }
        let Some((buffer, bind, _)) = self.instances.as_ref() else {
            return Err(RenderError::Device("the instance buffer was not created".into()));
        };
        self.queue.write_buffer(buffer, 0, &bytes);

        let unpadded = w * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if self.target.as_ref().is_none_or(|t| t.width != w || t.height != h) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("target"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(padded) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target { width: w, height: h, texture, view, readback });
        }
        let Some(target) = self.target.as_ref() else {
            return Err(RenderError::Device("the render target was not created".into()));
        };

        let clear = scene.clear;
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shapes"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear.r * clear.a),
                            g: f64::from(clear.g * clear.a),
                            b: f64::from(clear.b * clear.a),
                            a: f64::from(clear.a),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !scene.nodes.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_globals, &[]);
                pass.set_bind_group(1, bind, &[]);
                pass.draw(0..6, 0..count);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));

        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(RenderError::Device(format!("the GPU rejected the frame: {error}")));
        }

        let slice = target.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let mapped = rx.recv().ok().and_then(Result::ok);
        if mapped.is_none() {
            return Err(RenderError::Device("the frame could not be read back".into()));
        }
        let Ok(view) = slice.get_mapped_range() else {
            return Err(RenderError::Device("the read-back buffer could not be mapped".into()));
        };
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            out.extend_from_slice(&view[start..start + unpadded as usize]);
        }
        drop(view);
        target.readback.unmap();
        Ok(out)
    }
}

/// The process-wide renderer, created once. `None` when the machine has no usable adapter — the
/// caller then draws on the CPU and the preference row says why.
pub fn global() -> Option<&'static Mutex<Renderer>> {
    static GLOBAL: OnceLock<Option<Mutex<Renderer>>> = OnceLock::new();
    GLOBAL.get_or_init(|| Renderer::new().map(Mutex::new)).as_ref()
}

/// Draw one frame on the GPU when `effective()` says so; `None` means "draw it on the CPU".
///
/// The scene's own unsupported list, a missing adapter, a lost device and any readback failure all
/// come back as `None`: the canvas then takes its usual path, never an empty or half-drawn frame.
pub fn render_if_gpu(scene: &Scene) -> Option<Vec<u8>> {
    if !crate::prefs::effective().is_gpu() || !scene.is_supported() {
        return None;
    }
    let global = global()?;
    let mut renderer = global.lock().ok()?;
    renderer.render(scene).ok()
}

/// A document-space matrix as the affine helper `scene` expects, for callers that build scenes.
pub fn affine_of(m: [f32; 6]) -> Affine {
    Affine::new([f64::from(m[0]), f64::from(m[1]), f64::from(m[2]), f64::from(m[3]), f64::from(m[4]), f64::from(m[5])])
}

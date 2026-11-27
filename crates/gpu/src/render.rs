//! The wgpu side: adapter and device, two pipelines (instanced SDF shapes, tessellated meshes),
//! one multisampled offscreen target, readback.
//!
//! The renderer knows nothing about documents (see [`crate::scene`]); it draws a [`Scene`] into a
//! 4× multisampled `Rgba8Unorm` target, resolves it to a single-sample texture with **premultiplied**
//! blending (`One` / `OneMinusSrcAlpha`), then reads the pixels back row by row (every row padded to
//! wgpu's 256-byte copy alignment).
//!
//! Why multisampled: tessellated paths have no analytic coverage to compute in the shader, so the
//! samples are the coverage. The SDF shapes keep their own `fwidth` coverage; their quads are drawn a
//! few pixels larger than the shape so rasterization never clips the antialiased edge (otherwise a
//! pixel at the box's border would get coverage × samples instead of coverage).
//!
//! A scene that lists an unsupported feature is refused here, not drawn "best effort": the caller
//! (canvas or export) then renders the same frame with `figcraft-render` on the CPU.

use std::sync::{Mutex, OnceLock};

use bytemuck::{Pod, Zeroable};
use figcraft_geom::Affine;

use crate::scene::{Clip, Item, Paint, Rgba, Scene, ShapeKind, ShapeNode, Unsupported};

/// Samples per pixel the target uses. Tessellated geometry gets its coverage from these.
const SAMPLES: u32 = 4;

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

/// The SDF instance as the shader sees it. 256 bytes, every member 16-byte aligned, no padding.
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

/// A tessellated vertex: device pixels, premultiplied colour, device-pixel clip.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuMeshVertex {
    pos: [f32; 2],
    color: [f32; 4],
    clip: [f32; 4],
}

/// What one draw in the pass does, in painter's order.
enum Cmd {
    /// Instances `first .. first + count` of the instance buffer.
    Shapes { first: u32, count: u32 },
    /// Indices `first .. first + count` of the shared index buffer.
    Mesh { first: u32, count: u32 },
    /// Blurred drop shadow `n` (index into the frame's shadow plans).
    Shadow(usize),
    /// A blurred backdrop: instance `first` of the instance buffer, blurred with `sigma` device pixels,
    /// drawn in the box's place before the node's own content.
    Backdrop { first: u32, sigma: f32 },
    /// Mono grain over instance `n` of the frame's noise plans.
    Noise(usize),
    /// Rough-edge texture over instance `n` of the frame's texture plans.
    Texture(usize),
}

/// One shadow to draw: its silhouette (one instance), the blur radius in device pixels, the tint and
/// whether it is an inner shadow (masked by the shape's own coverage) or a drop shadow. The
/// silhouette is rendered into an offscreen target, blurred by six box passes (three across, three
/// down, exactly as `figcraft_render::fx::blur`), then drawn as a textured quad.
struct ShadowPlan {
    first: u32,
    count: u32,
    /// The same shape at zero offset, for an inner shadow's mask (empty for a drop shadow).
    cover_first: u32,
    cover_count: u32,
    sigma: f32,
    color: Rgba,
    clip: Clip,
    inner: bool,
}

/// The textures and bind groups one shadow needs for a frame.
///
/// The raw textures and buffers are kept alongside the views and bind groups that reference them so
/// each frame's resources live and die together (a bind group keeps its view alive, but saying so
/// explicitly is clearer than relying on it).
#[allow(dead_code)]
struct ShadowResources {
    silhouette_msaa: wgpu::Texture,
    silhouette_msaa_view: wgpu::TextureView,
    silhouette: wgpu::Texture,
    silhouette_view: wgpu::TextureView,
    ping: wgpu::Texture,
    ping_view: wgpu::TextureView,
    pong: wgpu::Texture,
    pong_view: wgpu::TextureView,
    /// The shape at zero offset, sampled as an inner shadow's mask.
    cover_msaa: Option<wgpu::Texture>,
    cover_msaa_view: Option<wgpu::TextureView>,
    cover_view: Option<wgpu::TextureView>,
    cover: Option<wgpu::Texture>,
    /// Six passes: inputs are silhouette → ping → pong → ping → pong → ping.
    blur_binds: Vec<wgpu::BindGroup>,
    /// Kept alive for the bind groups' uniform buffers.
    blur_params: Vec<wgpu::Buffer>,
    /// The final blurred texture + tint + clip (drop shadow).
    draw_bind: wgpu::BindGroup,
    /// The blurred texture + shape mask + tint + clip (inner shadow).
    inner_bind: Option<wgpu::BindGroup>,
    tint: wgpu::Buffer,
}

/// One mono grain layer: the instance that masks it, the CPU's cell hash seed and size, the share of
/// cells drawn, and the grain colour/alpha over the fill.
struct NoisePlan {
    first: u32,
    seed: u32,
    density: f32,
    fill_alpha: f32,
    cell: [f32; 2],
    color: Rgba,
}

/// Uniforms for [`NoisePlan`]; 48 bytes, every vec4/vec2 at its natural alignment.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct NoiseParams {
    seed: u32,
    density: f32,
    fill_alpha: f32,
    _pad0: f32,
    cell: [f32; 2],
    _pad1: [f32; 2],
    color: [f32; 4],
}

/// One rough-edge texture layer: the instance that masks it, the CPU's cell hash seed, the scatter
/// radius in device pixels, the shape clip flag, and the fill whose pixels are scattered.
struct TexturePlan {
    first: u32,
    seed: u32,
    radius: f32,
    clip_to_shape: bool,
    cell: [f32; 2],
    fill: Rgba,
}

/// Uniforms for [`TexturePlan`]; same 48-byte layout as [`NoiseParams`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextureParams {
    seed: u32,
    radius: f32,
    clip_to_shape: u32,
    _pad0: u32,
    cell: [f32; 2],
    _pad1: [f32; 2],
    color: [f32; 4],
}

/// A background blur's chain: the resolved frame (in place of the shadow's silhouette), two ping-pong
/// textures, six box passes, and the quad that shows the last pass through the box.
struct BackdropResources {
    _ping: wgpu::Texture,
    ping_view: wgpu::TextureView,
    _pong: wgpu::Texture,
    pong_view: wgpu::TextureView,
    /// Six passes: inputs are frame → ping → pong → ping → pong → ping.
    blur_binds: Vec<wgpu::BindGroup>,
    /// Kept alive for the bind groups' uniform buffers.
    _blur_params: Vec<wgpu::Buffer>,
    /// The final blurred texture, sampled at the node's box.
    draw_bind: wgpu::BindGroup,
}

/// Box radii for three passes approximating a Gaussian of `sigma` — the CPU's `fx::box_sizes`,
/// replicated so both sides blur with the same widths.
fn box_radii(sigma: f32) -> [u32; 3] {
    if !(sigma.is_finite() && sigma > 0.3) {
        return [0, 0, 0];
    }
    let sigma = f64::from(sigma);
    let n = 3.0;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    let r = |i: i64| -> u32 {
        let w = if i < m { wl } else { wu };
        ((w.max(1) - 1) / 2) as u32
    };
    [r(0), r(1), r(2)]
}

struct Target {
    width: u32,
    height: u32,
    /// The 4× attachment the pass draws into.
    /// Kept alive for the lifetime of `msaa_view` (a view keeps its texture alive, but holding both
    /// makes the intent explicit).
    #[allow(dead_code)]
    msaa: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    /// The resolved single-sample image, the copy source.
    resolve: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// Offscreen GPU renderer. One per process (the canvas and the exporter share it through
/// [`global`]), reused across frames.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    inner_pipeline: wgpu::RenderPipeline,
    backdrop_pipeline: wgpu::RenderPipeline,
    noise_pipeline: wgpu::RenderPipeline,
    texture_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    bind_globals: wgpu::BindGroup,
    instances_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    inner_layout: wgpu::BindGroupLayout,
    backdrop_layout: wgpu::BindGroupLayout,
    noise_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    instances: Option<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    mesh_verts: Option<(wgpu::Buffer, u64)>,
    mesh_indices: Option<(wgpu::Buffer, u64)>,
    target: Option<Target>,
    adapter: String,
    max_side: u32,
}

fn blend() -> Option<wgpu::BlendState> {
    Some(wgpu::BlendState {
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
    })
}

impl Renderer {
    /// Build a device and the pipelines. `None` when the machine has no adapter that meets the limits
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
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh.wgsl").into()),
        });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blur.wgsl").into()),
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let inner_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-inner-shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/inner.wgsl").into()),
        });
        let backdrop_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/backdrop.wgsl").into()),
        });
        let noise_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-noise"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/noise.wgsl").into()),
        });
        let texture_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figcraft-gpu-texture"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/texture.wgsl").into()),
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
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let inner_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inner shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let backdrop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let noise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("noise"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let shape_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape"),
            bind_group_layouts: &[Some(&globals_layout), Some(&instances_layout)],
            immediate_size: 0,
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let target_state =
            |_label: &str| wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: blend(), write_mask: wgpu::ColorWrites::ALL };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape"),
            layout: Some(&shape_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(target_state("shape"))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuMeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x2 },
                wgpu::VertexAttribute { offset: 8, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: 24, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
            ],
        };
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs"),
                buffers: &[Some(vertex_layout)],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs"),
                targets: &[Some(target_state("mesh"))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur"),
            bind_group_layouts: &[Some(&blur_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&globals_layout), Some(&shadow_layout)],
            immediate_size: 0,
        });
        let inner_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("inner shadow"),
            bind_group_layouts: &[Some(&globals_layout), Some(&inner_layout)],
            immediate_size: 0,
        });
        let backdrop_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop"),
            bind_group_layouts: &[Some(&backdrop_layout), Some(&instances_layout)],
            immediate_size: 0,
        });
        let noise_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("noise"),
            bind_group_layouts: &[Some(&noise_layout), Some(&instances_layout)],
            immediate_size: 0,
        });
        let texture_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("texture"),
            bind_group_layouts: &[Some(&texture_layout), Some(&instances_layout)],
            immediate_size: 0,
        });
        let single_state = |b: Option<wgpu::BlendState>| wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: b,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(None))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(blend()))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            // The main pass draws into the 4× target; a full-screen quad covers every sample with the
            // same blurred alpha, so multisampling changes nothing here.
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let inner_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("inner shadow"),
            layout: Some(&inner_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &inner_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &inner_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(blend()))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let backdrop_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("backdrop"),
            layout: Some(&backdrop_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &backdrop_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &backdrop_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(blend()))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let noise_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("noise"),
            layout: Some(&noise_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &noise_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &noise_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(blend()))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let texture_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("texture"),
            layout: Some(&texture_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &texture_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &texture_shader,
                entry_point: Some("fs"),
                targets: &[Some(single_state(blend()))],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
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
            mesh_pipeline,
            blur_pipeline,
            shadow_pipeline,
            inner_pipeline,
            backdrop_pipeline,
            noise_pipeline,
            texture_pipeline,
            sampler,
            globals,
            bind_globals,
            instances_layout,
            blur_layout,
            shadow_layout,
            inner_layout,
            backdrop_layout,
            noise_layout,
            texture_layout,
            instances: None,
            mesh_verts: None,
            mesh_indices: None,
            target: None,
            adapter: format!("{} ({:?})", info.name, info.backend),
            max_side,
        })
    }

    /// The adapter this renderer draws on, for the preferences row and logs.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// One shadow's offscreen targets, blur bind groups and draw bind group for this frame.
    fn shadow_resources(&self, width: u32, height: u32, plan: &ShadowPlan) -> Option<ShadowResources> {
        use wgpu::TextureUsages as U;
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = |label: &str, samples: u32, usage: U| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage,
                view_formats: &[],
            })
        };
        let silhouette_msaa = texture("shadow-msaa", SAMPLES, U::RENDER_ATTACHMENT);
        let silhouette_msaa_view = silhouette_msaa.create_view(&wgpu::TextureViewDescriptor::default());
        let silhouette = texture("shadow-silhouette", 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let silhouette_view = silhouette.create_view(&wgpu::TextureViewDescriptor::default());
        let ping = texture("shadow-ping", 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let ping_view = ping.create_view(&wgpu::TextureViewDescriptor::default());
        let pong = texture("shadow-pong", 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let pong_view = pong.create_view(&wgpu::TextureViewDescriptor::default());

        // An inner shadow also needs the shape at zero offset: its blur is masked by that coverage.
        let (cover_msaa, cover) = if plan.inner {
            let msaa = texture("inner-cover-msaa", SAMPLES, U::RENDER_ATTACHMENT);
            let resolve = texture("inner-cover", 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
            (Some(msaa), Some(resolve))
        } else {
            (None, None)
        };

        let radii = box_radii(plan.sigma);
        let mut blur_binds = Vec::with_capacity(6);
        let mut blur_params = Vec::with_capacity(6);
        let inputs: [&wgpu::TextureView; 6] = [&silhouette_view, &ping_view, &pong_view, &ping_view, &pong_view, &ping_view];
        for (i, input) in inputs.iter().enumerate() {
            // The CPU inverts the silhouette's alpha before blurring it (`fx::invert_alpha`), so the
            // first pass reads 255 - byte and the remaining five blur those bytes.
            let invert = i32::from(plan.inner && i == 0);
            let params = [radii[i % 3] as i32, i32::from(i < 3), invert, 0];
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blur params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blur"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
                    wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() },
                ],
            });
            blur_params.push(buffer);
            blur_binds.push(bind);
        }

        let tint = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow tint"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tint_values = [plan.color.r, plan.color.g, plan.color.b, plan.color.a, plan.clip.0[0], plan.clip.0[1], plan.clip.0[2], plan.clip.0[3]];
        self.queue.write_buffer(&tint, 0, bytemuck::cast_slice(&tint_values));
        let draw_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow draw"),
            layout: &self.shadow_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&pong_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: tint.as_entire_binding() },
            ],
        });
        let cover_view = cover.as_ref().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let cover_msaa_view = cover_msaa.as_ref().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let inner_bind = cover_view.as_ref().map(|view| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("inner shadow"),
                layout: &self.inner_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&pong_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: tint.as_entire_binding() },
                ],
            })
        });

        Some(ShadowResources {
            silhouette_msaa,
            silhouette_msaa_view,
            silhouette,
            silhouette_view,
            ping,
            ping_view,
            pong,
            pong_view,
            cover_msaa,
            cover_msaa_view,
            cover_view,
            cover,
            blur_binds,
            blur_params,
            draw_bind,
            inner_bind,
            tint,
        })
    }

    /// One grain layer's uniforms. There is nothing offscreen: the shader hashes and masks in place.
    fn noise_resources(&self, plan: &NoisePlan) -> (wgpu::Buffer, wgpu::BindGroup) {
        let params = NoiseParams {
            seed: plan.seed,
            density: plan.density,
            fill_alpha: plan.fill_alpha,
            _pad0: 0.0,
            cell: plan.cell,
            _pad1: [0.0; 2],
            color: [plan.color.r, plan.color.g, plan.color.b, plan.color.a],
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("noise params"),
            size: std::mem::size_of::<NoiseParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("noise"),
            layout: &self.noise_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        (buffer, bind)
    }

    /// One texture layer's uniforms. Nothing offscreen: the shader reads the fill at the moved pixel.
    fn texture_resources(&self, plan: &TexturePlan) -> (wgpu::Buffer, wgpu::BindGroup) {
        let params = TextureParams {
            seed: plan.seed,
            radius: plan.radius,
            clip_to_shape: u32::from(plan.clip_to_shape),
            _pad0: 0,
            cell: plan.cell,
            _pad1: [0.0; 2],
            color: [plan.fill.r, plan.fill.g, plan.fill.b, plan.fill.a],
        };
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture params"),
            size: std::mem::size_of::<TextureParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        (buffer, bind)
    }

    /// The blur chain for one background blur: the resolved frame as the source, then six box passes.
    fn backdrop_resources(&self, width: u32, height: u32, sigma: f32, source: &wgpu::TextureView) -> Option<BackdropResources> {
        use wgpu::TextureUsages as U;
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = |label: &str| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let ping = texture("backdrop-ping");
        let ping_view = ping.create_view(&wgpu::TextureViewDescriptor::default());
        let pong = texture("backdrop-pong");
        let pong_view = pong.create_view(&wgpu::TextureViewDescriptor::default());

        let radii = box_radii(sigma);
        let mut blur_binds = Vec::with_capacity(6);
        let mut blur_params = Vec::with_capacity(6);
        let inputs: [&wgpu::TextureView; 6] = [source, &ping_view, &pong_view, &ping_view, &pong_view, &ping_view];
        for (i, input) in inputs.iter().enumerate() {
            let params = [radii[i % 3] as i32, i32::from(i < 3), 0, 0];
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("backdrop blur params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("backdrop blur"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
                    wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() },
                ],
            });
            blur_binds.push(bind);
            blur_params.push(buffer);
        }

        let draw_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop draw"),
            layout: &self.backdrop_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&pong_view) }],
        });
        Some(BackdropResources { _ping: ping, ping_view, _pong: pong, pong_view, blur_binds, _blur_params: blur_params, draw_bind })
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

        // Walk the items once: instances, mesh vertices/indices and the draw list.
        let mut instances: Vec<GpuInstance> = Vec::new();
        let mut shadow_instances: Vec<GpuInstance> = Vec::new();
        let mut cover_instances: Vec<GpuInstance> = Vec::new();
        let mut shadow_plans: Vec<ShadowPlan> = Vec::new();
        let mut noise_plans: Vec<NoisePlan> = Vec::new();
        let mut texture_plans: Vec<TexturePlan> = Vec::new();
        let mut verts: Vec<GpuMeshVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut cmds: Vec<Cmd> = Vec::new();
        let mut batch = 0usize;
        for item in &scene.items {
            match item {
                Item::Shape(shape) => instances.push(GpuInstance::of(shape)),
                Item::Mesh(mesh) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                        batch = instances.len();
                    }
                    let opacity = mesh.opacity.clamp(0.0, 1.0);
                    let premul = mesh.color.premultiplied();
                    let color = [premul[0] * opacity, premul[1] * opacity, premul[2] * opacity, premul[3] * opacity];
                    let base = verts.len() as u32;
                    verts.extend(mesh.verts.iter().map(|p| GpuMeshVertex { pos: *p, color, clip: mesh.clip.0 }));
                    let first = indices.len() as u32;
                    indices.extend(mesh.indices.iter().map(|i| i + base));
                    cmds.push(Cmd::Mesh { first, count: mesh.indices.len() as u32 });
                }
                Item::Shadow(shadow) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                        batch = instances.len();
                    }
                    // The silhouette is the shape itself in white: the same SDF coverage the fill has,
                    // shifted by the shadow's offset (already in device pixels).
                    let caster = ShapeNode {
                        mat: shadow.mat,
                        size: shadow.size,
                        shape: shadow.shape,
                        paint: Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                        clip: shadow.clip,
                        opacity: 1.0,
                    };
                    let mut offset_caster = caster.clone();
                    offset_caster.mat[4] += shadow.offset[0];
                    offset_caster.mat[5] += shadow.offset[1];
                    let first = shadow_instances.len() as u32;
                    shadow_instances.push(GpuInstance::of(&offset_caster));
                    let (cover_first, cover_count) = if shadow.inner {
                        let first = cover_instances.len() as u32;
                        cover_instances.push(GpuInstance::of(&caster));
                        (first, 1)
                    } else {
                        (0, 0)
                    };
                    cmds.push(Cmd::Shadow(shadow_plans.len()));
                    shadow_plans.push(ShadowPlan {
                        first,
                        count: 1,
                        cover_first,
                        cover_count,
                        sigma: shadow.sigma,
                        color: shadow.color,
                        clip: shadow.clip,
                        inner: shadow.inner,
                    });
                }
                Item::Blur(blur) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                        batch = instances.len();
                    }
                    // The blurred layer is the same white silhouette the shadow chain renders; the
                    // fill's colour comes back as the tint.
                    let caster = ShapeNode {
                        mat: blur.mat,
                        size: blur.size,
                        shape: blur.shape,
                        paint: Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                        clip: blur.clip,
                        opacity: 1.0,
                    };
                    let first = shadow_instances.len() as u32;
                    shadow_instances.push(GpuInstance::of(&caster));
                    cmds.push(Cmd::Shadow(shadow_plans.len()));
                    shadow_plans.push(ShadowPlan {
                        first,
                        count: 1,
                        cover_first: 0,
                        cover_count: 0,
                        sigma: blur.sigma,
                        color: blur.color,
                        clip: blur.clip,
                        inner: false,
                    });
                }
                Item::Backdrop(backdrop) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                    }
                    // The backdrop quad reads the node's box from the instance buffer: one white
                    // instance, never drawn by the shape pipeline.
                    let caster = ShapeNode {
                        mat: backdrop.mat,
                        size: backdrop.size,
                        shape: backdrop.shape,
                        paint: Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                        clip: backdrop.clip,
                        opacity: 1.0,
                    };
                    let first = instances.len() as u32;
                    instances.push(GpuInstance::of(&caster));
                    batch = instances.len();
                    cmds.push(Cmd::Backdrop { first, sigma: backdrop.sigma });
                }
                Item::Noise(noise) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                    }
                    // The noise quad reads the node's box and clip from the instance buffer: one white
                    // instance, never drawn by the shape pipeline.
                    let caster = ShapeNode {
                        mat: noise.mat,
                        size: noise.size,
                        shape: noise.shape,
                        paint: Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                        clip: noise.clip,
                        opacity: 1.0,
                    };
                    let first = instances.len() as u32;
                    instances.push(GpuInstance::of(&caster));
                    batch = instances.len();
                    cmds.push(Cmd::Noise(noise_plans.len()));
                    noise_plans.push(NoisePlan {
                        first,
                        seed: noise.seed,
                        density: noise.density,
                        fill_alpha: noise.fill_alpha,
                        cell: noise.cell,
                        color: noise.color,
                    });
                }
                Item::Texture(texture) => {
                    if instances.len() > batch {
                        cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
                    }
                    let caster = ShapeNode {
                        mat: texture.mat,
                        size: texture.size,
                        shape: texture.shape,
                        paint: Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                        clip: texture.clip,
                        opacity: 1.0,
                    };
                    let first = instances.len() as u32;
                    instances.push(GpuInstance::of(&caster));
                    batch = instances.len();
                    cmds.push(Cmd::Texture(texture_plans.len()));
                    texture_plans.push(TexturePlan {
                        first,
                        seed: texture.seed,
                        radius: texture.radius,
                        clip_to_shape: texture.clip_to_shape,
                        cell: texture.cell,
                        fill: texture.fill,
                    });
                }
            }
        }
        if instances.len() > batch {
            cmds.push(Cmd::Shapes { first: batch as u32, count: (instances.len() - batch) as u32 });
        }

        // The instance buffer (at least one record: an empty storage binding is not allowed everywhere).
        let bytes: Vec<u8> =
            if instances.is_empty() { bytemuck::bytes_of(&GpuInstance::zeroed()).to_vec() } else { bytemuck::cast_slice(&instances).to_vec() };
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

        // Shadow silhouettes get their own instance buffer; the main one holds the frame's shapes.
        let shadow_bind: Option<(wgpu::Buffer, wgpu::BindGroup)> = if shadow_plans.is_empty() {
            None
        } else {
            let bytes: Vec<u8> = bytemuck::cast_slice(&shadow_instances).to_vec();
            let size = (bytes.len() as u64).next_power_of_two().max(256);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shadow instances"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, &bytes);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow instances"),
                layout: &self.instances_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            });
            Some((buffer, bind))
        };
        // Inner shadows also draw the same shape at zero offset into their mask target.
        let cover_bind: Option<(wgpu::Buffer, wgpu::BindGroup)> = if cover_instances.is_empty() {
            None
        } else {
            let bytes: Vec<u8> = bytemuck::cast_slice(&cover_instances).to_vec();
            let size = (bytes.len() as u64).next_power_of_two().max(256);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cover instances"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, &bytes);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cover instances"),
                layout: &self.instances_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            });
            Some((buffer, bind))
        };
        let shadow_resources: Vec<ShadowResources> = shadow_plans
            .iter()
            .map(|plan| self.shadow_resources(w, h, plan))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| RenderError::Device("the shadow targets were not created".into()))?;
        let noise_binds: Vec<(wgpu::Buffer, wgpu::BindGroup)> = noise_plans.iter().map(|plan| self.noise_resources(plan)).collect();
        let texture_binds: Vec<(wgpu::Buffer, wgpu::BindGroup)> = texture_plans.iter().map(|plan| self.texture_resources(plan)).collect();

        // Mesh buffers: one vertex buffer and one index buffer for the whole frame.
        if !verts.is_empty() {
            let bytes = bytemuck::cast_slice(&verts).to_vec();
            let need = bytes.len() as u64;
            if self.mesh_verts.as_ref().is_none_or(|(_, cap)| *cap < need) {
                let size = need.next_power_of_two().max(256);
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mesh vertices"),
                    size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.mesh_verts = Some((buffer, size));
            }
            if let Some((buffer, _)) = self.mesh_verts.as_ref() {
                self.queue.write_buffer(buffer, 0, &bytes);
            }
        }
        if !indices.is_empty() {
            let bytes = bytemuck::cast_slice(&indices).to_vec();
            let need = bytes.len() as u64;
            if self.mesh_indices.as_ref().is_none_or(|(_, cap)| *cap < need) {
                let size = need.next_power_of_two().max(256);
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mesh indices"),
                    size,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.mesh_indices = Some((buffer, size));
            }
            if let Some((buffer, _)) = self.mesh_indices.as_ref() {
                self.queue.write_buffer(buffer, 0, &bytes);
            }
        }

        let unpadded = w * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if self.target.as_ref().is_none_or(|t| t.width != w || t.height != h) {
            let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
            let msaa = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("target-msaa"),
                size,
                mip_level_count: 1,
                sample_count: SAMPLES,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let msaa_view = msaa.create_view(&wgpu::TextureViewDescriptor::default());
            let resolve = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("target"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = resolve.create_view(&wgpu::TextureViewDescriptor::default());
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(padded) * u64::from(h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target { width: w, height: h, msaa, msaa_view, resolve, view, readback });
        }
        let Some(target) = self.target.as_ref() else {
            return Err(RenderError::Device("the render target was not created".into()));
        };

        let clear = scene.clear;
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        // Shadows first: each silhouette into its own target, then six box passes (the CPU's blur).
        if let Some((_, bind_shadow)) = shadow_bind.as_ref() {
            for (plan, res) in shadow_plans.iter().zip(&shadow_resources) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow silhouette"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &res.silhouette_msaa_view,
                        depth_slice: None,
                        resolve_target: Some(&res.silhouette_view),
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_globals, &[]);
                pass.set_bind_group(1, bind_shadow, &[]);
                pass.draw(0..6, plan.first..plan.first + plan.count);
                drop(pass);

                if plan.inner
                    && let Some((_, bind_cover)) = cover_bind.as_ref()
                    && let (Some(cover_msaa_view), Some(cover_view)) = (res.cover_msaa_view.as_ref(), res.cover_view.as_ref())
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("inner cover"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: cover_msaa_view,
                            depth_slice: None,
                            resolve_target: Some(cover_view),
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &self.bind_globals, &[]);
                    pass.set_bind_group(1, bind_cover, &[]);
                    pass.draw(0..6, plan.cover_first..plan.cover_first + plan.cover_count);
                }
            }
            for res in &shadow_resources {
                for (k, blur_bind) in res.blur_binds.iter().enumerate() {
                    // Even passes write the ping-pong texture, odd ones the other; the last is `pong`.
                    let view = if k % 2 == 0 { &res.ping_view } else { &res.pong_view };
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow blur"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.blur_pipeline);
                    pass.set_bind_group(0, blur_bind, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&target.view),
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
            let mut bound_shapes = false;
            let mut bound_mesh = false;
            let mut backdrops: Vec<BackdropResources> = Vec::new();
            for cmd in &cmds {
                if let Cmd::Backdrop { first, sigma } = *cmd {
                    // The blur reads what has been drawn so far: close this segment (which resolves
                    // the MSAA target), blur the resolve into a backdrop chain, then resume the pass
                    // over the same samples and show it through the box.
                    drop(pass);
                    if let Some(res) = self.backdrop_resources(w, h, sigma, &target.view) {
                        for (k, blur_bind) in res.blur_binds.iter().enumerate() {
                            let view = if k % 2 == 0 { &res.ping_view } else { &res.pong_view };
                            let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("backdrop blur"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                            blur_pass.set_pipeline(&self.blur_pipeline);
                            blur_pass.set_bind_group(0, blur_bind, &[]);
                            blur_pass.draw(0..3, 0..1);
                        }
                        backdrops.push(res);
                    }
                    pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("frame (resumed)"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target.msaa_view,
                            depth_slice: None,
                            resolve_target: Some(&target.view),
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if let Some(res) = backdrops.last() {
                        pass.set_pipeline(&self.backdrop_pipeline);
                        pass.set_bind_group(0, &res.draw_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.draw(0..3, first..first + 1);
                    }
                    bound_shapes = false;
                    bound_mesh = false;
                    continue;
                }
                match *cmd {
                    Cmd::Shapes { first, count } => {
                        if !bound_shapes {
                            pass.set_pipeline(&self.pipeline);
                            pass.set_bind_group(0, &self.bind_globals, &[]);
                            pass.set_bind_group(1, bind, &[]);
                            bound_shapes = true;
                            // A pipeline switch drops the other side's state: the next mesh draw
                            // must bind its own pipeline and buffers again.
                            bound_mesh = false;
                        }
                        pass.draw(0..6, first..first + count);
                    }
                    Cmd::Mesh { first, count } => {
                        let (Some((vb, _)), Some((ib, _))) = (self.mesh_verts.as_ref(), self.mesh_indices.as_ref()) else {
                            continue;
                        };
                        if !bound_mesh {
                            pass.set_pipeline(&self.mesh_pipeline);
                            pass.set_bind_group(0, &self.bind_globals, &[]);
                            pass.set_vertex_buffer(0, vb.slice(..));
                            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            bound_mesh = true;
                            bound_shapes = false;
                        }
                        pass.draw_indexed(first..first + count, 0, 0..1);
                    }
                    Cmd::Shadow(index) => {
                        let Some(res) = shadow_resources.get(index) else {
                            continue;
                        };
                        let Some(plan) = shadow_plans.get(index) else {
                            continue;
                        };
                        if plan.inner {
                            let Some(inner_bind) = res.inner_bind.as_ref() else {
                                continue;
                            };
                            pass.set_pipeline(&self.inner_pipeline);
                            pass.set_bind_group(0, &self.bind_globals, &[]);
                            pass.set_bind_group(1, inner_bind, &[]);
                        } else {
                            pass.set_pipeline(&self.shadow_pipeline);
                            pass.set_bind_group(0, &self.bind_globals, &[]);
                            pass.set_bind_group(1, &res.draw_bind, &[]);
                        }
                        pass.draw(0..3, 0..1);
                        bound_shapes = false;
                        bound_mesh = false;
                    }
                    // Handled above, because it splits the pass.
                    Cmd::Backdrop { .. } => {}
                    Cmd::Noise(index) => {
                        let Some((_, noise_bind)) = noise_binds.get(index) else {
                            continue;
                        };
                        let Some(plan) = noise_plans.get(index) else {
                            continue;
                        };
                        pass.set_pipeline(&self.noise_pipeline);
                        pass.set_bind_group(0, noise_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.draw(0..3, plan.first..plan.first + 1);
                        bound_shapes = false;
                        bound_mesh = false;
                    }
                    Cmd::Texture(index) => {
                        let Some((_, texture_bind)) = texture_binds.get(index) else {
                            continue;
                        };
                        let Some(plan) = texture_plans.get(index) else {
                            continue;
                        };
                        pass.set_pipeline(&self.texture_pipeline);
                        pass.set_bind_group(0, texture_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.draw(0..3, plan.first..plan.first + 1);
                        bound_shapes = false;
                        bound_mesh = false;
                    }
                }
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &target.resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
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

#[cfg(test)]
mod tests {
    use super::box_radii;

    #[test]
    fn small_sigmas_mean_no_blur() {
        assert_eq!(box_radii(0.0), [0, 0, 0]);
        assert_eq!(box_radii(0.3), [0, 0, 0]);
    }

    #[test]
    fn the_three_box_widths_follow_the_cpu_approximation() {
        // The same arithmetic as `figcraft_render::fx::box_sizes`, checked independently here so a
        // change on either side breaks the parity test.
        assert_eq!(box_radii(1.0), [0, 0, 1]);
        assert_eq!(box_radii(5.0), [4, 4, 5]);
        assert_eq!(box_radii(10.0), [9, 9, 10]);
    }
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

//! Does this machine have a GPU worth drawing on?
//!
//! The probe creates a wgpu instance, asks for the high-performance adapter and classifies it. A
//! **software** adapter (Windows WARP, Mesa lavapipe, SwiftShader) is reported as unavailable for
//! [`crate::prefs::RenderBackend::Auto`]: its pixels are fine but it is slower than tiny-skia, so
//! "use the GPU when there is one" must not mean "use the CPU pretending to be a GPU".
//!
//! The result is cached: adapter enumeration costs tens of milliseconds on some drivers, and both the
//! preferences dialog and every frame ask for it.

use std::sync::OnceLock;

/// What kind of adapter answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceClass {
    /// A real GPU: discrete, integrated or a virtualised one (a passed-through card, a cloud GPU).
    Hardware,
    /// A software rasteriser (WARP, lavapipe, SwiftShader).
    Software,
    /// An adapter whose type the driver did not report.
    Unknown,
}

/// The probe's answer for this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Capability {
    /// An adapter exists **and** meets the limits this pipeline needs.
    pub available: bool,
    pub class: DeviceClass,
    /// The adapter's name, e.g. `NVIDIA GeForce RTX 4070`.
    pub name: String,
    /// The graphics API behind it, e.g. `Dx12`, `Vulkan`, `Metal`.
    pub backend: String,
    /// Why it is unavailable, in words a preference row can show.
    pub reason: Option<String>,
}

impl Capability {
    /// No adapter (or one that cannot run this pipeline), with the reason kept for the UI.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            class: DeviceClass::Unknown,
            name: String::new(),
            backend: String::new(),
            reason: Some(reason.into()),
        }
    }

    /// The one-line state the preferences row shows under the choice.
    pub fn summary(&self) -> String {
        if self.available {
            let kind = match self.class {
                DeviceClass::Hardware => "GPU",
                DeviceClass::Software => "software GPU",
                DeviceClass::Unknown => "GPU",
            };
            format!("{kind}: {} ({})", self.name, self.backend)
        } else {
            format!("CPU: {}", self.reason.as_deref().unwrap_or("no usable GPU adapter"))
        }
    }
}

/// The probe for this process, run once.
pub fn cached() -> &'static Capability {
    static CAP: OnceLock<Capability> = OnceLock::new();
    CAP.get_or_init(probe)
}

/// Storage buffers this pipeline needs: the instance buffer is read in the vertex **and** fragment
/// stages (the fragment evaluates the paint from the instance), so a backend with no fragment-stage
/// storage buffer (WebGL2) cannot run it.
pub const REQUIRED_STORAGE_BUFFERS: u32 = 4;

/// A storage binding must hold at least this many bytes for a scene of a few thousand shapes; bigger
/// scenes rebuild the instance buffer, but the binding limit itself has to clear this floor.
pub const REQUIRED_STORAGE_BINDING: u64 = 1 << 20;

#[cfg(feature = "wgpu")]
fn probe() -> Capability {
    let instance = wgpu::Instance::default();
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    })) {
        Ok(a) => a,
        Err(e) => return Capability::unavailable(format!("no graphics adapter ({e})")),
    };
    let info = adapter.get_info();
    let limits = adapter.limits();
    if limits.max_storage_buffers_per_shader_stage < REQUIRED_STORAGE_BUFFERS {
        return Capability::unavailable(format!(
            "{} reports {} storage buffers per shader stage (needs {})",
            info.name, limits.max_storage_buffers_per_shader_stage, REQUIRED_STORAGE_BUFFERS
        ));
    }
    if limits.max_storage_buffer_binding_size < REQUIRED_STORAGE_BINDING {
        return Capability::unavailable(format!(
            "{} limits storage bindings to {} bytes",
            info.name, limits.max_storage_buffer_binding_size
        ));
    }
    let class = match info.device_type {
        wgpu::DeviceType::Cpu => DeviceClass::Software,
        wgpu::DeviceType::Other => DeviceClass::Unknown,
        _ => DeviceClass::Hardware,
    };
    Capability {
        available: true,
        class,
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        reason: None,
    }
}

#[cfg(not(feature = "wgpu"))]
fn probe() -> Capability {
    Capability::unavailable("this build has no GPU backend (`figcraft-gpu` without the `wgpu` feature)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cached_probe_is_stable() {
        let a = cached();
        let b = cached();
        assert_eq!(a, b, "the probe runs once and keeps its answer");
    }

    #[test]
    fn summaries_name_the_side_that_draws() {
        let cap = Capability { available: true, class: DeviceClass::Hardware, name: "RTX 4070".into(), backend: "Dx12".into(), reason: None };
        assert!(cap.summary().contains("RTX 4070"));
        let cap = Capability::unavailable("no graphics adapter");
        assert!(cap.summary().starts_with("CPU:"));
    }
}

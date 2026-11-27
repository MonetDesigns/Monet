//! Which renderer draws the canvas: the preference, and what it means right now.
//!
//! Three values, one default:
//!
//! * `auto` — draw on the GPU when the machine has a **hardware** adapter, otherwise on the CPU.
//! * `gpu` — ask for the GPU; when there is none the frame still renders, on the CPU, and the
//!   preference row says why. A preference never turns into "the canvas is blank".
//! * `cpu` — force the CPU (driver workarounds, remote sessions, bisecting a GPU diff).
//!
//! The effective choice is process-wide rather than per document: it is a property of the machine,
//! like `Prefs::graphics_backend` (which picks the *window's* API, not the canvas's renderer).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::backend::{Capability, DeviceClass};

/// The preference's own value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderBackend {
    /// The GPU when there is a usable hardware adapter, the CPU otherwise.
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl RenderBackend {
    /// The value a settings file holds (`"auto"`, `"gpu"`, `"cpu"`). Anything else reads as `auto`,
    /// so a hand-edited file never costs the other preferences.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "gpu" => Self::Gpu,
            "cpu" => Self::Cpu,
            _ => Self::Auto,
        }
    }

    /// The value to save.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }
}

/// The renderer a frame actually uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effective {
    Gpu,
    Cpu,
}

impl Effective {
    pub fn is_gpu(self) -> bool {
        matches!(self, Self::Gpu)
    }
}

/// The preference resolved against this machine, with a sentence for the UI and the logs.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    pub effective: Effective,
    /// Why this side draws — always filled, also when the preference and the machine agree.
    pub reason: String,
}

/// Resolve `choice` against `capability`.
pub fn resolve(choice: RenderBackend, capability: &Capability) -> Resolution {
    match choice {
        RenderBackend::Cpu => Resolution { effective: Effective::Cpu, reason: "CPU is the chosen renderer".into() },
        RenderBackend::Gpu if capability.available => {
            Resolution { effective: Effective::Gpu, reason: format!("GPU: {} ({})", capability.name, capability.backend) }
        }
        RenderBackend::Gpu => Resolution {
            effective: Effective::Cpu,
            reason: format!("CPU: GPU requested but unavailable ({})", capability.reason.as_deref().unwrap_or("no adapter")),
        },
        RenderBackend::Auto if capability.available && capability.class != DeviceClass::Software => {
            Resolution { effective: Effective::Gpu, reason: format!("GPU: {} ({})", capability.name, capability.backend) }
        }
        RenderBackend::Auto if capability.available => {
            Resolution { effective: Effective::Cpu, reason: format!("CPU: {} is a software adapter", capability.name) }
        }
        RenderBackend::Auto => {
            Resolution { effective: Effective::Cpu, reason: format!("CPU: {}", capability.reason.as_deref().unwrap_or("no usable GPU adapter")) }
        }
    }
}

/// The process-wide choice, as a byte so it can live in an atomic.
static CHOICE: AtomicU8 = AtomicU8::new(0);

fn code(value: RenderBackend) -> u8 {
    match value {
        RenderBackend::Auto => 0,
        RenderBackend::Gpu => 1,
        RenderBackend::Cpu => 2,
    }
}

fn from_code(code: u8) -> RenderBackend {
    match code {
        1 => RenderBackend::Gpu,
        2 => RenderBackend::Cpu,
        _ => RenderBackend::Auto,
    }
}

/// Set the process-wide choice (the app does this from the saved preference at start, and from the
/// `pref.renderBackend` command when the user picks another).
pub fn set(value: RenderBackend) {
    CHOICE.store(code(value), Ordering::Relaxed);
}

/// The process-wide choice as set (before the environment override).
pub fn choice() -> RenderBackend {
    from_code(CHOICE.load(Ordering::Relaxed))
}

/// `FIGCRAFT_RENDER` wins over the saved preference: it is the "show me this frame on the CPU" switch
/// used when a driver issue is being narrowed down.
pub fn environment_override() -> Option<RenderBackend> {
    static ENV: OnceLock<Option<RenderBackend>> = OnceLock::new();
    *ENV.get_or_init(|| {
        let text = std::env::var("FIGCRAFT_RENDER").ok()?;
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(RenderBackend::Auto),
            "gpu" => Some(RenderBackend::Gpu),
            "cpu" => Some(RenderBackend::Cpu),
            _ => None,
        }
    })
}

/// The choice with the environment override applied. Until the app pushes a choice, the engine's
/// saved preference (mirrored in `figcraft_render`) is what counts — that way a plain `cargo run`
/// with a prefs file still lands on the right renderer.
pub fn requested() -> RenderBackend {
    if let Some(over) = environment_override() {
        return over;
    }
    let choice = choice();
    if choice != RenderBackend::Auto {
        return choice;
    }
    let saved = figcraft_render::render_backend();
    if saved.trim().is_empty() { RenderBackend::Auto } else { RenderBackend::parse(&saved) }
}

/// What this frame draws on, against the cached adapter probe.
pub fn effective() -> Effective {
    resolve(requested(), crate::backend::cached()).effective
}

/// The sentence the preferences row shows: the chosen value resolved against this machine.
pub fn status() -> Resolution {
    resolve(requested(), crate::backend::cached())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hardware() -> Capability {
        Capability { available: true, class: DeviceClass::Hardware, name: "RTX 4070".into(), backend: "Dx12".into(), reason: None }
    }

    fn software() -> Capability {
        Capability {
            available: true,
            class: DeviceClass::Software,
            name: "Microsoft Basic Render Driver".into(),
            backend: "Dx12".into(),
            reason: None,
        }
    }

    #[test]
    fn auto_uses_a_hardware_adapter_and_skips_a_software_one() {
        assert_eq!(resolve(RenderBackend::Auto, &hardware()).effective, Effective::Gpu);
        let soft = resolve(RenderBackend::Auto, &software());
        assert_eq!(soft.effective, Effective::Cpu, "a software adapter is not a GPU for `auto`");
        assert!(soft.reason.contains("software"), "{}", soft.reason);
    }

    #[test]
    fn gpu_without_an_adapter_still_renders_on_the_cpu() {
        let none = Capability::unavailable("no graphics adapter");
        let r = resolve(RenderBackend::Gpu, &none);
        assert_eq!(r.effective, Effective::Cpu);
        assert!(r.reason.contains("requested"), "{}", r.reason);
    }

    #[test]
    fn cpu_is_always_cpu() {
        assert_eq!(resolve(RenderBackend::Cpu, &hardware()).effective, Effective::Cpu);
    }

    #[test]
    fn values_round_trip_and_unknown_text_reads_as_auto() {
        for v in [RenderBackend::Auto, RenderBackend::Gpu, RenderBackend::Cpu] {
            assert_eq!(RenderBackend::parse(v.as_str()), v);
        }
        assert_eq!(RenderBackend::parse("GPU"), RenderBackend::Gpu);
        assert_eq!(RenderBackend::parse("metal"), RenderBackend::Auto);
    }

    #[test]
    fn the_process_choice_round_trips() {
        let was = choice();
        set(RenderBackend::Cpu);
        assert_eq!(choice(), RenderBackend::Cpu);
        set(was);
    }
}

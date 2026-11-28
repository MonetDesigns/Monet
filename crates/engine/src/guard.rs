//! The last line of defence: a panic inside a command becomes an [`EngineError::Internal`] and
//! the caller rolls the document back. It exists for bugs and doesn't replace `Result`.
//!
//! The browser build can't catch a panic (wasm32 always aborts, and stable Rust has no unwinding
//! std for it), so there a panicking command stops the page: its panic hook keeps the files with
//! unsaved changes for the next page instead (`apps/figcraft-web/src/recovery.rs`).

use std::cell::Cell;

use crate::{EngineError, Result};

thread_local! {
    /// Non-zero while a command runs inside [`run`]'s `catch_unwind`. The UI's panic hook reads
    /// this to avoid treating a recoverable command panic as a fatal crash report.
    static GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Whether the current thread is inside [`run`]'s panic-catching scope. A panic there is
/// recoverable by contract (the command returns `EngineError::Internal` and the document rolls
/// back), so it must not consume the process's one fatal crash report.
pub fn catching_command_panic() -> bool {
    GUARD_DEPTH.with(|d| d.get() > 0)
}

struct DepthReset;

impl Drop for DepthReset {
    fn drop(&mut self) {
        GUARD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

pub fn run<T>(cmd: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    // Keep the depth counted only while `catch_unwind` is on the stack: handling or dropping
    // the payload can itself panic, and that unwind must be reported as fatal.
    let caught = {
        GUARD_DEPTH.with(|d| d.set(d.get() + 1));
        let _reset = DepthReset;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    };
    match caught {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            log::error!("command `{cmd}` panicked: {msg}");
            Err(EngineError::Internal { cmd: cmd.to_string(), msg })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caught_command_panic_releases_the_guard_depth() {
        let mut inside = false;
        let result = run("test.command", || -> Result<()> {
            inside = catching_command_panic();
            panic!("boom");
        });
        assert!(matches!(result, Err(EngineError::Internal { .. })));
        assert!(inside, "the guard must be visible while the command unwinds");
        assert!(!catching_command_panic(), "the reset must run after the panic is caught");
    }
}

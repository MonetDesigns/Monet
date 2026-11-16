//! Windows platform adapter for the AccessKit tree.
//!
//! GPUI 0.2 has no `WM_GETOBJECT` hook. `accesskit_windows::SubclassingAdapter`
//! installs one for us via `SetWindowLongPtrW`; gpui-kit's newer GPUI does the
//! same internally. The adapter must be installed *before* the window is
//! shown, so `run()` opens the window with `show: false` and shows it here.

use accesskit::{ActionHandler, ActivationHandler, ActionRequest, TreeUpdate};
use accesskit_windows::{HWND, SubclassingAdapter};
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use smol::channel::{Receiver, Sender};

/// gpui-pre 0.3.12 asks AccessKit to request the initial tree lazily; we do
/// the same so no tree is built while no screen reader is listening.
struct Activate;

impl ActivationHandler for Activate {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        None
    }
}

/// AccessKit may call actions off the UI thread; queue them and drain on the
/// GPUI thread (same shape as gpui-pre's action listeners).
struct Actions(Sender<ActionRequest>);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.try_send(request);
    }
}

/// Owns the platform adapter and the queued assistive-technology actions.
pub(crate) struct PlatformA11y {
    adapter: SubclassingAdapter,
    actions: Receiver<ActionRequest>,
    hwnd: HWND,
}

impl PlatformA11y {
    /// Install into a not-yet-visible window; returns `None` on non-Win32
    /// backends or if the handle cannot be obtained.
    pub(crate) fn install(window: &Window) -> Option<Self> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else { return None };
        let hwnd = HWND(win32.hwnd.get() as *mut std::ffi::c_void);
        let (tx, actions) = smol::channel::unbounded();
        let adapter = SubclassingAdapter::new(hwnd, Activate, Actions(tx));
        Some(Self { adapter, actions, hwnd })
    }

    /// Show the window now that `WM_GETOBJECT` is intercepted.
    pub(crate) fn show(&self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                self.hwnd,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            );
        }
    }

    /// Send this frame's tree; no-op until a screen reader asks for one.
    pub(crate) fn update(&mut self, update: TreeUpdate) {
        if let Some(events) = self.adapter.update_if_active(|| update) {
            events.raise();
        }
    }

    pub(crate) fn drain(&self) -> Vec<ActionRequest> {
        let mut out = Vec::new();
        while let Ok(request) = self.actions.try_recv() {
            out.push(request);
        }
        out
    }
}

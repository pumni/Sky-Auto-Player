#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::Serialize;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime, WebviewWindow};
use tokio::sync::watch;
use windows_sys::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, SetLastError, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetParent, HTMAXBUTTON, HWND_TOP, IsWindow, IsZoomed, LWA_ALPHA, PostMessageW,
    RegisterClassExW, SC_MAXIMIZE, SC_RESTORE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, WA_INACTIVE,
    WM_ACTIVATE, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
    WM_SIZE, WM_SYSCOMMAND, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_OVERLAPPED, WS_VISIBLE,
};

const TITLEBAR_HEIGHT: u32 = 40;
const CAPTION_BUTTON_WIDTH: u32 = 46;
const CLOSE_BUTTONS_TO_THE_RIGHT: u32 = 1;
const SUBCLASS_ID: usize = 0x534b_5943_4150;
const CAPTION_ZOOM_HOVER_EVENT: &str = "sky:caption-zoom-hover";
const SNAP_CLASS: &[u16] = &[
    b'S' as u16,
    b'k' as u16,
    b'y' as u16,
    b'A' as u16,
    b'u' as u16,
    b't' as u16,
    b'o' as u16,
    b'P' as u16,
    b'l' as u16,
    b'a' as u16,
    b'y' as u16,
    b'e' as u16,
    b'r' as u16,
    b'S' as u16,
    b'n' as u16,
    b'a' as u16,
    b'p' as u16,
    0,
];

#[derive(Clone)]
struct SnapOverlayState {
    overlay: isize,
    titlebar_height: u32,
    button_width: u32,
    hover_bridge: Option<NativeHoverBridge>,
}

type OverlayRegistry = Mutex<HashMap<isize, SnapOverlayState>>;

static SNAP_OVERLAYS: LazyLock<OverlayRegistry> = LazyLock::new(|| Mutex::new(HashMap::new()));
static REGISTER_CLASS: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Clone)]
struct NativeHoverBridge {
    sender: watch::Sender<bool>,
    alive: Arc<AtomicBool>,
    tracking_nonclient: Arc<AtomicBool>,
}

impl NativeHoverBridge {
    fn new() -> (Self, watch::Receiver<bool>) {
        let (sender, receiver) = watch::channel(false);
        (
            Self {
                sender,
                alive: Arc::new(AtomicBool::new(true)),
                tracking_nonclient: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }

    fn set_hovered(&self, hovered: bool) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        self.sender.send_if_modified(|current| {
            if *current == hovered {
                false
            } else {
                *current = hovered;
                true
            }
        });
    }

    fn begin_nonclient_tracking(&self) -> bool {
        !self.tracking_nonclient.swap(true, Ordering::AcqRel)
    }

    fn reset(&self) {
        self.tracking_nonclient.store(false, Ordering::Release);
        self.set_hovered(false);
    }

    fn stop(&self) {
        self.alive.store(false, Ordering::Release);
        self.tracking_nonclient.store(false, Ordering::Release);
    }
}

#[derive(Clone, Serialize)]
struct CaptionZoomHoverPayload {
    hovered: bool,
}

/// `setup` runs from Tauri's main-thread `Ready` event. Reserve the parent
/// subclass synchronously so WM_DESTROY owns the captured HWND before queuing
/// child creation. The queued task checks that reservation before any Win32
/// call, so it becomes a no-op if the parent is destroyed first.
pub fn install<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let parent = window_hwnd(window)?;
    register_class()?;
    let (hover_bridge, receiver) = NativeHoverBridge::new();
    if !reserve_parent_with_hover(
        parent,
        &SNAP_OVERLAYS,
        &Win32CaptionOperations,
        Some(hover_bridge.clone()),
    )? {
        return Ok(());
    }
    start_hover_event_dispatch(
        window.app_handle().clone(),
        window.label().to_owned(),
        Arc::clone(&hover_bridge.alive),
        receiver,
    );

    window
        .run_on_main_thread(move || {
            if let Err(error) =
                finish_overlay_install(parent, &SNAP_OVERLAYS, &Win32CaptionOperations)
            {
                eprintln!("failed to install native maximize hit target: {error}");
            }
        })
        .map_err(|error| {
            cancel_pending_install(parent, &SNAP_OVERLAYS, &Win32CaptionOperations);
            format!("schedule Windows caption hit target: {error}")
        })
}

fn start_hover_event_dispatch<R: Runtime + 'static>(
    app_handle: AppHandle<R>,
    window_label: String,
    alive: Arc<AtomicBool>,
    mut receiver: watch::Receiver<bool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut last_emitted = false;
        while receiver.changed().await.is_ok() {
            if !alive.load(Ordering::Acquire) {
                break;
            }

            let hovered = *receiver.borrow_and_update();
            if hovered == last_emitted {
                continue;
            }
            last_emitted = hovered;

            let event_app = app_handle.clone();
            let event_label = window_label.clone();
            let event_alive = Arc::clone(&alive);
            let dispatcher = app_handle.clone();
            if let Err(error) = dispatcher.run_on_main_thread(move || {
                if !event_alive.load(Ordering::Acquire) {
                    return;
                }
                if let Err(error) = event_app.emit_to(
                    EventTarget::webview_window(event_label),
                    CAPTION_ZOOM_HOVER_EVENT,
                    CaptionZoomHoverPayload { hovered },
                ) {
                    eprintln!("failed to emit native caption hover state: {error}");
                }
            }) {
                eprintln!("failed to queue native caption hover state: {error}");
            }
        }
    });
}

fn window_hwnd<W>(window: &W) -> Result<isize, String>
where
    W: HasWindowHandle,
{
    let handle = window
        .window_handle()
        .map_err(|error| format!("window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get()),
        _ => Err("Windows caption hit target requires a Win32 window handle".into()),
    }
}

trait CaptionOperations {
    fn is_window(&self, parent: isize) -> bool;
    fn create_overlay(&self, parent: isize) -> Result<isize, String>;
    fn configure_overlay(&self, overlay: isize) -> Result<(), String>;
    fn set_parent_subclass(&self, parent: isize) -> bool;
    fn remove_parent_subclass(&self, parent: isize) -> bool;
    fn position_overlay(&self, parent: isize, state: SnapOverlayState) -> Result<(), String>;
    fn destroy_overlay(&self, overlay: isize) -> bool;
}

struct Win32CaptionOperations;

impl CaptionOperations for Win32CaptionOperations {
    fn is_window(&self, parent: isize) -> bool {
        // SAFETY: `parent` was extracted from Tauri's Win32 window handle;
        // IsWindow validates it without dereferencing caller-owned memory.
        !parent_as_hwnd(parent).is_null() && unsafe { IsWindow(parent_as_hwnd(parent)) != 0 }
    }

    fn create_overlay(&self, parent: isize) -> Result<isize, String> {
        // SAFETY: SNAP_CLASS is static NUL-terminated UTF-16, parent was
        // validated on the UI thread, and the null optional parameters are
        // permitted for this child window. All buffers live through the call.
        let overlay = unsafe {
            SetLastError(0);
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                SNAP_CLASS.as_ptr(),
                SNAP_CLASS.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                parent_as_hwnd(parent),
                std::ptr::null_mut(),
                module_instance(),
                std::ptr::null_mut(),
            )
        };
        if overlay.is_null() {
            // SAFETY: GetLastError has no pointer or handle preconditions.
            let error = unsafe { GetLastError() };
            Err(format!(
                "CreateWindowExW failed for the native caption hit target (Win32 error {error})"
            ))
        } else {
            Ok(overlay as isize)
        }
    }

    fn configure_overlay(&self, overlay: isize) -> Result<(), String> {
        // Alpha zero makes a layered child transparent to hit testing, so its
        // HTMAXBUTTON result never reaches the system Snap Layout path. One
        // alpha unit is visually imperceptible but keeps the target hittable.
        // SAFETY: the overlay HWND is the live child created for this registry
        // entry and remains on the same UI-thread lifecycle.
        let configured =
            unsafe { SetLayeredWindowAttributes(overlay_as_hwnd(overlay), 0, 1, LWA_ALPHA) };
        if configured == 0 {
            Err("SetLayeredWindowAttributes failed for the native caption hit target".into())
        } else {
            Ok(())
        }
    }

    fn set_parent_subclass(&self, parent: isize) -> bool {
        // SAFETY: parent is a validated live Tauri HWND; the callback has the
        // required system ABI and remains a static function for registration.
        unsafe {
            SetWindowSubclass(
                parent_as_hwnd(parent),
                Some(parent_subclass_proc),
                SUBCLASS_ID,
                0,
            ) != 0
        }
    }

    fn remove_parent_subclass(&self, parent: isize) -> bool {
        // SAFETY: this removes the same callback/id pair from the live parent
        // HWND on its UI-thread lifecycle.
        unsafe {
            RemoveWindowSubclass(
                parent_as_hwnd(parent),
                Some(parent_subclass_proc),
                SUBCLASS_ID,
            ) != 0
        }
    }

    fn position_overlay(&self, parent: isize, state: SnapOverlayState) -> Result<(), String> {
        position_overlay(parent, state)
    }

    fn destroy_overlay(&self, overlay: isize) -> bool {
        // SAFETY: the registry owns this child HWND and calls destruction only
        // after removing its entry, so the window is destroyed once.
        unsafe { DestroyWindow(overlay_as_hwnd(overlay)) != 0 }
    }
}

#[cfg(test)]
fn install_overlay(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) -> Result<(), String> {
    if reserve_parent(parent, registry, operations)? {
        finish_overlay_install(parent, registry, operations)?;
    }
    Ok(())
}

#[cfg(test)]
fn reserve_parent(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) -> Result<bool, String> {
    reserve_parent_with_hover(parent, registry, operations, None)
}

fn reserve_parent_with_hover(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
    hover_bridge: Option<NativeHoverBridge>,
) -> Result<bool, String> {
    if parent == 0 || !operations.is_window(parent) {
        return Err("cannot install native caption hit target on an invalid HWND".into());
    }

    // Setup and all native window callbacks are serialized on the GUI thread.
    // A second setup notification is therefore an idempotent no-op.
    if lock_overlays(registry).contains_key(&parent) {
        return Ok(false);
    }

    if !operations.set_parent_subclass(parent) {
        return Err("SetWindowSubclass failed for the native caption hit target".into());
    }

    lock_overlays(registry).insert(
        parent,
        SnapOverlayState {
            overlay: 0,
            titlebar_height: TITLEBAR_HEIGHT,
            button_width: CAPTION_BUTTON_WIDTH,
            hover_bridge,
        },
    );
    Ok(true)
}

fn finish_overlay_install(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) -> Result<(), String> {
    if !is_pending_install(parent, registry) {
        return Ok(());
    }
    if !operations.is_window(parent) {
        let removed = lock_overlays(registry).remove(&parent);
        stop_hover_bridge(removed.as_ref());
        return Err("parent HWND became unavailable before caption hit target creation".into());
    }

    let overlay = match operations.create_overlay(parent) {
        Ok(overlay) => overlay,
        Err(error) => {
            cancel_pending_install(parent, registry, operations);
            return Err(error);
        }
    };
    if !is_pending_install(parent, registry) {
        // CreateWindowExW can dispatch nested window messages. If the parent
        // was destroyed during that call, Windows owns the child teardown and
        // its HWND may already be invalid when CreateWindowExW returns.
        return Ok(());
    }
    if let Err(error) = operations.configure_overlay(overlay) {
        return rollback_child_install(parent, overlay, registry, operations, error);
    }

    let state = lock_overlays(registry)
        .get(&parent)
        .cloned()
        .unwrap_or(SnapOverlayState {
            overlay: 0,
            titlebar_height: TITLEBAR_HEIGHT,
            button_width: CAPTION_BUTTON_WIDTH,
            hover_bridge: None,
        });
    if !is_pending_install(parent, registry) {
        // WM_DESTROY removed the reservation while CreateWindowExW or child
        // configuration was reentrant. Parent destruction owns the child now.
        return Ok(());
    }
    if let Err(error) = operations.position_overlay(parent, SnapOverlayState { overlay, ..state }) {
        return rollback_child_install(parent, overlay, registry, operations, error);
    }

    let mut overlays = lock_overlays(registry);
    if let Some(state) = overlays.get_mut(&parent)
        && state.overlay == 0
    {
        state.overlay = overlay;
    }
    Ok(())
}

fn rollback_child_install(
    parent: isize,
    overlay: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
    error: String,
) -> Result<(), String> {
    if !is_pending_install(parent, registry) {
        // The WM_DESTROY callback already relinquished ownership and Windows
        // will destroy this child as part of parent destruction.
        return Ok(());
    }
    destroy_partial_overlay(operations, overlay);
    cancel_pending_install(parent, registry, operations);
    Err(error)
}

fn is_pending_install(parent: isize, registry: &OverlayRegistry) -> bool {
    lock_overlays(registry)
        .get(&parent)
        .is_some_and(|state| state.overlay == 0)
}

fn cancel_pending_install(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) {
    let removed = {
        let mut overlays = lock_overlays(registry);
        if overlays
            .get(&parent)
            .is_some_and(|state| state.overlay == 0)
        {
            overlays.remove(&parent)
        } else {
            None
        }
    };
    stop_hover_bridge(removed.as_ref());
    if removed.is_some() && !operations.remove_parent_subclass(parent) {
        eprintln!("RemoveWindowSubclass failed while rolling back caption hit target installation");
    }
}

fn stop_hover_bridge(state: Option<&SnapOverlayState>) {
    if let Some(bridge) = state.and_then(|state| state.hover_bridge.as_ref()) {
        bridge.stop();
    }
}

fn destroy_partial_overlay(operations: &impl CaptionOperations, overlay: isize) {
    if !operations.destroy_overlay(overlay) {
        eprintln!("DestroyWindow failed while rolling back caption hit target installation");
    }
}

fn register_class() -> Result<(), String> {
    REGISTER_CLASS
        // SAFETY: the class structure is fully initialized, SNAP_CLASS is a
        // static NUL-terminated name, and the callback uses the system ABI.
        .get_or_init(|| unsafe {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(overlay_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: module_instance(),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: SNAP_CLASS.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            if RegisterClassExW(&class) == 0 {
                let error = GetLastError();
                Err(format!(
                    "RegisterClassExW failed for the native caption hit target (Win32 error {error})"
                ))
            } else {
                Ok(())
            }
        })
        .clone()
}

fn module_instance() -> HINSTANCE {
    // SAFETY: a null module name requests the current process module.
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

fn parent_as_hwnd(parent: isize) -> HWND {
    parent as HWND
}

fn overlay_as_hwnd(overlay: isize) -> HWND {
    overlay as HWND
}

fn position_overlay(parent: isize, state: SnapOverlayState) -> Result<(), String> {
    let parent = parent_as_hwnd(parent);
    // SAFETY: RECT contains only integers, so zero is a valid initialized
    // output value for GetClientRect.
    let mut client = unsafe { std::mem::zeroed() };
    // SAFETY: parent is the active main-thread HWND and `client` is a writable
    // stack value that remains alive for the call.
    if unsafe { GetClientRect(parent, &mut client) } == 0 {
        return Err("GetClientRect failed while positioning the caption hit target".into());
    }
    // SAFETY: the same live parent HWND is used only for a scalar DPI query.
    let dpi = unsafe { GetDpiForWindow(parent) }.max(96) as u64;
    let button_width = scale(state.button_width, dpi);
    let titlebar_height = scale(state.titlebar_height, dpi);
    let x = (client.right - button_width * (CLOSE_BUTTONS_TO_THE_RIGHT as i32 + 1)).max(0);
    // SAFETY: parent and the registered overlay child are live on the UI
    // thread; SetWindowPos receives no borrowed pointers.
    if unsafe {
        SetWindowPos(
            overlay_as_hwnd(state.overlay),
            HWND_TOP,
            x,
            0,
            button_width,
            titlebar_height,
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
        )
    } == 0
    {
        Err("SetWindowPos failed while positioning the caption hit target".into())
    } else {
        Ok(())
    }
}

fn update_overlay_position(parent: HWND) -> Result<(), String> {
    let state = lock_overlays(&SNAP_OVERLAYS)
        .get(&(parent as isize))
        .cloned();
    if let Some(state) = state.filter(|state| state.overlay != 0) {
        reset_hover_bridge(state.hover_bridge.as_ref());
        position_overlay(parent as isize, state)
    } else {
        Ok(())
    }
}

fn hover_bridge_for_parent(parent: isize) -> Option<NativeHoverBridge> {
    lock_overlays(&SNAP_OVERLAYS)
        .get(&parent)
        .and_then(|state| state.hover_bridge.clone())
}

fn hover_bridge_for_overlay(overlay: isize) -> Option<NativeHoverBridge> {
    lock_overlays(&SNAP_OVERLAYS)
        .values()
        .find(|state| state.overlay == overlay)
        .and_then(|state| state.hover_bridge.clone())
}

fn reset_hover_bridge(bridge: Option<&NativeHoverBridge>) {
    if let Some(bridge) = bridge {
        bridge.reset();
    }
}

fn reset_hover_for_parent(parent: isize) {
    reset_hover_bridge(hover_bridge_for_parent(parent).as_ref());
}

fn reset_hover_for_overlay(overlay: isize) {
    reset_hover_bridge(hover_bridge_for_overlay(overlay).as_ref());
}

fn track_nonclient_mouse_leave(hwnd: HWND) -> bool {
    let mut tracking = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | TME_NONCLIENT,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: `tracking` is initialized with the live child HWND and is only
    // borrowed for the documented TrackMouseEvent call.
    unsafe { TrackMouseEvent(&mut tracking) != 0 }
}

fn scale(value: u32, dpi: u64) -> i32 {
    ((value as u64 * dpi + 48) / 96).max(1) as i32
}

/// WM_DESTROY arrives while this HWND is still valid and before Windows
/// destroys its child windows. Remove our callback and registry entry here;
/// Windows then destroys the overlay child once as part of parent destruction.
/// In particular, do not call DestroyWindow on that child in this path.
fn parent_destroying(parent: HWND) {
    parent_destroyed_from(parent as isize, &SNAP_OVERLAYS, &Win32CaptionOperations);
}

/// If the child is independently destroyed, forget it and detach the now
/// unnecessary parent callback. Parent-driven destruction has already removed
/// the map entry before Windows reaches the child.
fn child_destroyed(overlay: HWND) {
    child_destroyed_from(overlay as isize, &SNAP_OVERLAYS, &Win32CaptionOperations);
}

// SAFETY: Windows invokes this function only after the subclass is registered
// and supplies its HWND and message arguments according to the system ABI.
unsafe extern "system" fn parent_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    // SAFETY: Windows supplies this callback's HWND and message arguments;
    // the function has the required system ABI, does not retain pointers, and
    // forwards the message through DefSubclassProc before returning.
    unsafe {
        match message {
            WM_DESTROY => parent_destroying(hwnd),
            WM_ACTIVATE if (wparam & 0xffff) == WA_INACTIVE as usize => {
                reset_hover_for_parent(hwnd as isize);
            }
            WM_SIZE | WM_DPICHANGED => {
                if let Err(error) = update_overlay_position(hwnd) {
                    eprintln!("failed to update native caption hit target: {error}");
                }
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

// SAFETY: Windows invokes this procedure only for the registered overlay class
// and supplies its HWND and message arguments according to the system ABI.
unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Windows supplies this callback's live child HWND and message
    // arguments; queried parent handles come from GetParent and are used only
    // in documented window APIs before the callback returns.
    unsafe {
        match message {
            WM_NCHITTEST => HTMAXBUTTON as LRESULT,
            WM_NCMOUSEMOVE => {
                if let Some(bridge) = hover_bridge_for_overlay(hwnd as isize) {
                    if bridge.begin_nonclient_tracking() && !track_nonclient_mouse_leave(hwnd) {
                        bridge.tracking_nonclient.store(false, Ordering::Release);
                    }
                    bridge.set_hovered(true);
                }
                0
            }
            WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
                if let Some(bridge) = hover_bridge_for_overlay(hwnd as isize) {
                    bridge.tracking_nonclient.store(false, Ordering::Release);
                    bridge.set_hovered(false);
                }
                0
            }
            WM_NCLBUTTONDOWN | WM_LBUTTONDOWN => 0,
            WM_NCLBUTTONUP | WM_LBUTTONUP => {
                reset_hover_for_overlay(hwnd as isize);
                let parent = GetParent(hwnd);
                if !parent.is_null() {
                    let command = if IsZoomed(parent) != 0 {
                        SC_RESTORE
                    } else {
                        SC_MAXIMIZE
                    };
                    PostMessageW(parent, WM_SYSCOMMAND, command as usize, 0);
                }
                0
            }
            WM_MOUSEACTIVATE => 1,
            WM_NCDESTROY => {
                child_destroyed(hwnd);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_DESTROY => 0,
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

fn lock_overlays(registry: &OverlayRegistry) -> MutexGuard<'_, HashMap<isize, SnapOverlayState>> {
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn parent_destroyed_from(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) {
    let removed = { lock_overlays(registry).remove(&parent) };
    stop_hover_bridge(removed.as_ref());
    if removed.is_some() && !operations.remove_parent_subclass(parent) {
        eprintln!("RemoveWindowSubclass failed during native caption teardown");
    }
}

fn child_destroyed_from(
    overlay: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) {
    let (parent, removed) = {
        let mut overlays = lock_overlays(registry);
        let parent = overlays
            .iter()
            .find_map(|(parent, state)| (state.overlay == overlay).then_some(*parent));
        let removed = parent.and_then(|parent| overlays.remove(&parent));
        (parent, removed)
    };
    stop_hover_bridge(removed.as_ref());
    if let Some(parent) = parent
        && !operations.remove_parent_subclass(parent)
    {
        eprintln!("RemoveWindowSubclass failed after caption hit target destruction");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CAPTION_BUTTON_WIDTH, CaptionOperations, NativeHoverBridge, OverlayRegistry,
        SnapOverlayState, TITLEBAR_HEIGHT, child_destroyed_from, finish_overlay_install,
        install_overlay, lock_overlays, parent_destroyed_from, reserve_parent, scale,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Mutex;

    struct FakeCaptionOperations {
        valid_parent: bool,
        child_creation_fails: bool,
        configure_fails: bool,
        subclass_fails: bool,
        position_fails: bool,
        is_window_count: Cell<usize>,
        create_count: Cell<usize>,
        configure_count: Cell<usize>,
        set_subclass_count: Cell<usize>,
        remove_subclass_count: Rc<Cell<usize>>,
        position_count: Cell<usize>,
        destroy_count: Cell<usize>,
        on_create: RefCell<Option<Box<dyn Fn()>>>,
    }

    impl Default for FakeCaptionOperations {
        fn default() -> Self {
            Self {
                valid_parent: true,
                child_creation_fails: false,
                configure_fails: false,
                subclass_fails: false,
                position_fails: false,
                is_window_count: Cell::new(0),
                create_count: Cell::new(0),
                configure_count: Cell::new(0),
                set_subclass_count: Cell::new(0),
                remove_subclass_count: Rc::new(Cell::new(0)),
                position_count: Cell::new(0),
                destroy_count: Cell::new(0),
                on_create: RefCell::new(None),
            }
        }
    }

    impl CaptionOperations for FakeCaptionOperations {
        fn is_window(&self, _parent: isize) -> bool {
            self.is_window_count.set(self.is_window_count.get() + 1);
            self.valid_parent
        }

        fn create_overlay(&self, _parent: isize) -> Result<isize, String> {
            self.create_count.set(self.create_count.get() + 1);
            if self.child_creation_fails {
                Err("injected child creation failure".into())
            } else {
                if let Some(on_create) = self.on_create.borrow_mut().take() {
                    on_create();
                }
                Ok(200)
            }
        }

        fn configure_overlay(&self, _overlay: isize) -> Result<(), String> {
            self.configure_count.set(self.configure_count.get() + 1);
            if self.configure_fails {
                Err("injected child configuration failure".into())
            } else {
                Ok(())
            }
        }

        fn set_parent_subclass(&self, _parent: isize) -> bool {
            self.set_subclass_count
                .set(self.set_subclass_count.get() + 1);
            !self.subclass_fails
        }

        fn remove_parent_subclass(&self, _parent: isize) -> bool {
            self.remove_subclass_count
                .set(self.remove_subclass_count.get() + 1);
            true
        }

        fn position_overlay(&self, _parent: isize, _state: SnapOverlayState) -> Result<(), String> {
            self.position_count.set(self.position_count.get() + 1);
            if self.position_fails {
                Err("injected positioning failure".into())
            } else {
                Ok(())
            }
        }

        fn destroy_overlay(&self, _overlay: isize) -> bool {
            self.destroy_count.set(self.destroy_count.get() + 1);
            true
        }
    }

    fn registry() -> OverlayRegistry {
        Mutex::new(HashMap::new())
    }

    #[test]
    fn installs_once_and_duplicate_setup_is_idempotent() {
        let registry = registry();
        let operations = FakeCaptionOperations::default();
        install_overlay(100, &registry, &operations).unwrap();
        install_overlay(100, &registry, &operations).unwrap();

        assert_eq!(registry.lock().unwrap().len(), 1);
        assert_eq!(operations.create_count.get(), 1);
        assert_eq!(operations.set_subclass_count.get(), 1);
        assert_eq!(operations.destroy_count.get(), 0);
    }

    #[test]
    fn invalid_parent_and_child_creation_failure_leave_no_state() {
        let registry = registry();
        let invalid_operations = FakeCaptionOperations {
            valid_parent: false,
            ..FakeCaptionOperations::default()
        };
        assert!(install_overlay(100, &registry, &invalid_operations).is_err());
        assert!(registry.lock().unwrap().is_empty());

        let child_creation_operations = FakeCaptionOperations {
            child_creation_fails: true,
            ..FakeCaptionOperations::default()
        };
        assert!(install_overlay(100, &registry, &child_creation_operations).is_err());
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(child_creation_operations.destroy_count.get(), 0);
        assert_eq!(child_creation_operations.remove_subclass_count.get(), 1);
    }

    #[test]
    fn partial_configuration_failure_destroys_created_child() {
        let registry = registry();
        let operations = FakeCaptionOperations {
            configure_fails: true,
            ..FakeCaptionOperations::default()
        };

        assert!(install_overlay(100, &registry, &operations).is_err());
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.destroy_count.get(), 1);
        assert_eq!(operations.set_subclass_count.get(), 1);
        assert_eq!(operations.remove_subclass_count.get(), 1);
    }

    #[test]
    fn failed_subclass_installation_keeps_registry_empty_without_creating_child() {
        let registry = registry();
        let operations = FakeCaptionOperations {
            subclass_fails: true,
            ..FakeCaptionOperations::default()
        };

        let error = install_overlay(100, &registry, &operations).unwrap_err();
        assert!(error.contains("SetWindowSubclass"));
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.create_count.get(), 0);
        assert_eq!(operations.destroy_count.get(), 0);
        assert_eq!(operations.remove_subclass_count.get(), 0);
    }

    #[test]
    fn failed_positioning_removes_subclass_and_destroys_child() {
        let registry = registry();
        let operations = FakeCaptionOperations {
            position_fails: true,
            ..FakeCaptionOperations::default()
        };

        assert!(install_overlay(100, &registry, &operations).is_err());
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.remove_subclass_count.get(), 1);
        assert_eq!(operations.destroy_count.get(), 1);
    }

    #[test]
    fn parent_destruction_drops_registry_and_leaves_child_to_windows() {
        let registry = registry();
        let operations = FakeCaptionOperations::default();
        install_overlay(100, &registry, &operations).unwrap();

        parent_destroyed_from(100, &registry, &operations);
        parent_destroyed_from(100, &registry, &operations);

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.remove_subclass_count.get(), 1);
        // DestroyWindow(parent) owns recursive destruction of its child.
        assert_eq!(operations.destroy_count.get(), 0);
    }

    #[test]
    fn queued_child_creation_is_skipped_after_parent_destruction() {
        let registry = registry();
        let operations = FakeCaptionOperations::default();
        assert!(reserve_parent(100, &registry, &operations).unwrap());

        parent_destroyed_from(100, &registry, &operations);
        finish_overlay_install(100, &registry, &operations).unwrap();

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.create_count.get(), 0);
        assert_eq!(operations.remove_subclass_count.get(), 1);
        assert_eq!(operations.is_window_count.get(), 1);
    }

    #[test]
    fn destruction_reentered_during_child_creation_prevents_later_child_operations() {
        let registry = Rc::new(registry());
        let operations = FakeCaptionOperations::default();
        let registry_for_callback = Rc::clone(&registry);
        let remove_subclass_count = Rc::clone(&operations.remove_subclass_count);
        *operations.on_create.borrow_mut() = Some(Box::new(move || {
            if lock_overlays(registry_for_callback.as_ref())
                .remove(&100)
                .is_some()
            {
                remove_subclass_count.set(remove_subclass_count.get() + 1);
            }
        }));

        assert!(reserve_parent(100, registry.as_ref(), &operations).unwrap());
        finish_overlay_install(100, registry.as_ref(), &operations).unwrap();

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.configure_count.get(), 0);
        assert_eq!(operations.position_count.get(), 0);
        assert_eq!(operations.destroy_count.get(), 0);
        assert_eq!(operations.remove_subclass_count.get(), 1);
    }

    #[test]
    fn independent_child_destruction_forgets_state_and_detaches_parent() {
        let registry = registry();
        let operations = FakeCaptionOperations::default();
        install_overlay(100, &registry, &operations).unwrap();

        child_destroyed_from(200, &registry, &operations);
        parent_destroyed_from(100, &registry, &operations);

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.remove_subclass_count.get(), 1);
        assert_eq!(operations.destroy_count.get(), 0);
    }

    #[test]
    fn native_hover_channel_coalesces_transitions_and_closes_after_teardown() {
        let (bridge, mut receiver) = NativeHoverBridge::new();

        bridge.set_hovered(true);
        bridge.set_hovered(false);

        assert!(receiver.has_changed().unwrap());
        assert!(!*receiver.borrow_and_update());
        assert!(!receiver.has_changed().unwrap());

        bridge.stop();
        bridge.set_hovered(true);
        assert!(!receiver.has_changed().unwrap());
    }

    #[test]
    fn titlebar_overlay_matches_the_forty_pixel_caption_at_supported_dpi_scales() {
        assert_eq!(TITLEBAR_HEIGHT, 40);
        assert_eq!(CAPTION_BUTTON_WIDTH, 46);
        for (dpi, expected_height, expected_width) in [(96, 40, 46), (120, 50, 58), (144, 60, 69)] {
            assert_eq!(scale(TITLEBAR_HEIGHT, dpi), expected_height);
            assert_eq!(scale(CAPTION_BUTTON_WIDTH, dpi), expected_width);
        }
    }
}

#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use tauri::{Runtime, WebviewWindow};
use windows_sys::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, SetLastError, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetParent, HTMAXBUTTON, HWND_TOP, IsWindow, IsZoomed, LWA_ALPHA, PostMessageW,
    RegisterClassExW, SC_MAXIMIZE, SC_RESTORE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, WM_DESTROY,
    WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_NCHITTEST,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSEMOVE, WM_SIZE, WM_SYSCOMMAND, WNDCLASSEXW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_OVERLAPPED, WS_VISIBLE,
};

const TITLEBAR_HEIGHT: u32 = 48;
const CAPTION_BUTTON_WIDTH: u32 = 46;
const CLOSE_BUTTONS_TO_THE_RIGHT: u32 = 1;
const SUBCLASS_ID: usize = 0x534b_5943_4150;
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

#[derive(Clone, Copy)]
struct SnapOverlayState {
    overlay: isize,
    titlebar_height: u32,
    button_width: u32,
}

type OverlayRegistry = Mutex<HashMap<isize, SnapOverlayState>>;

static SNAP_OVERLAYS: LazyLock<OverlayRegistry> = LazyLock::new(|| Mutex::new(HashMap::new()));
static REGISTER_CLASS: OnceLock<Result<(), String>> = OnceLock::new();

/// `setup` runs from Tauri's main-thread `Ready` event. Reserve the parent
/// subclass synchronously so WM_DESTROY owns the captured HWND before queuing
/// child creation. The queued task checks that reservation before any Win32
/// call, so it becomes a no-op if the parent is destroyed first.
pub fn install<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let parent = window_hwnd(window)?;
    register_class()?;
    if !reserve_parent(parent, &SNAP_OVERLAYS, &Win32CaptionOperations)? {
        return Ok(());
    }

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
        !parent_as_hwnd(parent).is_null() && unsafe { IsWindow(parent_as_hwnd(parent)) != 0 }
    }

    fn create_overlay(&self, parent: isize) -> Result<isize, String> {
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
        let configured =
            unsafe { SetLayeredWindowAttributes(overlay_as_hwnd(overlay), 0, 1, LWA_ALPHA) };
        if configured == 0 {
            Err("SetLayeredWindowAttributes failed for the native caption hit target".into())
        } else {
            Ok(())
        }
    }

    fn set_parent_subclass(&self, parent: isize) -> bool {
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

fn reserve_parent(
    parent: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
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
        lock_overlays(registry).remove(&parent);
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
        .copied()
        .unwrap_or(SnapOverlayState {
            overlay: 0,
            titlebar_height: TITLEBAR_HEIGHT,
            button_width: CAPTION_BUTTON_WIDTH,
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
            overlays.remove(&parent).is_some()
        } else {
            false
        }
    };
    if removed && !operations.remove_parent_subclass(parent) {
        eprintln!("RemoveWindowSubclass failed while rolling back caption hit target installation");
    }
}

fn destroy_partial_overlay(operations: &impl CaptionOperations, overlay: isize) {
    if !operations.destroy_overlay(overlay) {
        eprintln!("DestroyWindow failed while rolling back caption hit target installation");
    }
}

fn register_class() -> Result<(), String> {
    REGISTER_CLASS
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
    let mut client = unsafe { std::mem::zeroed() };
    if unsafe { GetClientRect(parent, &mut client) } == 0 {
        return Err("GetClientRect failed while positioning the caption hit target".into());
    }
    let dpi = unsafe { GetDpiForWindow(parent) }.max(96) as u64;
    let button_width = scale(state.button_width, dpi);
    let titlebar_height = scale(state.titlebar_height, dpi);
    let x = (client.right - button_width * (CLOSE_BUTTONS_TO_THE_RIGHT as i32 + 1)).max(0);
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
        .copied();
    if let Some(state) = state.filter(|state| state.overlay != 0) {
        position_overlay(parent as isize, state)
    } else {
        Ok(())
    }
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

unsafe extern "system" fn parent_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    unsafe {
        match message {
            WM_DESTROY => parent_destroying(hwnd),
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

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_NCHITTEST => HTMAXBUTTON as LRESULT,
            WM_NCMOUSEMOVE => 0,
            WM_NCLBUTTONDOWN | WM_LBUTTONDOWN => 0,
            WM_NCLBUTTONUP | WM_LBUTTONUP => {
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
    let had_state = { lock_overlays(registry).remove(&parent).is_some() };
    if had_state && !operations.remove_parent_subclass(parent) {
        eprintln!("RemoveWindowSubclass failed during native caption teardown");
    }
}

fn child_destroyed_from(
    overlay: isize,
    registry: &OverlayRegistry,
    operations: &impl CaptionOperations,
) {
    let parent = {
        let mut overlays = lock_overlays(registry);
        let parent = overlays
            .iter()
            .find_map(|(parent, state)| (state.overlay == overlay).then_some(*parent));
        if let Some(parent) = parent {
            overlays.remove(&parent);
        }
        parent
    };
    if let Some(parent) = parent
        && !operations.remove_parent_subclass(parent)
    {
        eprintln!("RemoveWindowSubclass failed after caption hit target destruction");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptionOperations, OverlayRegistry, SnapOverlayState, child_destroyed_from,
        finish_overlay_install, install_overlay, lock_overlays, parent_destroyed_from,
        reserve_parent,
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
}

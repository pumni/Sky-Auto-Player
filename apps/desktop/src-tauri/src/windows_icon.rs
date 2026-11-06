#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, IsWindow, LoadImageW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_DESTROY, WM_SETICON,
};

const TAURI_APP_ICON_RESOURCE_ID: usize = 32512;
const DPI_BASE: u32 = 96;
const ICON_SUBCLASS_ID: usize = 0x534b_5949_434f;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeWindowIcons {
    small: isize,
    big: isize,
    generation: u64,
}

type IconRegistry = Mutex<HashMap<isize, NativeWindowIcons>>;
type IconDimensions = (i32, i32);
type SystemIconDimensions = (IconDimensions, IconDimensions);

static NATIVE_WINDOW_ICONS: OnceLock<IconRegistry> = OnceLock::new();
static NEXT_ICON_GENERATION: AtomicU64 = AtomicU64::new(1);

fn native_window_icons() -> &'static IconRegistry {
    NATIVE_WINDOW_ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn system_icon_dimensions(dpi: u32) -> Result<SystemIconDimensions, String> {
    let dpi = if dpi == 0 { DPI_BASE } else { dpi };
    // SAFETY: the calls use documented scalar metric identifiers and a
    // positive DPI; they do not dereference pointers.
    let small = unsafe {
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            GetSystemMetricsForDpi(SM_CYSMICON, dpi),
        )
    };
    // SAFETY: the calls use documented scalar metric identifiers and the
    // same positive DPI; they do not dereference pointers.
    let large = unsafe {
        (
            GetSystemMetricsForDpi(SM_CXICON, dpi),
            GetSystemMetricsForDpi(SM_CYICON, dpi),
        )
    };
    if small.0 <= 0 || small.1 <= 0 || large.0 <= 0 || large.1 <= 0 {
        return Err(format!(
            "GetSystemMetricsForDpi returned invalid icon dimensions: dpi={dpi}, small={}x{}, large={}x{}",
            small.0, small.1, large.0, large.1
        ));
    }
    Ok((small, large))
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
        _ => Err("window does not expose a Win32 HWND".into()),
    }
}

trait IconOperations {
    fn is_window(&self, hwnd: isize) -> bool;
    fn load_icon(&self, width: i32, height: i32) -> Result<isize, String>;
    fn set_icon_subclass(&self, hwnd: isize) -> bool;
    fn remove_icon_subclass(&self, hwnd: isize) -> bool;
    fn send_set_icon(&self, hwnd: isize, icon_kind: usize, icon: isize);
    fn destroy_icon(&self, icon: isize);
}

struct Win32IconOperations;

impl IconOperations for Win32IconOperations {
    fn is_window(&self, hwnd: isize) -> bool {
        // SAFETY: this nonzero HWND comes from Tauri's Win32 window handle;
        // IsWindow validates it without dereferencing caller-owned memory.
        hwnd != 0 && unsafe { IsWindow(hwnd_as_hwnd(hwnd)) != 0 }
    }

    fn load_icon(&self, width: i32, height: i32) -> Result<isize, String> {
        // SAFETY: null requests the current process module and has no borrowed
        // output pointer; the module is used only for a resource lookup.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module.is_null() {
            return Err("GetModuleHandleW failed while loading native window icons".into());
        }
        load_resource_icon(module, width, height)
    }

    fn set_icon_subclass(&self, hwnd: isize) -> bool {
        // SAFETY: hwnd is a validated live Tauri window; the callback has the
        // required system ABI and is static for the full registration period.
        unsafe {
            SetWindowSubclass(
                hwnd_as_hwnd(hwnd),
                Some(icon_subclass_proc),
                ICON_SUBCLASS_ID,
                0,
            ) != 0
        }
    }

    fn remove_icon_subclass(&self, hwnd: isize) -> bool {
        // SAFETY: this removes the same callback/id pair while the main-thread
        // window lifecycle still owns the HWND.
        unsafe {
            RemoveWindowSubclass(
                hwnd_as_hwnd(hwnd),
                Some(icon_subclass_proc),
                ICON_SUBCLASS_ID,
            ) != 0
        }
    }

    fn send_set_icon(&self, hwnd: isize, icon_kind: usize, icon: isize) {
        // SAFETY: the HWND and icon are owned by this operation's registry;
        // SendMessageW is synchronous and generation checks handle reentry.
        unsafe {
            SendMessageW(hwnd_as_hwnd(hwnd), WM_SETICON, icon_kind, icon);
        }
    }

    fn destroy_icon(&self, icon: isize) {
        // SAFETY: icons are created without LR_SHARED and the registry removes
        // each owned handle exactly once before destroying it.
        unsafe { DestroyIcon(icon_as_hicon(icon)) };
    }
}

/// Tauri calls setup from the GUI thread's Ready callback and delivers scale
/// changes on that same thread. This serializes icon refresh with WM_DESTROY;
/// no queued task can outlive the HWND used here.
pub fn apply_native_window_icons<W>(window: &W) -> Result<(), String>
where
    W: HasWindowHandle,
{
    let hwnd = window_hwnd(window)?;
    if hwnd == 0 {
        return Err("cannot apply native icons to a null HWND".into());
    }
    if !Win32IconOperations.is_window(hwnd) {
        return Err("cannot apply native icons to an invalid HWND".into());
    }
    // SAFETY: hwnd was extracted from the live Tauri window and validated.
    let dpi = unsafe { GetDpiForWindow(hwnd_as_hwnd(hwnd)) };
    let (small_dimensions, large_dimensions) = system_icon_dimensions(dpi)?;
    apply_icons(
        hwnd,
        small_dimensions,
        large_dimensions,
        native_window_icons(),
        &Win32IconOperations,
    )
}

fn apply_icons(
    hwnd: isize,
    small_dimensions: IconDimensions,
    large_dimensions: IconDimensions,
    registry: &IconRegistry,
    operations: &impl IconOperations,
) -> Result<(), String> {
    if hwnd == 0 || !operations.is_window(hwnd) {
        return Err("cannot apply native icons to an invalid HWND".into());
    }

    let small = operations.load_icon(small_dimensions.0, small_dimensions.1)?;
    let big = match operations.load_icon(large_dimensions.0, large_dimensions.1) {
        Ok(icon) => icon,
        Err(error) => {
            operations.destroy_icon(small);
            return Err(error);
        }
    };

    if !operations.set_icon_subclass(hwnd) {
        operations.destroy_icon(small);
        operations.destroy_icon(big);
        return Err("SetWindowSubclass failed for native window icon ownership".into());
    }

    let generation = NEXT_ICON_GENERATION.fetch_add(1, Ordering::Relaxed);
    let previous = lock_icons(registry).insert(
        hwnd,
        NativeWindowIcons {
            small,
            big,
            generation,
        },
    );

    // The owned pair is registered before SendMessageW because a window proc
    // can reenter and destroy the HWND while processing WM_SETICON. The
    // destruction callback then takes ownership exactly once. Check the
    // generation after each synchronous send so a second message is never sent
    // to a window that was destroyed or replaced reentrantly.
    operations.send_set_icon(hwnd, ICON_SMALL as usize, small);
    if !owns_generation(registry, hwnd, generation) {
        destroy_previous(previous, operations);
        return Err("native window was destroyed while applying its small icon".into());
    }

    operations.send_set_icon(hwnd, ICON_BIG as usize, big);
    if !owns_generation(registry, hwnd, generation) {
        destroy_previous(previous, operations);
        return Err("native window was destroyed while applying its large icon".into());
    }

    destroy_previous(previous, operations);
    Ok(())
}

fn owns_generation(registry: &IconRegistry, hwnd: isize, generation: u64) -> bool {
    lock_icons(registry)
        .get(&hwnd)
        .is_some_and(|icons| icons.generation == generation)
}

fn destroy_previous(previous: Option<NativeWindowIcons>, operations: &impl IconOperations) {
    if let Some(previous) = previous {
        destroy_icon_pair(previous, operations);
    }
}

fn destroy_icon_pair(icons: NativeWindowIcons, operations: &impl IconOperations) {
    operations.destroy_icon(icons.small);
    operations.destroy_icon(icons.big);
}

/// WM_DESTROY is delivered while this HWND is valid and after it has been
/// removed from the screen. No more icon assignment is needed: remove the
/// subclass and release our owned handles without asking Tauri for the HWND or
/// sending another message to a window in destruction.
fn window_destroyed(hwnd: isize) {
    release_icons_for_destroy(hwnd, native_window_icons(), &Win32IconOperations);
}

fn release_icons_for_destroy(
    hwnd: isize,
    registry: &IconRegistry,
    operations: &impl IconOperations,
) {
    let Some(icons) = lock_icons(registry).remove(&hwnd) else {
        return;
    };

    if !operations.remove_icon_subclass(hwnd) {
        eprintln!("RemoveWindowSubclass failed during native icon teardown");
    }
    destroy_icon_pair(icons, operations);
}

// SAFETY: Windows invokes this function only after the subclass is registered
// and supplies its HWND and message arguments according to the system ABI.
unsafe extern "system" fn icon_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    _subclass_id: usize,
    _ref_data: usize,
) -> isize {
    // SAFETY: Windows supplies this callback's HWND and message arguments;
    // the registered function has the system ABI and does not retain pointers.
    unsafe {
        if message == WM_DESTROY {
            window_destroyed(hwnd as isize);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

fn load_resource_icon(module: HINSTANCE, width: i32, height: i32) -> Result<isize, String> {
    // Explicit dimensions make LoadImageW select the closest native image from
    // the embedded ICO resource. Omitting LR_SHARED makes each result owned.
    // SAFETY: module is the current process module, the integer resource ID is
    // the documented MAKEINTRESOURCE form, and the returned handle is owned.
    let icon = unsafe {
        LoadImageW(
            module,
            TAURI_APP_ICON_RESOURCE_ID as *const u16,
            IMAGE_ICON,
            width,
            height,
            0,
        )
    };
    if icon.is_null() {
        Err(format!(
            "LoadImageW failed for {width}x{height} resource icon"
        ))
    } else {
        Ok(icon as isize)
    }
}

fn hwnd_as_hwnd(hwnd: isize) -> HWND {
    hwnd as HWND
}

fn icon_as_hicon(icon: isize) -> HICON {
    icon as HICON
}

fn lock_icons(registry: &IconRegistry) -> MutexGuard<'_, HashMap<isize, NativeWindowIcons>> {
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{
        IconDimensions, IconOperations, IconRegistry, NativeWindowIcons, apply_icons,
        release_icons_for_destroy,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Mutex;

    type SetIconCallback = Box<dyn Fn(isize, usize, isize)>;

    struct FakeIconOperations {
        valid_window: bool,
        fail_load_at: Option<usize>,
        subclass_fails: bool,
        load_count: Cell<usize>,
        next_icon: Cell<isize>,
        destroyed_icons: RefCell<Vec<isize>>,
        set_icon_calls: RefCell<Vec<(isize, usize, isize)>>,
        subclass_remove_count: Cell<usize>,
        set_icon_callback: RefCell<Option<SetIconCallback>>,
    }

    impl Default for FakeIconOperations {
        fn default() -> Self {
            Self {
                valid_window: true,
                fail_load_at: None,
                subclass_fails: false,
                load_count: Cell::new(0),
                next_icon: Cell::new(300),
                destroyed_icons: RefCell::new(Vec::new()),
                set_icon_calls: RefCell::new(Vec::new()),
                subclass_remove_count: Cell::new(0),
                set_icon_callback: RefCell::new(None),
            }
        }
    }

    impl IconOperations for FakeIconOperations {
        fn is_window(&self, _hwnd: isize) -> bool {
            self.valid_window
        }

        fn load_icon(&self, _width: i32, _height: i32) -> Result<isize, String> {
            let count = self.load_count.get() + 1;
            self.load_count.set(count);
            if self.fail_load_at == Some(count) {
                return Err("injected icon allocation failure".into());
            }
            let icon = self.next_icon.get();
            self.next_icon.set(icon + 1);
            Ok(icon)
        }

        fn set_icon_subclass(&self, _hwnd: isize) -> bool {
            !self.subclass_fails
        }

        fn remove_icon_subclass(&self, _hwnd: isize) -> bool {
            self.subclass_remove_count
                .set(self.subclass_remove_count.get() + 1);
            true
        }

        fn send_set_icon(&self, hwnd: isize, icon_kind: usize, icon: isize) {
            self.set_icon_calls
                .borrow_mut()
                .push((hwnd, icon_kind, icon));
            if let Some(callback) = self.set_icon_callback.borrow().as_ref() {
                callback(hwnd, icon_kind, icon);
            }
        }

        fn destroy_icon(&self, icon: isize) {
            self.destroyed_icons.borrow_mut().push(icon);
        }
    }

    fn registry() -> IconRegistry {
        Mutex::new(HashMap::new())
    }

    fn dimensions(seed: i32) -> (IconDimensions, IconDimensions) {
        ((seed, seed), (seed * 2, seed * 2))
    }

    fn installed_pair(registry: &IconRegistry, hwnd: isize) -> NativeWindowIcons {
        *registry
            .lock()
            .unwrap()
            .get(&hwnd)
            .expect("icons installed")
    }

    #[test]
    fn installs_owned_icons_and_releases_them_once_at_destroy() {
        let registry = registry();
        let operations = FakeIconOperations::default();
        let (small, big) = dimensions(16);
        apply_icons(100, small, big, &registry, &operations).unwrap();
        let pair = installed_pair(&registry, 100);

        release_icons_for_destroy(100, &registry, &operations);
        release_icons_for_destroy(100, &registry, &operations);

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(
            operations.destroyed_icons.borrow().as_slice(),
            [pair.small, pair.big]
        );
        assert_eq!(operations.subclass_remove_count.get(), 1);
        assert_eq!(operations.set_icon_calls.borrow().len(), 2);
    }

    #[test]
    fn invalid_window_and_partial_icon_allocation_do_not_leak() {
        let registry = registry();
        let invalid_operations = FakeIconOperations {
            valid_window: false,
            ..FakeIconOperations::default()
        };
        let (small, big) = dimensions(16);
        assert!(apply_icons(100, small, big, &registry, &invalid_operations).is_err());
        assert_eq!(invalid_operations.load_count.get(), 0);

        let partial_operations = FakeIconOperations {
            fail_load_at: Some(2),
            ..FakeIconOperations::default()
        };
        assert!(apply_icons(100, small, big, &registry, &partial_operations).is_err());
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(
            partial_operations.destroyed_icons.borrow().as_slice(),
            [300]
        );
        assert!(partial_operations.set_icon_calls.borrow().is_empty());
    }

    #[test]
    fn subclass_failure_releases_both_new_icons() {
        let registry = registry();
        let operations = FakeIconOperations {
            subclass_fails: true,
            ..FakeIconOperations::default()
        };
        let (small, big) = dimensions(16);

        assert!(apply_icons(100, small, big, &registry, &operations).is_err());

        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.destroyed_icons.borrow().as_slice(), [300, 301]);
        assert!(operations.set_icon_calls.borrow().is_empty());
    }

    #[test]
    fn dpi_replacement_releases_superseded_pair_then_final_pair_once() {
        let registry = registry();
        let operations = FakeIconOperations::default();
        let (small_96, big_96) = dimensions(16);
        let (small_144, big_144) = dimensions(24);
        apply_icons(100, small_96, big_96, &registry, &operations).unwrap();
        let first = installed_pair(&registry, 100);

        apply_icons(100, small_144, big_144, &registry, &operations).unwrap();
        let second = installed_pair(&registry, 100);

        assert_ne!(first.generation, second.generation);
        assert_eq!(
            operations.destroyed_icons.borrow().as_slice(),
            [first.small, first.big]
        );
        release_icons_for_destroy(100, &registry, &operations);
        assert_eq!(
            operations.destroyed_icons.borrow().as_slice(),
            [first.small, first.big, second.small, second.big]
        );
    }

    #[test]
    fn reentrant_destroy_during_dpi_apply_does_not_send_big_icon_or_leak() {
        let registry = Rc::new(registry());
        let operations = Rc::new(FakeIconOperations::default());
        let weak_operations = Rc::downgrade(&operations);
        let destroy_registry = Rc::clone(&registry);
        *operations.set_icon_callback.borrow_mut() = Some(Box::new(move |hwnd, _, _| {
            if let Some(operations) = weak_operations.upgrade() {
                release_icons_for_destroy(hwnd, &destroy_registry, operations.as_ref());
            }
        }));
        let (small, big) = dimensions(16);

        let result = apply_icons(100, small, big, registry.as_ref(), operations.as_ref());

        assert!(result.is_err());
        assert!(registry.lock().unwrap().is_empty());
        assert_eq!(operations.set_icon_calls.borrow().len(), 1);
        assert_eq!(operations.destroyed_icons.borrow().as_slice(), [300, 301]);
        assert_eq!(operations.subclass_remove_count.get(), 1);
    }
}

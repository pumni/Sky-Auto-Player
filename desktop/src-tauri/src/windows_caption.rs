#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::{LazyLock, Mutex, Once};
use tauri::{Runtime, WebviewWindow, Window};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetParent, HTMAXBUTTON, HWND_TOP, IsZoomed, LWA_ALPHA, PostMessageW, RegisterClassExW,
    SC_MAXIMIZE, SC_RESTORE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowPos, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSEMOVE, WM_SIZE,
    WM_SYSCOMMAND, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TRANSPARENT, WS_OVERLAPPED, WS_VISIBLE,
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

struct SnapOverlayState {
    overlay: isize,
    titlebar_height: u32,
    button_width: u32,
}

static SNAP_OVERLAYS: LazyLock<Mutex<HashMap<isize, SnapOverlayState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static REGISTER_CLASS: Once = Once::new();

pub fn install<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let hwnd = window_hwnd(window)?;
    window
        .run_on_main_thread(move || install_overlay(hwnd))
        .map_err(|error| format!("schedule Windows caption hit target: {error}"))
}

pub fn uninstall_window<R: Runtime>(window: &Window<R>) -> Result<(), String> {
    let hwnd = window_hwnd(window)?;
    window
        .run_on_main_thread(move || remove_overlay(hwnd as HWND))
        .map_err(|error| format!("schedule Windows caption cleanup: {error}"))
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

fn install_overlay(parent: isize) {
    unsafe {
        register_class();
        remove_overlay(parent as HWND);

        let overlay = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            SNAP_CLASS.as_ptr(),
            SNAP_CLASS.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            parent as HWND,
            std::ptr::null_mut(),
            module_instance(),
            std::ptr::null_mut(),
        );
        if overlay.is_null() {
            return;
        }

        // The child is input-active but visually transparent, so the HTML caption
        // glyph and its CSS states remain visible below the native hit target.
        SetLayeredWindowAttributes(overlay, 0, 0, LWA_ALPHA);

        let mut overlays = SNAP_OVERLAYS
            .lock()
            .expect("caption overlay state poisoned");
        overlays.insert(
            parent,
            SnapOverlayState {
                overlay: overlay as isize,
                titlebar_height: TITLEBAR_HEIGHT,
                button_width: CAPTION_BUTTON_WIDTH,
            },
        );
        drop(overlays);

        if SetWindowSubclass(parent as HWND, Some(parent_subclass_proc), SUBCLASS_ID, 0) == 0 {
            remove_overlay(parent as HWND);
            return;
        }
        update_overlay_position(parent as HWND);
    }
}

fn register_class() {
    REGISTER_CLASS.call_once(|| unsafe {
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
        RegisterClassExW(&class);
    });
}

fn module_instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

fn update_overlay_position(parent: HWND) {
    let (overlay, titlebar_height, button_width) = {
        let overlays = SNAP_OVERLAYS
            .lock()
            .expect("caption overlay state poisoned");
        let Some(state) = overlays.get(&(parent as isize)) else {
            return;
        };
        (
            state.overlay as HWND,
            state.titlebar_height,
            state.button_width,
        )
    };

    unsafe {
        let mut client = std::mem::zeroed();
        if GetClientRect(parent, &mut client) == 0 {
            return;
        }
        let dpi = GetDpiForWindow(parent).max(96) as u64;
        let button_width = scale(button_width, dpi);
        let titlebar_height = scale(titlebar_height, dpi);
        let x = (client.right - button_width * (CLOSE_BUTTONS_TO_THE_RIGHT as i32 + 1)).max(0);
        SetWindowPos(
            overlay,
            HWND_TOP,
            x,
            0,
            button_width,
            titlebar_height,
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
        );
    }
}

fn scale(value: u32, dpi: u64) -> i32 {
    ((value as u64 * dpi + 48) / 96).max(1) as i32
}

fn remove_overlay(parent: HWND) {
    unsafe {
        RemoveWindowSubclass(parent, Some(parent_subclass_proc), SUBCLASS_ID);
        if let Some(state) = SNAP_OVERLAYS
            .lock()
            .expect("caption overlay state poisoned")
            .remove(&(parent as isize))
        {
            DestroyWindow(state.overlay as HWND);
        }
    }
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
        if matches!(message, WM_SIZE | WM_DPICHANGED) {
            update_overlay_position(hwnd);
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
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

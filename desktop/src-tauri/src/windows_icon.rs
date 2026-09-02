#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
};

const TAURI_APP_ICON_RESOURCE_ID: usize = 32512;
const DPI_BASE: u32 = 96;

#[derive(Clone, Copy)]
struct NativeWindowIcons {
    small: isize,
    big: isize,
}

static NATIVE_WINDOW_ICONS: OnceLock<Mutex<HashMap<isize, NativeWindowIcons>>> = OnceLock::new();

type IconDimensions = (i32, i32);
type SystemIconDimensions = (IconDimensions, IconDimensions);

fn native_window_icons() -> &'static Mutex<HashMap<isize, NativeWindowIcons>> {
    NATIVE_WINDOW_ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn system_icon_dimensions(dpi: u32) -> Result<SystemIconDimensions, String> {
    let dpi = if dpi == 0 { DPI_BASE } else { dpi };
    let small = unsafe {
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            GetSystemMetricsForDpi(SM_CYSMICON, dpi),
        )
    };
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

fn window_hwnd<W>(window: &W) -> Result<HWND, String>
where
    W: HasWindowHandle,
{
    let handle = window
        .window_handle()
        .map_err(|error| format!("window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
        _ => Err("window does not expose a Win32 HWND".into()),
    }
}

fn load_resource_icon(module: HINSTANCE, width: i32, height: i32) -> Result<HICON, String> {
    // Passing explicit dimensions makes LoadImageW select the closest native
    // image from the embedded ICO resource instead of using the first image.
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
        Ok(icon as HICON)
    }
}

pub fn apply_native_window_icons<W>(window: &W) -> Result<(), String>
where
    W: HasWindowHandle,
{
    let hwnd = window_hwnd(window)?;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err("GetModuleHandleW failed".into());
    }

    let (small_dimensions, large_dimensions) = system_icon_dimensions(dpi)?;
    let small = load_resource_icon(module, small_dimensions.0, small_dimensions.1)?;
    let big = match load_resource_icon(module, large_dimensions.0, large_dimensions.1) {
        Ok(icon) => icon,
        Err(error) => {
            unsafe { DestroyIcon(small) };
            return Err(error);
        }
    };

    unsafe {
        // WM_SETICON associates separate small-caption and large-taskbar/Alt+Tab
        // handles with the HWND. The handles remain owned by this module until
        // the next DPI refresh or window destruction.
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
    }

    let previous = native_window_icons()
        .lock()
        .map_err(|_| "native window icon state lock poisoned".to_string())?
        .insert(
            hwnd as isize,
            NativeWindowIcons {
                small: small as isize,
                big: big as isize,
            },
        );
    if let Some(previous) = previous {
        unsafe {
            DestroyIcon(previous.small as HICON);
            DestroyIcon(previous.big as HICON);
        }
    }
    Ok(())
}

pub fn release_native_window_icons<W>(window: &W)
where
    W: HasWindowHandle,
{
    let Ok(hwnd) = window_hwnd(window) else {
        return;
    };
    let Ok(previous) = native_window_icons()
        .lock()
        .map(|mut icons| icons.remove(&(hwnd as isize)))
    else {
        return;
    };
    if let Some(previous) = previous {
        unsafe {
            DestroyIcon(previous.small as HICON);
            DestroyIcon(previous.big as HICON);
        }
    }
}

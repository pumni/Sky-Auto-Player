#![cfg(windows)]

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, SendMessageW, WM_SETICON,
};

const TAURI_APP_ICON_RESOURCE_ID: usize = 32512;
const DPI_BASE: u32 = 96;
const TITLEBAR_BASE_PX: u32 = 16;
const TASKBAR_BASE_PX: u32 = 24;

#[derive(Clone, Copy)]
struct NativeWindowIcons {
    small: isize,
    big: isize,
}

static NATIVE_WINDOW_ICONS: OnceLock<Mutex<HashMap<isize, NativeWindowIcons>>> = OnceLock::new();

fn native_window_icons() -> &'static Mutex<HashMap<isize, NativeWindowIcons>> {
    NATIVE_WINDOW_ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scaled(base_px: u32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { DPI_BASE } else { dpi };
    let physical_px =
        (u64::from(base_px) * u64::from(dpi) + u64::from(DPI_BASE / 2)) / u64::from(DPI_BASE);
    physical_px.clamp(1, i32::MAX as u64) as i32
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

fn load_resource_icon(module: HINSTANCE, size: i32) -> Result<HICON, String> {
    // Passing explicit dimensions makes LoadImageW select the closest native
    // image from the embedded ICO resource instead of using the first image.
    let icon = unsafe {
        LoadImageW(
            module,
            TAURI_APP_ICON_RESOURCE_ID as *const u16,
            IMAGE_ICON,
            size,
            size,
            0,
        )
    };
    if icon.is_null() {
        Err(format!("LoadImageW failed for {size}x{size} resource icon"))
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

    let small = load_resource_icon(module, scaled(TITLEBAR_BASE_PX, dpi))?;
    let big = match load_resource_icon(module, scaled(TASKBAR_BASE_PX, dpi)) {
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

#[cfg(test)]
mod tests {
    use super::scaled;

    #[test]
    fn scaled_icon_targets_round_to_common_windows_dpi_sizes() {
        assert_eq!(scaled(16, 96), 16);
        assert_eq!(scaled(16, 120), 20);
        assert_eq!(scaled(16, 144), 24);
        assert_eq!(scaled(16, 192), 32);
        assert_eq!(scaled(24, 96), 24);
        assert_eq!(scaled(24, 120), 30);
        assert_eq!(scaled(24, 144), 36);
        assert_eq!(scaled(24, 192), 48);
        assert_eq!(scaled(16, 0), 16);
    }
}

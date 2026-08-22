//! Small native progress surface owned by the updater process.
//!
//! The updater must remain visible after the Textual process exits.  The
//! Windows implementation keeps all HWND values inside this module and sends
//! state changes to a dedicated window thread through ordinary Win32 window
//! messages.

#[cfg(not(windows))]
mod platform {
    use crate::error::{Result, UpdaterError};
    use crate::progress::UpdatePhase;

    pub struct NativeProgressUi;

    impl NativeProgressUi {
        pub fn start(_current_version: &str, _target_version: &str) -> Result<Self> {
            Ok(Self)
        }

        pub fn set_phase(&self, _phase: UpdatePhase, _current: Option<u64>, _total: Option<u64>) {}

        pub fn show_failure(&self, _title: &str, _message: &str) {}

        pub fn show_rolled_back(&self, _message: &str) {}

        pub fn show_restart_failure(&self, _message: &str) {}

        pub fn show_restarting(&self) {}

        pub fn close_after_success(&mut self) {}

        pub fn wait_for_user_close(&mut self) {}
    }

    impl From<UpdaterError> for NativeProgressUi {
        fn from(_value: UpdaterError) -> Self {
            Self
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ptr::null_mut;
    use std::sync::{Arc, Mutex, OnceLock, mpsc};
    use std::thread::{self, JoinHandle};

    use crate::error::{Result, UpdaterError};
    use crate::progress::UpdatePhase;
    use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};

    const WM_REFRESH: u32 = 0x8001;
    const ID_STATUS: i32 = 1001;
    const ID_PROGRESS: i32 = 1002;
    const ID_DETAIL: i32 = 1003;
    const ID_CLOSE: i32 = 1004;
    const IDC_STATIC: i32 = -1;
    const PBM_SETPOS: u32 = 0x0402;
    const PBM_SETRANGE32: u32 = 0x0406;
    const PBM_SETMARQUEE: u32 = 0x040A;
    const WM_COMMAND: u32 = 0x0111;
    const WM_SIZE: u32 = 0x0005;
    const WM_CLOSE: u32 = 0x0010;
    const BN_CLICKED: u16 = 0;
    const WS_CHILD: u32 = 0x40000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_TABSTOP: u32 = 0x00010000;
    const WS_OVERLAPPEDWINDOW: u32 = 0x00CF0000;
    const SS_LEFT: u32 = 0;
    const BS_PUSHBUTTON: u32 = 0;
    const PBM_STYLE_MARQUEE: u32 = 0x08;
    const SW_SHOW: i32 = 5;
    const CW_USEDEFAULT: i32 = 0x80000000u32 as i32;
    const COLOR_WINDOW: i32 = 5;

    #[derive(Clone, Debug)]
    struct UiState {
        current_version: String,
        target_version: String,
        phase: UpdatePhase,
        current: Option<u64>,
        total: Option<u64>,
        title: String,
        message: String,
        terminal: bool,
        close_after_success: bool,
    }

    static STATE: OnceLock<Arc<Mutex<UiState>>> = OnceLock::new();

    #[derive(Clone, Copy)]
    struct UiHandles {
        heading: isize,
        version: isize,
        status: isize,
        progress: isize,
        detail: isize,
        close: isize,
    }

    static CONTROLS: OnceLock<UiHandles> = OnceLock::new();

    pub struct NativeProgressUi {
        state: Arc<Mutex<UiState>>,
        hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }

    impl NativeProgressUi {
        pub fn start(current_version: &str, target_version: &str) -> Result<Self> {
            let state = Arc::new(Mutex::new(UiState {
                current_version: current_version.into(),
                target_version: target_version.into(),
                phase: UpdatePhase::Starting,
                current: None,
                total: None,
                title: String::new(),
                message: String::new(),
                terminal: false,
                close_after_success: false,
            }));
            let _ = STATE.set(state.clone());
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread_state = state.clone();
            let thread = thread::Builder::new()
                .name("sky-updater-progress-ui".into())
                .spawn(move || window_thread(thread_state, ready_tx))
                .map_err(|error| UpdaterError::UiInitializationFailed(error.to_string()))?;
            let hwnd = ready_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|error| UpdaterError::UiInitializationFailed(error.to_string()))??;
            Ok(Self {
                state,
                hwnd,
                thread: Some(thread),
            })
        }

        pub fn set_phase(&self, phase: UpdatePhase, current: Option<u64>, total: Option<u64>) {
            if let Ok(mut state) = self.state.lock() {
                state.phase = phase;
                state.current = current;
                state.total = total;
                state.terminal = matches!(
                    phase,
                    UpdatePhase::Completed | UpdatePhase::Failed | UpdatePhase::RolledBack
                );
            }
            post_refresh(self.hwnd);
        }

        pub fn show_failure(&self, title: &str, message: &str) {
            self.set_terminal(UpdatePhase::Failed, title, message, false);
        }

        pub fn show_rolled_back(&self, message: &str) {
            self.set_terminal(
                UpdatePhase::RolledBack,
                "Update rolled back",
                message,
                false,
            );
        }

        pub fn show_restart_failure(&self, message: &str) {
            self.set_terminal(UpdatePhase::Failed, "Restart failed", message, false);
        }

        pub fn show_restarting(&self) {
            self.set_phase(UpdatePhase::Restarting, None, None);
        }

        pub fn close_after_success(&mut self) {
            if let Ok(mut state) = self.state.lock() {
                state.phase = UpdatePhase::Completed;
                state.terminal = true;
                state.close_after_success = true;
            }
            post_refresh(self.hwnd);
        }

        pub fn wait_for_user_close(&mut self) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }

        fn set_terminal(&self, phase: UpdatePhase, title: &str, message: &str, close: bool) {
            if let Ok(mut state) = self.state.lock() {
                state.phase = phase;
                state.title = title.chars().take(512).collect();
                state.message = message.chars().take(512).collect();
                state.terminal = true;
                state.close_after_success = close;
            }
            post_refresh(self.hwnd);
        }
    }

    impl Drop for NativeProgressUi {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                if let Ok(mut state) = self.state.lock() {
                    state.close_after_success = true;
                }
                post_refresh(self.hwnd);
                let _ = thread.join();
            }
        }
    }

    fn window_thread(
        state: Arc<Mutex<UiState>>,
        ready: mpsc::SyncSender<std::result::Result<isize, UpdaterError>>,
    ) {
        let result = create_window(state);
        match result {
            Ok(hwnd) => {
                let _ = ready.send(Ok(hwnd as isize));
                unsafe {
                    let mut message = std::mem::zeroed();
                    while windows_sys::Win32::UI::WindowsAndMessaging::GetMessageW(
                        &mut message,
                        null_mut(),
                        0,
                        0,
                    ) > 0
                    {
                        windows_sys::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
                        windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
                    }
                }
            }
            Err(error) => {
                let _ = ready.send(Err(error));
            }
        }
    }

    fn create_window(state: Arc<Mutex<UiState>>) -> Result<HWND> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, RegisterClassW, WNDCLASSW,
        };

        let class = wide("SkyAutoPlayerUpdaterProgress");
        let title = wide("Sky Auto Player Updater");
        let instance: HINSTANCE = std::ptr::null_mut();
        let window_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: (COLOR_WINDOW + 1) as _,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err(UpdaterError::UiInitializationFailed(
                "RegisterClassW failed".into(),
            ));
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                520,
                230,
                null_mut(),
                null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(UpdaterError::UiInitializationFailed(
                "CreateWindowExW failed".into(),
            ));
        }
        create_controls(hwnd, instance, &state)?;
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, SW_SHOW);
        }
        Ok(hwnd)
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> LRESULT {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DefWindowProcW, PostQuitMessage, WM_DESTROY,
        };
        match message {
            WM_REFRESH => {
                refresh_controls(hwnd);
                0
            }
            WM_SIZE => {
                layout_controls(hwnd);
                0
            }
            WM_CLOSE => 0,
            WM_COMMAND
                if (wparam & 0xFFFF) as i32 == ID_CLOSE
                    && ((wparam >> 16) & 0xFFFF) as u16 == BN_CLICKED =>
            {
                if STATE
                    .get()
                    .and_then(|state| state.lock().ok().map(|state| state.terminal))
                    .unwrap_or(false)
                {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                    }
                }
                0
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, _lparam) },
        }
    }

    fn post_refresh(hwnd: isize) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd as HWND,
                WM_REFRESH,
                0,
                0,
            );
        }
    }

    fn create_controls(hwnd: HWND, instance: HINSTANCE, state: &Arc<Mutex<UiState>>) -> Result<()> {
        use windows_sys::Win32::UI::WindowsAndMessaging::CreateWindowExW;

        let heading = create_child(
            hwnd,
            instance,
            "STATIC",
            "Updating Sky Auto Player",
            24,
            18,
            450,
            26,
            IDC_STATIC,
            SS_LEFT,
        )?;
        let version_text = state
            .lock()
            .map(|state| format!("v{} → v{}", state.current_version, state.target_version))
            .unwrap_or_else(|_| "Updating Sky Auto Player".into());
        let version = create_child(
            hwnd,
            instance,
            "STATIC",
            &version_text,
            24,
            48,
            450,
            24,
            IDC_STATIC,
            SS_LEFT,
        )?;
        let status = create_child(
            hwnd,
            instance,
            "STATIC",
            UpdatePhase::Starting.display_text(),
            24,
            82,
            450,
            24,
            ID_STATUS,
            SS_LEFT,
        )?;
        let progress_class = wide("msctls_progress32");
        let progress = unsafe {
            CreateWindowExW(
                0,
                progress_class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | PBM_STYLE_MARQUEE,
                24,
                114,
                450,
                20,
                hwnd,
                ID_PROGRESS as isize as _,
                instance,
                std::ptr::null(),
            )
        };
        if progress.is_null() {
            return Err(UpdaterError::UiInitializationFailed(
                "progress control creation failed".into(),
            ));
        }
        let detail = create_child(
            hwnd,
            instance,
            "STATIC",
            "Do not close this window. Sky Auto Player will restart automatically.",
            24,
            146,
            450,
            40,
            ID_DETAIL,
            SS_LEFT,
        )?;
        let close = create_child(
            hwnd,
            instance,
            "BUTTON",
            "Close",
            394,
            188,
            80,
            26,
            ID_CLOSE,
            BS_PUSHBUTTON | WS_TABSTOP,
        )?;
        let handles = UiHandles {
            heading: heading as isize,
            version: version as isize,
            status: status as isize,
            progress: progress as isize,
            detail: detail as isize,
            close: close as isize,
        };
        let _ = CONTROLS.set(handles);
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(close, 0);
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                progress,
                PBM_SETRANGE32,
                0,
                100,
            );
        }
        Ok(())
    }

    fn create_child(
        parent: HWND,
        instance: HINSTANCE,
        class_name: &str,
        text: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        id: i32,
        style: u32,
    ) -> Result<HWND> {
        use windows_sys::Win32::UI::WindowsAndMessaging::CreateWindowExW;
        let class = wide(class_name);
        let text = wide(text);
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                width,
                height,
                parent,
                id as isize as _,
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(UpdaterError::UiInitializationFailed(format!(
                "{class_name} control creation failed"
            )));
        }
        Ok(hwnd)
    }

    fn layout_controls(hwnd: HWND) {
        let Some(controls) = CONTROLS.get().copied() else {
            return;
        };
        let mut rect = unsafe { std::mem::zeroed() };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
            let width = (rect.right - rect.left - 48).max(200);
            for (child, x, y, height) in [
                (controls.heading, 24, 18, 26),
                (controls.version, 24, 48, 24),
                (controls.status, 24, 82, 24),
                (controls.progress, 24, 114, 20),
                (controls.detail, 24, 146, 40),
            ] {
                windows_sys::Win32::UI::WindowsAndMessaging::MoveWindow(
                    child as HWND,
                    x,
                    y,
                    width,
                    height,
                    1,
                );
            }
            windows_sys::Win32::UI::WindowsAndMessaging::MoveWindow(
                controls.close as HWND,
                (rect.right - rect.left - 104).max(24),
                (rect.bottom - rect.top - 42).max(188),
                80,
                26,
                1,
            );
        }
    }

    fn refresh_controls(hwnd: HWND) {
        let Some(controls) = CONTROLS.get().copied() else {
            return;
        };
        let Some(state) = STATE
            .get()
            .and_then(|state| state.lock().ok().map(|state| state.clone()))
        else {
            return;
        };
        let status = if state.title.is_empty() {
            state.phase.display_text().into()
        } else {
            state.title.clone()
        };
        let detail = if state.terminal {
            state.message.clone()
        } else {
            "Do not close this window. Sky Auto Player will restart automatically.".into()
        };
        let status = wide(&status);
        let detail = wide(&detail);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                controls.status as HWND,
                status.as_ptr(),
            );
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                controls.detail as HWND,
                detail.as_ptr(),
            );
            let percent = progress_percent(&state);
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                controls.progress as HWND,
                PBM_SETMARQUEE,
                if matches!(state.phase, UpdatePhase::FetchingRelease) {
                    1
                } else {
                    0
                },
                40,
            );
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                controls.progress as HWND,
                PBM_SETPOS,
                percent as usize,
                0,
            );
            windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                controls.close as HWND,
                if state.terminal { 1 } else { 0 },
            );
            if state.close_after_success {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            }
        }
    }

    fn progress_percent(state: &UiState) -> u32 {
        if state.phase == UpdatePhase::Completed {
            return 100;
        }
        if matches!(
            state.phase,
            UpdatePhase::BackingUp | UpdatePhase::Installing
        ) {
            if let (Some(current), Some(total)) = (state.current, state.total) {
                return if total == 0 {
                    0
                } else {
                    ((current * 100) / total).min(100) as u32
                };
            }
        }
        match state.phase {
            UpdatePhase::Starting => 0,
            UpdatePhase::WaitingForParent => 5,
            UpdatePhase::FetchingRelease => 10,
            UpdatePhase::VerifyingRelease => 18,
            UpdatePhase::Extracting => 28,
            UpdatePhase::VerifyingStaging => 40,
            UpdatePhase::Preflight => 48,
            UpdatePhase::BackingUp => 56,
            UpdatePhase::Installing => 72,
            UpdatePhase::VerifyingInstall => 84,
            UpdatePhase::Committing => 90,
            UpdatePhase::CleaningUp => 94,
            UpdatePhase::Restarting => 98,
            UpdatePhase::Completed => 100,
            UpdatePhase::Failed | UpdatePhase::RolledBack => 0,
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

pub use platform::NativeProgressUi;

impl crate::progress::ProgressSink for NativeProgressUi {
    fn publish(&self, event: crate::progress::ProgressEvent) {
        self.set_phase(event.phase, event.current, event.total);
    }
}

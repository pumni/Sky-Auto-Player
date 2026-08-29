use crate::app_state::AppState;
use tauri::{Manager, Runtime, Window};

pub fn close_window<R: Runtime + 'static>(window: Window<R>) {
    let app_handle = window.app_handle().clone();
    let state = app_handle.state::<AppState>().inner().clone();
    if !state.begin_close() {
        return;
    }

    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(supervisor) = state.supervisor() {
            supervisor.shutdown();
        }
        // `destroy` does not emit another CloseRequested event, so the
        // bounded shutdown path cannot recurse through this callback.
        let _ = window.destroy();
    });
}

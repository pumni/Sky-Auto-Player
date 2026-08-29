mod app_state;
mod bindings;
mod commands;
mod lifecycle;
mod ui_events;

mod core;

use lifecycle::{shutdown_core, start_core};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(app_state::AppState::default())
        .setup(|app| {
            start_core(app).map_err(|error| error.to_string())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::search_songs,
            commands::get_song_detail,
            commands::reload_library,
            commands::set_library_viewport,
            commands::get_settings,
            commands::patch_settings,
            commands::subscribe_ui_events,
            commands::shutdown,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                shutdown_core(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Sky Auto Player desktop shell");
}

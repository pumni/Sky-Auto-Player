mod app_state;
mod bindings;
mod commands;
mod lifecycle;
mod ui_events;

mod core;

use lifecycle::close_window;

#[cfg(feature = "desktop-runtime")]
type ShellRuntime = tauri::Wry;
#[cfg(all(not(feature = "desktop-runtime"), feature = "tauri-test"))]
type ShellRuntime = tauri::test::MockRuntime;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::<ShellRuntime>::default()
        .manage(app_state::AppState::default())
        .setup(|_| Ok(()))
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
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                close_window(window.clone());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Sky Auto Player desktop shell");
}

// The mock runtime is intentionally exercised in a no-default-features test
// build. Keeping the production Wry runtime out of that test binary avoids
// loading the native desktop webview during command-decoder unit tests.
#[cfg(all(test, feature = "tauri-test", not(feature = "desktop-runtime")))]
mod ipc_tests {
    use super::app_state::AppState;
    use super::core::CoreSupervisor;
    use serde_json::json;
    use std::path::PathBuf;
    use std::process::Command;
    use tauri::Manager;

    fn fake_core() -> Command {
        let python = std::env::var("SKY_PYTHON").unwrap_or_else(|_| "python".into());
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("fake_core.py");
        let mut command = Command::new(python);
        command
            .arg("-u")
            .arg(fixture)
            .arg("tauri_commands")
            .env("PYTHONUNBUFFERED", "1");
        command
    }

    fn request(body: serde_json::Value, callback: u32) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: "search_songs".into(),
            callback: tauri::ipc::CallbackFn(callback),
            error: tauri::ipc::CallbackFn(callback + 1),
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .expect("test URL"),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    #[test]
    fn generated_tauri_handler_decodes_params_envelope() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![super::commands::search_songs])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let supervisor = CoreSupervisor::spawn_with_command(fake_core()).expect("fake Core");
        app.state::<AppState>()
            .inner()
            .install_ready_for_test(supervisor.clone());
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");

        let valid = tauri::test::get_ipc_response(
            &webview,
            request(
                json!({
                    "params": {
                        "query": "Aurora",
                        "offset": 0,
                        "limit": 200
                    }
                }),
                0,
            ),
        )
        .expect("params envelope should reach command");
        let value: serde_json::Value = valid.deserialize().expect("search response JSON");
        assert_eq!(value["total"], 0);

        let wrong = tauri::test::get_ipc_response(
            &webview,
            request(
                json!({
                    "request": {
                        "query": "Aurora",
                        "offset": 0,
                        "limit": 200
                    }
                }),
                2,
            ),
        );
        assert!(wrong.is_err(), "legacy request envelope must fail");
        supervisor.shutdown();
    }
}

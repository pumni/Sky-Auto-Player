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
    run_inner(false);
}

/// Run the production shell with a packaging-only WebView smoke hook.
///
/// The hook is selected only by the hidden release self-test argument. It
/// dispatches a DOM event after the real frontend has loaded; React then
/// exercises the production bridge and closes through the normal controlled
/// lifecycle. No test command or alternate runtime is exposed to the user.
pub fn run_gui_smoke() {
    run_inner(true);
}

fn run_inner(gui_smoke: bool) {
    if let Err(error) = core::check_startup_update_guard() {
        eprintln!("Sky Auto Player startup refused: {error}");
        if gui_smoke {
            // The packaging smoke is a process-level gate. A startup guard
            // rejection must be observable as a failing child, rather than
            // looking like a clean return before Tauri's event loop starts.
            std::process::exit(2);
        }
        return;
    }
    let app_state = app_state::AppState::default();
    app_state.set_gui_smoke_exit(gui_smoke);
    let mut builder = tauri::Builder::<ShellRuntime>::default()
        .manage(app_state)
        .setup(move |app| {
            if gui_smoke {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(45));
                    eprintln!("packaged GUI smoke watchdog expired");
                    app_handle.exit(1);
                });
            }
            Ok(())
        });
    if gui_smoke {
        builder = builder.on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = webview.eval(
                    "window.__SKY_PHASE8_GUI_SMOKE__ = true; window.dispatchEvent(new Event('sky-phase8-gui-smoke'));",
                );
            }
        });
    }
    builder
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::search_songs,
            commands::get_song_detail,
            commands::reload_library,
            commands::set_library_viewport,
            commands::get_settings,
            commands::patch_settings,
            commands::check_for_update,
            commands::get_update_preferences,
            commands::patch_update_preferences,
            commands::begin_update_handoff,
            commands::set_diagnostics_enabled,
            commands::start_calibration,
            commands::cancel_calibration,
            commands::prepare_playback,
            commands::start_playback,
            commands::stop_playback,
            commands::pause_playback,
            commands::resume_playback,
            commands::skip_playback,
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

/// Validate the release shell/Core pairing without constructing a WebView.
///
/// This hidden, packaging-only entrypoint is used by the exact portable
/// artifact gate. It still uses the production launch command and
/// ``CoreSupervisor``; it merely replaces the interactive window with a
/// bounded bootstrap/shutdown assertion so CI never needs to synthesize a
/// physical input session.
pub fn selftest_packaged_shell() -> i32 {
    if let Err(error) = core::check_startup_update_guard() {
        eprintln!("packaged shell selftest startup guard failed: {error}");
        return 2;
    }
    let supervisor = match core::CoreSupervisor::spawn() {
        Ok(supervisor) => supervisor,
        Err(error) => {
            eprintln!("packaged shell selftest could not start Core: {error}");
            return 2;
        }
    };
    let bootstrap = supervisor.request("app.bootstrap", serde_json::json!({}));
    let result = match bootstrap {
        Ok(value) if value.get("native_build").is_some() => {
            supervisor.shutdown();
            if let Some(marker) = std::env::var_os("SKY_PHASE8_RESTART_MARKER") {
                let _ = std::fs::write(marker, b"bootstrap-ready\n");
            }
            0
        }
        Ok(_) => {
            eprintln!("packaged shell selftest bootstrap omitted native_build");
            supervisor.shutdown();
            1
        }
        Err(error) => {
            eprintln!("packaged shell selftest bootstrap failed: {error}");
            supervisor.shutdown();
            1
        }
    };
    if result == 0 {
        println!("Packaged Tauri/Core selftest: PASS");
    }
    result
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

    fn request(
        command: &str,
        body: serde_json::Value,
        callback: u32,
    ) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: command.into(),
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
                "search_songs",
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
                "search_songs",
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

    #[test]
    fn generated_tauri_handler_decodes_playback_command_payloads() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::prepare_playback,
                super::commands::start_playback,
                super::commands::stop_playback,
                super::commands::pause_playback,
                super::commands::resume_playback,
                super::commands::skip_playback,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let supervisor = CoreSupervisor::spawn_with_command(fake_core()).expect("fake Core");
        app.state::<AppState>()
            .inner()
            .install_ready_for_test(supervisor.clone());
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        let song_id = "c".repeat(32);
        let prepared = tauri::test::get_ipc_response(
            &webview,
            request(
                "prepare_playback",
                json!({
                    "params": {
                        "songId": song_id,
                        "generation": 1,
                        "config": {
                            "hold_frames": 1.0,
                            "tempo_scale": 1.0,
                            "fps": 60,
                            "dry_run": true
                        }
                    }
                }),
                10,
            ),
        )
        .expect("playback params envelope should reach command");
        let prepared_value: serde_json::Value = prepared.deserialize().expect("prepared JSON");
        assert_eq!(prepared_value["prepared_id"], "a".repeat(32));

        let started = tauri::test::get_ipc_response(
            &webview,
            request(
                "start_playback",
                json!({
                    "params": {
                        "preparedId": "a".repeat(32),
                        "decisions": []
                    }
                }),
                12,
            ),
        )
        .expect("playback start params envelope should reach command");
        let started_value: serde_json::Value = started.deserialize().expect("session JSON");
        assert_eq!(started_value["session_id"], "b".repeat(32));

        for (callback, command) in [
            (16, "stop_playback"),
            (18, "pause_playback"),
            (20, "resume_playback"),
            (22, "skip_playback"),
        ] {
            tauri::test::get_ipc_response(
                &webview,
                request(
                    command,
                    json!({"params": {"sessionId": "b".repeat(32)}}),
                    callback,
                ),
            )
            .unwrap_or_else(|error| panic!("{command} params envelope should decode: {error}"));
        }

        let wrong = tauri::test::get_ipc_response(
            &webview,
            request(
                "prepare_playback",
                json!({
                    "request": {
                        "songId": song_id,
                        "generation": 1,
                        "config": {
                            "hold_frames": 1.0,
                            "tempo_scale": 1.0,
                            "fps": 60,
                            "dry_run": true
                        }
                    }
                }),
                14,
            ),
        );
        assert!(wrong.is_err(), "legacy request envelope must fail");
        supervisor.shutdown();
    }

    #[test]
    fn generated_tauri_handler_decodes_diagnostics_and_calibration_payloads() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::set_diagnostics_enabled,
                super::commands::start_calibration,
                super::commands::cancel_calibration,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let supervisor = CoreSupervisor::spawn_with_command(fake_core()).expect("fake Core");
        app.state::<AppState>()
            .inner()
            .install_ready_for_test(supervisor.clone());
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");

        let enabled = tauri::test::get_ipc_response(
            &webview,
            request(
                "set_diagnostics_enabled",
                json!({"params": {"enabled": true}}),
                24,
            ),
        )
        .expect("diagnostics params envelope should decode");
        let enabled_value: serde_json::Value = enabled.deserialize().expect("enabled JSON");
        assert_eq!(enabled_value["enabled"], true);

        let started = tauri::test::get_ipc_response(
            &webview,
            request(
                "start_calibration",
                json!({
                    "params": {
                        "mode": "quick",
                        "className": null,
                        "polyphony": null,
                        "samples": null,
                        "timeoutSeconds": null
                    }
                }),
                26,
            ),
        )
        .expect("calibration params envelope should decode");
        let started_value: serde_json::Value = started.deserialize().expect("start JSON");
        assert_eq!(started_value["operation_id"], "d".repeat(32));

        let cancelled = tauri::test::get_ipc_response(
            &webview,
            request(
                "cancel_calibration",
                json!({"params": {"operationId": "d".repeat(32)}}),
                28,
            ),
        )
        .expect("calibration cancel params envelope should decode");
        let cancelled_value: serde_json::Value = cancelled.deserialize().expect("cancel JSON");
        assert_eq!(cancelled_value["state"], "cancelled");

        let wrong = tauri::test::get_ipc_response(
            &webview,
            request(
                "set_diagnostics_enabled",
                json!({"request": {"enabled": true}}),
                32,
            ),
        );
        assert!(wrong.is_err(), "legacy request envelope must fail");
        supervisor.shutdown();
    }

    #[test]
    fn generated_tauri_handler_decodes_typed_update_payloads() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::check_for_update,
                super::commands::get_update_preferences,
                super::commands::patch_update_preferences,
                super::commands::begin_update_handoff,
                super::commands::shutdown,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let supervisor = CoreSupervisor::spawn_with_command(fake_core()).expect("fake Core");
        app.state::<AppState>()
            .inner()
            .install_ready_for_test(supervisor.clone());
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");

        let check =
            tauri::test::get_ipc_response(&webview, request("check_for_update", json!({}), 50))
                .expect("update check must decode");
        let check_value: serde_json::Value = check.deserialize().expect("update check JSON");
        assert_eq!(check_value["state"], "available");

        let preferences = tauri::test::get_ipc_response(
            &webview,
            request(
                "patch_update_preferences",
                json!({
                    "params": {"autoCheck": false, "channel": "beta", "skipVersion": "3.6.0"}
                }),
                52,
            ),
        )
        .expect("update preference patch must decode");
        let preferences_value: serde_json::Value =
            preferences.deserialize().expect("update preferences JSON");
        assert_eq!(preferences_value["channel"], "beta");

        let handoff = tauri::test::get_ipc_response(
            &webview,
            request(
                "begin_update_handoff",
                json!({
                    "params": {"targetVersion": "3.6.0"}
                }),
                54,
            ),
        )
        .expect("update handoff must decode");
        let handoff_value: serde_json::Value = handoff.deserialize().expect("handoff JSON");
        assert_eq!(handoff_value["state"], "handoff_ready");
        assert_eq!(handoff_value["target_version"], "3.6.0");

        let shutdown = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 55))
            .expect("successful handoff must use the controlled shell close command");
        let _: serde_json::Value = shutdown.deserialize().expect("shutdown JSON");
        assert!(app.state::<AppState>().inner().is_closing_for_test());

        let wrong = tauri::test::get_ipc_response(
            &webview,
            request(
                "begin_update_handoff",
                json!({
                    "params": {"target_version": "3.6.0"}
                }),
                56,
            ),
        );
        assert!(wrong.is_err(), "snake_case frontend payload must fail");
        supervisor.shutdown();
    }

    #[test]
    fn generated_tauri_handler_runs_real_core_dry_run_lifecycle() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::bootstrap,
                super::commands::search_songs,
                super::commands::prepare_playback,
                super::commands::start_playback,
                super::commands::shutdown,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");

        let bootstrap =
            tauri::test::get_ipc_response(&webview, request("bootstrap", json!({}), 30))
                .expect("real Core bootstrap");
        let bootstrap_value: serde_json::Value = bootstrap.deserialize().expect("bootstrap JSON");
        let generation = bootstrap_value["catalog_generation"]
            .as_u64()
            .expect("catalog generation");

        let search = tauri::test::get_ipc_response(
            &webview,
            request(
                "search_songs",
                json!({
                    "params": {
                        "query": "blue",
                        "offset": 0,
                        "limit": 1,
                        "generation": generation
                    }
                }),
                34,
            ),
        )
        .expect("real Core search");
        let search_value: serde_json::Value = search.deserialize().expect("search JSON");
        let song_id = search_value["items"][0]["song_id"]
            .as_str()
            .expect("opaque song ID")
            .to_owned();

        let prepared = tauri::test::get_ipc_response(
            &webview,
            request(
                "prepare_playback",
                json!({
                    "params": {
                        "songId": song_id,
                        "generation": generation,
                        "config": {
                            "hold_frames": 1.0,
                            "tempo_scale": 1.0,
                            "fps": 60,
                            "dry_run": true
                        }
                    }
                }),
                38,
            ),
        )
        .expect("real Core dry-run prepare");
        let prepared_value: serde_json::Value = prepared.deserialize().expect("prepare JSON");
        let prepared_id = prepared_value["prepared_id"]
            .as_str()
            .expect("opaque prepared ID")
            .to_owned();
        let decisions = prepared_value["decisions"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["decision"].as_str())
            .map(|decision| json!([{"decision": decision, "accepted": true}]))
            .unwrap_or_else(|| json!([]));

        let started = tauri::test::get_ipc_response(
            &webview,
            request(
                "start_playback",
                json!({
                    "params": {
                        "preparedId": prepared_id,
                        "decisions": decisions
                    }
                }),
                42,
            ),
        )
        .expect("real Core dry-run start");
        let started_value: serde_json::Value = started.deserialize().expect("start JSON");
        assert_eq!(started_value["state"], "starting");

        let shutdown = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 46))
            .expect("real Core shutdown");
        let shutdown_value: serde_json::Value = shutdown.deserialize().expect("shutdown JSON");
        assert!(shutdown_value.is_null());
    }
}

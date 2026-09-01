mod app_state;
mod bindings;
mod command_ownership;
mod commands;
mod lifecycle;
mod native_runtime;
mod native_update;
mod startup_guard;
mod ui_events;

pub(crate) const DESKTOP_PROTOCOL_VERSION: u64 = 1;

#[cfg(test)]
mod core;

use lifecycle::close_window;
use native_runtime::TestSeams;

/// Write packaging-only GUI smoke phase markers when the release harness has
/// explicitly requested them. The trace is intentionally file-based because
/// the packaged child is launched with hidden windows and its stdout/stderr
/// are not a reliable startup diagnostic channel on Windows.
pub(crate) fn record_gui_smoke_phase(phase: &str) {
    let Some(path) = std::env::var_os("SKY_GUI_SMOKE_PHASE_LOG") else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    use std::io::Write;
    let _ = writeln!(file, "{timestamp} {phase}");
    let _ = file.flush();
}

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
    if gui_smoke {
        record_gui_smoke_phase("run_inner.enter");
        record_gui_smoke_phase("command_ownership.check.enter");
    }
    if !command_ownership::matrix_is_complete() {
        if gui_smoke {
            record_gui_smoke_phase("command_ownership.check.failed");
        }
        eprintln!("Sky Auto Player startup refused: incomplete command ownership matrix");
        return;
    }
    if gui_smoke {
        record_gui_smoke_phase("command_ownership.check.pass");
        record_gui_smoke_phase("startup_update_guard.check.enter");
    }
    if let Err(error) = startup_guard::check_startup_update_guard() {
        if gui_smoke {
            record_gui_smoke_phase("startup_update_guard.check.failed");
        }
        eprintln!("Sky Auto Player startup refused: {error}");
        if gui_smoke {
            // The packaging smoke is a process-level gate. A startup guard
            // rejection must be observable as a failing child, rather than
            // looking like a clean return before Tauri's event loop starts.
            std::process::exit(2);
        }
        return;
    }
    if gui_smoke {
        record_gui_smoke_phase("startup_update_guard.check.pass");
    }
    let app_state = if gui_smoke {
        app_state::AppState::with_test_seams(TestSeams::SafePackage)
    } else {
        app_state::AppState::default()
    };
    app_state.set_gui_smoke_exit(gui_smoke);
    let smoke_state = app_state.clone();
    let setup_state = smoke_state.clone();
    if gui_smoke {
        record_gui_smoke_phase("app_state.ready");
        record_gui_smoke_phase("tauri.builder.create");
    }
    let mut builder = tauri::Builder::<ShellRuntime>::default()
        .manage(app_state)
        .setup(move |app| {
            if gui_smoke {
                record_gui_smoke_phase("tauri.setup.enter");
                let app_handle = app.handle().clone();
                let watchdog_state = setup_state.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(45));
                    record_gui_smoke_phase("watchdog.expired");
                    watchdog_state.set_gui_smoke_failed();
                    record_gui_smoke_phase("watchdog.failure_recorded");
                    eprintln!("packaged GUI smoke watchdog expired");
                    app_handle.exit(1);
                    // Tauri's exit request can terminate the event loop while
                    // the native entrypoint would otherwise return success.
                    // The packaging harness must observe this watchdog as a
                    // process failure, never as a false green smoke.
                    std::process::exit(1);
                });
                record_gui_smoke_phase("watchdog.spawned");
                record_gui_smoke_phase("tauri.setup.complete");
            }
            Ok(())
        });
    if gui_smoke {
        builder = builder.on_page_load(|webview, payload| {
            match payload.event() {
                tauri::webview::PageLoadEvent::Started => {
                    record_gui_smoke_phase(&format!(
                        "webview.page_load.started {}",
                        payload.url()
                    ));
                }
                tauri::webview::PageLoadEvent::Finished => {
                    record_gui_smoke_phase(&format!(
                        "webview.page_load.finished {}",
                        payload.url()
                    ));
                    let result = webview.eval(
                        "(() => { window.__SKY_DESKTOP_GUI_SMOKE__ = true; const skySmoke = () => window.dispatchEvent(new Event('sky-desktop-gui-smoke')); skySmoke(); window.setTimeout(skySmoke, 100); window.setTimeout(skySmoke, 500); })();",
                    );
                    record_gui_smoke_phase(if result.is_ok() {
                        "webview.smoke_dispatched"
                    } else {
                        "webview.smoke_dispatch.failed"
                    });
                }
            }
        });
    }
    let result = builder
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
        .run(tauri::generate_context!());
    if gui_smoke {
        record_gui_smoke_phase("tauri.run.return");
    }
    if gui_smoke && smoke_state.gui_smoke_exit_code() != 0 {
        record_gui_smoke_phase("tauri.run.return.failed");
        // Tauri's graceful AppHandle::exit request can return the event loop
        // without propagating its code through this library entrypoint. Make
        // watchdog and frontend-failure paths fail closed at the process
        // boundary so the Python harness cannot accept a false green smoke.
        std::process::exit(smoke_state.gui_smoke_exit_code());
    }
    result.expect("error while running Sky Auto Player desktop shell");
}

/// Validate the release native desktop composition without constructing a WebView.
///
/// This hidden, packaging-only entrypoint is used by the exact portable
/// artifact gate. It uses the same native composition root as the production
/// shell and exercises safe, non-physical command seams before shutdown.
pub fn selftest_packaged_shell() -> i32 {
    if let Err(error) = startup_guard::check_startup_update_guard() {
        eprintln!("packaged shell selftest startup guard failed: {error}");
        return 2;
    }
    let runtime =
        match native_runtime::NativeDesktopRuntime::from_install_root_with_activity_and_seams(
            native_runtime::resolve_install_root().unwrap_or_else(|error| {
                eprintln!("packaged shell selftest install root failed: {error}");
                std::process::exit(2);
            }),
            app_state::ActivityCoordinator::default(),
            TestSeams::SafePackage,
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("packaged native selftest could not start runtime: {error}");
                return 2;
            }
        };
    if std::env::var_os("SKY_DESKTOP_RESTART_SELFTEST").is_some() {
        // The updater restart qualification must verify that the newly
        // installed application can bootstrap without mutating preserved
        // user state.  The ordinary packaged selftest below exercises all
        // mutating command paths; this restart seam is intentionally
        // read-only apart from the external marker.
        let result = match runtime.bootstrap() {
            Ok(bootstrap) if !bootstrap.native_build.native_build_commit.is_empty() => 0,
            Ok(_) => {
                eprintln!("packaged restart selftest omitted native build identity");
                1
            }
            Err(error) => {
                eprintln!("packaged restart selftest bootstrap failed: {error}");
                1
            }
        };
        runtime.shutdown();
        if result == 0 {
            if let Some(marker) = std::env::var_os("SKY_DESKTOP_RESTART_MARKER") {
                let _ = std::fs::write(marker, b"bootstrap-ready\n");
            }
            println!("Packaged restart bootstrap selftest: PASS");
        }
        return result;
    }
    let result = (|| -> Result<(), String> {
        let bootstrap = runtime.bootstrap()?;
        if bootstrap.native_build.native_build_commit.is_empty() {
            return Err("bootstrap omitted native build identity".into());
        }
        let settings: commands::SettingsDto =
            serde_json::from_value(runtime.dispatch("settings.get", serde_json::json!({}))?)
                .map_err(|error| format!("settings.get response: {error}"))?;
        let _patched: commands::SettingsDto = serde_json::from_value(runtime.dispatch(
            "settings.patch",
            serde_json::json!({"verboseHud": settings.verbose_hud}),
        )?)
        .map_err(|error| format!("settings.patch response: {error}"))?;
        let _update_preferences: commands::UpdatePreferencesDto = serde_json::from_value(
            runtime.dispatch("update.preferences.get", serde_json::json!({}))?,
        )
        .map_err(|error| format!("update.preferences.get response: {error}"))?;
        let _patched_update_preferences: commands::UpdatePreferencesDto =
            serde_json::from_value(runtime.dispatch(
                "update.preferences.patch",
                serde_json::json!({"autoCheck": settings.update_preferences.auto_check}),
            )?)
            .map_err(|error| format!("update.preferences.patch response: {error}"))?;
        let _update_check: commands::UpdateCheckDto =
            serde_json::from_value(runtime.dispatch("update.check", serde_json::json!({}))?)
                .map_err(|error| format!("update.check response: {error}"))?;
        let _diagnostics: commands::DiagnosticsEnabledDto =
            serde_json::from_value(runtime.dispatch(
                "diagnostics.set_enabled",
                serde_json::json!({"enabled": true}),
            )?)
            .map_err(|error| format!("diagnostics response: {error}"))?;
        let _diagnostics: commands::DiagnosticsEnabledDto =
            serde_json::from_value(runtime.dispatch(
                "diagnostics.set_enabled",
                serde_json::json!({"enabled": false}),
            )?)
            .map_err(|error| format!("diagnostics disable response: {error}"))?;
        let search: commands::CatalogSearchDto = serde_json::from_value(runtime.dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "",
                "offset": 0,
                "limit": 1,
                "generation": bootstrap.catalog_generation
            }),
        )?)
        .map_err(|error| format!("catalog.search response: {error}"))?;
        if let Some(row) = search.items.first() {
            let prepared: commands::PreparedPlaybackDto =
                serde_json::from_value(runtime.dispatch(
                    "playback.prepare",
                    serde_json::json!({
                        "songId": row.song_id,
                        "generation": search.generation,
                        "config": {
                            "hold_frames": settings.playback_defaults.hold_frames,
                            "tempo_scale": settings.playback_defaults.tempo_scale,
                            "fps": settings.playback_defaults.fps,
                            "dry_run": true
                        }
                    }),
                )?)
                .map_err(|error| format!("playback.prepare response: {error}"))?;
            if prepared.admission == commands::PlaybackAdmission::Ready {
                let prepared_id = prepared
                    .prepared_id
                    .ok_or("dry-run prepare omitted prepared_id")?;
                let _session: commands::PlaybackSessionDto =
                    serde_json::from_value(runtime.dispatch(
                        "playback.start",
                        serde_json::json!({"preparedId": prepared_id, "decisions": []}),
                    )?)
                    .map_err(|error| format!("playback.start response: {error}"))?;
            }
        }
        let calibration: commands::CalibrationStartAckDto = serde_json::from_value(
            runtime.dispatch("calibration.start", serde_json::json!({"mode": "quick"}))?,
        )
        .map_err(|error| format!("calibration.start response: {error}"))?;
        let state = runtime.wait_for_calibration_terminal(std::time::Duration::from_secs(5))?;
        if !matches!(
            state,
            ui_events::CalibrationState::Succeeded | ui_events::CalibrationState::Cancelled
        ) {
            return Err(format!("safe calibration ended in {state:?}"));
        }
        if calibration.operation_id.is_empty() {
            return Err("calibration.start omitted operation_id".into());
        }
        Ok(())
    })();
    runtime.shutdown();
    let result = match result {
        Ok(()) => {
            if let Some(marker) = std::env::var_os("SKY_DESKTOP_RESTART_MARKER") {
                let _ = std::fs::write(marker, b"bootstrap-ready\n");
            }
            0
        }
        Err(error) => {
            eprintln!("packaged native selftest failed: {error}");
            1
        }
    };
    if result == 0 {
        println!("Packaged Tauri/Native Desktop selftest: PASS");
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
        assert_eq!(value["offset"], 0);
        assert_eq!(value["limit"], 200);
        assert!(value["generation"].as_u64().is_some_and(|value| value > 0));

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
    }

    #[test]
    fn generated_tauri_handler_decodes_playback_command_payloads() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::bootstrap,
                super::commands::search_songs,
                super::commands::prepare_playback,
                super::commands::start_playback,
                super::commands::stop_playback,
                super::commands::shutdown,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        let bootstrap = tauri::test::get_ipc_response(&webview, request("bootstrap", json!({}), 1))
            .expect("native bootstrap should succeed");
        let bootstrap_value: serde_json::Value = bootstrap.deserialize().expect("bootstrap JSON");
        let generation = bootstrap_value["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = tauri::test::get_ipc_response(
            &webview,
            request(
                "search_songs",
                json!({"params":{"query":"blue","offset":0,"limit":1,"generation":generation}}),
                2,
            ),
        )
        .expect("native catalog search should succeed");
        let search_value: serde_json::Value = search.deserialize().expect("search JSON");
        let song_id = search_value["items"][0]["song_id"]
            .as_str()
            .expect("song ID");
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
        assert!(prepared_value["prepared_id"].as_str().is_some());
        let prepared_id = prepared_value["prepared_id"].as_str().unwrap_or_default();

        let started = tauri::test::get_ipc_response(
            &webview,
            request(
                "start_playback",
                json!({
                    "params": {
                        "preparedId": prepared_id,
                        "decisions": []
                    }
                }),
                12,
            ),
        )
        .expect("playback start params envelope should reach command");
        let started_value: serde_json::Value = started.deserialize().expect("session JSON");
        assert_eq!(started_value["state"], "starting");
        let session_id = started_value["session_id"].as_str().expect("session ID");

        tauri::test::get_ipc_response(
            &webview,
            request(
                "stop_playback",
                json!({"params":{"sessionId":session_id}}),
                16,
            ),
        )
        .expect("native stop params envelope should decode");

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
        let _ = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 18));
    }

    #[test]
    #[ignore = "legacy Core-owned IPC oracle; Native calibration route is covered by runtime tests"]
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
    #[ignore = "network/update-handoff fixture belongs to the retired Core route"]
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

    #[test]
    fn routed_python_settings_patch_invalidates_native_prepared_plan() {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .invoke_handler(tauri::generate_handler![
                super::commands::bootstrap,
                super::commands::search_songs,
                super::commands::prepare_playback,
                super::commands::patch_settings,
                super::commands::start_playback,
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

        let bootstrap =
            tauri::test::get_ipc_response(&webview, request("bootstrap", json!({}), 60))
                .expect("native bootstrap");
        let bootstrap: serde_json::Value = bootstrap.deserialize().expect("bootstrap JSON");
        let generation = bootstrap["catalog_generation"]
            .as_u64()
            .expect("generation");
        let search = tauri::test::get_ipc_response(
            &webview,
            request(
                "search_songs",
                json!({"params":{"query":"blue","offset":0,"limit":1,"generation":generation}}),
                62,
            ),
        )
        .expect("native search");
        let search: serde_json::Value = search.deserialize().expect("search JSON");
        let song_id = search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned();
        let prepared = tauri::test::get_ipc_response(
            &webview,
            request(
                "prepare_playback",
                json!({"params":{
                    "songId":song_id,
                    "generation":generation,
                    "config":{"hold_frames":1.0,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }}),
                64,
            ),
        )
        .expect("native prepare");
        let prepared: serde_json::Value = prepared.deserialize().expect("prepare JSON");
        let prepared_id = prepared["prepared_id"].as_str().expect("prepared ID");

        let patched = tauri::test::get_ipc_response(
            &webview,
            request(
                "patch_settings",
                json!({"params":{"playbackDefaults":{"tempo_scale":0.95}}}),
                66,
            ),
        )
        .expect("Python-owned settings patch");
        let _: serde_json::Value = patched.deserialize().expect("settings JSON");

        let start = tauri::test::get_ipc_response(
            &webview,
            request(
                "start_playback",
                json!({"params":{"preparedId":prepared_id,"decisions":[]}}),
                68,
            ),
        );
        assert!(
            start.is_err(),
            "settings patch must stale native prepared ID"
        );
        let _ = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 70));
        supervisor.shutdown();
    }
}

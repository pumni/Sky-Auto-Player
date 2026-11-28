mod app_state;
mod bindings;
mod command_ownership;
mod commands;
mod ipc_contract;
mod lifecycle;
mod native_runtime;
mod native_update;
mod power_lifecycle;
#[cfg(windows)]
mod single_instance;
mod startup_telemetry;
mod startup_window;
mod ui_events;
#[cfg(windows)]
mod windows_caption;
#[cfg(windows)]
mod windows_icon;

pub(crate) const DESKTOP_PROTOCOL_VERSION: u64 = 1;

pub fn startup_process_entry() {
    startup_telemetry::initialize();
    startup_telemetry::record("process.entry");
}

use lifecycle::close_window;
use std::path::PathBuf;
use std::sync::OnceLock;

static UPDATE_SAFETY_LOG: OnceLock<PathBuf> = OnceLock::new();

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

/// Write packaging-only updater shutdown evidence when the release harness
/// has explicitly requested it. This never exposes update internals to the
/// frontend and is inert for normal launches.
pub(crate) fn record_update_safety_phase(phase: &str) {
    let path = UPDATE_SAFETY_LOG
        .get()
        .cloned()
        .or_else(|| std::env::var_os("SKY_UPDATE_SAFETY_PHASE_LOG").map(PathBuf::from));
    let Some(path) = path else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    use std::io::Write;
    let _ = writeln!(file, "{phase}");
    let _ = file.flush();
}

#[cfg(feature = "desktop-runtime")]
pub(crate) type ShellRuntime = tauri::Wry;
#[cfg(all(not(feature = "desktop-runtime"), feature = "tauri-test"))]
pub(crate) type ShellRuntime = tauri::test::MockRuntime;
#[cfg(all(not(feature = "desktop-runtime"), not(feature = "tauri-test")))]
compile_error!(
    "sky_desktop_shell requires either `desktop-runtime` for the real Tauri shell or `tauri-test` for MockRuntime"
);
#[cfg(all(feature = "packaged-assets", not(feature = "desktop-runtime")))]
compile_error!("`packaged-assets` requires `desktop-runtime`");

fn main_window_state_flags() -> tauri_plugin_window_state::StateFlags {
    tauri_plugin_window_state::StateFlags::SIZE
        | tauri_plugin_window_state::StateFlags::POSITION
        | tauri_plugin_window_state::StateFlags::MAXIMIZED
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    exit_on_failure(run_inner(false, false));
}

/// Run the production shell with a packaging-only WebView smoke hook.
///
/// The hook is selected only by the hidden release self-test argument. It
/// dispatches a DOM event after the real frontend has loaded; React then
/// exercises the production bridge and closes through the normal controlled
/// lifecycle. No test command or alternate runtime is exposed to the user.
pub fn run_gui_smoke() {
    exit_on_failure(run_inner(true, false));
}

/// Run the real Tauri shell with an updater fixture-driven previous-v4 to
/// candidate-v4 workflow. The entry point is hidden from the product UI and
/// exists only for deterministic packaged qualification.
pub fn run_update_smoke() {
    let update_marker = update_smoke_marker("--selftest-update-marker");
    let expected_version_file = update_smoke_marker("--selftest-update-expected-version-file");
    if let (Some(marker), Some(expected_path)) = (update_marker.as_ref(), expected_version_file)
        && let Ok(expected_version) = std::fs::read_to_string(expected_path)
    {
        let expected_version = expected_version.trim();
        if expected_version == env!("CARGO_PKG_VERSION") {
            let _ = std::fs::write(
                marker,
                format!("update-complete:{}\n", env!("CARGO_PKG_VERSION")),
            );
            return;
        }
    }
    exit_on_failure(run_inner(false, true));
}

mod selftest;
use selftest::{packaged_update_install_ack_result, update_smoke_marker};
pub use selftest::{
    selftest_build_info, selftest_packaged_shell, selftest_update_install_rejection_during_playback,
};

fn exit_on_failure(exit_code: i32) {
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

fn run_inner(gui_smoke: bool, update_smoke: bool) -> i32 {
    #[cfg(windows)]
    let _single_instance_guard = match single_instance::SingleInstanceGuard::acquire() {
        Ok(guard) => guard,
        Err(single_instance::AcquireError::AlreadyRunning) => {
            match single_instance::existing_instance_policy(cfg!(debug_assertions)) {
                single_instance::ExistingInstancePolicy::FocusExisting => {
                    single_instance::focus_existing_instance();
                    eprintln!("Sky Auto Player is already running");
                    return 0;
                }
                single_instance::ExistingInstancePolicy::Refuse => {
                    let details = single_instance::existing_instance_details().unwrap_or_default();
                    eprintln!(
                        "Sky Auto Player startup refused: another Sky Auto Player process is active{details}; the newly compiled dev build was not launched"
                    );
                    return 1;
                }
            }
        }
        Err(error) => {
            eprintln!("Sky Auto Player startup refused: {error}");
            return 1;
        }
    };

    let update_marker = update_smoke_marker("--selftest-update-marker");
    let update_safety_marker = update_smoke_marker("--selftest-update-safety-marker");
    let expected_version_file = update_smoke_marker("--selftest-update-expected-version-file");
    if let Some(path) = update_safety_marker {
        let _ = UPDATE_SAFETY_LOG.set(path);
    }
    if gui_smoke {
        record_gui_smoke_phase("run_inner.enter");
        record_gui_smoke_phase("command_ownership.check.enter");
    }
    if !command_ownership::matrix_is_complete() {
        if gui_smoke {
            record_gui_smoke_phase("command_ownership.check.failed");
        }
        eprintln!("Sky Auto Player startup refused: incomplete command ownership matrix");
        return 1;
    }
    if gui_smoke {
        record_gui_smoke_phase("command_ownership.check.pass");
    }
    let app_state = app_state::AppState::default();
    app_state.set_gui_smoke_exit(gui_smoke);
    let smoke_state = app_state.clone();
    let setup_state = smoke_state.clone();
    if gui_smoke {
        record_gui_smoke_phase("app_state.ready");
        record_gui_smoke_phase("tauri.builder.create");
    }
    startup_telemetry::record("tauri.builder.start");
    let mut builder = tauri::Builder::<ShellRuntime>::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(main_window_state_flags())
                .build(),
        )
        .manage(app_state)
        .manage(startup_window::StartupWindowFailsafe::default())
        .setup(move |app| {
            use tauri::{Manager, path::BaseDirectory};

            startup_telemetry::record("tauri.setup.start");
            startup_window::schedule_failsafe(
                app.handle().clone(),
                app.state::<startup_window::StartupWindowFailsafe>()
                    .inner()
                    .clone(),
            );

            match app
                .path()
                .resolve("builtin-songs", BaseDirectory::Resource)
            {
                Ok(root) => setup_state.configure_resources(
                    sky_native_adapters::AppResources::from_builtin_catalog_root(root),
                )?,
                Err(error) => eprintln!(
                    "built-in catalog resource resolution unavailable; continuing with user sources: {error}"
                ),
            }
            setup_state.configure_update_service(app.handle().clone())?;
            if update_smoke {
                let app_handle = app.handle().clone();
                let update_state = setup_state.clone();
                let marker = update_marker.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let result = (|| -> Result<bool, String> {
                        let native = update_state.ensure_native_blocking()?;
                        let _check: commands::UpdateCheckAckDto = serde_json::from_value(
                            native.dispatch("update.check", serde_json::json!({"origin": "manual"}))?,
                        )
                        .map_err(|error| format!("packaged update check response: {error}"))?;
                        let snapshot = native.update_snapshot()?;
                        if let Some(target) = snapshot.available_version {
                            if let Some(expected_path) = expected_version_file.as_ref() {
                                std::fs::write(expected_path, format!("{target}\n")).map_err(
                                    |error| {
                                        format!(
                                            "packaged update expected-version handoff failed: {error}"
                                        )
                                    },
                                )?;
                            }
                            let install_ack: commands::UpdateInstallAckDto =
                                serde_json::from_value(native.dispatch(
                                    "update.begin_handoff",
                                    serde_json::json!({"targetVersion": target}),
                                )?)
                                .map_err(|error| {
                                    format!("packaged update install response: {error}")
                                })?;

                            if install_ack.accepted {
                                packaged_update_install_ack_result(install_ack, None)
                            } else {
                                let rejected_snapshot = native.update_snapshot()?;
                                packaged_update_install_ack_result(
                                    install_ack,
                                    Some(&rejected_snapshot),
                                )
                            }
                        } else if snapshot.state == ui_events::UpdateState::Current {
                            if let Some(marker) = marker.as_ref() {
                                let _ = std::fs::write(
                                    marker,
                                    format!("update-complete:{}\n", env!("CARGO_PKG_VERSION")),
                                );
                            }
                            native.shutdown();
                            Ok(true)
                        } else {
                            Err(snapshot
                                .error_detail
                                .unwrap_or_else(|| "packaged update check was inconclusive".into()))
                        }
                    })();
                    match result {
                        Ok(true) => app_handle.exit(0),
                        Ok(false) => {}
                        Err(error) => {
                            eprintln!("packaged update smoke failed: {error}");
                            if let Some(marker) = marker.as_ref() {
                                let _ = std::fs::write(marker, format!("update-failed:{error}\n"));
                            }
                            app_handle.exit(1);
                        }
                    }
                });
            }
            #[cfg(windows)]
            {
                use tauri::Manager;

                if let Some(window) = app.get_webview_window("main")
                    && let Err(error) = windows_icon::apply_native_window_icons(&window)
                {
                    eprintln!("failed to apply native Windows icons: {error}");
                }
                if let Some(window) = app.get_webview_window("main")
                    && let Err(error) = windows_caption::install(&window)
                {
                    eprintln!("failed to install native maximize hit target: {error}");
                }
            }
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
            startup_telemetry::record("tauri.setup.end");
            Ok(())
        });
    if startup_telemetry::enabled() || gui_smoke {
        let telemetry_enabled = startup_telemetry::enabled();
        builder = builder.on_page_load(move |webview, payload| match payload.event() {
            tauri::webview::PageLoadEvent::Started => {
                if telemetry_enabled {
                    let _ = webview.eval("window.__SKY_STARTUP_TELEMETRY_ENABLED__ = true;");
                }
                if gui_smoke {
                    record_gui_smoke_phase(&format!(
                        "webview.page_load.started {}",
                        payload.url()
                    ));
                }
            }
            tauri::webview::PageLoadEvent::Finished => {
                if gui_smoke {
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
            commands::startup_window_revealed,
            commands::search_songs,
            commands::get_song_detail,
            commands::reload_library,
            commands::set_library_viewport,
            commands::set_song_liked,
            commands::library_list_playlists,
            commands::library_create_playlist,
            commands::library_rename_playlist,
            commands::library_delete_playlist,
            commands::library_add_songs,
            commands::library_remove_songs,
            commands::library_import_local_files_to_playlist,
            commands::library_import_local_folder_to_playlist,
            commands::get_settings,
            commands::patch_settings,
            commands::check_for_update,
            commands::get_update_preferences,
            commands::patch_update_preferences,
            commands::begin_update_handoff,
            commands::set_diagnostics_enabled,
            commands::prepare_playback,
            commands::start_playback,
            commands::get_playback_status,
            commands::stop_playback,
            commands::pause_playback,
            commands::resume_playback,
            commands::skip_playback,
            commands::export_sender_trace,
            commands::subscribe_ui_events,
            commands::shutdown,
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                close_window(window.clone());
            }
            #[cfg(windows)]
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                if let Err(error) = windows_icon::apply_native_window_icons(window) {
                    eprintln!("failed to refresh native Windows icons: {error}");
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!());
    if gui_smoke {
        record_gui_smoke_phase("tauri.run.return");
    }
    if gui_smoke && smoke_state.gui_smoke_exit_code() != 0 {
        record_gui_smoke_phase("tauri.run.return.failed");
        return smoke_state.gui_smoke_exit_code();
    }
    result.expect("error while running Sky Auto Player desktop shell");
    0
}

// The mock runtime is intentionally exercised in a no-default-features test
// build. Keeping the production Wry runtime out of that test binary avoids
// loading the native desktop webview during command-decoder unit tests.
#[cfg(all(test, feature = "tauri-test", not(feature = "desktop-runtime")))]
#[path = "lib_tests/ipc_tests.rs"]
mod ipc_tests;

#[cfg(test)]
#[path = "lib_tests/update_smoke_tests.rs"]
mod update_smoke_tests;

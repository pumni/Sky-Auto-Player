use super::app_state::AppState;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn startup_window_is_hidden_and_retains_geometry_restore_flags() {
    let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .expect("Tauri configuration must remain valid JSON");
    let windows = config["app"]["windows"]
        .as_array()
        .expect("Tauri app windows must remain an array");
    assert_eq!(windows.len(), 1, "startup must retain one native window");
    let main = &windows[0];
    assert_eq!(main["label"], "main");
    assert_eq!(main["visible"], false);
    assert_eq!(main["backgroundColor"], json!([18, 19, 20, 255]));

    let flags = super::main_window_state_flags();
    assert!(flags.contains(tauri_plugin_window_state::StateFlags::SIZE));
    assert!(flags.contains(tauri_plugin_window_state::StateFlags::POSITION));
    assert!(flags.contains(tauri_plugin_window_state::StateFlags::MAXIMIZED));
    assert!(!flags.contains(tauri_plugin_window_state::StateFlags::VISIBLE));

    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../capabilities/main.json"))
            .expect("main capability must remain valid JSON");
    let permissions = capability["permissions"]
        .as_array()
        .expect("main capability permissions must remain an array");
    assert_eq!(
        permissions
            .iter()
            .filter(|permission| permission.as_str() == Some("core:window:allow-show"))
            .count(),
        1,
        "startup reveal must use only the narrow window show permission"
    );
}

struct TestInstallRoot(PathBuf);

impl Drop for TestInstallRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_install_root() -> (sky_native_adapters::AppPaths, TestInstallRoot) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-desktop-ipc-{suffix}"));
    let paths = sky_native_adapters::AppPaths::from_test_sandbox(&root);
    paths.ensure_mutable_directories().expect("test dirs");
    fs::write(paths.settings_path(), "{\"schema_version\":3}\n").expect("test config");
    let source_song =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..\\..\\..\\songs\\blue.json");
    fs::copy(source_song, paths.user_music_root().join("blue.json")).expect("test song");
    (paths, TestInstallRoot(root))
}

fn request(command: &str, body: serde_json::Value, callback: u32) -> tauri::webview::InvokeRequest {
    request_from_url(
        command,
        body,
        callback,
        if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        },
    )
}

fn request_from_url(
    command: &str,
    body: serde_json::Value,
    callback: u32,
    url: &str,
) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: command.into(),
        callback: tauri::ipc::CallbackFn(callback),
        error: tauri::ipc::CallbackFn(callback + 1),
        url: url.parse().expect("test URL"),
        body: tauri::ipc::InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    }
}

#[test]
fn bootstrap_reports_native_gui_smoke_authorization() {
    let (paths, _cleanup) = test_install_root();
    let state = AppState::with_test_paths(paths);
    let app = tauri::test::mock_builder()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![super::commands::bootstrap])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock Tauri app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");

    let ordinary = tauri::test::get_ipc_response(&webview, request("bootstrap", json!({}), 0))
        .expect("ordinary bootstrap should succeed");
    let ordinary: serde_json::Value = ordinary.deserialize().expect("bootstrap JSON");
    assert_eq!(ordinary["gui_smoke"], false);

    let mut legacy = ordinary.clone();
    legacy
        .as_object_mut()
        .expect("bootstrap response object")
        .remove("gui_smoke");
    let legacy: super::commands::BootstrapDto =
        serde_json::from_value(legacy).expect("legacy bootstrap without gui_smoke");
    assert!(!legacy.gui_smoke);

    state.set_gui_smoke_exit(true);
    let authorized = tauri::test::get_ipc_response(&webview, request("bootstrap", json!({}), 2))
        .expect("authorized bootstrap should succeed");
    let authorized: serde_json::Value = authorized.deserialize().expect("bootstrap JSON");
    assert_eq!(authorized["gui_smoke"], true);
}

#[test]
fn generated_tauri_handler_decodes_params_envelope() {
    let (paths, _cleanup) = test_install_root();
    let app = tauri::test::mock_builder()
        .manage(AppState::with_test_paths(paths))
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
            json!({"params":{"query":"Aurora","offset":0,"limit":200}}),
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
            json!({"request":{"query":"Aurora","offset":0,"limit":200}}),
            2,
        ),
    );
    assert!(wrong.is_err(), "legacy request envelope must fail");
}

#[cfg(windows)]
#[test]
fn generated_capability_allows_local_main_and_rejects_unlisted_or_remote_commands() {
    let (paths, _cleanup) = test_install_root();
    let app = tauri::test::mock_builder()
        .manage(AppState::with_test_paths(paths))
        .invoke_handler(tauri::generate_handler![super::commands::search_songs])
        .build(tauri::generate_context!())
        .expect("mock Tauri app with generated capability manifest");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");

    let local = tauri::test::get_ipc_response(
        &webview,
        request(
            "search_songs",
            json!({"params":{"query":"Aurora","offset":0,"limit":200}}),
            30,
        ),
    );
    assert!(local.is_ok(), "local main command should be authorized");

    let unlisted =
        tauri::test::get_ipc_response(&webview, request("not_registered", json!({}), 32));
    assert!(unlisted.is_err(), "unregistered commands must fail closed");

    let remote = tauri::test::get_ipc_response(
        &webview,
        request_from_url(
            "search_songs",
            json!({"params":{"query":"Aurora","offset":0,"limit":200}}),
            34,
            "https://evil.example/",
        ),
    );
    assert!(
        remote.is_err(),
        "remote origins must not reach custom commands"
    );
}

#[test]
fn generated_tauri_handler_decodes_native_playback_payloads() {
    let (paths, _cleanup) = test_install_root();
    let app = tauri::test::mock_builder()
        .manage(AppState::with_test_paths(paths))
        .invoke_handler(tauri::generate_handler![
            super::commands::bootstrap,
            super::commands::search_songs,
            super::commands::reload_library,
            super::commands::prepare_playback,
            super::commands::start_playback,
            super::commands::get_playback_status,
            super::commands::stop_playback,
            super::commands::shutdown,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock Tauri app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");
    let reload = tauri::test::get_ipc_response(&webview, request("reload_library", json!({}), 3))
        .expect("native catalog reload should succeed");
    let reload: serde_json::Value = reload.deserialize().expect("reload JSON");
    let generation = reload["generation"].as_u64().expect("generation");
    let idle_status =
        tauri::test::get_ipc_response(&webview, request("get_playback_status", json!({}), 6))
            .expect("playback status command should succeed before start");
    let idle_status: serde_json::Value = idle_status.deserialize().expect("status JSON");
    assert!(idle_status["active"].is_null());
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
                json!({"params":{"songId":song_id,"generation":generation,"config":{"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}}}),
                10,
            ),
        )
        .expect("playback params envelope should reach command");
    let prepared_value: serde_json::Value = prepared.deserialize().expect("prepared JSON");
    let prepared_id = prepared_value["prepared_id"].as_str().expect("prepared ID");
    let started = tauri::test::get_ipc_response(
        &webview,
        request(
            "start_playback",
            json!({"params":{"preparedId":prepared_id,"decisions":[]}}),
            12,
        ),
    )
    .expect("playback start params envelope should decode");
    let started_value: serde_json::Value = started.deserialize().expect("session JSON");
    assert_eq!(started_value["state"], "starting");
    let session_id = started_value["session_id"].as_str().expect("session ID");
    let active_status =
        tauri::test::get_ipc_response(&webview, request("get_playback_status", json!({}), 14))
            .expect("playback status command should report the active session");
    let active_status: serde_json::Value = active_status.deserialize().expect("status JSON");
    assert_eq!(active_status["active"]["session_id"], session_id);
    tauri::test::get_ipc_response(
        &webview,
        request(
            "stop_playback",
            json!({"params":{"sessionId":session_id}}),
            16,
        ),
    )
    .expect("native stop params should decode");
    let wrong = tauri::test::get_ipc_response(
        &webview,
        request(
            "prepare_playback",
            json!({"request":{"songId":song_id,"generation":generation,"config":{"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}}}),
            14,
        ),
    );
    assert!(wrong.is_err(), "legacy request envelope must fail");
    let _ = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 18));
}

#[test]
fn native_settings_patch_invalidates_prepared_plan() {
    let (paths, _cleanup) = test_install_root();
    let app = tauri::test::mock_builder()
        .manage(AppState::with_test_paths(paths))
        .invoke_handler(tauri::generate_handler![
            super::commands::bootstrap,
            super::commands::search_songs,
            super::commands::reload_library,
            super::commands::prepare_playback,
            super::commands::patch_settings,
            super::commands::start_playback,
            super::commands::shutdown,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock Tauri app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");
    let reload = tauri::test::get_ipc_response(&webview, request("reload_library", json!({}), 61))
        .expect("native catalog reload");
    let reload: serde_json::Value = reload.deserialize().expect("reload JSON");
    let generation = reload["generation"].as_u64().expect("generation");
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
    let song_id = search["items"][0]["song_id"].as_str().expect("song ID");
    let prepared = tauri::test::get_ipc_response(
            &webview,
            request(
                "prepare_playback",
                json!({"params":{"songId":song_id,"generation":generation,"config":{"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}}}),
                64,
            ),
        )
        .expect("native prepare");
    let prepared: serde_json::Value = prepared.deserialize().expect("prepare JSON");
    let prepared_id = prepared["prepared_id"].as_str().expect("prepared ID");
    tauri::test::get_ipc_response(
        &webview,
        request(
            "patch_settings",
            json!({"params":{"playbackDefaults":{"tempo_scale":0.95}}}),
            66,
        ),
    )
    .expect("native settings patch");
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
}

#[test]
fn native_settings_patch_auto_check_only_does_not_require_update_service() {
    let (paths, _cleanup) = test_install_root();
    let app = tauri::test::mock_builder()
        .manage(AppState::with_test_paths(paths))
        .invoke_handler(tauri::generate_handler![
            super::commands::patch_settings,
            super::commands::patch_update_preferences,
            super::commands::get_update_preferences,
            super::commands::shutdown,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock Tauri app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");

    // Patch auto_check only via patch_settings
    let res = tauri::test::get_ipc_response(
        &webview,
        request(
            "patch_settings",
            json!({"params":{"updatePreferences":{"autoCheck":false}}}),
            80,
        ),
    )
    .expect("patch settings auto_check");
    let res: serde_json::Value = res.deserialize().expect("deserialize");
    assert_eq!(res["update_preferences"]["auto_check"], false);

    // Patch auto_check only via patch_update_preferences
    let res2 = tauri::test::get_ipc_response(
        &webview,
        request(
            "patch_update_preferences",
            json!({"params":{"autoCheck":true}}),
            82,
        ),
    )
    .expect("patch update_preferences auto_check");
    let res2: serde_json::Value = res2.deserialize().expect("deserialize");
    assert_eq!(res2["auto_check"], true);

    // Patch skip_version via patch_settings
    let res3 = tauri::test::get_ipc_response(
        &webview,
        request(
            "patch_settings",
            json!({"params":{"updatePreferences":{"skipVersion":"4.1.0"}}}),
            84,
        ),
    )
    .expect("patch settings skip_version");
    let res3: serde_json::Value = res3.deserialize().expect("deserialize");
    assert_eq!(res3["update_preferences"]["skip_version"], "4.1.0");

    let _ = tauri::test::get_ipc_response(&webview, request("shutdown", json!({}), 86));
}

#[test]
fn candidate_invalidation_matrix_for_settings_and_update_preferences_patches() {
    use crate::native_update::NativeUpdateCandidate;
    use crate::ui_events::{UpdateChannel, UpdateState};

    let (paths, _cleanup) = test_install_root();
    let state = AppState::with_test_paths(paths);
    let app = tauri::test::mock_builder()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            super::commands::patch_settings,
            super::commands::patch_update_preferences,
            super::commands::shutdown,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock Tauri app");

    state
        .configure_update_service(app.handle().clone())
        .expect("configure update service");
    let update_service = state
        .update_service()
        .expect("lock")
        .expect("update service");
    let runtime = state.ensure_native_blocking().expect("runtime");

    let set_available = |version: &str| {
        let mut s = update_service.state.lock().unwrap();
        s.channel = UpdateChannel::Stable;
        s.candidate = Some(NativeUpdateCandidate {
            version: version.to_string(),
            channel: UpdateChannel::Stable,
            release_notes: Some("Release notes".to_string()),
            published_at: None,
        });
        s.state = UpdateState::Available;
    };

    // --- 1. Generic patch_settings path ---

    // 1a. Available + auto_check-only mutation => candidate/snapshot remains Available
    set_available("4.2.0");
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: Some(false),
                channel: None,
                skip_version: None,
            }),
        })
        .expect("patch auto_check false");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.2.0"));

    // 1b. Available + Stable->Stable no-op => candidate remains Available
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: Some(UpdateChannel::Stable),
                skip_version: None,
            }),
        })
        .expect("patch stable->stable");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.2.0"));

    // 1c. Available + skip X->X no-op => candidate remains Available
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: None,
                skip_version: Some("4.1.0".to_string()),
            }),
        })
        .expect("set skip_version");
    set_available("4.2.0");
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: None,
                skip_version: Some("4.1.0".to_string()),
            }),
        })
        .expect("patch skip 4.1.0->4.1.0");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.2.0"));

    // 1d. Available + actual skip change => Idle and candidate cleared
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: None,
                skip_version: Some("4.2.0".to_string()),
            }),
        })
        .expect("patch skip 4.1.0->4.2.0");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Idle);
    assert!(snap.available_version.is_none());

    // --- 2. Dedicated patch_update_preferences path ---

    // 2a. Available + auto_check-only mutation => candidate/snapshot remains Available
    set_available("4.3.0");
    runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: Some(true),
            channel: None,
            skip_version: None,
        })
        .expect("patch auto_check true");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.3.0"));

    // 2b. Available + Stable->Stable no-op => candidate remains Available
    runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: None,
            channel: Some(UpdateChannel::Stable),
            skip_version: None,
        })
        .expect("patch stable->stable");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.3.0"));

    // 2c. Available + skip X->X no-op => candidate remains Available
    runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: None,
            channel: None,
            skip_version: Some("4.2.0".to_string()),
        })
        .expect("patch skip 4.2.0->4.2.0");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Available);
    assert_eq!(snap.available_version.as_deref(), Some("4.3.0"));

    // 2d. Available + actual skip change => Idle and candidate cleared
    runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: None,
            channel: None,
            skip_version: Some("4.3.0".to_string()),
        })
        .expect("patch skip 4.2.0->4.3.0");
    let snap = update_service.current_snapshot();
    assert_eq!(snap.state, UpdateState::Idle);
    assert!(snap.available_version.is_none());
}

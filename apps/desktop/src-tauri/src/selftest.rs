use crate::{app_state, commands, native_runtime, ui_events};
use std::path::PathBuf;

pub(super) fn packaged_update_install_ack_result(
    ack: commands::UpdateInstallAckDto,
    snapshot: Option<&ui_events::UpdateSnapshotPayload>,
) -> Result<bool, String> {
    if ack.accepted {
        // Accepted means the official updater transaction owns the process
        // lifecycle now. The current process is expected to exit/restart.
        return Ok(false);
    }

    let snapshot = snapshot.ok_or_else(|| {
        "packaged update install was rejected without an authoritative native snapshot".to_string()
    })?;

    if snapshot.state != ui_events::UpdateState::Error {
        return Err(format!(
            "packaged update install was rejected without Error state: {:?}",
            snapshot.state
        ));
    }

    let code = snapshot.error_code.ok_or_else(|| {
        "packaged update install was rejected without a typed native error code".to_string()
    })?;

    let detail = snapshot
        .error_detail
        .as_deref()
        .unwrap_or("no diagnostic detail");

    Err(format!(
        "packaged update install rejected ({code:?}): {detail}"
    ))
}
/// Prove the packaged update admission boundary remains fail-closed while
/// physical playback owns the activity gate. This is a hidden qualification
/// seam only; it does not add an updater protocol or install path.
pub fn selftest_update_install_rejection_during_playback() -> i32 {
    let activity = app_state::ActivityCoordinator::default();
    let _playback = match activity.reserve_playback("packaged-qualification") {
        Ok(lease) => lease,
        Err(error) => {
            eprintln!("packaged playback admission self-test could not start: {error}");
            return 1;
        }
    };
    match activity.reserve_update() {
        Err(app_state::ActivityReservationError::PhysicalPlaybackActive) => 0,
        Ok(_) => {
            eprintln!("packaged update admission self-test accepted installation during playback");
            1
        }
        Err(error) => {
            eprintln!("packaged update admission self-test returned unexpected error: {error}");
            1
        }
    }
}
pub(super) fn update_smoke_marker(flag: &str) -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            let value = args.next()?;
            if value.len() <= 4096 && !value.contains('\0') {
                return Some(PathBuf::from(value));
            }
            return None;
        }
    }
    None
}
/// Validate the packaged native desktop composition without constructing a WebView.
///
/// This hidden, packaging-only entrypoint uses the same native composition
/// root as the production shell and exercises safe, non-physical command
/// seams before shutdown.
pub fn selftest_packaged_shell() -> i32 {
    let paths = match sky_native_adapters::AppPaths::resolve() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("packaged shell selftest path resolution failed: {error}");
            std::process::exit(2);
        }
    };
    let resources = sky_native_adapters::AppResources::from_resource_dir(paths.install_root());
    let selftest_paths = paths.clone();
    let runtime = match native_runtime::NativeDesktopRuntime::from_paths_with_activity(
        paths,
        resources,
        app_state::ActivityCoordinator::default(),
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("packaged native selftest could not start runtime: {error}");
            return 2;
        }
    };
    if std::env::var_os("SKY_DESKTOP_RESTART_SELFTEST").is_some() {
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
        let builtin_count = match runtime.builtin_catalog_status()? {
            sky_native_adapters::BuiltinCatalogStatus::Available { song_count }
                if song_count > 0 =>
            {
                song_count
            }
            sky_native_adapters::BuiltinCatalogStatus::Available { .. } => {
                return Err("packaged built-in catalog is empty".into());
            }
            sky_native_adapters::BuiltinCatalogStatus::Unavailable { code } => {
                return Err(format!("packaged built-in catalog unavailable: {code:?}"));
            }
        };
        let initial_catalog: commands::CatalogSearchDto =
            serde_json::from_value(runtime.dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":200}),
            )?)
            .map_err(|error| format!("packaged catalog search response: {error}"))?;
        if initial_catalog.total as usize != builtin_count {
            return Err(format!(
                "packaged All Songs count {} does not match built-in manifest count {builtin_count}",
                initial_catalog.total
            ));
        }
        let mut catalog_generation = Some(initial_catalog.generation);
        if std::env::var_os("SKY_BUILTIN_CATALOG_FRESH_SELFTEST").is_some() {
            selftest_paths
                .ensure_mutable_directories()
                .map_err(|error| format!("fresh packaged app-data setup failed: {error}"))?;
            if !sky_native_adapters::snapshot_directory(selftest_paths.user_music_root())
                .map_err(|error| format!("fresh user songs snapshot failed: {error}"))?
                .is_empty()
            {
                return Err("fresh packaged app-data songs directory is not empty".into());
            }
            let user_song = selftest_paths.user_music_root().join("packaged-user.json");
            std::fs::write(
                &user_song,
                br#"{"name":"Packaged user","songNotes":[{"time":0,"key":"1Key0"}]}"#,
            )
            .map_err(|error| format!("fresh user song write failed: {error}"))?;
            let reloaded: commands::CatalogReloadDto =
                serde_json::from_value(runtime.dispatch("catalog.reload", serde_json::json!({}))?)
                    .map_err(|error| format!("packaged catalog reload response: {error}"))?;
            let composed: commands::CatalogSearchDto = serde_json::from_value(runtime.dispatch(
                "catalog.search",
                serde_json::json!({
                    "query":"",
                    "offset":0,
                    "limit":200,
                    "generation":reloaded.generation
                }),
            )?)
            .map_err(|error| format!("composed catalog search response: {error}"))?;
            if composed.total as usize != builtin_count + 1 {
                return Err(format!(
                    "All Songs did not compose one user song with built-ins: {}",
                    composed.total
                ));
            }
            catalog_generation = Some(reloaded.generation);
            std::fs::remove_file(&user_song)
                .map_err(|error| format!("fresh user song cleanup failed: {error}"))?;
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
        match runtime.dispatch("update.check", serde_json::json!({"origin": "background"})) {
            Ok(value) => {
                let _update_check: commands::UpdateCheckAckDto = serde_json::from_value(value)
                    .map_err(|error| format!("update.check response: {error}"))?;
            }
            Err(error) if error.starts_with("update_service_unavailable") => {}
            Err(error) => return Err(format!("unexpected update.check failure: {error}")),
        }
        match runtime.dispatch(
            "update.begin_handoff",
            serde_json::json!({"targetVersion": "9.9.9"}),
        ) {
            Ok(value) => {
                let _update_install: commands::UpdateInstallAckDto = serde_json::from_value(value)
                    .map_err(|error| format!("update.begin_handoff response: {error}"))?;
            }
            Err(error) if error.starts_with("update_service_unavailable") => {}
            Err(error) => return Err(format!("unexpected update.begin_handoff failure: {error}")),
        }
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
                "generation": catalog_generation
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
                            "timing_margin_us": settings.playback_defaults.timing_margin_us,
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

/// Emit only compile-time native build metadata for release qualification.
/// This path has no application composition, I/O, playback, or
/// update side effects.
pub fn selftest_build_info() -> i32 {
    let dto = native_runtime::native_build_dto();
    let payload = serde_json::json!({
        "schema_version": 1,
        "native_build_commit": dto.native_build_commit,
        "native_version": dto.native_version,
        "rustc_version": dto.rustc_version,
        "win32_backend": dto.win32_backend,
    });
    match serde_json::to_string(&payload) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("failed to serialize native build metadata: {error}");
            1
        }
    }
}

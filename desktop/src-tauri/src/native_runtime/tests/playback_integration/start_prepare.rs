use super::*;

#[test]
fn target_integrity_startup_admission_fails_only_for_mismatch() {
    use sky_dispatch_win32::focus::TargetIntegrityCompatibility;

    assert!(target_integrity_startup_failure(TargetIntegrityCompatibility::Compatible).is_none());
    assert!(target_integrity_startup_failure(TargetIntegrityCompatibility::Unknown).is_none());
    assert_eq!(
        target_integrity_startup_failure(TargetIntegrityCompatibility::Mismatch),
        Some(
            "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput"
        )
    );
}

#[test]
fn target_integrity_admission_precedes_focus_and_session_arm() {
    let source = include_str!("../../../native_runtime/playback/session.rs").replace("\r\n", "\n");
    let create = source
        .split("fn create_native_player(")
        .nth(1)
        .expect("native player creation chain");
    let integrity = create
        .find("target_integrity_startup_failure")
        .expect("startup integrity admission");
    let focus = create
        .find("focus_window_and_verify")
        .expect("target focus verification");
    let session = create
        .find("NativeDispatchSession::new_with_power_endpoint")
        .expect("native session construction");
    assert!(integrity < focus);
    assert!(integrity < session);

    let arm = source.find("player.arm(0)").expect("physical session arm");
    let create_call = source
        .find("self.create_native_player(")
        .expect("native player creation call");
    let supervisor_spawn = source
        .find("let spawn_result = thread::Builder::new()")
        .expect("supervisor thread must be spawned before worker activation");
    assert!(create_call < arm);
    assert!(supervisor_spawn < arm);
}

#[test]
fn playback_start_validation_distinguishes_malformed_and_stale_ids() {
    let malformed = super::NativePlaybackStartRequest {
        prepared_id: "A".repeat(32),
        decisions: Vec::new(),
    };
    let error = validate_playback_start_request(&malformed).expect_err("invalid ID");
    assert!(error.starts_with("invalid_params:"));

    let well_formed_stale = super::NativePlaybackStartRequest {
        prepared_id: "a".repeat(32),
        decisions: Vec::new(),
    };
    validate_playback_start_request(&well_formed_stale).expect("well-formed ID");
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let error = service
        .start(
            well_formed_stale.into_public(),
            &ApplicationSettings::default(),
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
        .expect_err("stale ID");
    assert!(error.contains("stale or already consumed"));
}

#[test]
fn playback_start_validation_bounds_decisions_before_admission() {
    let oversized = super::NativePlaybackStartRequest {
        prepared_id: "a".repeat(32),
        decisions: vec![
            crate::commands::PlaybackDecisionAcceptanceDto {
                decision: crate::commands::PlaybackDecision::Proceed,
                accepted: false,
            };
            super::MAX_DECISION_COUNT + 1
        ],
    };
    let error = validate_playback_start_request(&oversized).expect_err("bounded decisions");
    assert!(error.contains("decisions must be a bounded array"));
}

#[test]
fn routed_playback_start_validates_input_before_confirmation_and_lookup() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-start-validation-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/ready.json"),
        r#"{"name":"Ready","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "ready",
                "offset": 0,
                "limit": 10,
                "generation": generation
            }),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
    let prepared_id = prepared["prepared_id"]
        .as_str()
        .expect("prepared ID")
        .to_owned();

    let malformed = runtime.dispatch(
        "playback.start",
        serde_json::json!({"preparedId":"A".repeat(32),"decisions":[]}),
    );
    assert!(
        malformed
            .expect_err("malformed ID")
            .contains("invalid_params: prepared_id is invalid")
    );

    let oversized = runtime.dispatch(
        "playback.start",
        serde_json::json!({
            "preparedId": prepared_id,
            "decisions": (0..=MAX_DECISION_COUNT)
                .map(|_| serde_json::json!({"decision":"proceed","accepted":false}))
                .collect::<Vec<_>>()
        }),
    );
    assert!(
        oversized
            .expect_err("oversized decisions")
            .contains("decisions must be a bounded array")
    );

    let wrong_confirmation = runtime.dispatch(
        "playback.start",
        serde_json::json!({
            "preparedId": prepared_id,
            "decisions": [{"decision":"proceed","accepted":true}]
        }),
    );
    assert!(
        wrong_confirmation
            .expect_err("ready plan must reject confirmation")
            .contains("ready playback accepts no risk decisions")
    );

    let valid = runtime
        .dispatch(
            "playback.start",
            serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
        )
        .expect("valid ready start");
    assert_eq!(valid["state"], "starting");
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_prepared_ids_are_random_lowercase_hex_and_eviction_is_fifo() {
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..128 {
        let id = opaque_native_id().expect("native ID");
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert!(ids.insert(id));
    }

    let mut prepared = std::collections::VecDeque::new();
    for value in 0..(MAX_PREPARED_PLANS + 3) {
        prepared.push_back(value);
        retain_prepared_capacity(&mut prepared);
    }
    assert_eq!(prepared.len(), MAX_PREPARED_PLANS);
    assert_eq!(prepared.front(), Some(&3));
    assert_eq!(prepared.back(), Some(&(MAX_PREPARED_PLANS + 2)));
}

#[test]
fn settings_fingerprint_excludes_appearance_and_auto_play() {
    let mut settings = sky_app_core::settings::ApplicationSettings::default();
    settings.playback_defaults.hold_frames = 1.25;
    settings.playback_defaults.tempo_scale = 0.95;
    settings.playback_defaults.fps = 120;
    settings.telemetry_enabled = true;
    let default_settings_fingerprint =
        settings_fingerprint(&settings).expect("settings fingerprint");
    // Palette and Auto Play do not affect a prepared schedule's identity.
    assert_eq!(
        default_settings_fingerprint,
        "dba3b575ae5967affc8a3d71f6aa4cadf9cec6104eff40ccdc83d5dfd6a8b46d"
    );
    let mut palette_changed = settings.clone();
    palette_changed.palette = "slate".into();
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&palette_changed).expect("palette is not timing identity")
    );
    let mut auto_play_changed = settings.clone();
    auto_play_changed.playback_behavior.auto_play = false;
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&auto_play_changed).expect("Auto Play is not timing identity")
    );
    let mut changed_settings = settings.clone();
    changed_settings.telemetry_enabled = false;
    assert_ne!(
        default_settings_fingerprint,
        settings_fingerprint(&changed_settings).expect("unclassified settings remain distinct")
    );
    changed_settings = settings.clone();
    changed_settings.playback_defaults.timing_margin_us = 900;
    assert_ne!(
        default_settings_fingerprint,
        settings_fingerprint(&changed_settings).expect("changed margin fingerprint")
    );
    changed_settings.playback_defaults.timing_margin_us =
        sky_app_core::settings::DEFAULT_TIMING_MARGIN_US;
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&changed_settings).expect("restored timing margin fingerprint")
    );

    let song = parse_song_json(
        br#"{"name":"Fingerprint","songNotes":[{"time":0,"key":"Key0"}]}"#,
        "fingerprint",
    )
    .expect("song");
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 800).expect("policy");
    let schedule = build_schedule_with_policy(&song, 0.95, &policy).expect("schedule");
    let config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 0.95,
        fps: 60,
        dry_run: true,
    };
    assert_eq!(
        plan_fingerprint(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &config,
            &schedule,
            &policy,
        )
        .expect("plan fingerprint"),
        "37c8cb8b8e70d3cab60fc794781cbeb794d6bf59b78e38ca1ccbeee0f4afc94d"
    );
}

#[test]
fn native_detail_matches_python_tempo_oracle_corpus() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/song_planning/oracle_cases.json"
    ))
    .expect("tempo fixture");
    let cases = fixture["detail_tempo_cases"]
        .as_array()
        .expect("detail tempo cases");
    assert_eq!(cases.len(), 5);
    for case in cases {
        let tempo = case["tempo_scale"].as_f64().expect("tempo");
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-tempo-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(
            root.join("config.json"),
            format!(r#"{{"schema_version":3,"default_tempo_scale":{tempo}}}"#),
        )
        .expect("config");
        fs::write(
            root.join("songs/tempo.json"),
            serde_json::to_vec(&case["raw"]).expect("raw song"),
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        runtime.bootstrap().expect("bootstrap");
        let search = runtime
            .search(crate::commands::CatalogSearchRequest {
                query: String::new(),
                offset: 0,
                limit: 10,
                generation: None,
                source: crate::commands::LibrarySource::default(),
            })
            .expect("search");
        let detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: search.items[0].song_id.clone(),
                generation: Some(search.generation),
            })
            .expect("detail");
        let actual = serde_json::to_value(&detail).expect("detail JSON");
        assert_eq!(actual["duration_us"], case["duration_us"], "tempo {tempo}");
        assert_eq!(actual["risk"]["level"], case["risk_level"], "tempo {tempo}");
        assert_eq!(
            actual["risk"]["recommendations"], case["risk_recommendations"],
            "tempo {tempo}"
        );
        assert_eq!(
            actual["recommendation"]["recommended_hold_frames"],
            case["recommendation"]["recommended_hold_frames"],
            "tempo {tempo}"
        );
        assert_eq!(
            actual["recommendation"]["recommended_tempo_scale"],
            case["recommendation"]["recommended_tempo_scale"],
            "tempo {tempo}"
        );
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn native_prepare_preserves_python_validation_failed_contract() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-prepare-contract-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/repeat.json"),
        r#"{"name":"Repeat","songNotes":[{"time":0,"key":"Key0"},{"time":1,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "repeat",
                "offset": 0,
                "limit": 10,
                "generation": generation
            }),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"].as_str().expect("song ID");
    let prepared = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId": song_id,
                "generation": generation,
                "config": {
                    "hold_frames": 1.0,
                    "timing_margin_us": 800,
                    "tempo_scale": 1.0,
                    "fps": 60,
                    "dry_run": false
                }
            }),
        )
        .expect("blocked preparation is a typed DTO");
    assert_eq!(prepared["admission"], "blocked");
    assert_eq!(prepared["error_code"], "validation_failed");
    assert_eq!(
        prepared["error_message"],
        "Detected 1 infeasible same-key repeat(s): the authored interval is shorter than the configured hold."
    );
    assert_eq!(prepared["prepared_id"], Value::Null);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_prepare_binds_advisories_to_variant_fingerprints_without_gating_model_pressure() {
    fn input(
        song_id: &str,
        song: &super::Song,
        settings: &ApplicationSettings,
        config: PlaybackConfigDto,
        severity: &str,
        recommendation: bool,
    ) -> super::NativePreparedInput {
        let timing_policy = MaterializedTimingPolicy::from_user_margin(
            config.fps,
            config.hold_frames,
            config.timing_margin_us,
        )
        .expect("fixture timing policy");
        let schedule = build_schedule_with_policy(song, config.tempo_scale, &timing_policy)
            .expect("fixture schedule");
        let mut risk = super::analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            config.hold_frames,
            config.tempo_scale,
        );
        risk.severity = severity.into();
        if recommendation {
            risk.suggested_hold_frames = Some(1.25);
            risk.suggested_tempo_scale = Some(0.9);
        }
        super::NativePreparedInput {
            song_id: song_id.into(),
            generation: 1,
            config,
            song: song.clone(),
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            source_epoch: 0,
        }
    }

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let settings = ApplicationSettings::default();
    let song = super::parse_song_json(
        br#"{"name":"Variant advisory","songNotes":[{"time":0,"key":"Key0"}]}"#,
        "variant-advisory",
    )
    .expect("fixture song");
    let base_config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: false,
    };

    let ready = service
        .prepare(input(
            "variant-advisory",
            &song,
            &settings,
            base_config.clone(),
            "low",
            false,
        ))
        .expect("modeled pressure remains a ready plan");
    assert_eq!(ready.admission, crate::commands::PlaybackAdmission::Ready);
    let ready_variant = ready.variants.first().expect("Proceed variant");
    let ready_advisory = ready_variant
        .timing_advisory
        .as_ref()
        .expect("Proceed advisory");
    assert_eq!(
        ready_advisory.status,
        crate::commands::TimingAdvisoryStatus::ModeledRisk
    );
    assert_eq!(
        ready_advisory.plan_fingerprint,
        ready_variant.plan_fingerprint
    );
    assert_eq!(ready_variant.config.hold_frames, base_config.hold_frames);
    assert_eq!(
        ready_variant.config.timing_margin_us,
        base_config.timing_margin_us
    );
    assert_eq!(ready_variant.config.tempo_scale, base_config.tempo_scale);
    assert_eq!(ready_variant.config.fps, base_config.fps);
    assert_eq!(ready_variant.config.dry_run, base_config.dry_run);
    assert!(!ready_advisory.receiver_acknowledged);

    let confirmation = service
        .prepare(input(
            "variant-advisory",
            &song,
            &settings,
            base_config.clone(),
            "medium",
            true,
        ))
        .expect("confirmation variants");
    assert_eq!(
        confirmation.admission,
        crate::commands::PlaybackAdmission::ConfirmationRequired
    );
    assert_eq!(confirmation.variants.len(), 3);
    let prepared_id = confirmation
        .prepared_id
        .as_ref()
        .expect("prepared plan identity");
    let prepared = service.prepared.lock().expect("prepared plan queue");
    let record = prepared
        .iter()
        .find(|(id, _)| id == prepared_id)
        .map(|(_, record)| record)
        .expect("retained prepared plan");
    let mut fingerprints = std::collections::HashSet::new();
    for variant_dto in &confirmation.variants {
        let advisory = variant_dto
            .timing_advisory
            .as_ref()
            .expect("variant advisory");
        assert_eq!(advisory.plan_fingerprint, variant_dto.plan_fingerprint);
        assert!(fingerprints.insert(variant_dto.plan_fingerprint.as_str()));
        let native_variant = record
            .variants
            .get(&variant_dto.decision)
            .expect("matching native variant");
        assert_eq!(native_variant.fingerprint, variant_dto.plan_fingerprint);
        assert_eq!(
            native_variant.config.hold_frames,
            variant_dto.config.hold_frames
        );
        assert_eq!(
            native_variant.config.timing_margin_us,
            variant_dto.config.timing_margin_us
        );
        assert_eq!(
            native_variant.config.tempo_scale,
            variant_dto.config.tempo_scale
        );
        assert_eq!(native_variant.config.fps, variant_dto.config.fps);
        assert_eq!(native_variant.config.dry_run, variant_dto.config.dry_run);
        assert!(!advisory.receiver_acknowledged);
    }
    let recommended = confirmation
        .variants
        .iter()
        .find(|variant| variant.decision == crate::commands::PlaybackDecision::UseRecommended)
        .expect("recommended variant");
    assert_eq!(recommended.config.hold_frames, 1.25);
    assert_eq!(recommended.config.tempo_scale, 0.9);
    let dry_run = confirmation
        .variants
        .iter()
        .find(|variant| variant.decision == crate::commands::PlaybackDecision::DryRun)
        .expect("Dry Run variant");
    assert!(dry_run.config.dry_run);
    assert!(
        dry_run
            .timing_advisory
            .as_ref()
            .expect("Dry Run advisory")
            .dry_run_is_hypothetical
    );
}

#[test]
fn settings_patch_cancels_inflight_prepare_without_waiting_for_whole_stream_analysis() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-prepare-settings-race-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let request = crate::commands::PlaybackPrepareRequest {
        song_id,
        generation,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        },
    };
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *runtime
        .playback
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let prepare_runtime = Arc::clone(&runtime);
    let prepare = thread::spawn(move || prepare_runtime.prepare_playback(request));
    entered.wait();
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: Some("slate".into()),
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: Some(crate::commands::PlaybackPatch {
                hold_frames: None,
                timing_margin_us: None,
                tempo_scale: Some(0.95),
                fps: None,
            }),
            auto_play: None,
            update_preferences: None,
        })
        .expect("settings mutation completes while analysis is paused");
    release.wait();
    assert!(
        prepare
            .join()
            .expect("Prepare worker")
            .expect_err("settings mutation invalidates old source snapshot")
            .contains("superseded")
    );
    assert!(
        runtime
            .playback
            .prepared
            .lock()
            .expect("prepared cache")
            .is_empty()
    );

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn palette_only_patch_preserves_inflight_prepare_and_native_start_token() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-prepare-palette-race-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":5}"#).expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let request = crate::commands::PlaybackPrepareRequest {
        song_id,
        generation,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        },
    };
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *runtime
        .playback
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let prepare_runtime = Arc::clone(&runtime);
    let prepare = thread::spawn(move || prepare_runtime.prepare_playback(request));
    entered.wait();
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: Some("slate".into()),
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: None,
        })
        .expect("palette mutation completes while analysis is paused");
    assert_eq!(
        runtime
            .reload_settings_snapshot()
            .expect("settings snapshot")
            .palette,
        "slate"
    );
    release.wait();

    let prepared = prepare
        .join()
        .expect("Prepare worker")
        .expect("palette patch preserves in-flight preparation");
    let prepared_id = prepared.prepared_id.as_ref().expect("prepared ID");
    let decisions =
        if prepared.admission == crate::commands::PlaybackAdmission::ConfirmationRequired {
            serde_json::json!([{"decision":"proceed","accepted":true}])
        } else {
            serde_json::json!([])
        };
    runtime
        .dispatch(
            "playback.start",
            serde_json::json!({"preparedId":prepared_id,"decisions":decisions}),
        )
        .expect("native token remains valid after appearance-only patch");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn auto_play_patch_keeps_prepared_native_playback_valid() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-auto-play-native-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":5}"#).expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": search["items"][0]["song_id"],
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
    let prepared_id = prepared["prepared_id"].as_str().expect("prepared ID");
    let palette_updated = runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: Some("slate".into()),
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: None,
        })
        .expect("palette settings patch");
    assert_eq!(palette_updated.palette, "slate");
    let updated = runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: Some(false),
            update_preferences: None,
        })
        .expect("Auto Play settings patch");
    assert!(!updated.auto_play);
    let decisions = if prepared["admission"] == "confirmation_required" {
        serde_json::json!([{"decision":"proceed","accepted":true}])
    } else {
        serde_json::json!([])
    };
    assert!(
        runtime
            .dispatch(
                "playback.start",
                serde_json::json!({
                    "preparedId":prepared_id,
                    "decisions":decisions
                }),
            )
            .is_ok()
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mixed_palette_and_timing_patch_rejects_stale_native_prepared_token() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-mixed-settings-native-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":5}"#).expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let prepared = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId": search["items"][0]["song_id"],
                "generation": generation,
                "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
            }),
        )
        .expect("prepare");
    let prepared_id = prepared["prepared_id"].as_str().expect("prepared ID");
    let decisions = if prepared["admission"] == "confirmation_required" {
        serde_json::json!([{"decision":"proceed","accepted":true}])
    } else {
        serde_json::json!([])
    };

    runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: Some("slate".into()),
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: Some(crate::commands::PlaybackPatch {
                hold_frames: None,
                timing_margin_us: None,
                tempo_scale: Some(0.9),
                fps: None,
            }),
            auto_play: None,
            update_preferences: None,
        })
        .expect("mixed palette and timing mutation");

    let start = runtime
        .dispatch(
            "playback.start",
            serde_json::json!({"preparedId":prepared_id,"decisions":decisions}),
        )
        .expect_err("mixed timing patch must invalidate the native token");
    assert!(start.contains("stale or already consumed"), "{start}");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

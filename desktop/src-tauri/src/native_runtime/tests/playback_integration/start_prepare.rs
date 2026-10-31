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
    // Appearance mode, palette, and Auto Play do not affect a prepared schedule's identity.
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
    let mut appearance_changed = settings.clone();
    appearance_changed.appearance_mode = sky_app_core::settings::AppearanceMode::Light;
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&appearance_changed).expect("appearance is not timing identity")
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
fn native_detail_matches_tempo_recommendation_contract_cases() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/song_planning/schedule_contract_cases.json"
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
                conversion_mode: None,
                target_pitch: None,
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
fn native_prepare_keeps_typed_validation_failure_contract() {
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
fn native_catalog_and_prepare_share_sheet_plan_with_exact_loss_consent_and_source_guard() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-sheet-prepare-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/legacy.json"),
        r#"{"name":"Legacy","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("legacy sheet");
    fs::write(
        root.join("songs/legacy-physical.json"),
        r#"{"name":"Legacy physical","songNotes":[{"time":0,"key":"Key0"},{"time":0,"key":"Key1"},{"time":0,"key":"Key2"},{"time":0,"key":"Key3"},{"time":0,"key":"Key4"},{"time":0,"key":"Key5"},{"time":0,"key":"Key6"},{"time":0,"key":"Key7"},{"time":0,"key":"Key8"},{"time":0,"key":"Key9"}]}"#,
    )
    .expect("legacy physical-risk sheet");
    let composed = r#"{
        "type":"composed","version":5,"bpm":120,"pitch":"C",
        "data":{"appName":"Sky"},"columnTempos":[0,0],
        "tracks":[
            {"instrument":{"name":"Piano"},"notes":[[0,72,1]]},
            {"instrument":{"name":"Piano"},"notes":[[1,74,1]]}
        ]
    }"#;
    fs::write(root.join("songs/composed.json"), composed).expect("composed sheet");
    fs::write(
        root.join("songs/recorded.json"),
        r#"{"type":"recorded","version":4,"data":{"appName":"Sky"},"tracks":[{"instrument":{"name":"Piano"},"notes":[[74,0,250]]}]}"#,
    )
    .expect("recorded sheet");
    fs::write(
        root.join("songs/risk.json"),
        r#"{"type":"composed","version":5,"bpm":120,"pitch":"C","data":{"appName":"Sky"},"columnTempos":[0],"tracks":[{"instrument":{"name":"Piano"},"notes":[[0,60,1],[0,62,1],[0,64,1],[0,65,1],[0,67,1],[0,69,1],[0,71,1],[0,72,1],[0,74,1],[0,86,1]]}]}"#,
    )
    .expect("risk and conversion-loss sheet");
    fs::write(
        root.join("songs/unsupported.json"),
        r#"{"type":"composed","version":99,"data":{"appName":"Sky"},"songNotes":[{"time":0,"key":"Key0"}],"columns":[],"instruments":[]}"#,
    )
    .expect("unsupported sheet with legacy projection");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({"playbackDefaults":{"tempoScale":0.9}}),
        )
        .expect("set non-default catalog timing context");
    let generation = catalog_generation(&runtime);
    let find_song = |query: &str| {
        let search = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": query,
                    "offset": 0,
                    "limit": 10,
                    "generation": generation
                }),
            )
            .expect("catalog search");
        search["items"][0]["song_id"]
            .as_str()
            .expect("song ID")
            .to_owned()
    };
    let config = serde_json::json!({
        "hold_frames":1.0,
        "timing_margin_us":800,
        "tempo_scale":0.9,
        "fps":60,
        "dry_run":false
    });

    let legacy_id = find_song("legacy");
    let legacy_detail = runtime
        .dispatch(
            "catalog.detail",
            serde_json::json!({"songId":legacy_id,"generation":generation}),
        )
        .expect("legacy detail");
    let legacy_prepare = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({"songId":legacy_id,"generation":generation,"config":config}),
        )
        .expect("legacy prepare");
    assert_eq!(
        legacy_detail["sheet_analysis"]["format_label"],
        "Legacy flat"
    );
    assert_eq!(
        legacy_prepare["song"]["sheet_analysis"],
        legacy_detail["sheet_analysis"]
    );
    assert!(legacy_prepare["prepared_id"].as_str().is_some());

    let composed_id = find_song("composed");
    let composed_detail = runtime
        .dispatch(
            "catalog.detail",
            serde_json::json!({"songId":composed_id,"generation":generation}),
        )
        .expect("composed detail");
    let composed_prepare = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({"songId":composed_id,"generation":generation,"config":config}),
        )
        .expect("composed prepare");
    assert_eq!(
        composed_detail["sheet_analysis"]["format_label"],
        "Composed v5"
    );
    assert_eq!(composed_detail["sheet_analysis"]["source_track_count"], 2);
    let mut detail_analysis = composed_detail["sheet_analysis"].clone();
    let mut prepared_analysis = composed_prepare["song"]["sheet_analysis"].clone();
    detail_analysis["confirmation_fingerprint"] = Value::Null;
    prepared_analysis["confirmation_fingerprint"] = Value::Null;
    assert_eq!(prepared_analysis, detail_analysis);
    assert_eq!(composed_prepare["admission"], "ready");
    assert!(composed_prepare["prepared_id"].as_str().is_some());

    let recorded_id = find_song("recorded");
    let conversion_mode = serde_json::json!({"kind":"single_track","track_id":0});
    let selected_detail = runtime
        .dispatch(
            "catalog.detail",
            serde_json::json!({
                "songId":recorded_id,
                "generation":generation,
                "conversionMode":conversion_mode,
                "targetPitch":"D"
            }),
        )
        .expect("selected recorded detail");
    assert_eq!(
        selected_detail["sheet_analysis"]["format_label"],
        "Recorded v4"
    );
    assert_eq!(selected_detail["sheet_analysis"]["target_pitch"], "D");
    assert_eq!(
        selected_detail["sheet_analysis"]["readiness"],
        "requires_confirmation"
    );
    assert!(
        selected_detail["sheet_analysis"]["authored_sustain_lost_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    let _detail_confirmation = selected_detail["sheet_analysis"]["confirmation_fingerprint"]
        .as_str()
        .expect("catalog detail confirmation token")
        .to_owned();
    let prepare_request = |fingerprint: Option<String>| {
        let mut request = serde_json::json!({
            "songId":recorded_id,
            "generation":generation,
            "config":config,
            "conversionMode":conversion_mode,
            "targetPitch":"D"
        });
        if let Some(fingerprint) = fingerprint {
            request["conversionConfirmationFingerprint"] = fingerprint.into();
        }
        request
    };
    let proposal = runtime
        .dispatch("playback.prepare", prepare_request(None))
        .expect("loss proposal");
    assert_eq!(proposal["error_code"], "conversion_confirmation_required");
    assert!(proposal["prepared_id"].is_null());
    let confirmation = proposal["song"]["sheet_analysis"]["confirmation_fingerprint"]
        .as_str()
        .expect("exact playback config consent token")
        .to_owned();
    let wrong = runtime
        .dispatch(
            "playback.prepare",
            prepare_request(Some("stale-token".into())),
        )
        .expect("stale consent proposal");
    assert_eq!(wrong["error_code"], "conversion_confirmation_required");
    assert!(wrong["prepared_id"].is_null());
    let accepted = runtime
        .dispatch("playback.prepare", prepare_request(Some(confirmation)))
        .expect("explicit loss consent");
    assert_eq!(
        accepted["song"]["sheet_analysis"]["confirmation_received"],
        true
    );
    let prepared_id = accepted["prepared_id"]
        .as_str()
        .expect("prepared after consent")
        .to_owned();
    fs::write(root.join("songs/recorded.json"), composed).expect("rewrite source after prepare");
    let stale_start = runtime.dispatch(
        "playback.start",
        serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
    );
    assert!(stale_start.unwrap_err().contains("source sheet changed"));

    let risk_id = find_song("risk");
    let risk_detail = runtime
        .dispatch(
            "catalog.detail",
            serde_json::json!({"songId":risk_id,"generation":generation}),
        )
        .expect("risk detail");
    assert_eq!(
        risk_detail["sheet_analysis"]["readiness"],
        "requires_confirmation"
    );
    assert_eq!(risk_detail["risk"]["level"], "medium");
    assert_eq!(
        risk_detail["recommendation"]["recommended_tempo_scale"],
        0.9
    );
    assert!(
        risk_detail["recommendation"]["summary"]
            .as_str()
            .unwrap()
            .contains("prepare again")
    );
    let _risk_detail_token = risk_detail["sheet_analysis"]["confirmation_fingerprint"]
        .as_str()
        .expect("risk sheet detail token")
        .to_owned();
    let risk_proposal = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({"songId":risk_id,"generation":generation,"config":config}),
        )
        .expect("risk sheet loss proposal");
    assert!(risk_proposal["prepared_id"].is_null());
    assert_eq!(
        risk_proposal["song"]["recommendation"]["recommended_tempo_scale"],
        0.9
    );
    assert_eq!(
        risk_proposal["song"]["risk"]["level"],
        risk_detail["risk"]["level"]
    );
    assert_eq!(
        risk_proposal["song"]["risk"]["recommendations"],
        risk_detail["risk"]["recommendations"]
    );
    let risk_token = risk_proposal["song"]["sheet_analysis"]["confirmation_fingerprint"]
        .as_str()
        .expect("risk playback config consent token")
        .to_owned();
    let changed_timing = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId":risk_id,
                "generation":generation,
                "config":{
                    "hold_frames":1.0,
                    "timing_margin_us":800,
                    "tempo_scale":0.95,
                    "fps":60,
                    "dry_run":false
                },
                "conversionConfirmationFingerprint":risk_token.clone()
            }),
        )
        .expect("timing change produces a fresh conversion proposal");
    assert!(changed_timing["prepared_id"].is_null());
    assert_eq!(
        changed_timing["error_code"],
        "conversion_confirmation_required"
    );
    assert_ne!(
        changed_timing["song"]["sheet_analysis"]["confirmation_fingerprint"],
        risk_proposal["song"]["sheet_analysis"]["confirmation_fingerprint"]
    );
    let risk_accepted = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId":risk_id,
                "generation":generation,
                "config":config,
                "conversionConfirmationFingerprint":risk_token
            }),
        )
        .expect("loss consent leaves physical risk admission active");
    assert_eq!(risk_accepted["admission"], "confirmation_required");
    assert_eq!(risk_accepted["risk"]["level"], "medium");
    let decisions = risk_accepted["decisions"].as_array().unwrap();
    assert!(decisions.iter().any(|item| item["decision"] == "proceed"));
    assert!(decisions.iter().any(|item| item["decision"] == "dry_run"));
    assert!(
        decisions
            .iter()
            .all(|item| item["decision"] != "use_recommended")
    );
    let variants = risk_accepted["variants"].as_array().unwrap();
    let dry_run = variants
        .iter()
        .find(|item| item["decision"] == "dry_run")
        .expect("same-timing dry-run alternative");
    assert_eq!(
        dry_run["config"]["hold_frames"],
        risk_accepted["config"]["hold_frames"]
    );
    assert_eq!(
        dry_run["config"]["tempo_scale"],
        risk_accepted["config"]["tempo_scale"]
    );
    assert_eq!(dry_run["config"]["dry_run"], true);
    assert!(
        variants
            .iter()
            .all(|item| item["decision"] != "use_recommended")
    );
    assert!(
        risk_accepted["risk"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("prepare again")
    );
    assert!(risk_accepted["prepared_id"].as_str().is_some());

    let legacy_risk_id = find_song("legacy-physical");
    let legacy_risk = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId":legacy_risk_id,
                "generation":generation,
                "config":config
            }),
        )
        .expect("legacy risk recommendations remain available");
    assert_eq!(legacy_risk["risk"]["level"], "medium");
    assert!(
        legacy_risk["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["decision"] == "use_recommended")
    );

    let unsupported_id = find_song("unsupported");
    let unsupported_detail = runtime
        .dispatch(
            "catalog.detail",
            serde_json::json!({"songId":unsupported_id,"generation":generation}),
        )
        .expect("unsupported detail");
    assert_eq!(
        unsupported_detail["sheet_analysis"]["readiness"],
        "unsupported"
    );
    let unsupported_prepare = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({"songId":unsupported_id,"generation":generation,"config":config}),
        )
        .expect("unsupported prepare is a typed blocked result");
    assert_eq!(unsupported_prepare["error_code"], "unsupported_sheet");
    assert!(unsupported_prepare["prepared_id"].is_null());
    assert_eq!(unsupported_prepare["song"]["note_count"], 0);

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
            sheet_analysis: None,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            publication_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            source_path: std::path::PathBuf::new(),
            source_hash: String::new(),
            plan_identity: "test-plan".into(),
            plan_source: None,
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
        conversion_mode: None,
        target_pitch: None,
        conversion_confirmation_fingerprint: None,
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
            appearance_mode: None,
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
fn appearance_and_palette_only_patch_preserves_inflight_prepare_and_native_start_token() {
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
        conversion_mode: None,
        target_pitch: None,
        conversion_confirmation_fingerprint: None,
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
            appearance_mode: Some(crate::commands::AppearanceMode::Light),
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
    assert_eq!(
        runtime
            .reload_settings_snapshot()
            .expect("settings snapshot")
            .appearance_mode,
        sky_app_core::settings::AppearanceMode::Light
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
            appearance_mode: None,
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
            appearance_mode: None,
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
            appearance_mode: None,
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

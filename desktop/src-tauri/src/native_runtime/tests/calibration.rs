use super::*;

#[test]
fn calibration_budget_preserves_mode_specific_reserves() {
    let single = calibration_budget(
        CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS,
        crate::ui_events::CalibrationMode::Quick,
    )
    .expect("minimum single budget");
    assert_eq!(
        single.native_budget_seconds,
        CALIBRATION_MIN_NATIVE_TOTAL_SECONDS as u64
    );
    assert_eq!(single.child_timeout_seconds, 7.0);
    assert!(
        calibration_budget(
            CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS - 0.01,
            crate::ui_events::CalibrationMode::Quick,
        )
        .is_err()
    );

    let full = calibration_budget(
        CALIBRATION_MIN_FULL_TIMEOUT_SECONDS,
        crate::ui_events::CalibrationMode::Full,
    )
    .expect("minimum full budget");
    assert_eq!(
        full.native_budget_seconds,
        CALIBRATION_MIN_NATIVE_TOTAL_SECONDS as u64
    );
    assert_eq!(full.child_timeout_seconds, 7.0);
    let maximum = calibration_budget(
        CALIBRATION_DEFAULT_TIMEOUT_SECONDS,
        crate::ui_events::CalibrationMode::Full,
    )
    .expect("maximum budget");
    assert_eq!(maximum.native_budget_seconds, 114);
    assert_eq!(maximum.child_timeout_seconds, 115.0);
}

#[test]
fn synthetic_calibration_requires_explicit_package_composition() {
    let root = std::env::temp_dir().join("sky-native-calibration-no-child");
    let activity = ActivityCoordinator::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let playback = Arc::new(NativePlaybackService::new(activity.clone()));
    let request = CalibrationStartRequest {
        mode: CalibrationMode::Quick,
        class_name: None,
        polyphony: None,
        samples: None,
        timeout_seconds: None,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let safe = NativeCalibrationService::from_test_sandbox(
        &root,
        activity.clone(),
        events.clone(),
        playback.clone(),
        TestSeams::SafePackage,
    );
    assert!(
        safe.run_child(&request, &cancel, Arc::new(Mutex::new(None)))
            .is_ok()
    );

    let production = NativeCalibrationService::from_test_sandbox(
        &root,
        activity,
        events,
        playback,
        TestSeams::Disabled,
    );
    let error = production
        .run_child(&request, &cancel, Arc::new(Mutex::new(None)))
        .expect_err("production composition must select the real child");
    assert!(matches!(
        error,
        CalibrationRunError::Failed(message) if message.contains("failed to start")
    ));
}

#[test]
fn duplicate_calibration_start_keeps_already_running_contract() {
    let activity = ActivityCoordinator::default();
    let reservation = activity.reserve_calibration().expect("first reservation");
    let service = NativeCalibrationService::from_test_sandbox(
        &std::env::temp_dir(),
        activity,
        Arc::new(Mutex::new(NativeEventHub::default())),
        Arc::new(NativePlaybackService::new(ActivityCoordinator::default())),
        TestSeams::SafePackage,
    );
    let done = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    service.set_running_operation_for_test("a".repeat(32), done, reservation, None);
    let error = Arc::new(service)
        .start(CalibrationStartRequest {
            mode: CalibrationMode::Quick,
            class_name: None,
            polyphony: None,
            samples: None,
            timeout_seconds: None,
        })
        .expect_err("duplicate calibration");
    assert!(error.starts_with("already_running:"));
}

#[test]
fn calibration_shutdown_before_commit_cannot_mutate_after_return() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-calibration-close-before-{suffix}"));
    let paths = AppPaths::from_test_sandbox(&root);
    paths.ensure_mutable_directories().expect("cache directory");
    let cache = paths.calibration_cache_path();
    let original = br#"{"sentinel":"unchanged"}"#.to_vec();
    fs::write(&cache, &original).expect("sentinel cache");

    let activity = ActivityCoordinator::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let playback = Arc::new(NativePlaybackService::new(activity.clone()));
    let service = Arc::new(NativeCalibrationService::new(
        paths,
        activity.clone(),
        events.clone(),
        playback.clone(),
        TestSeams::SafePackage,
    ));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    service.pause_before_admission_for_test(entered.clone(), release.clone());
    service
        .start(quick_calibration_request())
        .expect("start calibration");
    entered.wait();

    let shutdown_service = Arc::clone(&service);
    let shutdown = std::thread::spawn(move || {
        shutdown_service.shutdown_with_timeout(Duration::from_millis(50))
    });
    let closing_deadline = Instant::now() + Duration::from_secs(1);
    while !service.publication_is_closing_for_test() && Instant::now() < closing_deadline {
        std::thread::yield_now();
    }
    assert!(
        service.publication_is_closing_for_test(),
        "shutdown crossed Closing"
    );
    assert!(
        !shutdown.join().expect("bounded shutdown thread"),
        "blocked worker must be contained at the shutdown deadline"
    );

    assert_eq!(fs::read(&cache).expect("cache after shutdown"), original);
    assert_eq!(
        playback.settings_invalidation_count.load(Ordering::Relaxed),
        0
    );
    assert_eq!(calibration_finished_count(&events), 0);
    assert!(!activity.is_calibration_active());
    // The worker is now detached behind the Closing gate.  Releasing it
    // proves that it can finish only by discarding its temporary artifact;
    // it cannot mutate production state after shutdown returned.
    release.wait();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        fs::read(&cache).expect("cache after worker release"),
        original
    );
    assert_eq!(
        playback.settings_invalidation_count.load(Ordering::Relaxed),
        0
    );
    assert_eq!(calibration_finished_count(&events), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn calibration_commit_before_shutdown_has_one_bounded_outcome() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-calibration-commit-before-{suffix}"));
    let paths = AppPaths::from_test_sandbox(&root);
    paths.ensure_mutable_directories().expect("cache directory");
    let activity = ActivityCoordinator::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let playback = Arc::new(NativePlaybackService::new(activity.clone()));
    let service = Arc::new(NativeCalibrationService::new(
        paths.clone(),
        activity.clone(),
        events.clone(),
        playback.clone(),
        TestSeams::SafePackage,
    ));
    let cache = paths.calibration_cache_path();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    service.pause_after_admission_for_test(entered.clone(), release.clone());
    service
        .start(quick_calibration_request())
        .expect("start calibration");
    entered.wait();

    let shutdown_service = Arc::clone(&service);
    let shutdown = std::thread::spawn(move || shutdown_service.shutdown());
    let closing_deadline = Instant::now() + Duration::from_secs(1);
    while !service.publication_is_closing_for_test() && Instant::now() < closing_deadline {
        std::thread::yield_now();
    }
    assert!(
        service.publication_is_closing_for_test(),
        "shutdown crossed Closing"
    );
    release.wait();
    shutdown.join().expect("shutdown thread");

    let resolution = load_calibration_resolution(&cache);
    assert_eq!(
        resolution.source,
        sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE
    );
    assert_eq!(
        playback.settings_invalidation_count.load(Ordering::Relaxed),
        1
    );
    assert!(calibration_finished_count(&events) <= 1);
    assert!(!activity.is_calibration_active());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn calibration_shutdown_hung_worker_is_bounded_and_nonpublishing() {
    let root = std::env::temp_dir().join(format!(
        "sky-calibration-hung-worker-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let activity = ActivityCoordinator::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let playback = Arc::new(NativePlaybackService::new(activity.clone()));
    let service = Arc::new(NativeCalibrationService::from_test_sandbox(
        &root,
        activity.clone(),
        events.clone(),
        playback.clone(),
        TestSeams::SafePackage,
    ));
    let reservation = activity.reserve_calibration().expect("calibration slot");
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Arc::new(AtomicBool::new(false));
    let worker_finished = Arc::new(AtomicBool::new(false));
    let worker_release = release.clone();
    let worker_finished_flag = worker_finished.clone();
    let worker = std::thread::spawn(move || {
        while !worker_release.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
        worker_finished_flag.store(true, Ordering::Release);
    });
    service.set_running_operation_for_test("h".repeat(32), done, reservation, Some(worker));

    let started = Instant::now();
    let worker_joined = service.shutdown_with_timeout(Duration::from_millis(50));
    assert!(!worker_joined);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!activity.is_calibration_active());
    assert_eq!(
        playback.settings_invalidation_count.load(Ordering::Relaxed),
        0
    );
    assert_eq!(calibration_finished_count(&events), 0);

    release.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker_finished.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(worker_finished.load(Ordering::Acquire));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn routed_calibration_start_maps_physical_activity_to_playback_active() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-calibration-playback-active-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let activity = ActivityCoordinator::default();
    let playback = activity
        .reserve_playback("native-session")
        .expect("physical playback reservation");
    let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
        root.clone(),
        activity,
        TestSeams::SafePackage,
    )
    .expect("runtime");
    let error = runtime
        .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
        .expect_err("physical playback must block calibration");
    assert!(error.starts_with("playback_active:"), "{error}");
    drop(playback);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn routed_duplicate_calibration_start_maps_to_already_running() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-calibration-already-running-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let activity = ActivityCoordinator::default();
    let reservation = activity
        .reserve_calibration()
        .expect("calibration reservation");
    let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
        root.clone(),
        activity,
        TestSeams::SafePackage,
    )
    .expect("runtime");
    let error = runtime
        .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
        .expect_err("duplicate calibration must be rejected");
    assert!(error.starts_with("already_running:"), "{error}");
    drop(reservation);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn calibration_cache_writer_and_loader_round_trip_out_of_envelope() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let cases = [
        (0_i64, Some(300_u64)),
        (677, Some(777)),
        (1900, Some(2000)),
        (2100, None),
    ];
    for (worst, expected_margin) in cases {
        let root = std::env::temp_dir().join(format!("sky-calibration-roundtrip-{suffix}-{worst}"));
        let mut raw = safe_calibration_evidence();
        raw["pair_buckets"]["1"]["hot"]["sendinput_shrink_us"]["max"] = Value::from(worst);
        let paths = AppPaths::from_test_sandbox(&root);
        paths.ensure_mutable_directories().expect("cache directory");
        publish_calibration_cache(paths.cache_root(), &raw).expect("cache publication");
        let cache = paths.calibration_cache_path();
        let resolution = load_calibration_resolution(&cache);
        assert_eq!(
            resolution.transport_reserve_us,
            expected_margin.unwrap_or(300)
        );
        assert_eq!(resolution.qualified, expected_margin.is_some());
        let recommendation = timing_margin_recommendation(&cache);
        assert_eq!(
            recommendation.recommended_timing_margin_us,
            Some(match worst {
                0 => 400,
                677 => 900,
                1_900 => 2_100,
                _ => sky_app_core::settings::DEFAULT_TIMING_MARGIN_US,
            })
        );
        assert_eq!(recommendation.qualified, expected_margin.is_some());
        assert_eq!(
            resolution.source,
            if expected_margin.is_some() {
                sky_native_adapters::CALIBRATION_MARGIN_SOURCE_DEVICE
            } else {
                sky_native_adapters::CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE
            }
        );
        if expected_margin.is_none() {
            let value: Value =
                serde_json::from_slice(&fs::read(&cache).expect("cache")).expect("json");
            assert!(value["transport_margin_us"].is_null());
            assert!(
                !value["calibration_timing_qualified"]
                    .as_bool()
                    .unwrap_or(true)
            );
        }
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn calibrated_timing_margin_reports_insufficient_headroom_without_clamping() {
    let max = sky_app_core::settings::MAX_TIMING_MARGIN_US;
    let guard = CALIBRATION_TIMING_MARGIN_GUARD_US;

    assert_eq!(
        recommended_calibrated_timing_margin_us(max - guard),
        Some(max)
    );
    assert_eq!(
        recommended_calibrated_timing_margin_us(max - guard + 1),
        None
    );
    assert_eq!(recommended_calibrated_timing_margin_us(u64::MAX), None);
}

#[test]
fn unqualified_margin_recommendation_uses_default_timing_margin() {
    let path = std::env::temp_dir().join(format!(
        "sky-timing-margin-recommendation-{}-missing.json",
        std::process::id()
    ));
    let recommendation = timing_margin_recommendation(&path);
    assert_eq!(
        recommendation.recommended_timing_margin_us,
        Some(sky_app_core::settings::DEFAULT_TIMING_MARGIN_US)
    );
    assert_eq!(recommendation.source, "default_fallback");
}

#[test]
fn calibration_recommendation_never_changes_the_persisted_user_margin() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-calibration-advisory-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(
        root.join("config.json"),
        r#"{"schema_version":4,"default_timing_margin_us":1200}"#,
    )
    .expect("user settings");
    let runtime = NativeDesktopRuntime::from_install_root_with_activity_and_seams(
        root.clone(),
        ActivityCoordinator::default(),
        TestSeams::SafePackage,
    )
    .expect("runtime");
    let mut raw = safe_calibration_evidence();
    raw["pair_buckets"]["1"]["hot"]["sendinput_shrink_us"]["max"] = Value::from(677_i64);
    publish_calibration_cache(runtime.paths().cache_root(), &raw).expect("cache");

    let settings = runtime
        .dispatch("settings.get", Value::Object(Default::default()))
        .expect("settings");
    assert_eq!(settings["playback_defaults"]["timing_margin_us"], 1_200);
    assert_eq!(
        settings["timing_margin_recommendation"]["recommended_timing_margin_us"],
        900
    );
    assert_eq!(settings["timing_margin_recommendation"]["qualified"], true);
    let persisted: Value =
        serde_json::from_slice(&fs::read(root.join("config.json")).expect("read settings"))
            .expect("JSON settings");
    assert_eq!(persisted["default_timing_margin_us"], 1_200);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn user_margin_changes_authored_schedule_and_plan_fingerprint() {
    let song = parse_song_json(
        br#"{"name":"Policy","songNotes":[{"time":0,"key":"Key0"},{"time":100,"key":"Key1"}]}"#,
        "policy",
    )
    .expect("song");
    let default_policy =
        MaterializedTimingPolicy::from_user_margin(60, 1.0, 800).expect("default policy");
    let increased_policy =
        MaterializedTimingPolicy::from_user_margin(60, 1.0, 900).expect("increased margin");
    let default_schedule =
        build_schedule_with_policy(&song, 1.0, &default_policy).expect("default schedule");
    let increased_schedule = build_schedule_with_policy(&song, 1.0, &increased_policy)
        .expect("increased-margin schedule");
    assert!(increased_schedule.actions[1].at_us > default_schedule.actions[1].at_us);
    assert_eq!(default_schedule.actions[1].at_us, 17_467);
    assert_eq!(increased_schedule.actions[1].at_us, 17_567);
    let default_config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: true,
    };
    let default_fingerprint = plan_fingerprint(
        "policy",
        &default_config,
        &default_schedule,
        &default_policy,
    )
    .expect("default fingerprint");
    let increased_config = PlaybackConfigDto {
        timing_margin_us: 900,
        ..default_config.clone()
    };
    let increased_fingerprint = plan_fingerprint(
        "policy",
        &increased_config,
        &increased_schedule,
        &increased_policy,
    )
    .expect("increased-margin fingerprint");
    assert_ne!(default_fingerprint, increased_fingerprint);

    // Calibration recommendations are not inputs to materialization or identity.
    let same_policy =
        MaterializedTimingPolicy::from_user_margin(60, 1.0, 800).expect("same policy");
    let same_schedule =
        build_schedule_with_policy(&song, 1.0, &same_policy).expect("same authored schedule");
    assert_eq!(
        default_fingerprint,
        plan_fingerprint("policy", &default_config, &same_schedule, &same_policy)
            .expect("unchanged recommendation identity")
    );
}

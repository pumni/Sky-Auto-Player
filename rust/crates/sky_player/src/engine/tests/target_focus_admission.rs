use super::support::*;
use super::*;

#[test]
fn target_stamp_rearms_preflight_without_rechecking_steady_state() {
    let backend = TrackedKeyState::with_emitter(|codes, _key_up| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let mut verified = None;
    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));

    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));

    let next_generation = TargetStamp {
        generation: 2,
        ..stamp
    };
    ensure_preflight_for_target(&backend, next_generation, &mut verified).unwrap();
    assert_eq!(verified, Some(next_generation));
}

#[test]
fn cached_preflight_does_not_notice_a_late_physical_down_at_final_boundary() {
    let physical_mask = Arc::new(AtomicU16::new(0));
    let preflight_calls = Arc::new(AtomicU64::new(0));
    let send_calls = Arc::new(AtomicU64::new(0));
    let preflight_mask = Arc::clone(&physical_mask);
    let probe_count = Arc::clone(&preflight_calls);
    let sender_count = Arc::clone(&send_calls);
    let mut backend = TrackedKeyState::with_packet_emitter(move |packet| {
        sender_count.fetch_add(1, Ordering::SeqCst);
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: None,
                completed_ticks: None,
                timing_error: None,
            },
        }
    });
    backend.set_preflight_physical_mask_probe(move || {
        probe_count.fetch_add(1, Ordering::SeqCst);
        preflight_mask.load(Ordering::Acquire)
    });

    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let mut verified_target = None;
    ensure_preflight_for_target(&backend, stamp, &mut verified_target).unwrap();
    assert_eq!(preflight_calls.load(Ordering::SeqCst), 1);
    assert_eq!(verified_target, Some(stamp));

    backend.set_physical_study_hook({
        let physical_mask = Arc::clone(&physical_mask);
        move |pending_down_mask| physical_mask.store(pending_down_mask, Ordering::Release)
    });
    ensure_preflight_for_target(&backend, stamp, &mut verified_target).unwrap();
    assert_eq!(preflight_calls.load(Ordering::SeqCst), 1);

    let packet = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid single-key Down packet");
    let outcome =
        backend.send_prepared_physical_packet_at_final_boundary(&packet, Some(QpcTicks::ZERO));
    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(physical_mask.load(Ordering::Acquire), 1);
    assert_eq!(preflight_calls.load(Ordering::SeqCst), 1);
    assert_eq!(send_calls.load(Ordering::SeqCst), 1);

    let mut fresh_verification = None;
    assert!(ensure_preflight_for_target(&backend, stamp, &mut fresh_verification).is_err());
    assert_eq!(preflight_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fresh_verification, None);
}

#[test]
fn admission_epoch_invalidation_requires_new_preflight_even_for_same_stamp() {
    let backend = TrackedKeyState::with_emitter(|codes, _key_up| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let mut verified = Some(stamp);
    assert_eq!(verified, Some(stamp));
    verified = None;
    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));
}

#[test]
fn failed_preflight_does_not_cache_a_stamp() {
    let backend = TrackedKeyState::new();
    let stamp = TargetStamp {
        hwnd: 0,
        generation: 1,
    };
    let mut verified = Some(TargetStamp {
        hwnd: 123,
        generation: 1,
    });
    assert!(ensure_preflight_for_target(&backend, stamp, &mut verified).is_err());
    assert_eq!(verified, None);
}

#[test]
fn target_change_is_rejected_at_the_final_send_boundary() {
    let target = SessionTarget::new(123, 1);
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    assert!(target_stamp_still_current(&target, stamp));
    target.publish(456);
    assert!(!target_stamp_still_current(&target, stamp));
}

#[test]
fn fresh_target_proof_is_admitted_after_aba_invalidates_old_proof() {
    let target = SessionTarget::new(123, 0);
    let old_proof = TargetStamp {
        hwnd: 123,
        generation: 0,
    };
    target.publish(456);
    target.publish(123);

    assert!(!target_stamp_still_current(&target, old_proof));
    let (hwnd, generation) = target.load_stable().expect("fresh stable target");
    let focus_active = AtomicBool::new(false);
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp { hwnd, generation },
            require_focus: false,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::Allowed
    );
}

#[test]
fn historical_pair_check_false_accepts_a_deterministic_aba_interleaving() {
    let target = AtomicIsize::new(123);
    let generation = AtomicU64::new(0);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 0,
    };

    // Reproduce the old final reader's exact order: generation first, HWND
    // second. The writer completes A -> B -> A between those two loads.
    let observed_generation = generation.load(Ordering::Acquire);
    target.store(456, Ordering::Release);
    generation.fetch_add(1, Ordering::AcqRel);
    target.store(123, Ordering::Release);
    generation.fetch_add(1, Ordering::AcqRel);
    let observed_hwnd = target.load(Ordering::Acquire);

    assert_eq!(observed_generation, expected.generation);
    assert_eq!(observed_hwnd, expected.hwnd);
    assert!(observed_generation == expected.generation && observed_hwnd == expected.hwnd);
    assert_eq!(generation.load(Ordering::Acquire), 2);
}

#[test]
fn focus_admission_uses_the_expected_hwnd_without_reloading_target() {
    let focus_active = AtomicBool::new(false);
    assert!(focus_matches_hwnd(false, &focus_active, 123));
}

#[test]
fn early_focus_gate_is_atomic_only_and_final_admission_queries_once() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let focus_active = AtomicBool::new(true);
    assert!(focus_matches(true, &focus_active));
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    let target = SessionTarget::new(123, 1);
    bind_test_owner_identity(&target, 123, 1, 1);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::Allowed
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn final_down_owner_proof_rejects_recycled_hwnd_owner() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(200)));
    assert_eq!(
        sky_dispatch_win32::focus::foreground_window_owner_matches(123, 100),
        sky_dispatch_win32::focus::ForegroundOwnerMatch::OwnerMismatch
    );

    sky_dispatch_win32::focus::reset_foreground_query_count();
    let target = SessionTarget::new(123, 1);
    bind_test_owner_identity(&target, 123, 1, 100);
    let focus_active = AtomicBool::new(true);
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::Mismatch)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
}

#[test]
fn final_down_owner_proof_has_typed_failures_and_one_query_per_suffix() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    let focus_active = AtomicBool::new(true);

    let missing = SessionTarget::new(123, 1);
    missing.require_owner_identity();
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &missing,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityAbsent)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 0);

    let stale = SessionTarget::new(123, 1);
    bind_test_owner_identity(&stale, 123, 1, 100);
    stale.publish(456);
    stale.publish(123);
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 3,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &stale,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityStale)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    let terminated = SessionTarget::new(123, 1);
    bind_test_owner_identity(&terminated, 123, 1, 100);
    terminated.invalidate_owner_identity();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &terminated,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::ProcessTerminated)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    let healthy = SessionTarget::new(123, 1);
    bind_test_owner_identity(&healthy, 123, 1, 100);
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(None));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &healthy,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::QueryUnavailable)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);

    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(100)));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &healthy,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::Allowed
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);

    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(None));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: false,
            focus_active: &focus_active,
            target: &missing,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::Allowed
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 0);

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn final_down_rechecks_identity_after_the_post_focus_race_hook() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(100)));
    let target = SessionTarget::new(123, 1);
    bind_test_owner_identity(&target, 123, 1, 100);
    let focus_active = AtomicBool::new(true);
    let quit = AtomicBool::new(false);
    let skip = AtomicBool::new(false);
    let panic = AtomicBool::new(false);
    let pause = AtomicBool::new(false);
    let supervisor = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));
    let system_power = super::shared::SystemPowerState::default();
    let hook: super::config::FinalGateRaceHook =
        Arc::new(|_focus, target, _quit, _skip, _panic, _pause, _power| {
            target
                .invalidate_owner_identity_with(super::target::OwnerIdentityFailure::OwnerMismatch);
        });
    let controls = FinalControlSignals {
        quit_requested: &quit,
        skip_requested: &skip,
        panic_requested: &panic,
        desired_pause: &pause,
        supervisor_expired: &supervisor,
        system_power: Some(&system_power),
    };
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: Some(&hook),
            post_focus_control_signals: Some(controls),
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::Mismatch)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn target_aba_rejects_stale_owner_authority_until_full_rebind() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(100)));
    let target = SessionTarget::new(123, 1);
    bind_test_owner_identity(&target, 123, 1, 100);
    assert!(target.publish(456));
    assert!(target.publish(123));
    let expected = TargetStamp {
        hwnd: 123,
        generation: 3,
    };
    let focus_active = AtomicBool::new(true);

    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::OwnerRejected(OwnerAdmissionFailure::IdentityStale)
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 0);

    assert!(target.bind_owner_identity(123, 3, 100));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::Allowed
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn retained_window_authority_closes_once_on_terminal_teardown() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 50_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "identity-close-test-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 70_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "identity-close-test-up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("schedule");
    let options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.set_target_hwnd(123);
    session
        .bind_window_identity_authority(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
        )
        .expect("bind synthetic test authority");
    let before = sky_dispatch_win32::focus::window_identity_authority_drop_count_for_test();

    session.close_window_identity_authority();
    session.close_window_identity_authority();
    assert_eq!(
        sky_dispatch_win32::focus::window_identity_authority_drop_count_for_test(),
        before + 1
    );
    drop(session);
    assert_eq!(
        sky_dispatch_win32::focus::window_identity_authority_drop_count_for_test(),
        before + 1
    );
}

#[test]
fn final_down_admission_rejects_target_change_before_send() {
    let target = SessionTarget::new(456, 2);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: false,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::TargetChanged
    );
}

#[test]
fn final_down_target_admission_checks_target_before_focus() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let target = SessionTarget::new(123, 1);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::FocusLost
    );
}

#[test]
fn final_admission_requires_fresh_foreground_match_and_rechecks_atomic_focus() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    sky_dispatch_win32::focus::reset_foreground_query_count();

    // The supervisor hint is stale-true, but the fresh foreground proof rejects
    // the Down before the sender boundary.
    let focus_active = AtomicBool::new(true);
    let target = SessionTarget::new(123, 1);
    bind_test_owner_identity(&target, 123, 1, 1);
    let final_admission = final_down_target_admission(FinalTargetSignals {
        expected: TargetStamp {
            hwnd: 123,
            generation: 1,
        },
        require_focus: true,
        focus_active: &focus_active,
        target: &target,
        post_focus_race_hook: None,
        post_focus_control_signals: None,
    });
    assert_eq!(final_admission, FinalDownVerdict::FocusLost);
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);

    // Once the producer publishes the loss, the early atomic gate suppresses
    // the boundary without issuing a second foreground query.
    focus_active.store(false, Ordering::Release);
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 123,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::FocusLost
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn prepared_down_final_foreground_proof_has_exact_query_scope() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    let mut matching = ProductionDispatchTestHarness::new_down_only();
    matching.config.focus.require_focus = true;
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let mut matching_stream = matching.build_prepared_stream_for_test();
    let matching_step =
        matching.dispatch_prepared_current_at_lateness_for_test(&mut matching_stream, 10_000);
    assert!(matches!(
        matching_step,
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    let mut mismatch = ProductionDispatchTestHarness::new_down_only();
    mismatch.config.focus.require_focus = true;
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let mismatch_calls = mismatch.configure_send_counter();
    let mut mismatch_stream = mismatch.build_prepared_stream_for_test();
    let mismatch_step =
        mismatch.dispatch_prepared_current_at_lateness_for_test(&mut mismatch_stream, 10_000);
    assert!(!matches!(
        mismatch_step,
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(mismatch_calls.load(Ordering::SeqCst), 0);
    assert_eq!(mismatch.resources.coordinator.cursor, 0);
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);

    let mut disabled = ProductionDispatchTestHarness::new_down_only();
    disabled.config.focus.require_focus = false;
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let mut disabled_stream = disabled.build_prepared_stream_for_test();
    let disabled_step =
        disabled.dispatch_prepared_current_at_lateness_for_test(&mut disabled_stream, 10_000);
    assert!(matches!(
        disabled_step,
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    let mut up_only = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
    up_only.config.focus.require_focus = true;
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let up_only_plan = up_only.plan_current_dispatch();
    let up_only_step = up_only.dispatch_at_plan_target_for_test(&up_only_plan);
    assert!(!matches!(
        up_only_step,
        super::worker::DispatchStep::Terminate(_)
    ));
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn prepared_down_owner_rejections_do_not_send_or_advance_accounting() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    for (owner_override, expected_reason) in [
        (Some(Some(200)), "prepared_down_owner_mismatch"),
        (Some(None), "prepared_down_owner_query_unavailable"),
    ] {
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(owner_override);
        sky_dispatch_win32::focus::reset_foreground_query_count();
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.config.focus.require_focus = true;
        let calls = harness.configure_send_counter();
        let mut stream = harness.build_prepared_stream_for_test();
        let accounting_before = harness.generation_accounting_for_test();
        let cursor_before = harness.resources.coordinator.cursor;

        let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

        assert!(matches!(
            step,
            super::worker::DispatchStep::TerminateStatic(reason) if reason == expected_reason
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(harness.resources.coordinator.cursor, cursor_before);
        assert_eq!(harness.generation_accounting_for_test(), accounting_before);
        assert_eq!(harness.release_obligation_mask_for_test(), 0);
        assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
        assert_eq!(sky_dispatch_win32::focus::owner_query_count(), 1);
        assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
    }
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn prepared_final_gate_buckets_count_early_late_target_and_focus_rejections() {
    let mut early_control = ProductionDispatchTestHarness::new_down_only();
    let early_calls = early_control.configure_send_counter();
    let mut early_stream = early_control.build_prepared_stream_for_test();
    early_control.set_final_gate_race_hook(|_, _, quit, _, _, _, _| {
        quit.store(true, Ordering::Release);
    });
    assert!(matches!(
        early_control.dispatch_prepared_current_at_lateness_for_test(&mut early_stream, 2_000),
        super::worker::DispatchStep::Continue
    ));
    assert_eq!(early_calls.load(Ordering::SeqCst), 0);
    assert_eq!(early_control.local_metrics.final_gate_control_rejections, 1);
    assert_eq!(early_control.local_metrics.final_gate_target_changes, 0);

    let mut late_control = ProductionDispatchTestHarness::new_down_only();
    let late_calls = late_control.configure_send_counter();
    let mut late_stream = late_control.build_prepared_stream_for_test();
    late_control.set_post_focus_revalidation_race_hook(|_, _, _, _, _, pause, _| {
        pause.store(true, Ordering::Release);
    });
    assert!(matches!(
        late_control.dispatch_prepared_current_at_lateness_for_test(&mut late_stream, 2_000),
        super::worker::DispatchStep::Continue
    ));
    assert_eq!(late_calls.load(Ordering::SeqCst), 0);
    assert_eq!(late_control.local_metrics.final_gate_control_rejections, 1);

    let mut target_change = ProductionDispatchTestHarness::new_down_only();
    let target_calls = target_change.configure_send_counter();
    let mut target_stream = target_change.build_prepared_stream_for_test();
    target_change.set_final_gate_race_hook(|_, target, _, _, _, _, _| {
        target.publish(456);
    });
    assert!(matches!(
        target_change.dispatch_prepared_current_at_lateness_for_test(&mut target_stream, 2_000),
        super::worker::DispatchStep::Continue
    ));
    assert_eq!(target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(target_change.local_metrics.final_gate_target_changes, 1);

    let mut focus_loss = ProductionDispatchTestHarness::new_down_only();
    focus_loss.config.focus.require_focus = true;
    focus_loss.runtime.musical_physical_commit_started = true;
    focus_loss.focus_active.store(false, Ordering::Release);
    let focus_calls = focus_loss.configure_send_counter();
    let mut focus_stream = focus_loss.build_prepared_stream_for_test();
    assert!(matches!(
        focus_loss.dispatch_prepared_current_at_lateness_for_test(&mut focus_stream, 2_000),
        super::worker::DispatchStep::Continue
    ));
    assert_eq!(focus_calls.load(Ordering::SeqCst), 0);
    assert_eq!(focus_loss.local_metrics.final_gate_focus_losses, 1);
}

#[test]
fn prepared_and_strict_paths_match_shared_final_down_fault_matrix() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ControlAfterLease {
        Stop,
        Skip,
        Panic,
        Pause,
        Suspend,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum OwnerFailureAfterModifier {
        ProcessTerminated,
        Mismatch,
        QueryUnavailable,
        WindowUnavailable,
        ProcessIdentityMismatch,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fault {
        Allowed,
        TargetAbaAfterModifier,
        FocusLossAfterModifier,
        ForegroundWindowSwitch,
        ForegroundOwnerSwitch,
        ForegroundOwnerUnavailable,
        OwnerFailureAfterModifier(OwnerFailureAfterModifier),
        ModifierHeld,
        ModifierAcquiredAfterCheck,
        LeaseExpired,
        LeaseUnproven,
        LeaseClockFailure,
        ControlAfterLease(ControlAfterLease),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Evidence {
        step_class: u8,
        attempts: u64,
        control_rejections: u64,
        target_rejections: u64,
        focus_rejections: u64,
        lease_rejections: u64,
        cursor: usize,
        active_mask: u16,
        release_obligations: u16,
        lease_samples: u64,
        accounting_changed: bool,
        focus_paused: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DispatchPath {
        PreparedNormal,
        StrictDynamic,
    }

    fn run_case(path: DispatchPath, fault: Fault) -> Evidence {
        let requires_target_owner = matches!(
            fault,
            Fault::FocusLossAfterModifier
                | Fault::ForegroundWindowSwitch
                | Fault::ForegroundOwnerSwitch
                | Fault::ForegroundOwnerUnavailable
                | Fault::OwnerFailureAfterModifier(_)
        );
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.set_strict_timing_for_test(path == DispatchPath::StrictDynamic);
        harness.set_require_focus_for_test(requires_target_owner);
        if requires_target_owner {
            harness.bind_owner_identity_for_test(1);
        }
        if matches!(
            fault,
            Fault::FocusLossAfterModifier | Fault::ForegroundWindowSwitch
        ) {
            harness.runtime.musical_physical_commit_started = true;
        }
        match fault {
            Fault::ForegroundWindowSwitch => {
                sky_dispatch_win32::focus::set_foreground_window_for_test(Some(999));
            }
            Fault::ForegroundOwnerSwitch => {
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(2)));
            }
            Fault::ForegroundOwnerUnavailable => {
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(None));
            }
            _ => {
                sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(1)));
            }
        }
        let calls = harness.configure_send_counter();
        let accounting_before = harness.generation_accounting_for_test();
        let lease_timeout_ticks = harness
            .qpc_duration_from_us_for_test(100)
            .expect("lease timeout ticks")
            .as_u64();
        let modifier_held = Arc::new(AtomicBool::new(fault == Fault::ModifierHeld));
        if matches!(
            fault,
            Fault::ModifierHeld | Fault::ModifierAcquiredAfterCheck
        ) {
            let modifier_held_for_query = Arc::clone(&modifier_held);
            harness.set_modifier_key_state_query_for_test(move |_| {
                if modifier_held_for_query.load(Ordering::Acquire) {
                    i16::MIN
                } else {
                    0
                }
            });
        }

        match fault {
            Fault::TargetAbaAfterModifier => {
                harness.set_post_modifier_revalidation_race_hook(|_, target, _, _, _, _, _| {
                    target.publish(2);
                    target.publish(1);
                });
            }
            Fault::FocusLossAfterModifier => {
                harness.set_post_modifier_revalidation_race_hook(|focus, _, _, _, _, _, _| {
                    focus.store(false, Ordering::Release)
                });
            }
            Fault::OwnerFailureAfterModifier(failure) => {
                harness.set_post_modifier_revalidation_race_hook(
                    move |_, target, _, _, _, _, _| {
                        let failure = match failure {
                            OwnerFailureAfterModifier::ProcessTerminated => {
                                super::target::OwnerIdentityFailure::ProcessTerminated
                            }
                            OwnerFailureAfterModifier::Mismatch => {
                                super::target::OwnerIdentityFailure::OwnerMismatch
                            }
                            OwnerFailureAfterModifier::QueryUnavailable => {
                                super::target::OwnerIdentityFailure::QueryUnavailable
                            }
                            OwnerFailureAfterModifier::WindowUnavailable => {
                                super::target::OwnerIdentityFailure::WindowUnavailable
                            }
                            OwnerFailureAfterModifier::ProcessIdentityMismatch => {
                                super::target::OwnerIdentityFailure::ProcessIdentityMismatch
                            }
                        };
                        target.invalidate_owner_identity_with(failure);
                    },
                );
            }
            Fault::ModifierAcquiredAfterCheck => {
                harness.set_post_modifier_revalidation_race_hook(move |_, _, _, _, _, _, _| {
                    modifier_held.store(true, Ordering::Release)
                });
            }
            Fault::LeaseExpired => {
                harness
                    .set_supervisor_lease_timeout_for_test(100)
                    .expect("lease timeout");
                harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(1_000));
                harness.set_final_lease_qpc_sample_override_for_test(Ok(QpcTicks::from_raw(
                    1_000 + lease_timeout_ticks + 1,
                )));
            }
            Fault::LeaseUnproven => {
                harness
                    .set_supervisor_lease_timeout_for_test(100)
                    .expect("lease timeout");
                harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(0));
                harness.set_final_lease_qpc_sample_override_for_test(Ok(QpcTicks::from_raw(1_001)));
            }
            Fault::LeaseClockFailure => {
                harness
                    .set_supervisor_lease_timeout_for_test(100)
                    .expect("lease timeout");
                harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(1_000));
                harness.set_final_lease_qpc_sample_override_for_test(Err(
                    sky_dispatch_win32::clock::QpcError::CounterUnavailable,
                ));
            }
            Fault::ControlAfterLease(control) => {
                harness
                    .set_supervisor_lease_timeout_for_test(100)
                    .expect("lease timeout");
                harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(1_000));
                harness.set_final_lease_qpc_sample_override_for_test(Ok(QpcTicks::from_raw(
                    1_000 + lease_timeout_ticks,
                )));
                let interrupt = Arc::clone(&harness.interrupt);
                harness.set_post_lease_revalidation_race_hook(
                    move |_, _, quit, skip, panic, pause, system_power| match control {
                        ControlAfterLease::Stop => quit.store(true, Ordering::Release),
                        ControlAfterLease::Skip => skip.store(true, Ordering::Release),
                        ControlAfterLease::Panic => panic.store(true, Ordering::Release),
                        ControlAfterLease::Pause => pause.store(true, Ordering::Release),
                        ControlAfterLease::Suspend => {
                            assert!(system_power.notify(true, interrupt.as_ref()));
                        }
                    },
                );
            }
            Fault::ForegroundWindowSwitch
            | Fault::ForegroundOwnerSwitch
            | Fault::ForegroundOwnerUnavailable
            | Fault::ModifierHeld
            | Fault::Allowed => {}
        }

        let step = match path {
            DispatchPath::PreparedNormal => {
                harness.prepare_prepared_stream_for_test();
                harness.dispatch_prepared_current_at_lateness_without_stream_for_test(10_000)
            }
            DispatchPath::StrictDynamic => {
                harness.advance_playback_time_us(100_000);
                let plan = harness.plan_current_dispatch();
                harness.dispatch_at_plan_target_for_test(&plan)
            }
        };
        let step_class = match step {
            super::worker::DispatchStep::Dispatched => 0,
            super::worker::DispatchStep::Continue => 1,
            super::worker::DispatchStep::Terminate(_)
            | super::worker::DispatchStep::TerminateStatic(_) => 2,
            super::worker::DispatchStep::NoWork => 3,
        };
        Evidence {
            step_class,
            attempts: calls.load(Ordering::SeqCst),
            control_rejections: harness.local_metrics.final_gate_control_rejections,
            target_rejections: harness.local_metrics.final_gate_target_changes,
            focus_rejections: harness.local_metrics.final_gate_focus_losses,
            lease_rejections: harness.local_metrics.final_gate_lease_expirations,
            cursor: harness.resources.coordinator.cursor,
            active_mask: harness.backend_active_mask(),
            release_obligations: harness.release_obligation_mask_for_test(),
            lease_samples: harness.final_lease_qpc_sample_count_for_test(),
            accounting_changed: harness.generation_accounting_for_test() != accounting_before,
            focus_paused: harness
                .resources
                .playback
                .has_pause_reason(PauseReason::Focus),
        }
    }

    struct ForegroundOwnerOverrideReset;
    impl Drop for ForegroundOwnerOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_owner_override_reset = ForegroundOwnerOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(1)));

    let faults = [
        Fault::Allowed,
        Fault::TargetAbaAfterModifier,
        Fault::FocusLossAfterModifier,
        Fault::ForegroundWindowSwitch,
        Fault::ForegroundOwnerSwitch,
        Fault::ForegroundOwnerUnavailable,
        Fault::OwnerFailureAfterModifier(OwnerFailureAfterModifier::ProcessTerminated),
        Fault::OwnerFailureAfterModifier(OwnerFailureAfterModifier::Mismatch),
        Fault::OwnerFailureAfterModifier(OwnerFailureAfterModifier::QueryUnavailable),
        Fault::OwnerFailureAfterModifier(OwnerFailureAfterModifier::WindowUnavailable),
        Fault::OwnerFailureAfterModifier(OwnerFailureAfterModifier::ProcessIdentityMismatch),
        Fault::ModifierHeld,
        Fault::ModifierAcquiredAfterCheck,
        Fault::LeaseExpired,
        Fault::LeaseUnproven,
        Fault::LeaseClockFailure,
        Fault::ControlAfterLease(ControlAfterLease::Stop),
        Fault::ControlAfterLease(ControlAfterLease::Skip),
        Fault::ControlAfterLease(ControlAfterLease::Panic),
        Fault::ControlAfterLease(ControlAfterLease::Pause),
        Fault::ControlAfterLease(ControlAfterLease::Suspend),
    ];

    for fault in faults {
        let prepared = run_case(DispatchPath::PreparedNormal, fault);
        let strict = run_case(DispatchPath::StrictDynamic, fault);
        assert_eq!(prepared, strict, "path divergence for {fault:?}");
        assert_eq!(
            prepared.attempts,
            u64::from(matches!(
                fault,
                Fault::Allowed | Fault::ModifierAcquiredAfterCheck
            )),
            "transport attempts for {fault:?}"
        );
        assert_eq!(
            prepared.step_class,
            match fault {
                Fault::Allowed | Fault::ModifierAcquiredAfterCheck => 0,
                Fault::TargetAbaAfterModifier
                | Fault::FocusLossAfterModifier
                | Fault::ForegroundWindowSwitch
                | Fault::ControlAfterLease(_) => 1,
                _ => 2,
            },
            "final verdict class for {fault:?}"
        );
        assert_eq!(
            prepared.lease_samples,
            u64::from(matches!(
                fault,
                Fault::LeaseExpired
                    | Fault::LeaseUnproven
                    | Fault::LeaseClockFailure
                    | Fault::ControlAfterLease(_)
            )),
            "lease sample count for {fault:?}"
        );
        assert_eq!(
            prepared.focus_paused,
            matches!(
                fault,
                Fault::FocusLossAfterModifier | Fault::ForegroundWindowSwitch
            ),
            "focus pause state for {fault:?}"
        );
    }
}

#[test]
fn stale_published_focus_false_rejects_matching_foreground_before_fresh_query() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::reset_foreground_query_count();

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.config.focus.require_focus = true;
    harness.runtime.musical_physical_commit_started = true;
    harness.focus_active.store(false, Ordering::Release);
    let calls = harness.configure_send_counter();
    let mut stream = harness.build_prepared_stream_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.resources.coordinator.cursor, 0);
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    assert!(stream.current().is_some());
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(PauseReason::Focus)
    );
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn fresh_focus_rejection_preserves_current_frame_and_miss_lifecycle_state() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    sky_dispatch_win32::focus::reset_foreground_query_count();

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.config.focus.require_focus = true;
    harness.runtime.musical_physical_commit_started = true;
    let calls = harness.configure_send_counter();
    let mut stream = harness.build_prepared_stream_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "fresh focus rejection sent input"
    );
    assert_eq!(harness.resources.coordinator.cursor, 0);
    assert!(
        stream.current().is_some(),
        "fresh rejection consumed the frame"
    );
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
    assert_eq!(harness.prepared_normal_backlog_count_for_test(), 0);
    assert_eq!(harness.prepared_up_prefix_recovery_sends_for_test(), 0);
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    assert_eq!(
        harness
            .resources
            .coordinator
            .generation_status_counts()
            .get("dropped_expired")
            .copied()
            .unwrap_or_default(),
        0
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn same_frozen_prepared_frame_is_readmitted_after_normal_focus_restore() {
    use super::test_support::MockSendInputCapture;

    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::reset_foreground_query_count();

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-frame-restore-first".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "same-frame-restore-current".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                // Keep the session live until the test explicitly requests
                // quit after observing the restored Down's completed send.
                scheduled_us: 60_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-frame-restore-first-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 61_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "same-frame-restore-current-up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("same-frame restore schedule");
    let send_call_count = Arc::new(AtomicU64::new(0));
    let send_input_capture = Arc::new(std::sync::Mutex::new(Vec::new()));
    let restore_reconcile_count = Arc::new(AtomicU64::new(0));
    let startup_ordering_hook = Arc::new(StartupOrderingHook::default());
    startup_ordering_hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownAdmission, 2);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&send_input_capture));
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    // The diagnostic profile intentionally bypasses the worker-owned prepared
    // stream. Exercise the normal prepared path while retaining its observer
    // trace so this regression covers the immutable frame and telemetry.
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = true;
    // The production lifecycle remains intact; a zero test grace keeps the
    // frozen current frame inside this deterministic proof's sender window.
    options.focus.focus_restore_grace_us = 0;
    options.startup_ordering_hook = Some(Arc::clone(&startup_ordering_hook));
    options.restore_race_hook = Some({
        let restore_reconcile_count = Arc::clone(&restore_reconcile_count);
        Arc::new(move |_, _| {
            restore_reconcile_count.fetch_add(1, Ordering::Release);
        })
    });

    let session = NativeDispatchSession::new(options).expect("same-frame session admission");
    session.set_target_hwnd(123);
    session.set_focus_hint(true);
    session.arm(0).expect("worker arm without test preroll");

    let first_deadline = Instant::now() + Duration::from_secs(3);
    while send_call_count.load(Ordering::Acquire) < 1 {
        let snapshot = session.snapshot_lite();
        assert!(
            !snapshot.is_finished,
            "first Down finished early: {snapshot:?}"
        );
        assert!(Instant::now() < first_deadline, "first Down did not send");
        session
            .publish_supervisor_progress()
            .expect("heartbeat before same-frame loss");
        std::thread::sleep(Duration::from_millis(1));
    }

    // Stop the worker at the second Down's final-admission checkpoint so the
    // foreground loss cannot race ahead of the frozen frame. The worker has
    // already returned from the first sender and committed that Down.
    assert!(
        startup_ordering_hook.wait_for_startup_checkpoint(Duration::from_secs(3)),
        "current frame did not reach its final-admission checkpoint"
    );
    // Keep the published focus hint true. The next prepared frame is rejected
    // only by the fresh foreground proof, which must leave the frozen cursor
    // and normal miss/expiry counters untouched.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    startup_ordering_hook.release_startup_checkpoint();
    let pause_deadline = Instant::now() + Duration::from_secs(3);
    while !session.snapshot_lite().is_paused {
        let snapshot = session.snapshot_lite();
        assert!(
            !snapshot.is_finished,
            "fresh focus rejection terminated: {snapshot:?}"
        );
        assert!(
            Instant::now() < pause_deadline,
            "fresh focus rejection did not pause: snapshot={:?}",
            session.snapshot(),
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat during fresh focus rejection");
        std::thread::sleep(Duration::from_millis(1));
    }
    let paused = session.snapshot();
    assert!(
        paused.is_paused,
        "fresh focus rejection did not pause: {paused:?}"
    );
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);
    assert_eq!(
        send_input_capture
            .lock()
            .expect("captured paused send inputs")
            .as_slice(),
        [MockSendInputCapture::Packet {
            packet: PhysicalPacket::new(0, 1 << 0),
            status: SendTransactionStatus::Complete,
            confirmed_mask: 1 << 0,
            attempts: 1,
        }],
        "the foreground-rejected current frame must not reach the sender"
    );
    assert_eq!(
        paused
            .generation_status_counts
            .get("dropped_expired")
            .copied()
            .unwrap_or_default(),
        0
    );
    assert_eq!(paused.missed_unobserved_backlog_boundaries, 0);
    assert_eq!(paused.final_sender_window_expirations, 0);

    // The normal focus-restore path invalidates the old Down authorization,
    // reconciles the still-current prepared frame, reacquires a fresh target
    // proof, and only then exits the pause.
    startup_ordering_hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownAdmission, 1);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let restore_deadline = Instant::now() + Duration::from_secs(3);
    while restore_reconcile_count.load(Ordering::Acquire) == 0 {
        let snapshot = session.snapshot_lite();
        assert!(
            !snapshot.is_finished,
            "focus restore terminated: {snapshot:?}"
        );
        assert!(
            Instant::now() < restore_deadline,
            "focus restore reconciliation did not run"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat during normal focus restore");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        startup_ordering_hook.wait_for_startup_checkpoint(Duration::from_secs(3)),
        "restored current frame did not reach its final-admission checkpoint"
    );
    assert_eq!(restore_reconcile_count.load(Ordering::Acquire), 1);
    startup_ordering_hook.release_startup_checkpoint();
    let send_deadline = Instant::now() + Duration::from_secs(3);
    // Restoration first sends the verified cleanup Up, then the resumed
    // authored Down after the global release-spacing barrier expires. The
    // sender-call counter increments at mock call entry, so require the exact
    // completed packet capture as well before issuing quit. The worker commits
    // the synchronous send result before it can observe and finalize that
    // queued quit request.
    loop {
        let restored_packet_completed = send_input_capture
            .lock()
            .expect("captured restored send inputs")
            .iter()
            .any(|capture| match capture {
                MockSendInputCapture::Packet {
                    packet,
                    status,
                    confirmed_mask,
                    attempts,
                } => {
                    *packet == PhysicalPacket::new(0, 1 << 1)
                        && *status == SendTransactionStatus::Complete
                        && *confirmed_mask == 1 << 1
                        && *attempts == 1
                }
                MockSendInputCapture::SingleEmitter { .. } => false,
            });
        let snapshot = session.snapshot_lite();
        if send_call_count.load(Ordering::Acquire) >= 3 && restored_packet_completed {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "same current frame was not readmitted: {snapshot:?}"
        );
        assert!(
            Instant::now() < send_deadline,
            "same current prepared frame did not complete and commit: snapshot={snapshot:?}, calls={}, captures={:?}, down_window={:?}, sender_attempts={}, sender_start_qpc={}, now_qpc={:?}",
            send_call_count.load(Ordering::Acquire),
            send_input_capture
                .lock()
                .expect("captured failed restore send inputs")
                .as_slice(),
            startup_ordering_hook.last_down_window(),
            startup_ordering_hook.down_sender_attempt_count(),
            startup_ordering_hook.last_down_sender_start_qpc_ticks(),
            QpcClock::initialize().and_then(QpcClock::now)
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat after normal focus restore");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(restore_reconcile_count.load(Ordering::Acquire), 1);
    let restored = session.snapshot();
    assert_eq!(
        restored
            .generation_status_counts
            .get("dropped_expired")
            .copied()
            .unwrap_or_default(),
        0
    );
    assert_eq!(restored.missed_unobserved_backlog_boundaries, 0);
    assert_eq!(restored.final_sender_window_expirations, 0);
    assert_eq!(restored.timeline_rebase_count, 0);
    session.quit().expect("quit same-frame restore session");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("same-frame worker join")
    );
    assert!(session.snapshot_lite().is_finished);
    assert_eq!(
        session.terminal_outcome(),
        Some("quit"),
        "focus restore terminal state: {:?}",
        session.snapshot()
    );
    let telemetry: serde_json::Value = serde_json::from_str(
        &session
            .take_telemetry_json()
            .expect("same-frame telemetry JSON"),
    )
    .expect("valid same-frame telemetry JSON");
    let records = telemetry["records"]
        .as_array()
        .expect("same-frame records array");
    assert_eq!(telemetry["accepted"].as_u64(), Some(records.len() as u64));
    assert_eq!(telemetry["dropped"].as_u64(), Some(0));
    assert_eq!(telemetry["observer_queue_dropped"].as_u64(), Some(0));
    assert_eq!(telemetry["truncated"].as_bool(), Some(false));
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            record["trace_record_index"].as_u64(),
            Some(index as u64),
            "observer drain must export the complete ordered trace: {records:#?}"
        );
    }
    let send_inputs = send_input_capture
        .lock()
        .expect("captured send inputs")
        .clone();
    assert_eq!(
        send_call_count.load(Ordering::Acquire),
        4,
        "mock sender calls"
    );
    assert_eq!(
        send_inputs,
        [
            MockSendInputCapture::Packet {
                packet: PhysicalPacket::new(0, 1 << 0),
                status: SendTransactionStatus::Complete,
                confirmed_mask: 1 << 0,
                attempts: 1,
            },
            MockSendInputCapture::SingleEmitter {
                scan_codes: vec![0x15],
                key_up: true,
            },
            MockSendInputCapture::Packet {
                packet: PhysicalPacket::new(0, 1 << 1),
                status: SendTransactionStatus::Complete,
                confirmed_mask: 1 << 1,
                attempts: 1,
            },
            MockSendInputCapture::SingleEmitter {
                scan_codes: vec![0x16],
                key_up: true,
            },
        ],
        "initial authored Down, verified focus cleanup Up, same frozen authored Down, and terminal cleanup Up"
    );
    let authored_records = |source_action_index| {
        records
            .iter()
            .filter(|record| {
                record["source_action_index"].as_u64() == Some(source_action_index)
                    && record["down_mask"].as_u64().unwrap_or_default() != 0
            })
            .collect::<Vec<_>>()
    };
    let current_lifecycle_records = authored_records(1);
    assert!(
        !current_lifecycle_records.is_empty(),
        "missing current frame trace records: {records:#?}"
    );
    for record in &current_lifecycle_records {
        if record["sent_count"].as_u64() == Some(0) {
            assert_eq!(record["send_attempts"].as_u64(), Some(0));
            assert_eq!(
                record["send_status"].as_u64(),
                Some(u64::from(TRACE_SEND_STATUS_NOT_ATTEMPTED))
            );
        }
    }
    let sent_records = |source_action_index| {
        authored_records(source_action_index)
            .into_iter()
            .filter(|record| {
                record["outcome"].as_u64() == Some(trace_outcome_code("sent") as u64)
                    && record["sent_count"].as_u64() == Some(1)
                    && record["send_attempts"].as_u64() == Some(1)
                    && record["send_status"].as_u64()
                        == Some(u64::from(trace_send_status_code(
                            SendTransactionStatus::Complete,
                        )))
            })
            .collect::<Vec<_>>()
    };
    let first_down = sent_records(0);
    assert_eq!(
        first_down.len(),
        1,
        "first authored Down trace: {records:#?}"
    );
    let current_down = sent_records(1);
    assert_eq!(
        current_down.len(),
        1,
        "restored authored Down trace: {records:#?}"
    );
    assert_eq!(
        first_down[0]["compiled_packet_index"].as_u64(),
        Some(0),
        "the first authored packet identity must remain intact"
    );
    assert!(
        current_down[0]["trace_record_index"].as_u64()
            > first_down[0]["trace_record_index"].as_u64(),
        "the restored current packet must follow the first authored packet: {records:#?}"
    );
    assert!(
        current_lifecycle_records
            .iter()
            .filter_map(|record| record["trace_record_index"].as_u64())
            .filter(|index| *index != current_down[0]["trace_record_index"].as_u64().unwrap())
            .all(|index| index < current_down[0]["trace_record_index"].as_u64().unwrap()),
        "the successful readmission trace must follow any earlier no-send observation: {records:#?}"
    );
    assert_eq!(
        current_down[0]["compiled_packet_index"].as_u64(),
        Some(1),
        "the same frozen compiled packet must be readmitted"
    );
}

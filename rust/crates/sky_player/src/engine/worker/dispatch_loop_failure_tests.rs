fn sender_failure_record_for_test(
    key_count: usize,
    status: SendTransactionStatus,
    inserted_count: u8,
) -> String {
    let mut harness = if key_count == 1 {
        ProductionDispatchTestHarness::new_down_only()
    } else {
        ProductionDispatchTestHarness::new_down_chord(key_count)
    };
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = (status != SendTransactionStatus::ClockFailureAfterSend)
            .then_some(started);
        let confirmed_mask = if inserted_count == 0 {
            0
        } else {
            packet.down_mask & packet.down_mask.wrapping_neg()
        };
        SendTransactionOutcome {
            status,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask,
                skipped_mask: 0,
                first_inserted: inserted_count,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: match status {
                    SendTransactionStatus::ZeroProgress => PacketRetryReason::ZeroProgress,
                    SendTransactionStatus::IntegrityLost => {
                        PacketRetryReason::PartialProgress {
                            inserted_count,
                        }
                    }
                    _ => PacketRetryReason::None,
                },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: completed,
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::StagedDispatchOutcome::Exit
    ));
    let record = harness
        .runtime
        .terminal_error
        .as_ref()
        .expect("terminal sender error record")
        .clone();
    let cleanup = harness.finalize_for_test();
    assert_eq!(cleanup.active_mask, 0);
    assert_eq!(cleanup.possibly_active_mask, 0);
    record
}

#[test]
fn failed_sender_paths_retain_bounded_attempt_and_clock_evidence() {
    let zero = sender_failure_record_for_test(1, SendTransactionStatus::ZeroProgress, 0);
    assert!(
        zero.contains("requested=1 inserted=0")
            && zero.contains("attempts=1 status=ZeroProgress"),
        "{zero}"
    );
    assert!(zero.contains("retry_reason=ZeroProgress win32_error=Some(5)"));
    assert!(zero.contains("pre_call_qpc=") && zero.contains("completion_qpc="));

    let partial = sender_failure_record_for_test(2, SendTransactionStatus::IntegrityLost, 1);
    assert!(partial.contains(
        "source_action=0 source_chord=0 up_mask=0x0000 down_mask=0x0003"
    ));
    assert!(partial.contains("requested=2 inserted=1"));
    assert!(partial.contains("attempts=1 status=IntegrityLost"));
    assert!(partial.contains("retry_reason=PartialProgress { inserted_count: 1 }"));

    let clock_failure = sender_failure_record_for_test(
        1,
        SendTransactionStatus::ClockFailureAfterSend,
        1,
    );
    assert!(clock_failure.contains("status=ClockFailureAfterSend"));
    assert!(clock_failure.contains("completion_qpc=unavailable"));
}

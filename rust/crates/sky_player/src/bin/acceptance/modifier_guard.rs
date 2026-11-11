use super::{
    EventRecord, PhysicalExpectation, PreTerminalResult, RunArgs, Scenario, Verdict,
    reconcile_event_sequence, write_report,
};
use sky_dispatch_win32::input::InstrumentKeyProfileSpec;
use sky_player::engine::{NativeSessionOptions, StartupOrderingHook};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Default)]
pub(super) struct ModifierHeldAfterOwnedLatch(Arc<AtomicBool>);

impl ModifierHeldAfterOwnedLatch {
    pub(super) fn is_armed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(super) fn confirm_first_down_after_commit(
        &self,
        checkpoint_reached: bool,
        sink_result: &PreTerminalResult,
        sink_events: &[EventRecord],
        expected_first_down: PhysicalExpectation,
    ) -> Result<(), String> {
        if !checkpoint_reached {
            return Err("FirstDownCommitted checkpoint was not reached".into());
        }
        if !matches!(sink_result, PreTerminalResult::Satisfied) {
            return Err(format!("ReceiveOnly first-Down proof was not satisfied: {sink_result:?}"));
        }
        reconcile_event_sequence(sink_events, &[("key_press", expected_first_down)])?;
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "modifier-held-after-owned fixture was already armed".to_string())?;
        Ok(())
    }
}

pub(super) fn requires_test_query_seam(scenario: Scenario) -> bool {
    matches!(
        scenario,
        Scenario::ModifierHeldFinalBoundary | Scenario::ModifierHeldAfterOwned
    )
}

pub(super) fn unsupported_query_seam_reason(scenario: Scenario) -> Option<&'static str> {
    (requires_test_query_seam(scenario) && !cfg!(feature = "test-support"))
        .then_some("modifier-held native cases require the test-support query seam")
}

pub(super) fn query_for_scenario(
    scenario: Scenario,
    query_count: Arc<AtomicU64>,
    after_owned_latch: ModifierHeldAfterOwnedLatch,
) -> Option<Arc<dyn Fn(i32) -> i16 + Send + Sync>> {
    #[cfg(not(feature = "test-support"))]
    let _ = (&scenario, &query_count, &after_owned_latch);
    #[cfg(feature = "test-support")]
    {
        match scenario {
            Scenario::ModifierHeldFinalBoundary => Some(Arc::new(move |virtual_key| {
                query_count.fetch_add(1, Ordering::SeqCst);
                if virtual_key == 0x10 { i16::MIN } else { 0 }
            })),
            Scenario::ModifierHeldAfterOwned => Some(Arc::new(move |virtual_key| {
                query_count.fetch_add(1, Ordering::SeqCst);
                if after_owned_latch.is_armed() && virtual_key == 0x10 {
                    i16::MIN
                } else {
                    0
                }
            })),
            _ => None,
        }
    }
    #[cfg(not(feature = "test-support"))]
    None
}

pub(super) fn native_session_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    profile: Option<InstrumentKeyProfileSpec>,
    timing_margin_us: u64,
    scenario: Scenario,
    game_fps: u16,
) -> (NativeSessionOptions, Arc<AtomicU64>, ModifierHeldAfterOwnedLatch) {
    let query_count = Arc::new(AtomicU64::new(0));
    let after_owned_latch = ModifierHeldAfterOwnedLatch::default();
    let options = super::scenarios::production_options_for_fps(
        schedule,
        profile,
        timing_margin_us,
        scenario,
        Arc::clone(&query_count),
        after_owned_latch.clone(),
        game_fps,
    );
    (options, query_count, after_owned_latch)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn arm_after_owned_down(
    args: &RunArgs,
    session: &sky_player::engine::NativeDispatchSession,
    hook: &StartupOrderingHook,
    sink_cursor: super::LogCursor,
    fresh_sink: &super::ReadyRecord,
    first_down: PhysicalExpectation,
    modifier_held_latch: &ModifierHeldAfterOwnedLatch,
    modifier_query_count: &AtomicU64,
) -> Result<(), i32> {
    let checkpoint_reached = hook.wait_for_startup_checkpoint(std::time::Duration::from_secs(5));
    let sink_result = if checkpoint_reached {
        super::wait_for_sink_events(
            &args.sink_events,
            sink_cursor,
            fresh_sink,
            &[first_down],
            &[],
        )
    } else {
        PreTerminalResult::IncompleteTimeout { complete: false }
    };
    let sink_window_at_checkpoint = super::read_log_window_status(
        &args.sink_events,
        sink_cursor,
        fresh_sink,
        super::RECEIVE_ONLY_ROLE,
    );
    let confirmation = match sink_window_at_checkpoint.as_ref() {
        Ok(window) => modifier_held_latch.confirm_first_down_after_commit(
            checkpoint_reached,
            &sink_result,
            &window.events,
            first_down,
        ),
        Err(error) => Err(format!("could not read committed sink event window: {error}")),
    };
    if let Err(error) = confirmation {
        let quit_result = session
            .quit()
            .map(|()| "ok".to_string())
            .unwrap_or_else(|quit_error| format!("error: {quit_error}"));
        hook.release_startup_checkpoint();
        let join_result = match session.join(std::time::Duration::from_secs(5)) {
            Ok(joined) => serde_json::json!({"completed": joined, "error": null}),
            Err(join_error) => serde_json::json!({"completed": false, "error": join_error}),
        };
        let snapshot = session.snapshot();
        let sink_window_after_cleanup = super::read_log_window_status(
            &args.sink_events,
            sink_cursor,
            fresh_sink,
            super::RECEIVE_ONLY_ROLE,
        );
        let verdict = if !checkpoint_reached
            || matches!(
                &sink_result,
                PreTerminalResult::ObservationInconclusive(_)
                    | PreTerminalResult::IncompleteTimeout { complete: false }
            )
            || sink_window_at_checkpoint.is_err()
        {
            Verdict::Inconclusive
        } else {
            Verdict::Fail
        };
        return Err(super::write_report(
            args,
            verdict,
            &error,
            serde_json::json!({
                "first_down_committed_checkpoint_reached": checkpoint_reached,
                "receive_only_first_down_result": format!("{sink_result:?}"),
                "receive_only_window_at_checkpoint": sink_window_at_checkpoint.as_ref().ok().map(|window| serde_json::json!({"complete": window.complete, "events": window.events})),
                "receive_only_window_after_cleanup": sink_window_after_cleanup.as_ref().ok().map(|window| serde_json::json!({"complete": window.complete, "events": window.events})),
                "modifier_held_fixture_armed": modifier_held_latch.is_armed(),
                "modifier_guard_query_count": modifier_query_count.load(Ordering::SeqCst),
                "cleanup": {
                    "quit_request_result": quit_result,
                    "join_result": join_result,
                    "worker_snapshot": super::snapshot_json(&snapshot),
                }
            }),
        ));
    }
    hook.release_startup_checkpoint();
    Ok(())
}

pub(super) fn attach_query_evidence(
    object: &mut serde_json::Map<String, serde_json::Value>,
    scenario: Scenario,
    query_count: u64,
) {
    object.insert("modifier_guard_query_count".to_string(), serde_json::json!(query_count));
    object.insert(
        "modifier_guard_query_source".to_string(),
        serde_json::json!(if requires_test_query_seam(scenario) {
            "deterministic_test_support"
        } else {
            "production_GetAsyncKeyState"
        }),
    );
}

pub(super) fn validate_rejection(
    scenario: Scenario,
    terminal_error: Option<&str>,
    query_count: u64,
    sink_events: &[EventRecord],
    profile: Option<&InstrumentKeyProfileSpec>,
    cleanup_attempted_mask: u64,
    cleanup_attempts: u64,
) -> Result<(), String> {
    let _ = query_count;
    if terminal_error.is_none_or(|error| !error.contains("physical_modifier_held")) {
        return Err("modifier guard did not produce the expected terminal result".into());
    }

    if scenario == Scenario::ModifierHeldFinalBoundary {
        if !sink_events.is_empty() || cleanup_attempted_mask != 0 || cleanup_attempts != 0 {
            return Err("first-Down modifier rejection emitted gameplay or manufactured cleanup".into());
        }
        return Ok(());
    }

    let first = super::expected_physical_keys(profile, &[0])[0];
    let expected = [("key_press", first), ("key_release", first)];
    super::reconcile_event_sequence(sink_events, &expected)?;
    if cleanup_attempted_mask != 1 || cleanup_attempts != 1 {
        return Err("modifier rejection cleanup was not limited to the prior owned key".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn terminal_result(
    args: &RunArgs,
    details: &mut Option<serde_json::Value>,
    scenario: Scenario,
    terminal_error: Option<&str>,
    query_count: u64,
    sink_events: &[EventRecord],
    profile: Option<&InstrumentKeyProfileSpec>,
    cleanup_attempted_mask: u64,
    cleanup_attempts: u64,
) -> Option<i32> {
    if !requires_test_query_seam(scenario) {
        return None;
    }
    let evidence = details.take().expect("modifier result evidence is available");
    match validate_rejection(
        scenario,
        terminal_error,
        query_count,
        sink_events,
        profile,
        cleanup_attempted_mask,
        cleanup_attempts,
    ) {
        Ok(()) => Some(write_report(
            args,
            Verdict::Pass,
            "modifier guard final-boundary evidence passed",
            evidence,
        )),
        Err(error) => Some(write_report(args, Verdict::Fail, &error, evidence)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_down_event(kind: &str, scan_code: u16) -> EventRecord {
        EventRecord {
            schema_version: super::super::EVENT_SCHEMA_VERSION,
            run_id: "modifier-fixture-test".into(),
            role: super::super::RECEIVE_ONLY_ROLE.into(),
            event_log_id: "identity-bound-test-stream".into(),
            sequence: 1,
            kind: kind.into(),
            scan_code,
            extended: false,
            virtual_key: 0x59,
            message: if kind == "key_press" { 0x0100 } else { 0x0101 },
            observed_utc: "2026-11-11T00:00:00Z".into(),
        }
    }

    fn first_down() -> PhysicalExpectation {
        PhysicalExpectation {
            scan_code: 0x15,
            extended: false,
        }
    }

    #[test]
    fn modifier_fixture_arms_only_after_committed_identity_bound_first_down() {
        let latch = ModifierHeldAfterOwnedLatch::default();
        let events = [first_down_event("key_press", 0x15)];
        assert!(!latch.is_armed());
        latch
            .confirm_first_down_after_commit(
                true,
                &PreTerminalResult::Satisfied,
                &events,
                first_down(),
            )
            .expect("committed sink Down proof arms the fixture");
        assert!(latch.is_armed());
    }

    #[test]
    fn modifier_fixture_rejects_a_premature_hold_before_first_down_commit() {
        let latch = ModifierHeldAfterOwnedLatch::default();
        let events = [first_down_event("key_press", 0x15)];
        assert!(
            latch
                .confirm_first_down_after_commit(
                    false,
                    &PreTerminalResult::Satisfied,
                    &events,
                    first_down(),
                )
                .is_err()
        );
        assert!(!latch.is_armed());
    }

    #[test]
    fn modifier_fixture_rejects_missing_or_wrong_receive_only_first_down() {
        let missing = ModifierHeldAfterOwnedLatch::default();
        assert!(
            missing
                .confirm_first_down_after_commit(
                    true,
                    &PreTerminalResult::IncompleteTimeout { complete: true },
                    &[],
                    first_down(),
                )
                .is_err()
        );
        assert!(!missing.is_armed());

        let wrong = ModifierHeldAfterOwnedLatch::default();
        let events = [first_down_event("key_release", 0x15)];
        assert!(
            wrong
                .confirm_first_down_after_commit(
                    true,
                    &PreTerminalResult::Satisfied,
                    &events,
                    first_down(),
                )
                .is_err()
        );
        assert!(!wrong.is_armed());
    }

    #[test]
    fn modifier_fixture_checkpoint_timeout_keeps_shift_unarmed() {
        let hook = sky_player::engine::StartupOrderingHook::default();
        let reached = hook.wait_for_startup_checkpoint(std::time::Duration::ZERO);
        let latch = ModifierHeldAfterOwnedLatch::default();
        assert!(!reached);
        assert!(
            latch
                .confirm_first_down_after_commit(
                    reached,
                    &PreTerminalResult::IncompleteTimeout { complete: false },
                    &[],
                    first_down(),
                )
                .is_err()
        );
        assert!(!latch.is_armed());
    }

    #[test]
    fn modifier_rejection_fails_when_owned_key_cleanup_is_not_exactly_once() {
        let first = first_down();
        let events = [
            first_down_event("key_press", first.scan_code),
            first_down_event("key_release", first.scan_code),
        ];
        let result = validate_rejection(
            Scenario::ModifierHeldAfterOwned,
            Some("physical_modifier_held"),
            10,
            &events,
            None,
            1,
            0,
        );
        assert!(result.is_err());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn modifier_query_count_is_diagnostic_and_not_the_hold_trigger() {
        let query_count = Arc::new(AtomicU64::new(0));
        let latch = ModifierHeldAfterOwnedLatch::default();
        let query = query_for_scenario(
            Scenario::ModifierHeldAfterOwned,
            Arc::clone(&query_count),
            latch.clone(),
        )
        .expect("modifier test-support query");
        for _ in 0..32 {
            assert_eq!(query(0x10), 0);
        }
        assert_eq!(query_count.load(Ordering::SeqCst), 32);
        assert!(!latch.is_armed());
    }
}

use super::{
    NativeDispatchSession, RunArgs, Scenario, Value,
    Verdict, wait_for_focus_pause, wait_for_resume, wait_for_startup_armed, write_report,
};
use serde_json::{Map, json};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcClock;
use sky_player::engine::{StartupCheckpoint, StartupOrderingHook};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub(super) struct AcceptanceHookCleanup(Option<Arc<StartupOrderingHook>>);

impl AcceptanceHookCleanup {
    pub(super) fn new(hook: Option<Arc<StartupOrderingHook>>) -> Self {
        Self(hook)
    }
}

impl Drop for AcceptanceHookCleanup {
    fn drop(&mut self) {
        if let Some(hook) = self.0.as_ref() {
            hook.release_startup_checkpoint();
            hook.release_completed_up_through(u64::MAX);
            hook.release_watchdog();
        }
    }
}

pub(super) fn hook_for_scenario(scenario: Scenario) -> Option<Arc<StartupOrderingHook>> {
    match scenario {
        Scenario::PauseResumeReleaseFloor => {
            let hook = Arc::new(StartupOrderingHook::default());
            hook.enable_release_floor_control(10_000);
            Some(hook)
        }
        Scenario::LeaseAgeWatchdogDelayed => {
            let hook = Arc::new(StartupOrderingHook::default());
            hook.hold_watchdog();
            Some(hook)
        }
        Scenario::OwnerProcessTermination => {
            let hook = Arc::new(StartupOrderingHook::default());
            hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownAdmission, 1);
            Some(hook)
        }
        Scenario::ModifierHeldAfterOwned => {
            let hook = Arc::new(StartupOrderingHook::default());
            hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
            Some(hook)
        }
        _ => None,
    }
}

pub(super) fn run_release_floor_case(
    args: &RunArgs,
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<Vec<Value>, i32> {
    if !wait_for_startup_armed(session) {
        return Err(write_release_floor_failure(
            args,
            session,
            hook,
            ReleaseFloorFailure::initial(
                "startup_armed",
                "production session did not reach Armed publication before release-floor cycles",
            ),
        ));
    }
    match run_release_floor_cycles(session, hook) {
        Ok(cycles) => Ok(cycles),
        Err(failure) => Err(write_release_floor_failure(args, session, hook, failure)),
    }
}

fn write_release_floor_failure(
    args: &RunArgs,
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
    failure: ReleaseFloorFailure,
) -> i32 {
    let quit_request_result = session
        .quit()
        .map(|()| "ok".to_string())
        .unwrap_or_else(|error| format!("error: {error}"));
    hook.release_startup_checkpoint();
    hook.release_completed_up_through(u64::MAX);
    let join_result = match session.join(Duration::from_secs(5)) {
        Ok(joined) => json!({"completed": joined, "error": null}),
        Err(error) => json!({"completed": false, "error": error}),
    };
    let snapshot = session.snapshot();
    let ReleaseFloorFailure {
        reason,
        stage,
        verdict,
        completed_cycles,
        partial_cycle,
    } = failure;
    let details = json!({
        "release_floor_cycles": completed_cycles,
        "partial_release_floor_cycle": partial_cycle,
        "release_floor_failure": {
            "stage": stage,
            "reason": reason,
            "quit_request_result": quit_request_result,
            "join_result": join_result,
            "worker_snapshot": super::snapshot_json(&snapshot),
        }
    });
    write_report(args, verdict, &reason, details)
}

fn wait_for_up_count(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
    required_count: u64,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if hook.completed_up_count() >= required_count {
            return true;
        }
        if session.snapshot().is_finished || Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

struct PartialReleaseFloorCycle {
    fields: Map<String, Value>,
}

impl PartialReleaseFloorCycle {
    fn new(cycle: u64, release_gap_us: u64, release_gap_ticks: u64) -> Self {
        let mut fields = Map::new();
        fields.insert("cycle".into(), json!(cycle));
        fields.insert("stage".into(), json!("waiting_for_successful_up"));
        fields.insert("successful_up_completion_observed".into(), json!(false));
        for field in [
            "pre_pause_window_sequence",
            "pre_pause_authored_target_qpc_ticks",
            "pre_pause_due_admission_checkpoint_qpc_ticks",
            "pause_request_qpc_ticks",
            "pause_acknowledgment_qpc_ticks",
            "post_cleanup_qpc_ticks",
            "release_barrier_qpc_ticks",
            "resume_target_qpc_ticks",
            "resume_request_qpc_ticks",
            "resume_window_baseline_sequence",
            "resumed_window_sequence",
            "resumed_authored_down_target_qpc_ticks",
            "resumed_down_not_before_qpc_ticks",
            "authored_due_admission_checkpoint_qpc_ticks",
            "down_sender_start_qpc_ticks",
            "down_sender_attempt_count",
            "latest_window_sequence_at_observation",
            "latest_window_authored_target_qpc_ticks",
            "latest_window_not_before_qpc_ticks",
            "latest_window_observation_qpc_ticks",
        ] {
            fields.insert(field.into(), Value::Null);
        }
        fields.insert("min_release_gap_us".into(), json!(release_gap_us));
        fields.insert("min_release_gap_ticks".into(), json!(release_gap_ticks));
        fields.insert("floor_passed".into(), json!(false));
        Self { fields }
    }

    fn set(&mut self, field: &str, value: Value) {
        self.fields.insert(field.to_string(), value);
    }

    fn stage(&mut self, stage: &str) {
        self.set("stage", json!(stage));
    }

    fn into_value(self) -> Value {
        Value::Object(self.fields)
    }
}

struct ReleaseFloorFailure {
    reason: String,
    stage: String,
    verdict: Verdict,
    completed_cycles: Vec<Value>,
    partial_cycle: Option<Value>,
}

fn resumed_staged_window_is_due_before_release_floor(
    baseline_sequence: u64,
    observed_sequence: u64,
    authored_target_qpc: u64,
    packet_not_before_qpc: u64,
    now_qpc: u64,
    release_barrier_qpc: u64,
) -> bool {
    observed_sequence > baseline_sequence
        && authored_target_qpc != 0
        && packet_not_before_qpc >= release_barrier_qpc
        && authored_target_qpc <= now_qpc
        && release_floor_blocks_down(now_qpc, release_barrier_qpc)
}

fn release_floor_blocks_down(now_qpc: u64, release_barrier_qpc: u64) -> bool {
    now_qpc < release_barrier_qpc
}

fn down_sender_attempt_count_unchanged(current_attempts: u64, prior_attempts: u64) -> bool {
    current_attempts == prior_attempts
}

impl ReleaseFloorFailure {
    fn new(
        reason: impl Into<String>,
        stage: impl Into<String>,
        completed_cycles: &[Value],
        partial_cycle: Option<Value>,
    ) -> Self {
        let reason = reason.into();
        let verdict = if reason.starts_with("CASE-NOT-EXERCISED:")
            || reason.starts_with("release-floor oracle failed")
        {
            Verdict::Fail
        } else {
            Verdict::Inconclusive
        };
        Self {
            reason,
            stage: stage.into(),
            verdict,
            completed_cycles: completed_cycles.to_vec(),
            partial_cycle,
        }
    }

    fn initial(stage: &str, reason: &str) -> Self {
        Self::new(reason, stage, &[], None)
    }
}

fn run_release_floor_cycles(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<Vec<Value>, ReleaseFloorFailure> {
    let qpc = match QpcClock::initialize() {
        Ok(qpc) => qpc,
        Err(error) => {
            return Err(ReleaseFloorFailure::initial(
                "qpc_initialization",
                &format!("QPC initialization failed: {error:?}"),
            ));
        }
    };
    let release_gap_us = session.effective_min_release_gap_us();
    let release_gap_ticks = match qpc.duration_from_us(release_gap_us) {
        Ok(ticks) => ticks.as_u64(),
        Err(error) => {
            return Err(ReleaseFloorFailure::initial(
                "release_gap_conversion",
                &format!("effective release-gap conversion failed: {error:?}"),
            ));
        }
    };
    let resume_delay_ticks = match qpc.duration_from_us(2_000) {
        Ok(ticks) => ticks.as_u64(),
        Err(error) => {
            return Err(ReleaseFloorFailure::initial(
                "resume_delay_conversion",
                &format!("resume delay conversion failed: {error:?}"),
            ));
        }
    };
    let mut evidence = Vec::with_capacity(20);

    for cycle in 1..=20_u64 {
        let mut partial = PartialReleaseFloorCycle::new(
            cycle,
            release_gap_us,
            release_gap_ticks,
        );
        macro_rules! fail_cycle {
            ($stage:expr, $reason:expr) => {
                return Err(ReleaseFloorFailure::new(
                    $reason,
                    $stage,
                    &evidence,
                Some(partial.into_value()),
                ))
            };
        }
        if !wait_for_up_count(session, hook, cycle) {
            fail_cycle!("waiting_for_successful_up", format!(
                "CASE-NOT-EXERCISED: actual successful Up completion {cycle} was not observed"
            ));
        }
        partial.set("successful_up_completion_observed", json!(true));
        // Hold the worker after the next authored Down target is due. This
        // makes the precondition independent of machine scheduling: the
        // authored target is already in the past before cleanup establishes
        // the new verified-completion + G release barrier.
        hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownAdmission, 1);
        hook.release_completed_up_through(cycle);
        if !hook.wait_for_startup_checkpoint(Duration::from_secs(5)) {
            hook.release_startup_checkpoint();
            hook.release_completed_up_through(u64::MAX);
            fail_cycle!("waiting_for_pre_pause_admission_checkpoint", format!(
                "CASE-NOT-EXERCISED: next authored Down did not reach its admission checkpoint at cycle {cycle}"
            ));
        }
        partial.stage("pre_pause_admission_checkpoint");
        let (pre_pause_window_sequence, pre_pause_authored_target_qpc, _) =
            hook.last_down_window_snapshot();
        partial.set("pre_pause_window_sequence", json!(pre_pause_window_sequence));
        partial.set(
            "pre_pause_authored_target_qpc_ticks",
            json!(pre_pause_authored_target_qpc),
        );
        let pre_pause_due_checkpoint_qpc = match qpc.now() {
            Ok(ticks) => ticks.as_u64(),
            Err(error) => {
                hook.release_startup_checkpoint();
                hook.release_completed_up_through(u64::MAX);
                fail_cycle!("pre_pause_due_checkpoint_qpc", format!(
                    "QPC read failed at due Down checkpoint in cycle {cycle}: {error:?}"
                ));
            }
        };
        partial.set(
            "pre_pause_due_admission_checkpoint_qpc_ticks",
            json!(pre_pause_due_checkpoint_qpc),
        );
        if pre_pause_authored_target_qpc == 0
            || pre_pause_authored_target_qpc > pre_pause_due_checkpoint_qpc
        {
            hook.release_startup_checkpoint();
            hook.release_completed_up_through(u64::MAX);
            fail_cycle!("pre_pause_due_oracle", format!(
                "CASE-NOT-EXERCISED: authored target was not due at the pre-pause admission checkpoint in cycle {cycle} (authored={pre_pause_authored_target_qpc}, checkpoint={pre_pause_due_checkpoint_qpc})"
            ));
        }
        partial.stage("requesting_pause");
        let pause_request_qpc = match qpc.now() {
            Ok(ticks) => ticks.as_u64(),
            Err(error) => {
                hook.release_startup_checkpoint();
                hook.release_completed_up_through(u64::MAX);
                fail_cycle!("pause_request_qpc", format!("QPC read failed before pause request at cycle {cycle}: {error:?}"));
            }
        };
        partial.set("pause_request_qpc_ticks", json!(pause_request_qpc));
        hook.clear_pause_ack_qpc();
        if let Err(error) = session.pause() {
            hook.release_startup_checkpoint();
            hook.release_completed_up_through(u64::MAX);
            fail_cycle!("pause_request", format!("manual pause request failed at cycle {cycle}: {error}"));
        }
        hook.release_startup_checkpoint();
        partial.stage("waiting_for_pause_acknowledgment");
        if !wait_for_focus_pause(session) {
            fail_cycle!("waiting_for_pause_acknowledgment", format!("manual pause did not acknowledge at cycle {cycle}"));
        }

        let ack_deadline = Instant::now() + Duration::from_secs(1);
        let acknowledgment_qpc = loop {
            let ticks = hook.pause_ack_qpc_ticks();
            if ticks != 0 {
                break ticks;
            }
            if session.snapshot().is_finished || Instant::now() >= ack_deadline {
                fail_cycle!("pause_acknowledgment_qpc", format!(
                    "manual pause acknowledgment QPC was not captured at cycle {cycle}"
                ));
            }
            thread::sleep(Duration::from_micros(100));
        };
        partial.set("pause_acknowledgment_qpc_ticks", json!(acknowledgment_qpc));
        let cleanup_qpc = hook.resume_cleanup_qpc_ticks();
        if cleanup_qpc == 0 || acknowledgment_qpc < cleanup_qpc {
            fail_cycle!("post_cleanup_qpc", format!(
                "post-cleanup QPC provenance is missing or unordered at cycle {cycle}"
            ));
        }
        partial.set("post_cleanup_qpc_ticks", json!(cleanup_qpc));
        let barrier_qpc = match cleanup_qpc.checked_add(release_gap_ticks) {
            Some(barrier) => barrier,
            None => fail_cycle!(
                "release_barrier_qpc",
                format!("resume barrier overflow at cycle {cycle}")
            ),
        };
        partial.set("release_barrier_qpc_ticks", json!(barrier_qpc));

        let resume_target = match acknowledgment_qpc.checked_add(resume_delay_ticks) {
            Some(target) => target,
            None => fail_cycle!(
                "resume_target_qpc",
                format!("resume target overflow at cycle {cycle}")
            ),
        };
        partial.set("resume_target_qpc_ticks", json!(resume_target));
        partial.stage("waiting_for_resume_target");
        loop {
            let now = match qpc.now() {
                Ok(ticks) => ticks.as_u64(),
                Err(error) => fail_cycle!(
                    "pre_resume_qpc",
                    format!("QPC read failed before resume at cycle {cycle}: {error:?}")
                ),
            };
            if now >= resume_target {
                break;
            }
            let remaining_us = qpc
                .duration_to_us(DurationTicks::from_raw(resume_target - now))
                .unwrap_or(1)
                .clamp(1, 500);
            thread::sleep(Duration::from_micros(remaining_us));
        }
        let (resume_window_baseline_sequence, _, _) = hook.last_down_window_snapshot();
        partial.set(
            "resume_window_baseline_sequence",
            json!(resume_window_baseline_sequence),
        );
        partial.stage("requesting_resume");
        if let Err(error) = session.resume() {
            fail_cycle!("resume_request", format!("manual resume request failed at cycle {cycle}: {error}"));
        }
        let resume_request_qpc = match qpc.now() {
            Ok(ticks) => ticks.as_u64(),
            Err(error) => fail_cycle!(
                "resume_request_qpc",
                format!("QPC read failed after resume at cycle {cycle}: {error:?}")
            ),
        };
        partial.set("resume_request_qpc_ticks", json!(resume_request_qpc));
        if !release_floor_blocks_down(resume_request_qpc, barrier_qpc) {
            fail_cycle!("resume_request_floor_oracle", format!(
                "CASE-NOT-EXERCISED: resume request reached/passed the cleanup release floor at cycle {cycle}"
            ));
        }
        partial.stage("waiting_for_resume_acknowledgment");
        if !wait_for_resume(session) {
            fail_cycle!("waiting_for_resume_acknowledgment", format!("manual resume did not commit at cycle {cycle}"));
        }

        partial.stage("waiting_for_resumed_authored_target_due_before_barrier");
        let due_deadline = Instant::now() + Duration::from_secs(5);
        let authored_due_checkpoint_qpc = loop {
            let (window_sequence, window_target_qpc, window_not_before_qpc) =
                hook.last_down_window_snapshot();
            let now_qpc = match qpc.now() {
                Ok(ticks) => ticks.as_u64(),
                Err(error) => fail_cycle!(
                    "resumed_due_qpc",
                    format!("QPC read failed while proving resumed Down target at cycle {cycle}: {error:?}")
                ),
            };
            partial.set("latest_window_sequence_at_observation", json!(window_sequence));
            partial.set(
                "latest_window_authored_target_qpc_ticks",
                json!(window_target_qpc),
            );
            partial.set(
                "latest_window_not_before_qpc_ticks",
                json!(window_not_before_qpc),
            );
            partial.set("latest_window_observation_qpc_ticks", json!(now_qpc));
            if window_sequence > resume_window_baseline_sequence {
                partial.set("resumed_window_sequence", json!(window_sequence));
                partial.set(
                    "resumed_authored_down_target_qpc_ticks",
                    json!(window_target_qpc),
                );
                partial.set(
                    "resumed_down_not_before_qpc_ticks",
                    json!(window_not_before_qpc),
                );
            }
            if !release_floor_blocks_down(now_qpc, barrier_qpc) {
                fail_cycle!("resumed_due_before_barrier", format!(
                    "CASE-NOT-EXERCISED: resumed authored target was not observed due before release floor at cycle {cycle} (authored={}, barrier={barrier_qpc}, now={now_qpc})",
                    if window_sequence > resume_window_baseline_sequence { window_target_qpc } else { 0 }
                ));
            }
            if resumed_staged_window_is_due_before_release_floor(
                resume_window_baseline_sequence,
                window_sequence,
                window_target_qpc,
                window_not_before_qpc,
                now_qpc,
                barrier_qpc,
            )
            {
                if !down_sender_attempt_count_unchanged(
                    hook.down_sender_attempt_count(),
                    cycle,
                ) {
                    partial.set("down_sender_attempt_count", json!(hook.down_sender_attempt_count()));
                    fail_cycle!("sender_attempt_before_barrier", format!(
                        "release-floor oracle failed at cycle {cycle}: a Down sender attempt occurred before the release barrier"
                    ));
                }
                partial.set("authored_due_admission_checkpoint_qpc_ticks", json!(now_qpc));
                break now_qpc;
            }
            if session.snapshot().is_finished || Instant::now() >= due_deadline {
                partial.set("down_sender_attempt_count", json!(hook.down_sender_attempt_count()));
                fail_cycle!("resumed_window_or_due_timeout", format!(
                    "CASE-NOT-EXERCISED: resumed authored target did not become observable before the release floor in cycle {cycle}"
                ));
            }
            thread::sleep(Duration::from_micros(100));
        };
        if !wait_for_up_count(session, hook, cycle + 1) {
            partial.set("down_sender_attempt_count", json!(hook.down_sender_attempt_count()));
            fail_cycle!("waiting_for_resumed_pair", format!(
                "CASE-NOT-EXERCISED: resumed Down/Up pair {cycle} did not complete"
            ));
        }

        let (_, authored_target_qpc, down_not_before_qpc) = hook.last_down_window_snapshot();
        let down_sender_start_qpc = hook.last_down_sender_start_qpc_ticks();
        let down_sender_attempt_count = hook.down_sender_attempt_count();
        partial.set("down_sender_start_qpc_ticks", json!(down_sender_start_qpc));
        partial.set("down_sender_attempt_count", json!(down_sender_attempt_count));
        partial.set(
            "resumed_authored_down_target_qpc_ticks",
            json!(authored_target_qpc),
        );
        partial.set(
            "resumed_down_not_before_qpc_ticks",
            json!(down_not_before_qpc),
        );
        if authored_target_qpc == 0 || authored_target_qpc >= barrier_qpc {
            fail_cycle!("authored_target_floor_oracle", format!(
                "CASE-NOT-EXERCISED: resumed authored Down target {authored_target_qpc} was not before release floor {barrier_qpc} at cycle {cycle} (cleanup QPC {cleanup_qpc})"
            ));
        }
        if authored_target_qpc > authored_due_checkpoint_qpc
            || authored_due_checkpoint_qpc >= barrier_qpc
        {
            fail_cycle!("authored_due_checkpoint_oracle", format!(
                "CASE-NOT-EXERCISED: authored target was not proven due before the release floor at cycle {cycle} (authored={authored_target_qpc}, due checkpoint={authored_due_checkpoint_qpc}, barrier={barrier_qpc})"
            ));
        }
        if down_not_before_qpc < barrier_qpc
            || down_sender_start_qpc < barrier_qpc
            || down_sender_attempt_count != cycle + 1
        {
            fail_cycle!("physical_release_floor_oracle", format!(
                "release-floor oracle failed at cycle {cycle}: authored={authored_target_qpc}, barrier={barrier_qpc}, not_before={down_not_before_qpc}, sender_start={down_sender_start_qpc}, down_attempts={down_sender_attempt_count}"
            ));
        }
        partial.set("floor_passed", json!(true));
        partial.stage("completed");
        evidence.push(partial.into_value());
    }

    hook.release_completed_up_through(u64::MAX);
    Ok(evidence)
}

pub(super) fn begin_delayed_watchdog_case(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<(super::release_gap_stress::IndependentHelperLivenessProbe, bool), String> {
    if !wait_for_startup_armed(session) {
        return Err("production session did not reach Armed publication before delayed-watchdog proof".to_string());
    }
    session
        .publish_supervisor_progress()
        .map_err(|error| format!("failed to seed delayed-watchdog lease progress: {error}"))?;
    let helper = super::release_gap_stress::IndependentHelperLivenessProbe::start()
        .map_err(|error| format!("could not start independent helper liveness probe: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !hook.watchdog_waiting() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    Ok((helper, hook.watchdog_waiting()))
}

pub(super) struct CaseDetails<'a> {
    pub(super) scenario: Scenario,
    pub(super) release_floor_cycles: &'a [Value],
    pub(super) watchdog_was_held: bool,
    pub(super) hook: Option<&'a StartupOrderingHook>,
    pub(super) terminal_outcome: Option<&'a str>,
    pub(super) final_gate_lease_expirations: u64,
    pub(super) terminal_error: Option<&'a str>,
    pub(super) terminal_trigger_reason: Option<&'a str>,
    pub(super) helper_liveness_ticks: u64,
}

pub(super) fn attach_case_details(object: &mut Map<String, Value>, details: CaseDetails<'_>) {
    if details.scenario == Scenario::PauseResumeReleaseFloor {
        object.insert(
            "resume_release_floor_cycles".to_string(),
            json!({
                "requested_cycle_count": 20,
                "completed_cycle_count": details.release_floor_cycles.len(),
                "initial_worker_delay_us": 10_000,
                "worker_test_hook": "feature-gated successful-Up barrier",
                "cycles": details.release_floor_cycles
            }),
        );
    }
    if details.scenario == Scenario::LeaseAgeWatchdogDelayed {
        let hook = details.hook.expect("delayed-watchdog acceptance hook");
        object.insert(
            "delayed_watchdog_lease_age".to_string(),
            json!({
                "watchdog_held_before_age_check": details.watchdog_was_held,
                "watchdog_hold_flag_at_report": hook.watchdog_is_held(),
                "watchdog_waiting_observed": hook.watchdog_waiting(),
                "down_sender_attempt_count": hook.down_sender_attempt_count(),
                "final_gate_lease_expirations": details.final_gate_lease_expirations,
                "auxiliary_thread_liveness_ticks": details.helper_liveness_ticks,
                "terminal_outcome": details.terminal_outcome,
                "terminal_error": details.terminal_error,
                "terminal_trigger_reason": details.terminal_trigger_reason
            }),
        );
    }
}

pub(super) fn expected_delayed_watchdog_expiry(
    scenario: Scenario,
    terminal_trigger_reason: Option<&str>,
    terminal_error: Option<&str>,
    final_gate_lease_expirations: u64,
) -> bool {
    scenario == Scenario::LeaseAgeWatchdogDelayed
        && terminal_trigger_reason == Some("supervisor_lease_expired")
        && super::terminal_error_matches_reason(terminal_error, "supervisor_lease_expired")
        && final_gate_lease_expirations > 0
}

pub(super) fn release_floor_failure(
    args: &RunArgs,
    cycle_count: usize,
    final_release_obligation_mask: u16,
    release_floor_violation_count: u64,
    details: &Value,
) -> Option<i32> {
    if cycle_count == 20 && final_release_obligation_mask == 0 && release_floor_violation_count == 0 {
        return None;
    }
    Some(write_report(
        args,
        Verdict::Fail,
        "pause/resume release-floor case did not complete twenty clean barrier cycles",
        details.clone(),
    ))
}

pub(super) struct DelayedWatchdogOracle<'a> {
    pub(super) watchdog_was_held: bool,
    pub(super) down_sender_attempt_count: u64,
    pub(super) final_gate_lease_expirations: u64,
    pub(super) terminal_outcome: Option<&'a str>,
    pub(super) terminal_error: Option<&'a str>,
    pub(super) terminal_trigger_reason: Option<&'a str>,
    pub(super) final_release_obligation_mask: u16,
    pub(super) sink_events_empty: bool,
    pub(super) helper_liveness_ticks: u64,
    pub(super) details: Value,
}

pub(super) fn finish_delayed_watchdog_case(
    args: &RunArgs,
    hook: &StartupOrderingHook,
    oracle: DelayedWatchdogOracle<'_>,
) -> i32 {
    if !oracle.watchdog_was_held {
        return write_report(
            args,
            Verdict::Fail,
            "CASE-NOT-EXERCISED: watchdog did not reach the controlled pre-check barrier",
            oracle.details,
        );
    }
    if !hook.watchdog_is_held()
        || oracle.down_sender_attempt_count != 0
        || oracle.final_gate_lease_expirations == 0
        || oracle.terminal_trigger_reason != Some("supervisor_lease_expired")
        || oracle.terminal_outcome != Some("error")
        || !super::terminal_error_matches_reason(oracle.terminal_error, "supervisor_lease_expired")
        || oracle.final_release_obligation_mask != 0
        || !oracle.sink_events_empty
        || oracle.helper_liveness_ticks == 0
    {
        return write_report(
            args,
            Verdict::Fail,
            "delayed watchdog final Down lease-age oracle failed",
            oracle.details,
        );
    }
    write_report(
        args,
        Verdict::Pass,
        "expired supervisor lease caused the expected terminal block at final Down admission before the sender while the helper remained alive",
        oracle.details,
    )
}

#[cfg(test)]
mod release_floor_tests {
    use super::{
        PartialReleaseFloorCycle, ReleaseFloorFailure, Verdict,
        down_sender_attempt_count_unchanged, release_floor_blocks_down,
        resumed_staged_window_is_due_before_release_floor,
    };
    use serde_json::json;
    use sky_dispatch_win32::clock::QpcClock;
    use std::num::NonZeroU64;

    #[test]
    fn release_barrier_oracle_uses_effective_gap_for_60_and_120_fps() {
        let qpc = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_003).unwrap());
        for (fps, expected_g_us) in [(60_u64, 17_167_u64), (120, 8_834)] {
            let frame_us = 1_000_000_u64.div_ceil(fps);
            let effective_g_us = frame_us + 500;
            assert_eq!(effective_g_us, expected_g_us);

            let g_ticks = qpc
                .duration_from_us(effective_g_us)
                .expect("G conversion rounds upward at non-divisible QPC frequency")
                .as_u64();
            let cleanup_completion = 1_000_000_u64;
            let barrier = cleanup_completion.checked_add(g_ticks).unwrap();
            assert!(release_floor_blocks_down(barrier - 1, barrier), "{fps} FPS G-1");
            assert!(!release_floor_blocks_down(barrier, barrier), "{fps} FPS exact G");

            let mut cycle = PartialReleaseFloorCycle::new(1, effective_g_us, g_ticks);
            cycle.set("post_cleanup_qpc_ticks", json!(cleanup_completion));
            cycle.set("release_barrier_qpc_ticks", json!(barrier));
            let report = cycle.into_value();
            assert_eq!(report["min_release_gap_us"], expected_g_us);
            assert_eq!(report["min_release_gap_ticks"], g_ticks);
            assert_eq!(report["release_barrier_qpc_ticks"], barrier);
        }
    }

    #[test]
    fn resumed_authored_target_can_be_due_before_barrier_without_down_sender_attempt() {
        let authored_target = 90;
        let now = 100;
        let release_barrier = 120;
        let packet_not_before = release_barrier;
        let sender_attempts_before = 4;
        let sender_attempts_at_observation = 4;

        assert!(resumed_staged_window_is_due_before_release_floor(
            7,
            8,
            authored_target,
            packet_not_before,
            now,
            release_barrier,
        ));
        assert!(down_sender_attempt_count_unchanged(
            sender_attempts_at_observation,
            sender_attempts_before,
        ));
        assert!(!down_sender_attempt_count_unchanged(5, sender_attempts_before));
    }

    #[test]
    fn not_exercised_failure_preserves_completed_and_partial_cycle_evidence() {
        let completed = vec![json!({"cycle": 1, "stage": "completed"})];
        let mut partial = PartialReleaseFloorCycle::new(2, 100, 100);
        partial.stage("waiting_for_resumed_authored_target_due_before_barrier");
        partial.set("post_cleanup_qpc_ticks", json!(50));
        partial.set("pause_acknowledgment_qpc_ticks", json!(60));
        partial.set("resume_request_qpc_ticks", json!(70));
        partial.set("release_barrier_qpc_ticks", json!(80));
        partial.set("resumed_window_sequence", json!(9));
        partial.set("resumed_authored_down_target_qpc_ticks", json!(65));
        partial.set("resumed_down_not_before_qpc_ticks", json!(80));
        partial.set("latest_window_sequence_at_observation", json!(9));
        partial.set("latest_window_authored_target_qpc_ticks", json!(65));
        partial.set("latest_window_not_before_qpc_ticks", json!(80));
        partial.set("latest_window_observation_qpc_ticks", json!(70));
        partial.set("down_sender_attempt_count", json!(2));

        let failure = ReleaseFloorFailure::new(
            "CASE-NOT-EXERCISED: synthetic authored window was not admitted",
            "resumed_due_before_barrier",
            &completed,
            Some(partial.into_value()),
        );

        assert_eq!(failure.verdict, Verdict::Fail);
        assert_eq!(failure.stage, "resumed_due_before_barrier");
        assert_eq!(failure.completed_cycles, completed);
        let partial = failure.partial_cycle.expect("partial cycle retained");
        assert_eq!(partial["cycle"], 2);
        assert_eq!(partial["post_cleanup_qpc_ticks"], 50);
        assert_eq!(partial["pause_acknowledgment_qpc_ticks"], 60);
        assert_eq!(partial["resume_request_qpc_ticks"], 70);
        assert_eq!(partial["release_barrier_qpc_ticks"], 80);
        assert_eq!(partial["resumed_window_sequence"], 9);
        assert_eq!(partial["resumed_authored_down_target_qpc_ticks"], 65);
        assert_eq!(partial["resumed_down_not_before_qpc_ticks"], 80);
        assert_eq!(partial["down_sender_attempt_count"], 2);
        assert_eq!(partial["down_sender_start_qpc_ticks"], serde_json::Value::Null);
        assert_eq!(partial["floor_passed"], false);
    }
}

#[cfg(test)]
mod delayed_watchdog_tests {
    use super::expected_delayed_watchdog_expiry;
    use crate::acceptance::Scenario;

    #[test]
    fn detailed_terminal_error_preserves_supervisor_expiry_classification() {
        assert!(expected_delayed_watchdog_expiry(
            Scenario::LeaseAgeWatchdogDelayed,
            Some("supervisor_lease_expired"),
            Some("dispatch gate rejected before SendInput: source_action=0 source_chord=0 up_mask=0x0000 down_mask=0x0001 requested=1 confirmed_count=unavailable first_inserted=unavailable attempts=0 status=gate_rejected win32_error=unavailable pre_call_qpc=unavailable completion_qpc=unavailable: supervisor_lease_expired"),
            1,
        ));
    }
}


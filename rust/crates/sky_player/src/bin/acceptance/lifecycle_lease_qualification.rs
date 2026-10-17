use super::{
    ACCEPTANCE_FRAME_US, NativeDispatchSession, RunArgs, Scenario, Value,
    Verdict, wait_for_focus_pause, wait_for_resume, wait_for_startup_ready, write_report,
};
use serde_json::{Map, json};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcClock;
use sky_player::engine::StartupOrderingHook;
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
        _ => None,
    }
}

pub(super) fn run_release_floor_case(
    args: &RunArgs,
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<Vec<Value>, i32> {
    if !wait_for_startup_ready(session) {
        return Err(write_report(
            args,
            Verdict::Inconclusive,
            "production session did not reach startup_ready before release-floor cycles",
            json!({}),
        ));
    }
    match run_release_floor_cycles(session, hook) {
        Ok(cycles) => Ok(cycles),
        Err(error) => {
            hook.release_completed_up_through(u64::MAX);
            let _ = session.quit();
            let _ = session.join(Duration::from_secs(5));
            let verdict = if error.starts_with("CASE-NOT-EXERCISED:")
                || error.starts_with("release-floor oracle failed")
            {
                Verdict::Fail
            } else {
                Verdict::Inconclusive
            };
            Err(write_report(
                args,
                verdict,
                &error,
                json!({"release_floor_cycles": []}),
            ))
        }
    }
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

fn run_release_floor_cycles(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<Vec<Value>, String> {
    let qpc = QpcClock::initialize().map_err(|error| format!("QPC initialization failed: {error:?}"))?;
    let frame_ticks = qpc
        .duration_from_us(ACCEPTANCE_FRAME_US)
        .map_err(|error| format!("frame conversion failed: {error:?}"))?
        .as_u64();
    let resume_delay_ticks = qpc
        .duration_from_us(2_000)
        .map_err(|error| format!("resume delay conversion failed: {error:?}"))?
        .as_u64();
    let mut evidence = Vec::with_capacity(20);

    for cycle in 1..=20_u64 {
        if !wait_for_up_count(session, hook, cycle) {
            return Err(format!(
                "CASE-NOT-EXERCISED: actual successful Up completion {cycle} was not observed"
            ));
        }
        // Let the next authored Down target fall below the post-cleanup global
        // frame floor even at the 500us acceptance margin. This stays inside
        // the test fixture and makes the native oracle exercise the barrier.
        thread::sleep(Duration::from_millis(4));
        hook.clear_pause_ack_qpc();
        session
            .pause()
            .map_err(|error| format!("manual pause request failed at cycle {cycle}: {error}"))?;
        hook.release_completed_up_through(cycle);
        if !wait_for_focus_pause(session) {
            return Err(format!("manual pause did not acknowledge at cycle {cycle}"));
        }

        let ack_deadline = Instant::now() + Duration::from_secs(1);
        let acknowledgment_qpc = loop {
            let ticks = hook.pause_ack_qpc_ticks();
            if ticks != 0 {
                break ticks;
            }
            if session.snapshot().is_finished || Instant::now() >= ack_deadline {
                return Err(format!(
                    "manual pause acknowledgment QPC was not captured at cycle {cycle}"
                ));
            }
            thread::sleep(Duration::from_micros(100));
        };
        let cleanup_qpc = hook.resume_cleanup_qpc_ticks();
        if cleanup_qpc == 0 || acknowledgment_qpc < cleanup_qpc {
            return Err(format!(
                "post-cleanup QPC provenance is missing or unordered at cycle {cycle}"
            ));
        }
        let barrier_qpc = cleanup_qpc
            .checked_add(frame_ticks)
            .ok_or_else(|| format!("resume barrier overflow at cycle {cycle}"))?;

        let resume_target = acknowledgment_qpc
            .checked_add(resume_delay_ticks)
            .ok_or_else(|| format!("resume target overflow at cycle {cycle}"))?;
        loop {
            let now = qpc
                .now()
                .map_err(|error| format!("QPC read failed before resume at cycle {cycle}: {error:?}"))?
                .as_u64();
            if now >= resume_target {
                break;
            }
            let remaining_us = qpc
                .duration_to_us(DurationTicks::from_raw(resume_target - now))
                .unwrap_or(1)
                .clamp(1, 500);
            thread::sleep(Duration::from_micros(remaining_us));
        }
        session
            .resume()
            .map_err(|error| format!("manual resume request failed at cycle {cycle}: {error}"))?;
        let resume_request_qpc = qpc
            .now()
            .map_err(|error| format!("QPC read failed after resume at cycle {cycle}: {error:?}"))?
            .as_u64();
        if resume_request_qpc >= barrier_qpc {
            return Err(format!(
                "CASE-NOT-EXERCISED: resume request reached/passed the cleanup release floor at cycle {cycle}"
            ));
        }
        if !wait_for_resume(session) {
            return Err(format!("manual resume did not commit at cycle {cycle}"));
        }
        if !wait_for_up_count(session, hook, cycle + 1) {
            return Err(format!(
                "CASE-NOT-EXERCISED: resumed Down/Up pair {cycle} did not complete"
            ));
        }

        let (authored_target_qpc, down_not_before_qpc) = hook.last_down_window();
        let down_sender_start_qpc = hook.last_down_sender_start_qpc_ticks();
        let down_sender_attempt_count = hook.down_sender_attempt_count();
        if authored_target_qpc == 0 || authored_target_qpc >= barrier_qpc {
            return Err(format!(
                "CASE-NOT-EXERCISED: resumed authored Down target {authored_target_qpc} was not before release floor {barrier_qpc} at cycle {cycle} (cleanup QPC {cleanup_qpc})"
            ));
        }
        if down_not_before_qpc < barrier_qpc
            || down_sender_start_qpc < barrier_qpc
            || down_sender_attempt_count != cycle + 1
        {
            return Err(format!(
                "release-floor oracle failed at cycle {cycle}: authored={authored_target_qpc}, barrier={barrier_qpc}, not_before={down_not_before_qpc}, sender_start={down_sender_start_qpc}, down_attempts={down_sender_attempt_count}"
            ));
        }
        evidence.push(json!({
            "cycle": cycle,
            "successful_up_completion_observed": true,
            "pause_acknowledgment_qpc_ticks": acknowledgment_qpc,
            "post_cleanup_qpc_ticks": cleanup_qpc,
            "frame_ticks": frame_ticks,
            "release_barrier_qpc_ticks": barrier_qpc,
            "resume_request_qpc_ticks": resume_request_qpc,
            "resumed_authored_down_target_qpc_ticks": authored_target_qpc,
            "resumed_down_not_before_qpc_ticks": down_not_before_qpc,
            "resumed_down_sender_start_qpc_ticks": down_sender_start_qpc,
            "down_sender_attempt_count": down_sender_attempt_count,
            "floor_passed": true
        }));
    }

    hook.release_completed_up_through(u64::MAX);
    Ok(evidence)
}

pub(super) fn begin_delayed_watchdog_case(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<(super::release_gap_stress::IndependentHelperLivenessProbe, bool), String> {
    if !wait_for_startup_ready(session) {
        return Err("production session did not reach startup_ready before delayed-watchdog proof".to_string());
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
    pub(super) final_gate_lease_expirations: u64,
    pub(super) terminal_error: Option<&'a str>,
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
                "terminal_error": details.terminal_error
            }),
        );
    }
}

pub(super) fn expected_delayed_watchdog_expiry(
    scenario: Scenario,
    terminal_error: Option<&str>,
) -> bool {
    scenario == Scenario::LeaseAgeWatchdogDelayed
        && terminal_error == Some("supervisor_lease_expired")
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
    pub(super) terminal_error: Option<&'a str>,
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
        || oracle.terminal_error != Some("supervisor_lease_expired")
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
        "watchdog stayed behind the test barrier; final Down lease-age proof rejected before the sender while the helper remained alive",
        oracle.details,
    )
}


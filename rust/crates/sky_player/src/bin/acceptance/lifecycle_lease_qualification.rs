use super::{
    NativeDispatchSession, RunArgs, Scenario, Value, Verdict, wait_for_startup_armed, write_report,
};
use serde_json::{Map, json};
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
            hook.release_watchdog();
        }
    }
}

pub(super) fn hook_for_scenario(scenario: Scenario) -> Option<Arc<StartupOrderingHook>> {
    match scenario {
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

pub(super) fn begin_delayed_watchdog_case(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<(super::generation_stress::IndependentHelperLivenessProbe, bool), String> {
    if !wait_for_startup_armed(session) {
        return Err("production session did not reach Armed publication before delayed-watchdog proof".to_string());
    }
    session
        .publish_supervisor_progress()
        .map_err(|error| format!("failed to seed delayed-watchdog lease progress: {error}"))?;
    let helper = super::generation_stress::IndependentHelperLivenessProbe::start()
        .map_err(|error| format!("could not start independent helper liveness probe: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !hook.watchdog_waiting() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    Ok((helper, hook.watchdog_waiting()))
}

pub(super) struct CaseDetails<'a> {
    pub(super) scenario: Scenario,
    pub(super) watchdog_was_held: bool,
    pub(super) hook: Option<&'a StartupOrderingHook>,
    pub(super) terminal_outcome: Option<&'a str>,
    pub(super) final_gate_lease_expirations: u64,
    pub(super) terminal_error: Option<&'a str>,
    pub(super) terminal_trigger_reason: Option<&'a str>,
    pub(super) helper_liveness_ticks: u64,
}

pub(super) fn attach_case_details(object: &mut Map<String, Value>, details: CaseDetails<'_>) {
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


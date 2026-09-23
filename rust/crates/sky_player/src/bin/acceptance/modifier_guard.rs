use super::{EventRecord, RunArgs, Scenario, Verdict, write_report};
use sky_dispatch_win32::input::InstrumentKeyProfileSpec;
use sky_player::engine::NativeSessionOptions;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;

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
) -> Option<Arc<dyn Fn(i32) -> i16 + Send + Sync>> {
    #[cfg(not(feature = "test-support"))]
    let _ = (&scenario, &query_count);
    #[cfg(feature = "test-support")]
    {
        match scenario {
            Scenario::ModifierHeldFinalBoundary => Some(Arc::new(move |virtual_key| {
                query_count.fetch_add(1, Ordering::SeqCst);
                if virtual_key == 0x10 { i16::MIN } else { 0 }
            })),
            Scenario::ModifierHeldAfterOwned => Some(Arc::new(move |virtual_key| {
                let index = query_count.fetch_add(1, Ordering::SeqCst);
                if index >= 5 && virtual_key == 0x10 { i16::MIN } else { 0 }
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
) -> (NativeSessionOptions, Arc<AtomicU64>) {
    let query_count = Arc::new(AtomicU64::new(0));
    let options = super::scenarios::production_options(
        schedule,
        profile,
        timing_margin_us,
        scenario,
        Arc::clone(&query_count),
    );
    (options, query_count)
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
    let expected_queries = if scenario == Scenario::ModifierHeldFinalBoundary {
        5
    } else {
        10
    };
    if terminal_error.is_none_or(|error| !error.contains("physical_modifier_held"))
        || query_count != expected_queries
    {
        return Err("modifier guard did not produce the expected fixed-query terminal result".into());
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

use super::Scenario;
use serde_json::{Value, json};
use sky_dispatch_win32::focus::{
    retain_window_identity, set_foreground_owner_for_test,
};
use sky_player::engine::{NativeDispatchSession, StartupOrderingHook};
use std::time::Duration;

pub(super) struct BindingEvidence {
    pub(super) actual_owner_pid: u32,
    pub(super) expected_owner_pid: u32,
}

impl BindingEvidence {
    pub(super) fn is_failure_case(&self, scenario: Scenario) -> bool {
        expected_failure_reason(scenario).is_some()
    }

    pub(super) fn report(&self) -> Value {
        json!({
            "pid": self.actual_owner_pid,
            "expected_owner_pid": self.expected_owner_pid,
        })
    }

    pub(super) fn insert_report_fields(
        &self,
        scenario: Scenario,
        object: &mut serde_json::Map<String, Value>,
    ) {
        object.insert("owner_identity_binding_requested".to_string(), json!(true));
        if scenario == Scenario::OwnerProcessTermination {
            object.insert(
                "owner_process_termination_fixture".to_string(),
                json!({
                    "method": "test_support authority invalidation at FirstDownAdmission checkpoint",
                    "os_process_terminated": false,
                }),
            );
        }
        object.insert(
            "owner_identity_failure_case".to_string(),
            json!(self.is_failure_case(scenario)),
        );
        object.insert("retained_process_identity".to_string(), self.report());
    }
}

pub(super) fn bind(
    session: &NativeDispatchSession,
    sink_hwnd: isize,
    scenario: Scenario,
) -> Result<BindingEvidence, String> {
    session.set_target_hwnd(sink_hwnd);
    let authority = retain_window_identity(sink_hwnd)?;
    let actual_owner_pid = authority.identity().owner_pid;
    let expected_owner_pid = if scenario == Scenario::OwnerMismatch {
        actual_owner_pid.wrapping_add(1).max(1)
    } else {
        actual_owner_pid
    };
    let result = if scenario == Scenario::OwnerMismatch {
        session.bind_window_identity_authority_for_test(authority, expected_owner_pid)
    } else {
        session.bind_window_identity_authority(authority)
    };
    result?;

    if scenario == Scenario::OwnerQueryFailure {
        set_foreground_owner_for_test(Some(None));
    }

    Ok(BindingEvidence {
        actual_owner_pid,
        expected_owner_pid,
    })
}

pub(super) fn invalidate_at_first_down_admission(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
) -> Result<(), String> {
    if !hook.wait_for_startup_checkpoint(Duration::from_secs(5)) {
        hook.release_startup_checkpoint();
        return Err("worker did not reach the first Down admission checkpoint".into());
    }
    session.invalidate_window_identity_authority_for_test();
    hook.release_startup_checkpoint();
    Ok(())
}

pub(super) fn clear_test_override(scenario: Scenario) {
    if scenario == Scenario::OwnerQueryFailure {
        set_foreground_owner_for_test(None);
    }
}

pub(super) fn expected_failure_reason(scenario: Scenario) -> Option<&'static str> {
    match scenario {
        Scenario::OwnerMismatch => Some("prepared_down_owner_mismatch"),
        Scenario::OwnerQueryFailure => Some("prepared_down_owner_query_unavailable"),
        Scenario::OwnerProcessTermination => Some("prepared_down_owner_process_terminated"),
        _ => None,
    }
}

pub(super) fn failure_matches(scenario: Scenario, terminal_error: Option<&str>) -> bool {
    expected_failure_reason(scenario)
        .is_some_and(|expected| terminal_error == Some(expected))
}

pub(super) fn failure_qualified(
    scenario: Scenario,
    terminal_error: Option<&str>,
    sink_events_empty: bool,
    release_obligation_mask: u16,
) -> bool {
    failure_matches(scenario, terminal_error)
        && sink_events_empty
        && release_obligation_mask == 0
}

pub(super) fn failure_result(
    scenario: Scenario,
    terminal_error: Option<&str>,
    sink_events_empty: bool,
    release_obligation_mask: u16,
) -> Option<(super::Verdict, &'static str)> {
    expected_failure_reason(scenario)?;
    let qualified = failure_qualified(
        scenario,
        terminal_error,
        sink_events_empty,
        release_obligation_mask,
    );
    Some((
        if qualified && scenario == Scenario::OwnerProcessTermination {
            super::Verdict::Inconclusive
        } else if qualified {
            super::Verdict::Pass
        } else {
            super::Verdict::Fail
        },
        if qualified && scenario == Scenario::OwnerProcessTermination {
            "synthetic owner-authority invalidation stopped before the sender with zero release obligation; the OS owner process was not terminated"
        } else if qualified {
            "window-owner authority failure stopped before the prepared sender"
        } else {
            "window-owner authority failure did not stop before gameplay delivery with zero release obligation"
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{Scenario, failure_result};

    #[test]
    fn synthetic_authority_revocation_does_not_claim_process_termination() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("prepared_down_owner_process_terminated"),
            true,
            0,
        );

        assert!(matches!(
            result,
            Some((
                super::super::Verdict::Inconclusive,
                "synthetic owner-authority invalidation stopped before the sender with zero release obligation; the OS owner process was not terminated"
            ))
        ));
    }
}

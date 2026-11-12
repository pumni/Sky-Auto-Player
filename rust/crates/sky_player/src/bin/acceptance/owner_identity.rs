use super::Scenario;
use serde_json::{Value, json};
use sky_dispatch_win32::focus::{
    WindowIdentity, retain_window_identity, set_foreground_owner_for_test,
};
use sky_player::engine::{NativeDispatchSession, StartupOrderingHook};
use std::time::Duration;

pub(super) struct BindingEvidence {
    pub(super) actual_owner_pid: u32,
    pub(super) expected_owner_pid: u32,
    retained_identity: WindowIdentity,
    retained_image_identity: String,
    process_termination: Option<ProcessTerminationEvidence>,
}

#[derive(Clone, Debug)]
struct ProcessTerminationEvidence {
    owner_pid: u32,
    process_start_time_filetime: u64,
    process_image_basename: String,
    exit_code: u32,
}

impl ProcessTerminationEvidence {
    // The Win32 TerminateProcess helper proves termination; this only binds its retained evidence to the sink identity.
    fn matches_retained_sink_identity(&self, identity: &WindowIdentity) -> bool {
        self.owner_pid == identity.owner_pid
            && self.process_start_time_filetime == identity.process_start_time_filetime
            && self.process_image_basename == identity.process_image_basename
            && self.exit_code == 0x646
    }
}

impl BindingEvidence {
    pub(super) fn is_failure_case(&self, scenario: Scenario) -> bool {
        expected_failure_reason(scenario).is_some()
    }

    pub(super) fn process_termination_verified(&self) -> bool {
        self.process_termination
            .as_ref()
            .is_some_and(|evidence| evidence.matches_retained_sink_identity(&self.retained_identity))
    }

    pub(super) fn report(&self) -> Value {
        json!({
            "pid": self.actual_owner_pid,
            "expected_owner_pid": self.expected_owner_pid,
            "hwnd": self.retained_identity.hwnd,
            "title": self.retained_identity.title,
            "process_start_time_filetime": self.retained_identity.process_start_time_filetime,
            "process_image_basename": self.retained_identity.process_image_basename,
        })
    }

    pub(super) fn insert_report_fields(
        &self,
        scenario: Scenario,
        object: &mut serde_json::Map<String, Value>,
        down_sender_attempt_count: Option<u64>,
    ) {
        object.insert("owner_identity_binding_requested".to_string(), json!(true));
        if scenario == Scenario::OwnerProcessTermination {
            let termination = self.process_termination.as_ref();
            object.insert(
                "down_sender_attempt_count".to_string(),
                json!(down_sender_attempt_count),
            );
            object.insert(
                "owner_process_termination_fixture".to_string(),
                json!({
                    "method": "TerminateProcess on the identity-verified ReceiveOnly sink at FirstDownAdmission",
                    "os_process_terminated": self.process_termination_verified(),
                    "owner_pid": self.retained_identity.owner_pid,
                    "owner_hwnd": self.retained_identity.hwnd,
                    "owner_process_start_time_filetime": self.retained_identity.process_start_time_filetime,
                    "owner_process_image_basename": self.retained_identity.process_image_basename,
                    "exit_code": termination.map(|value| value.exit_code),
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
    let retained_identity = authority.identity().clone();
    let retained_image_identity = authority.process_image_identity().to_string();
    let actual_owner_pid = retained_identity.owner_pid;
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
        retained_identity,
        retained_image_identity,
        process_termination: None,
    })
}

pub(super) fn terminate_at_first_down_admission(
    binding: &mut BindingEvidence,
    hook: &StartupOrderingHook,
) -> Result<(), String> {
    if !hook.wait_for_startup_checkpoint(Duration::from_secs(5)) {
        hook.release_startup_checkpoint();
        return Err("worker did not reach the owner termination admission checkpoint".into());
    }
    let result = sky_dispatch_win32::focus::terminate_receive_only_sink_process_for_test(
        &binding.retained_identity,
        &binding.retained_image_identity,
    );
    hook.release_startup_checkpoint();
    let exit_code = result?;
    binding.process_termination = Some(ProcessTerminationEvidence {
        owner_pid: binding.retained_identity.owner_pid,
        process_start_time_filetime: binding.retained_identity.process_start_time_filetime,
        process_image_basename: binding.retained_identity.process_image_basename.clone(),
        exit_code,
    });
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

fn terminal_error_matches_reason(terminal_error: Option<&str>, expected: &str) -> bool {
    terminal_error.is_some_and(|error| {
        error == expected
            || error
                .strip_suffix(expected)
                .is_some_and(|prefix| prefix.ends_with(": "))
    })
}

pub(super) fn failure_matches(
    scenario: Scenario,
    terminal_error: Option<&str>,
    process_termination_verified: bool,
) -> bool {
    expected_failure_reason(scenario)
        .is_some_and(|expected| terminal_error_matches_reason(terminal_error, expected))
        || (scenario == Scenario::OwnerProcessTermination
            && process_termination_verified
            && terminal_error_matches_reason(terminal_error, "focus_lost_during_preroll"))
}

pub(super) fn failure_qualified(
    scenario: Scenario,
    terminal_error: Option<&str>,
    sink_events_empty: bool,
    release_obligation_mask: u16,
    process_termination_verified: bool,
    down_sender_attempt_count: Option<u64>,
) -> bool {
    failure_matches(scenario, terminal_error, process_termination_verified)
        && sink_events_empty
        && release_obligation_mask == 0
        && (scenario != Scenario::OwnerProcessTermination || down_sender_attempt_count == Some(0))
}

pub(super) fn failure_result(
    scenario: Scenario,
    terminal_error: Option<&str>,
    sink_events_empty: bool,
    release_obligation_mask: u16,
    process_termination_verified: bool,
    down_sender_attempt_count: Option<u64>,
) -> Option<(super::Verdict, &'static str)> {
    expected_failure_reason(scenario)?;
    let qualified = failure_qualified(
        scenario,
        terminal_error,
        sink_events_empty,
        release_obligation_mask,
        process_termination_verified,
        down_sender_attempt_count,
    );
    Some((
        if qualified && scenario == Scenario::OwnerProcessTermination && process_termination_verified {
            super::Verdict::Pass
        } else if qualified && scenario == Scenario::OwnerProcessTermination {
            super::Verdict::Inconclusive
        } else if qualified {
            super::Verdict::Pass
        } else {
            super::Verdict::Fail
        },
        if qualified
            && scenario == Scenario::OwnerProcessTermination
            && process_termination_verified
            && terminal_error_matches_reason(terminal_error, "focus_lost_during_preroll")
        {
            "identity-verified ReceiveOnly owner process termination destroyed the foreground window; preroll failed closed before the sender"
        } else if qualified && scenario == Scenario::OwnerProcessTermination && process_termination_verified {
            "identity-verified ReceiveOnly owner process was terminated by the OS at first Down admission; no sender attempt or release obligation remained"
        } else if qualified && scenario == Scenario::OwnerProcessTermination {
            "owner termination checkpoint stopped before the sender, but the OS owner process termination was not verified"
        } else if scenario == Scenario::OwnerProcessTermination
            && down_sender_attempt_count != Some(0)
        {
            "owner process termination did not prove zero Down sender attempts"
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
    fn detailed_terminal_diagnostic_preserves_expected_owner_failure() {
        let result = failure_result(
            Scenario::OwnerMismatch,
            Some("dispatch gate rejected before SendInput: source_action=0: prepared_down_owner_mismatch"),
            true,
            0,
            false,
            None,
        );

        assert!(matches!(
            result,
            Some((
                super::super::Verdict::Pass,
                "window-owner authority failure stopped before the prepared sender"
            ))
        ));
    }

    #[test]
    fn synthetic_authority_revocation_does_not_claim_process_termination() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("prepared_down_owner_process_terminated"),
            true,
            0,
            false,
            Some(0),
        );

        assert!(matches!(
            result,
            Some((
                super::super::Verdict::Inconclusive,
                "owner termination checkpoint stopped before the sender, but the OS owner process termination was not verified"
            ))
        ));
    }

    #[test]
    fn verified_receive_only_process_termination_qualifies_owner_case() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("prepared_down_owner_process_terminated"),
            true,
            0,
            true,
            Some(0),
        );

        assert!(matches!(
            result,
            Some((
                super::super::Verdict::Pass,
                "identity-verified ReceiveOnly owner process was terminated by the OS at first Down admission; no sender attempt or release obligation remained"
            ))
        ));
    }

    #[test]
    fn verified_receive_only_termination_that_destroys_foreground_fails_closed() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("dispatch gate rejected before SendInput: source_action=0: focus_lost_during_preroll"),
            true,
            0,
            true,
            Some(0),
        );

        assert!(matches!(
            result,
            Some((
                super::super::Verdict::Pass,
                "identity-verified ReceiveOnly owner process termination destroyed the foreground window; preroll failed closed before the sender"
            ))
        ));
    }

    #[test]
    fn verified_receive_only_termination_with_down_attempt_fails() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("prepared_down_owner_process_terminated"),
            true,
            0,
            true,
            Some(1),
        );

        assert!(matches!(result, Some((super::super::Verdict::Fail, _))));
    }

    #[test]
    fn missing_down_attempt_evidence_does_not_pass_termination_case() {
        let result = failure_result(
            Scenario::OwnerProcessTermination,
            Some("prepared_down_owner_process_terminated"),
            true,
            0,
            true,
            None,
        );

        assert!(!matches!(result, Some((super::super::Verdict::Pass, _))));
    }
}

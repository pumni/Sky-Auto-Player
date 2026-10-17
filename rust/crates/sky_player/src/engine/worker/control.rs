use crate::engine::shared::SupervisorLeaseState;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) enum CommandControl {
    Continue,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalTrigger {
    Quit,
    Skip,
    PanicReleaseRequested,
    SupervisorLeaseExpired,
    SystemSuspend,
    FocusAuthorityInvalidated,
    TargetAuthorityInvalidated,
    OwnerAuthorityInvalidated,
    WorkerPanic,
}

impl TerminalTrigger {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Skip => "skipped",
            Self::PanicReleaseRequested => "panic_release_requested",
            Self::SupervisorLeaseExpired => "supervisor_lease_expired",
            Self::SystemSuspend => "system_suspend_during_preparation",
            Self::FocusAuthorityInvalidated => "focus_authority_invalidated_during_preparation",
            Self::TargetAuthorityInvalidated => "target_authority_invalidated_during_preparation",
            Self::OwnerAuthorityInvalidated => "owner_authority_invalidated_during_preparation",
            Self::WorkerPanic => "worker_panic",
        }
    }

    pub(crate) const fn requires_error_outcome(self) -> bool {
        matches!(
            self,
            Self::PanicReleaseRequested
                | Self::SupervisorLeaseExpired
                | Self::SystemSuspend
                | Self::FocusAuthorityInvalidated
                | Self::TargetAuthorityInvalidated
                | Self::OwnerAuthorityInvalidated
                | Self::WorkerPanic
        )
    }
}

pub(super) struct CommandControlSignals<'a> {
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) supervisor_expired: &'a SupervisorLeaseState,
}

pub(super) struct CommandControlRuntime<'a> {
    pub(super) terminal_trigger: &'a mut Option<TerminalTrigger>,
}

pub(super) struct CommandControlInput<'a> {
    pub(super) signals: CommandControlSignals<'a>,
    pub(super) runtime: CommandControlRuntime<'a>,
}

#[inline]
fn consume_panic_request_if_pending(panic_requested: &AtomicBool, command_exit: bool) -> bool {
    if command_exit || !panic_requested.load(Ordering::Acquire) {
        return false;
    }
    panic_requested.swap(false, Ordering::AcqRel)
}

#[inline]
fn latch_terminal_trigger(
    terminal_trigger: &mut Option<TerminalTrigger>,
    trigger: TerminalTrigger,
) {
    terminal_trigger.get_or_insert(trigger);
}

/// Observe stop signals and hand terminal release ownership to finalization.
///
/// This boundary deliberately does not release keys, inspect/cancel the
/// coordinator, or publish diagnostics. The worker finalizer owns the single
/// terminal release transaction and performs it before full-ledger work.
pub(super) fn process_command_control(context: CommandControlInput<'_>) -> CommandControl {
    let CommandControlInput {
        signals:
            CommandControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                supervisor_expired,
            },
        runtime: CommandControlRuntime { terminal_trigger },
    } = context;

    if supervisor_expired.is_expired() {
        latch_terminal_trigger(terminal_trigger, TerminalTrigger::SupervisorLeaseExpired);
        return CommandControl::Exit;
    }

    let command_exit =
        quit_requested.load(Ordering::Acquire) || skip_requested.load(Ordering::Acquire);
    if command_exit {
        let trigger = if skip_requested.load(Ordering::Acquire) {
            TerminalTrigger::Skip
        } else {
            TerminalTrigger::Quit
        };
        latch_terminal_trigger(terminal_trigger, trigger);
        return CommandControl::Exit;
    }

    if consume_panic_request_if_pending(panic_requested, command_exit) {
        let trigger = if supervisor_expired.is_expired() {
            TerminalTrigger::SupervisorLeaseExpired
        } else {
            TerminalTrigger::PanicReleaseRequested
        };
        latch_terminal_trigger(terminal_trigger, trigger);
        return CommandControl::Exit;
    }

    CommandControl::Continue
}

#[cfg(test)]
mod tests {
    use super::{TerminalTrigger, consume_panic_request_if_pending};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn terminal_trigger_names_preserve_stop_identity() {
        assert_eq!(TerminalTrigger::Quit.as_str(), "quit");
        assert_eq!(TerminalTrigger::Skip.as_str(), "skipped");
        assert_eq!(
            TerminalTrigger::PanicReleaseRequested.as_str(),
            "panic_release_requested"
        );
        assert_eq!(
            TerminalTrigger::SupervisorLeaseExpired.as_str(),
            "supervisor_lease_expired"
        );
    }

    #[test]
    fn panic_fast_path_does_not_rmw_when_flag_is_clear() {
        let panic_requested = AtomicBool::new(false);

        assert!(!consume_panic_request_if_pending(&panic_requested, false));
        panic_requested.store(true, Ordering::Release);
        assert!(consume_panic_request_if_pending(&panic_requested, false));
        assert!(!panic_requested.load(Ordering::Acquire));
    }

    #[test]
    fn panic_fast_path_source_loads_before_conditional_consume() {
        let source = include_str!("control.rs");
        let helper = source
            .split("fn consume_panic_request_if_pending")
            .nth(1)
            .expect("panic fast path helper")
            .split("fn latch_terminal_trigger")
            .next()
            .expect("panic fast path helper body");
        assert!(helper.find("load(Ordering::Acquire)").unwrap() < helper.find("swap(").unwrap());
    }
}

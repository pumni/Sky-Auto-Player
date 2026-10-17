use super::{
    DRAIN_POLL_MS, FileEventWindowReader, LogCursor, NativeCleanupEvidence, PhysicalExpectation,
    RECEIVE_ONLY_ROLE, RealDrainClock, Verdict, validate_event_prefix, write_report,
};
use sky_player::engine::NativeDispatchSession;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PreTerminalResult {
    Satisfied,
    TrustedFailure(String),
    ObservationInconclusive(String),
    IncompleteTimeout { complete: bool },
}

impl PreTerminalResult {
    pub(super) fn verdict(&self) -> Verdict {
        match self {
            Self::Satisfied => Verdict::Pass,
            Self::TrustedFailure(_) | Self::IncompleteTimeout { complete: true } => Verdict::Fail,
            Self::ObservationInconclusive(_) | Self::IncompleteTimeout { complete: false } => {
                Verdict::Inconclusive
            }
        }
    }

    fn reason(&self) -> &str {
        match self {
            Self::Satisfied => "pre-terminal physical evidence is complete",
            Self::TrustedFailure(reason) | Self::ObservationInconclusive(reason) => reason,
            Self::IncompleteTimeout { complete: true } => {
                "expected physical evidence did not arrive before the pre-terminal deadline"
            }
            Self::IncompleteTimeout { complete: false } => {
                "event stream remained truncated at the pre-terminal deadline"
            }
        }
    }
}

pub(super) fn wait_for_sink_events(
    path: &Path,
    cursor: LogCursor,
    ready: &super::ReadyRecord,
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
) -> PreTerminalResult {
    let mut reader = FileEventWindowReader {
        path,
        cursor,
        ready,
        role: RECEIVE_ONLY_ROLE,
    };
    let mut clock = RealDrainClock {
        started: Instant::now(),
        poll_ms: DRAIN_POLL_MS / 2,
    };
    wait_for_sink_events_with(&mut reader, &mut clock, expected_down, expected_up)
}

pub(super) fn wait_for_sink_events_with<R, C>(
    reader: &mut R,
    clock: &mut C,
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
) -> PreTerminalResult
where
    R: super::EventWindowReader,
    C: super::DrainClock,
{
    loop {
        let now = clock.elapsed_ms();
        let sample = match reader.read() {
            Ok(sample) => sample,
            Err(error) => return PreTerminalResult::ObservationInconclusive(error),
        };
        match validate_event_prefix(&sample.events, expected_down, expected_up, false) {
            Ok(true) if sample.complete => return PreTerminalResult::Satisfied,
            Ok(_) => {}
            Err(error) => return PreTerminalResult::TrustedFailure(error),
        }
        if now >= super::PRETERMINAL_DEADLINE_MS {
            return PreTerminalResult::IncompleteTimeout {
                complete: sample.complete,
            };
        }
        clock.wait_poll();
    }
}

pub(super) fn finish_preterminal(
    args: &super::RunArgs,
    session: &NativeDispatchSession,
    result: PreTerminalResult,
) -> Option<i32> {
    if matches!(result, PreTerminalResult::Satisfied) {
        return None;
    }
    let observer_verdict = result.verdict();
    let _ = session.quit();
    let _ = session.join(Duration::from_secs(5));
    let snapshot = session.snapshot();
    let release = snapshot.release_outcome.as_ref();
    let evidence = NativeCleanupEvidence {
        terminal_error: snapshot.terminal_error.is_some(),
        partial_events: snapshot.sendinput_partial_events,
        zero_progress_events: snapshot.sendinput_zero_progress_failures,
        active_count: snapshot.active_count,
        possibly_active_count: snapshot.possibly_active_count,
        failed_release_count: snapshot.failed_release_count,
        release_obligation_mask: snapshot.final_release_obligation_mask,
        release_failed: release.is_some_and(|outcome| !outcome.released_successfully),
        stuck_mask: release.map_or(0, |outcome| outcome.stuck_mask),
        verification_inconclusive: release.is_some_and(|outcome| outcome.verification_inconclusive),
        transport_anomaly: release.is_some_and(|outcome| outcome.transport_anomaly),
    };
    let cleanup_anomaly = evidence.is_anomalous();
    let verdict = super::preterminal_verdict(observer_verdict, cleanup_anomaly);
    let reason = if cleanup_anomaly && observer_verdict != Verdict::Fail {
        "native terminal cleanup/residue evidence is not clean".to_string()
    } else {
        result.reason().to_string()
    };
    Some(write_report(
        args,
        verdict,
        &reason,
        super::snapshot_json(&snapshot),
    ))
}

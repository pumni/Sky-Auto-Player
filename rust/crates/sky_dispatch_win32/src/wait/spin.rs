use super::{WaitFailure, WaitOutcome, WaitResult};
use crate::clock::QpcTicks;
use crate::event::OwnedEvent;
pub(crate) fn spin_duration_ticks(
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
) -> Result<crate::clock::DurationTicks, WaitFailure> {
    let Some(started_ticks) = started_ticks else {
        return Ok(crate::clock::DurationTicks::ZERO);
    };
    completed_ticks
        .checked_duration_since(started_ticks)
        .map_err(|_| WaitFailure::Clock)
}

pub(crate) fn wait_result_with_spin(
    outcome: WaitOutcome,
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
) -> WaitResult {
    match spin_duration_ticks(started_ticks, completed_ticks) {
        Ok(spin_ticks) => WaitResult {
            outcome,
            wake_qpc: Some(completed_ticks),
            spin_ticks,
        },
        Err(failure) => WaitResult {
            outcome: WaitOutcome::Failed(failure),
            wake_qpc: Some(completed_ticks),
            spin_ticks: crate::clock::DurationTicks::ZERO,
        },
    }
}

pub(crate) fn deadline_wait_result(
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
    interrupt: &OwnedEvent,
    event_wait_enabled: bool,
    observed_generation: u64,
) -> WaitResult {
    if event_wait_enabled && interrupt.signal_generation() != observed_generation {
        // The command atomics are the authoritative final admission state.
        // The auto-reset event is intentionally not consumed here: doing so
        // would put WaitForSingleObject(..., 0) back in the precision path.
        return wait_result_with_spin(WaitOutcome::Interrupted, started_ticks, completed_ticks);
    }
    wait_result_with_spin(WaitOutcome::Deadline, started_ticks, completed_ticks)
}

#[cfg(test)]
mod tests {
    use super::deadline_wait_result;
    use crate::event::OwnedEvent;
    use crate::wait::WaitOutcome;

    #[test]
    fn successful_deadline_handoff_does_not_consume_the_event() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(event.signal());
        let observed_generation = event.signal_generation();

        let result = deadline_wait_result(
            None,
            crate::clock::QpcTicks::ZERO,
            &event,
            true,
            observed_generation,
        );

        assert_eq!(result.outcome, WaitOutcome::Deadline);
        assert_eq!(event.take_count(), 0);
    }

    #[test]
    fn generation_change_replans_without_consuming_the_event() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        let observed_generation = event.signal_generation();
        assert!(event.signal());

        let result = deadline_wait_result(
            None,
            crate::clock::QpcTicks::ZERO,
            &event,
            true,
            observed_generation,
        );

        assert_eq!(result.outcome, WaitOutcome::Interrupted);
        assert_eq!(event.take_count(), 0);
    }
}

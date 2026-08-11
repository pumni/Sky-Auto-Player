use super::{WaitFailure, WaitOutcome, WaitResult};
use crate::clock::QpcTicks;
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
) -> WaitResult {
    wait_result_with_spin(WaitOutcome::Deadline, started_ticks, completed_ticks)
}

#[cfg(test)]
mod tests {
    use super::deadline_wait_result;
    use crate::wait::WaitOutcome;

    #[test]
    fn completed_deadline_is_always_classified_as_deadline() {
        let result = deadline_wait_result(None, crate::clock::QpcTicks::ZERO);
        assert_eq!(result.outcome, WaitOutcome::Deadline);
    }

    #[test]
    fn completed_deadline_preserves_spin_duration() {
        let result = deadline_wait_result(
            Some(crate::clock::QpcTicks::from_raw(1_000)),
            crate::clock::QpcTicks::from_raw(1_500),
        );

        assert_eq!(result.outcome, WaitOutcome::Deadline);
        assert_eq!(
            result.spin_ticks,
            crate::clock::DurationTicks::from_raw(500)
        );
    }
}

use super::{WaitFailure, WaitOutcome, WaitResult};
use crate::clock::{QpcClock, QpcTicks};
use crate::event::OwnedEvent;
pub(crate) fn spin_duration_us(
    qpc_clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
) -> Result<u64, WaitFailure> {
    let Some(started_ticks) = started_ticks else {
        return Ok(0);
    };
    let elapsed_ticks = completed_ticks
        .checked_duration_since(started_ticks)
        .map_err(|_| WaitFailure::Clock)?;
    qpc_clock
        .duration_to_us(elapsed_ticks)
        .map_err(|_| WaitFailure::Clock)
}

pub(crate) fn wait_result_with_spin(
    outcome: WaitOutcome,
    qpc_clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
) -> WaitResult {
    match spin_duration_us(qpc_clock, started_ticks, completed_ticks) {
        Ok(spin_us) => WaitResult { outcome, spin_us },
        Err(failure) => WaitResult {
            outcome: WaitOutcome::Failed(failure),
            spin_us: 0,
        },
    }
}

pub(crate) fn deadline_wait_result(
    qpc_clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    completed_ticks: QpcTicks,
    interrupt: &OwnedEvent,
    event_wait_enabled: bool,
    observed_generation: u64,
) -> WaitResult {
    if event_wait_enabled {
        match (
            interrupt.signal_generation() != observed_generation,
            interrupt.try_take(),
        ) {
            (_, true) => {
                // The one final zero-time consume is allowed at the deadline
                // handoff, including the SetEvent-to-generation publication
                // window. There is no event poll in the spin loop itself.
                return wait_result_with_spin(
                    WaitOutcome::Interrupted,
                    qpc_clock,
                    started_ticks,
                    completed_ticks,
                );
            }
            (true, false) => {
                // A changed generation without an owned event means another
                // consumer won the auto-reset event; the deadline is safe.
            }
            (false, false) => {}
        }
    }
    wait_result_with_spin(
        WaitOutcome::Deadline,
        qpc_clock,
        started_ticks,
        completed_ticks,
    )
}

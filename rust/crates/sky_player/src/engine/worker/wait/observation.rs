use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcTicks;
use sky_dispatch_win32::wait::WaitOutcome;

#[derive(Clone, Copy, Debug)]
pub struct WaitObservation {
    pub outcome: WaitOutcome,
    pub wake_qpc: Option<QpcTicks>,
    pub spin_ticks: DurationTicks,
    pub physical_target_qpc: QpcTicks,
    pub planned_wait_ticks: DurationTicks,
    pub deadline_ticks: TimelineTicks,
    pub epoch_qpc: QpcTicks,
    pub allow_pre_epoch_startup_dispatch: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_arm_target_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_return_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))]
    pub deadline_crossing_qpc: Option<QpcTicks>,
    #[cfg(any(test, feature = "test-support"))]
    pub waiter_return_qpc: Option<QpcTicks>,
}

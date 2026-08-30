use super::super::WorkerRuntime;
use sky_dispatch_win32::clock::QpcTicks;

pub(crate) fn take_deadline_wake_qpc(
    runtime: &mut WorkerRuntime,
    _final_policy_qpc: QpcTicks,
) -> Option<QpcTicks> {
    runtime.last_dispatch_deadline_wake_qpc.take()
}

#[cfg(test)]
mod tests {
    use super::take_deadline_wake_qpc;
    use crate::engine::worker::WorkerRuntime;
    use sky_dispatch_win32::clock::QpcTicks;

    #[test]
    fn deadline_wake_is_consumed_by_only_one_observation() {
        let mut runtime = WorkerRuntime {
            last_dispatch_deadline_wake_qpc: Some(QpcTicks::from_raw(100)),
            ..WorkerRuntime::default()
        };
        assert_eq!(
            take_deadline_wake_qpc(&mut runtime, QpcTicks::from_raw(125)),
            Some(QpcTicks::from_raw(100))
        );
        assert_eq!(
            take_deadline_wake_qpc(&mut runtime, QpcTicks::from_raw(150)),
            None
        );
    }
}

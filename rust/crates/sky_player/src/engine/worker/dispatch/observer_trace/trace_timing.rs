use super::super::observation::DownObservation;
use crate::engine::TraceTiming;

pub(in crate::engine::worker::dispatch) struct DownSendTraceTiming {
    pub(in crate::engine::worker::dispatch) core_post_send_us: u64,
    pub(in crate::engine::worker::dispatch) completion_residual_us: u64,
    pub(in crate::engine::worker::dispatch) post_send_metrics_available: bool,
    pub(in crate::engine::worker::dispatch) completion_error_ticks: i64,
    pub(in crate::engine::worker::dispatch) authored_completion_error_ticks: i64,
    pub(in crate::engine::worker::dispatch) pre_call_ticks:
        Option<sky_dispatch_core::time::TimelineTicks>,
    pub(in crate::engine::worker::dispatch) sendinput_completion_ticks:
        Option<sky_dispatch_core::time::TimelineTicks>,
    pub(in crate::engine::worker::dispatch) wake_ticks:
        Option<sky_dispatch_core::time::TimelineTicks>,
    pub(in crate::engine::worker::dispatch) observer_qpc_ticks: sky_dispatch_win32::clock::QpcTicks,
}

pub(in crate::engine::worker::dispatch) fn down_send_trace_timing(
    observation: &DownObservation,
    timing: DownSendTraceTiming,
) -> TraceTiming {
    let trace = observation.trace;
    TraceTiming {
        authored_ticks: trace.authored_ticks,
        effective_deadline_ticks: trace.effective_deadline_ticks,
        wake_ticks: timing.wake_ticks,
        wake_qpc_ticks: observation.wake_qpc.map(|ticks| ticks.as_u64()),
        physical_eligibility_qpc_ticks: Some(
            observation
                .physical_timing_window
                .packet_not_before_qpc
                .as_u64(),
        ),
        #[cfg(any(test, feature = "test-support"))]
        waiter_deadline_crossing_qpc_ticks: observation.precision_handoff.and_then(|handoff| {
            handoff
                .waiter_deadline_crossing_qpc
                .map(|ticks| ticks.as_u64())
        }),
        #[cfg(not(any(test, feature = "test-support")))]
        waiter_deadline_crossing_qpc_ticks: None,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_qpc_ticks: observation
            .precision_handoff
            .and_then(|handoff| handoff.timer_arm_qpc.map(|ticks| ticks.as_u64())),
        #[cfg(not(any(test, feature = "test-support")))]
        timer_arm_qpc_ticks: None,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_target_qpc_ticks: observation
            .precision_handoff
            .and_then(|handoff| handoff.timer_arm_target_qpc.map(|ticks| ticks.as_u64())),
        #[cfg(not(any(test, feature = "test-support")))]
        timer_arm_target_qpc_ticks: None,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_generation: observation
            .precision_handoff
            .and_then(|handoff| handoff.timer_arm_generation),
        #[cfg(not(any(test, feature = "test-support")))]
        timer_arm_generation: None,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_generation: observation
            .precision_handoff
            .and_then(|handoff| handoff.timer_return_generation),
        #[cfg(not(any(test, feature = "test-support")))]
        timer_return_generation: None,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_qpc_ticks: observation
            .precision_handoff
            .and_then(|handoff| handoff.timer_return_qpc.map(|ticks| ticks.as_u64())),
        #[cfg(not(any(test, feature = "test-support")))]
        timer_return_qpc_ticks: None,
        #[cfg(any(test, feature = "test-support"))]
        waiter_return_qpc_ticks: observation
            .precision_handoff
            .and_then(|handoff| handoff.waiter_return_qpc.map(|ticks| ticks.as_u64())),
        #[cfg(not(any(test, feature = "test-support")))]
        waiter_return_qpc_ticks: None,
        target_crossing_qpc_ticks: observation
            .precision_handoff
            .and_then(|handoff| handoff.target_crossing_qpc.map(|ticks| ticks.as_u64())),
        physical_target_qpc_ticks: Some(observation.physical_target_qpc.as_u64()),
        physical_not_before_qpc_ticks: Some(
            observation
                .physical_timing_window
                .packet_not_before_qpc
                .as_u64(),
        ),
        hold_floor_qpc_ticks: Some(
            observation
                .physical_timing_window
                .musical_up_not_before_qpc
                .as_u64(),
        ),
        release_floor_qpc_ticks: Some(
            observation
                .physical_timing_window
                .down_not_before_qpc
                .as_u64(),
        ),
        previous_sendinput_completion_qpc_ticks: observation
            .previous_sendinput_completion_qpc
            .map(|ticks| ticks.as_u64()),
        previous_sender_completion_known: observation.previous_sender_completion_known,
        hold_floor_mask: observation.physical_timing_window.hold_floor_mask,
        release_floor_mask: observation.physical_timing_window.release_floor_mask,
        final_policy_qpc_ticks: observation.final_policy_qpc.map(|ticks| ticks.as_u64()),
        pre_call_qpc_ticks: observation.pre_call_qpc.map(|ticks| ticks.as_u64()),
        sendinput_completion_qpc_ticks: Some(observation.sendinput_completion_qpc.as_u64()),
        post_send_qpc_ticks: observation.dispatch_ready_qpc.map(|ticks| ticks.as_u64()),
        observation_qpc_ticks: Some(timing.observer_qpc_ticks.as_u64()),
        pre_call_ticks: timing.pre_call_ticks,
        sendinput_completion_ticks: timing.sendinput_completion_ticks,
        completion_residual_us: timing.completion_residual_us,
        core_post_send_duration_us: timing.core_post_send_us,
        post_send_metrics_available: timing.post_send_metrics_available,
        completion_error_ticks: timing.completion_error_ticks,
        authored_completion_error_ticks: timing.authored_completion_error_ticks,
    }
}

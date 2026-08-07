//! Deferred observer state and the drain that consumes it.
//!
//! This module owns the allocation-free `DispatchObservation` snapshot, its
//! fixed-capacity ring queue, and the `drain_down_send_outcome` function the
//! dispatch loop runs during idle slack.  Keeping these separate from
//! `observer.rs` respects the module line-limit invariants while preserving
//! the hard-path boundary: the drain is the only place that touches the
//! estimator, health windows, lateness accounting, and metric publication for
//! a note-on observation.

use super::super::super::{
    ActionKind, LatencyClass, QpcClock, SendLatencyEstimator, SharedMetrics, TrackedKeyState,
    try_publish_metrics,
};
use super::super::{
    DispatchHealthObservation, DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    observe_dispatch_health, publish_backend_metrics, record_lateness, record_lead_saturation,
    signed_delta, update_estimator_after_send_class,
};

/// Fixed capacity of the worker thread's deferred dispatch-observation queue.
/// Owner: the single worker thread.  Drops the oldest sample when full so the
/// estimator and health windows prefer the most recent evidence.
pub(crate) const OBSERVATION_QUEUE_CAPACITY: usize = 64;

/// One complete, allocation-free snapshot of a dispatched send, captured at the
/// `dispatch_ready` boundary (after the coordinator commit and the mandatory
/// telemetry trace).  Consumed later by the observer drain (estimator update,
/// health windows, lateness, diagnostic metric publication).  All fields are
/// plain `Copy` scalars or `Copy` enums.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DispatchObservation {
    Down(DownObservation),
    Up(UpObservation),
}

/// Note-on portion of a dispatched-send snapshot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DownObservation {
    pub path: DispatchPath,
    pub latency_class: LatencyClass,
    pub estimator_kind: Option<ActionKind>,
    pub lead_down_saturated: bool,
    pub lead_down: u64,
    pub sender_duration_us: u64,
    pub delivered_count: usize,
    pub batch_intent_count: usize,
    pub completion_error_us: i64,
    pub clean_directional_sample: bool,
    pub completed_effective: u64,
    pub authored_batch_scheduled_us: u64,
    pub batch_scheduled_us: u64,
    /// §8.6 typed `dispatch_ready_q - sender_completed_q`, in microseconds.
    /// Replaces the old `iteration_ready_us - completed_effective` subtraction.
    pub core_post_send_us: u64,
    pub send_warn_us: u64,
    pub bookkeeping_warn_us: u64,
    pub force_publish: bool,
}

/// Note-off portion of a dispatched-send snapshot.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UpObservation {
    pub latency_class: LatencyClass,
    pub sender_duration_us: u64,
    pub sent_count: usize,
    pub scan_count: usize,
    pub lead_up: u64,
    pub lead_up_saturated: bool,
    pub completed_effective: u64,
    pub scheduled_us: u64,
    pub deferred_by_us: u64,
    pub up_completion_error_us: i64,
    pub clean_up_sample: bool,
    /// §8.6 typed `dispatch_ready_q - sender_completed_q`, in microseconds.
    pub core_post_send_us: u64,
    pub send_warn_us: u64,
    pub bookkeeping_warn_us: u64,
    pub force_publish: bool,
}

/// Fixed-size, allocation-free ring buffer of deferred dispatch observations.
/// `push` is O(1) and drops the oldest entry when the ring is full, incrementing
/// the observer-dropped counter in the caller and updating the queue
/// high-watermark.  Never blocks and never resizes.
#[derive(Debug)]
pub(crate) struct PendingObservationQueue {
    entries: [Option<DispatchObservation>; OBSERVATION_QUEUE_CAPACITY],
    head: usize,
    len: usize,
}

impl Default for PendingObservationQueue {
    fn default() -> Self {
        Self {
            entries: [None; OBSERVATION_QUEUE_CAPACITY],
            head: 0,
            len: 0,
        }
    }
}

impl PendingObservationQueue {
    pub(crate) fn push(
        &mut self,
        observation: DispatchObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
    ) {
        if self.len == self.entries.len() {
            // Full: drop the oldest entry so the newest evidence is retained.
            self.entries[self.head] = None;
            self.head = (self.head + 1) % self.entries.len();
            *dropped_samples = dropped_samples.saturating_add(1);
        } else {
            self.len += 1;
        }
        let tail = (self.head + self.len - 1) % self.entries.len();
        self.entries[tail] = Some(observation);
        *high_watermark = (*high_watermark).max(self.len as u64);
    }

    pub(crate) fn pop_front(&mut self) -> Option<DispatchObservation> {
        if self.len == 0 {
            return None;
        }
        let entry = self.entries[self.head].take();
        self.head = (self.head + 1) % self.entries.len();
        self.len -= 1;
        entry
    }
}

/// Deferred observer consumed by the dispatch loop's slack budget.  Applies
/// the estimator update, health-window observation, lateness accounting, and
/// diagnostic metric publication for one previously-enqueued note-on sample.
/// Every mutation here is droppable and never terminates the worker; nothing
/// in this function may allocate.  Uses the `core_post_send_us` typed boundary
/// captured at the hard-phase `dispatch_ready` sample.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_down_send_outcome(
    observation: &DownObservation,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    now_us: u64,
) {
    local_metrics.send_warn_threshold_us = observation.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = observation.bookkeeping_warn_us;
    match observation.path {
        DispatchPath::DownOnly { .. } => {
            local_metrics.send_down_warn_threshold_us = observation.send_warn_us;
        }
        DispatchPath::UpOnly { .. } => {
            local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
        }
        DispatchPath::Mixed { .. } => {
            local_metrics.send_mixed_warn_threshold_us = observation.send_warn_us;
        }
    }
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    local_metrics.core_post_send_max_us = local_metrics
        .core_post_send_max_us
        .max(observation.core_post_send_us);
    if config.estimator.enable_adaptive_lead && observation.lead_down_saturated {
        match observation.path {
            DispatchPath::UpOnly { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_up,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(
                    observation.completed_effective,
                    observation.batch_scheduled_us,
                ),
            ),
            DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_down,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(
                    observation.completed_effective,
                    observation.batch_scheduled_us,
                ),
            ),
        }
    }
    if config.estimator.enable_adaptive_lead
        && let Some(kind) = observation.estimator_kind
    {
        // Deferred estimator updates are droppable by design: a failure here
        // must never terminate the worker, so it is swallowed and left to the
        // next sample to recover.
        let _ = update_estimator_after_send_class(
            estimator,
            kind,
            observation.sender_duration_us,
            observation.delivered_count,
            observation.batch_intent_count,
            observation.lead_down,
            observation.completion_error_us,
            observation.clean_directional_sample,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(
            observation.completed_effective,
            observation.authored_batch_scheduled_us,
        ),
        false,
        false,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: observation.sender_duration_us,
            post_send_duration_us: observation.core_post_send_us,
            path: observation.path,
            send_warn_us: observation.send_warn_us,
            bookkeeping_warn_us: observation.bookkeeping_warn_us,
            elapsed_us: observation.completed_effective,
        },
        health.options.window_policy(),
        &mut health.send_pure_window,
        &mut health.bookkeeping_window,
        local_metrics,
    );
    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    try_publish_metrics(local_metrics, metrics, now_us, observation.force_publish);
}

/// Deferred observer for a note-off release.  Mirrors the note-on drain:
/// lead-saturation accounting, adaptive-lead estimator update, lateness
/// accounting, health-window observation, and diagnostic metric publication.
/// Every mutation is droppable and never terminates the worker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_up_send_outcome(
    observation: &UpObservation,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    now_us: u64,
) {
    local_metrics.send_warn_threshold_us = observation.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = observation.bookkeeping_warn_us;
    local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    local_metrics.core_post_send_max_us = local_metrics
        .core_post_send_max_us
        .max(observation.core_post_send_us);
    if config.estimator.enable_adaptive_lead && observation.lead_up_saturated {
        record_lead_saturation(
            &mut local_metrics.lead_saturation_count_up,
            &mut local_metrics.positive_residual_at_cap,
            observation.scan_count,
            signed_delta(observation.completed_effective, observation.scheduled_us),
        );
    }
    if config.estimator.enable_adaptive_lead {
        let _ = update_estimator_after_send_class(
            estimator,
            ActionKind::Up,
            observation.sender_duration_us,
            observation.sent_count,
            observation.scan_count,
            observation.lead_up,
            observation.up_completion_error_us,
            observation.clean_up_sample,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(observation.completed_effective, observation.scheduled_us),
        true,
        observation.deferred_by_us > 0,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: observation.sender_duration_us,
            post_send_duration_us: observation.core_post_send_us,
            path: DispatchPath::UpOnly {
                up_count: observation.scan_count,
            },
            send_warn_us: observation.send_warn_us,
            bookkeeping_warn_us: observation.bookkeeping_warn_us,
            elapsed_us: observation.completed_effective,
        },
        health.options.window_policy(),
        &mut health.send_pure_window,
        &mut health.bookkeeping_window,
        local_metrics,
    );
    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    try_publish_metrics(local_metrics, metrics, now_us, observation.force_publish);
}

/// Pops and drains at most one observation, returning the observed drain
/// duration in microseconds (0 when nothing was drained or on clock error).
/// The observer drain is droppable by design, so a QPC failure here degrades
/// to 0 instead of terminating the worker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_one_observer(
    pending: &mut PendingObservationQueue,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    qpc_clock: QpcClock,
    now_us: u64,
) -> u64 {
    let Some(observation) = pending.pop_front() else {
        return 0;
    };
    let drain_start = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => return 0,
    };
    match &observation {
        DispatchObservation::Down(down) => drain_down_send_outcome(
            down,
            config,
            health,
            local_metrics,
            last_published_error,
            metrics,
            backend,
            estimator,
            now_us,
        ),
        DispatchObservation::Up(up) => drain_up_send_outcome(
            up,
            config,
            health,
            local_metrics,
            last_published_error,
            metrics,
            backend,
            estimator,
            now_us,
        ),
    }
    let drain_end = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => return 0,
    };
    match drain_end.checked_duration_since(drain_start) {
        Ok(duration) => qpc_clock.duration_to_us(duration).unwrap_or_default(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::worker::health::DispatchPath;

    fn sample(n: u64) -> DispatchObservation {
        DispatchObservation::Down(DownObservation {
            path: DispatchPath::DownOnly { down_count: 1 },
            latency_class: LatencyClass::Hot,
            estimator_kind: Some(ActionKind::Down),
            lead_down_saturated: false,
            lead_down: n,
            sender_duration_us: n,
            delivered_count: 1,
            batch_intent_count: 1,
            completion_error_us: 0,
            clean_directional_sample: false,
            completed_effective: n,
            authored_batch_scheduled_us: 0,
            batch_scheduled_us: 0,
            core_post_send_us: 1,
            send_warn_us: 0,
            bookkeeping_warn_us: 0,
            force_publish: false,
        })
    }

    #[test]
    fn drop_oldest_when_full_preserves_newest() {
        let mut dropped = 0u64;
        let mut high_watermark = 0u64;
        let mut queue = PendingObservationQueue::default();
        let capacity = OBSERVATION_QUEUE_CAPACITY;
        for i in 0..capacity + 3 {
            queue.push(sample(i as u64), &mut dropped, &mut high_watermark);
        }
        // Capacity bounded: the three oldest samples were dropped.
        assert_eq!(dropped, 3);
        assert_eq!(high_watermark, capacity as u64);
        // The retained samples are the newest three, in FIFO order.
        let mut seen = Vec::new();
        while let Some(obs) = queue.pop_front() {
            match obs {
                DispatchObservation::Down(down) => seen.push(down.lead_down),
                DispatchObservation::Up(_) => panic!("unexpected up observation"),
            }
        }
        assert_eq!(seen[..3], [3, 4, 5]);
        assert_eq!(seen.len(), capacity);
    }

    #[test]
    fn pop_front_is_fifo_and_empty_after_drain() {
        let mut queue = PendingObservationQueue::default();
        let mut dropped = 0u64;
        let mut high = 0u64;
        queue.push(sample(10), &mut dropped, &mut high);
        queue.push(sample(20), &mut dropped, &mut high);
        let lead_of = |obs: DispatchObservation| -> u64 {
            match obs {
                DispatchObservation::Down(down) => down.lead_down,
                DispatchObservation::Up(_) => 0,
            }
        };
        assert_eq!(queue.pop_front().map(lead_of), Some(10));
        assert_eq!(queue.pop_front().map(lead_of), Some(20));
        assert_eq!(queue.pop_front().map(lead_of), None);
    }
}

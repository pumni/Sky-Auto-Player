use super::TrackedKeyState;
use crate::engine::telemetry::{SharedMetrics, WorkerMetricsLocal};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::ActionKind;

pub(crate) const SEND_WARNING_MARGIN_US: u64 = 50;
pub(crate) const HEALTH_WINDOW_CAPACITY: usize = 64;
/// Existing cold-start per-event prior used by the estimator for a mixed
/// packet. Mixed packets are one syscall, so this is an event increment, not
/// the sum of two independent directional leads.
pub(crate) const MIXED_PACKET_PER_EXTRA_EVENT_US: u64 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DispatchHealthOptions {
    pub(crate) send_warn_floor_us: u64,
    pub(crate) bookkeeping_warn_us: u64,
    pub(crate) wait_warn_us: u64,
    pub(crate) window_capacity: usize,
    pub(crate) bad_sample_count: usize,
    pub(crate) degrade_hold_us: u64,
    pub(crate) recovery_hold_us: u64,
}

impl Default for DispatchHealthOptions {
    fn default() -> Self {
        Self {
            send_warn_floor_us: 300,
            bookkeeping_warn_us: 300,
            wait_warn_us: 300,
            window_capacity: HEALTH_WINDOW_CAPACITY,
            bad_sample_count: 4,
            degrade_hold_us: 1_000_000,
            recovery_hold_us: 2_000_000,
        }
    }
}

impl DispatchHealthOptions {
    pub(crate) fn send_warn_threshold_us(self, expected_send_us: u64) -> u64 {
        self.send_warn_floor_us
            .max(expected_send_us.saturating_add(SEND_WARNING_MARGIN_US))
    }

    pub(crate) fn window_policy(self) -> HealthWindowPolicy {
        HealthWindowPolicy {
            minimum_samples: self.window_capacity.min(HEALTH_WINDOW_CAPACITY),
            bad_sample_count: self.bad_sample_count,
            degrade_hold_us: self.degrade_hold_us,
            recovery_hold_us: self.recovery_hold_us,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HealthState {
    Healthy,
    Degraded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HealthTransition {
    None,
    EnteredDegraded,
    Recovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HealthWindowPolicy {
    pub(crate) minimum_samples: usize,
    pub(crate) bad_sample_count: usize,
    pub(crate) degrade_hold_us: u64,
    pub(crate) recovery_hold_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchPath {
    DownOnly { down_count: usize },
    UpOnly { up_count: usize },
    Mixed { up_count: usize, down_count: usize },
}

impl DispatchPath {
    pub(crate) fn event_count(self) -> usize {
        self.down_count().saturating_add(self.up_count())
    }

    pub(crate) fn down_count(self) -> usize {
        match self {
            Self::DownOnly { down_count } | Self::Mixed { down_count, .. } => down_count,
            Self::UpOnly { .. } => 0,
        }
    }

    pub(crate) fn up_count(self) -> usize {
        match self {
            Self::UpOnly { up_count } | Self::Mixed { up_count, .. } => up_count,
            Self::DownOnly { .. } => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrozenDispatchBudget {
    pub(crate) path: DispatchPath,
    pub(crate) observed_polyphony: usize,
    pub(crate) send_warn_us: u64,
    pub(crate) bookkeeping_warn_us: u64,
}

pub(crate) fn build_dispatch_budget(
    estimator: &SendLatencyEstimator,
    path: DispatchPath,
    latency_class: LatencyClass,
    options: DispatchHealthOptions,
    strict_timing: bool,
) -> FrozenDispatchBudget {
    let estimate_for = |kind: ActionKind, count: usize| {
        estimator.estimate_lead_with_class_and_policy(kind, count, latency_class, strict_timing)
    };
    let expected_send_us = match path {
        DispatchPath::DownOnly { down_count } => {
            let estimate = estimate_for(ActionKind::Down, down_count);
            let cold = estimator
                .estimate_lead_with_class_and_policy(
                    ActionKind::Down,
                    down_count,
                    LatencyClass::Cold,
                    strict_timing,
                )
                .components
                .cold_reserve_us;
            estimate.components.syscall_us.max(cold)
        }
        DispatchPath::UpOnly { up_count } => {
            let estimate = estimate_for(ActionKind::Up, up_count);
            let cold = estimator
                .estimate_lead_with_class_and_policy(
                    ActionKind::Up,
                    up_count,
                    LatencyClass::Cold,
                    strict_timing,
                )
                .components
                .cold_reserve_us;
            estimate.components.syscall_us.max(cold)
        }
        DispatchPath::Mixed {
            up_count,
            down_count,
        } => {
            let up = estimate_for(ActionKind::Up, up_count);
            let down = estimate_for(ActionKind::Down, down_count);
            up.components
                .syscall_us
                .max(down.components.syscall_us)
                .saturating_add(
                    MIXED_PACKET_PER_EXTRA_EVENT_US
                        .saturating_mul(path.event_count().saturating_sub(1) as u64),
                )
        }
    };
    FrozenDispatchBudget {
        path,
        observed_polyphony: path.event_count(),
        send_warn_us: options.send_warn_threshold_us(expected_send_us),
        bookkeeping_warn_us: options.bookkeeping_warn_us,
    }
}

/// Fixed-capacity classification history for one performance signal.
///
/// The ring deliberately stores only the result of comparing an observation
/// with the budget frozen for that observation. Retaining raw durations would
/// allow a later estimator/polyphony threshold to reclassify history while it
/// is being evicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HealthWindow<const N: usize> {
    over_budget: [bool; N],
    next: usize,
    len: usize,
    bad_count: usize,
    degraded: bool,
    bad_started_us: Option<u64>,
    healthy_started_us: Option<u64>,
}

impl<const N: usize> Default for HealthWindow<N> {
    fn default() -> Self {
        Self {
            over_budget: [false; N],
            next: 0,
            len: 0,
            bad_count: 0,
            degraded: false,
            bad_started_us: None,
            healthy_started_us: None,
        }
    }
}

impl<const N: usize> HealthWindow<N> {
    pub(crate) fn observe(
        &mut self,
        over_budget: bool,
        now_us: u64,
        policy: HealthWindowPolicy,
    ) -> HealthTransition {
        if N == 0 || policy.minimum_samples == 0 || policy.bad_sample_count == 0 {
            self.reset();
            return HealthTransition::None;
        }

        let minimum_samples = policy.minimum_samples.min(N);
        if self.len == N {
            if self.over_budget[self.next] {
                self.bad_count = self.bad_count.saturating_sub(1);
            }
        } else {
            self.len += 1;
        }
        self.over_budget[self.next] = over_budget;
        if over_budget {
            self.bad_count += 1;
        }
        self.next = (self.next + 1) % N;

        if self.len < minimum_samples {
            self.bad_started_us = None;
            self.healthy_started_us = None;
            return HealthTransition::None;
        }

        let bad_window = self.bad_count >= policy.bad_sample_count;
        if bad_window {
            self.healthy_started_us = None;
            let started = self.bad_started_us.get_or_insert(now_us);
            if !self.degraded && now_us.saturating_sub(*started) >= policy.degrade_hold_us {
                self.degraded = true;
                return HealthTransition::EnteredDegraded;
            }
        } else {
            self.bad_started_us = None;
            if self.degraded {
                let started = self.healthy_started_us.get_or_insert(now_us);
                if now_us.saturating_sub(*started) >= policy.recovery_hold_us {
                    self.degraded = false;
                    self.healthy_started_us = None;
                    return HealthTransition::Recovered;
                }
            } else {
                self.healthy_started_us = None;
            }
        }
        HealthTransition::None
    }

    pub(crate) fn state(self) -> HealthState {
        if self.degraded {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        }
    }

    pub(crate) fn is_degraded(self) -> bool {
        matches!(self.state(), HealthState::Degraded)
    }

    pub(crate) fn bad_count(self) -> usize {
        self.bad_count
    }

    pub(crate) fn sample_count(self) -> usize {
        self.len
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(crate) fn record_degraded_sample(value_us: u64, threshold_us: u64, samples: &mut u64) {
    if value_us > threshold_us {
        *samples = samples.saturating_add(1);
    }
}

pub(crate) fn record_lateness(
    lateness_us: i64,
    is_release: bool,
    deferred_release: bool,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if deferred_release {
        return;
    }
    let clamped = lateness_us.max(0) as u64;
    local_metrics.lateness_us = clamped;
    if is_release {
        local_metrics.release_max_us = local_metrics.release_max_us.max(clamped);
        if clamped > 2_000 {
            local_metrics.release_late_2ms = local_metrics.release_late_2ms.saturating_add(1);
        }
        return;
    }
    local_metrics.max_lateness_us = local_metrics.max_lateness_us.max(clamped);
    if clamped > 10_000 {
        local_metrics.late_10ms = local_metrics.late_10ms.saturating_add(1);
    }
    if clamped > 5_000 {
        local_metrics.late_5ms = local_metrics.late_5ms.saturating_add(1);
    }
    if clamped > 2_000 {
        local_metrics.late_2ms = local_metrics.late_2ms.saturating_add(1);
    }
    local_metrics.recent_latencies.push(lateness_us);
}

pub(crate) fn record_input_path_health<const N: usize>(
    observed_us: u64,
    budget_us: u64,
    elapsed_us: u64,
    policy: HealthWindowPolicy,
    window: &mut HealthWindow<N>,
) -> HealthTransition {
    if budget_us == 0 {
        window.reset();
        return HealthTransition::None;
    }
    window.observe(observed_us > budget_us, elapsed_us, policy)
}

pub(crate) fn focus_gate_matches(
    require_focus: bool,
    validated_focus_active: bool,
    target_hwnd: isize,
    foreground_matches: bool,
) -> bool {
    if !require_focus {
        return true;
    }
    let hwnd_matches = target_hwnd != 0 && foreground_matches;
    validated_focus_active && hwnd_matches
}

pub(crate) fn publish_backend_metrics(
    backend: &TrackedKeyState,
    local_metrics: &mut WorkerMetricsLocal,
    shared_metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
) {
    local_metrics.active_count = backend.active_mask.count_ones() as u64;
    local_metrics.keys_dropped = backend.keys_dropped;
    local_metrics.possibly_active_count = backend.possibly_active_mask.count_ones() as u64;
    local_metrics.failed_release_count = backend.failed_release_mask.count_ones() as u64;
    // The healthy dispatch path never takes this lock. Error text is
    // published only when the backend error state changes, including the
    // transition back to None after a successful recovery.
    if last_published_error.as_ref() != backend.last_error.as_ref() {
        let mut published = shared_metrics.last_error.lock();
        *published = backend.last_error.clone();
        *last_published_error = backend.last_error.clone();
    }
    local_metrics.chord_split_events = backend.chord_split_events;
    local_metrics.sendinput_partial_events = backend.sendinput_partial_events;
    local_metrics.sendinput_zero_progress_failures = backend.sendinput_zero_progress_failures;
    local_metrics.chords_rejected = backend.chords_rejected;
    local_metrics.authored_keys_rejected = backend.authored_keys_rejected;
    local_metrics.keys_inserted_before_failure = backend.keys_inserted_before_failure;
    local_metrics.keys_rolled_back = backend.keys_rolled_back;
    local_metrics.rollback_residue_keys = backend.rollback_residue_keys;
}

#[cfg(test)]
mod tests {
    use super::{
        DispatchHealthOptions, DispatchPath, HealthState, HealthTransition, HealthWindow,
        HealthWindowPolicy, SEND_WARNING_MARGIN_US, build_dispatch_budget, record_degraded_sample,
        record_input_path_health,
    };
    use sky_dispatch_core::estimator::LatencyClass;
    use sky_dispatch_core::estimator::SendLatencyEstimator;

    #[test]
    fn send_warning_budget_grows_with_expected_polyphony_cost() {
        let options = DispatchHealthOptions::default();
        assert_eq!(
            options.send_warn_threshold_us(0),
            options.send_warn_floor_us
        );
        assert_eq!(
            options.send_warn_threshold_us(900),
            900 + SEND_WARNING_MARGIN_US
        );
        assert!(options.send_warn_threshold_us(900) > options.send_warn_threshold_us(50));
    }

    #[test]
    fn health_paths_have_independent_default_thresholds() {
        let options = DispatchHealthOptions::default();
        assert_eq!(options.bookkeeping_warn_us, 300);
        assert_eq!(options.wait_warn_us, 300);
        assert_eq!(options.window_capacity, 64);
        assert_eq!(options.bad_sample_count, 4);
        assert_eq!(options.degrade_hold_us, 1_000_000);
        assert_eq!(options.recovery_hold_us, 2_000_000);
    }

    #[test]
    fn degraded_sample_counter_only_counts_samples_over_budget() {
        let mut samples = 0;
        record_degraded_sample(300, 300, &mut samples);
        record_degraded_sample(301, 300, &mut samples);
        assert_eq!(samples, 1);
    }

    #[test]
    fn health_hysteresis_uses_bounded_window_and_time_holds() {
        let mut window = HealthWindow::<4>::default();
        let policy = HealthWindowPolicy {
            minimum_samples: 4,
            bad_sample_count: 2,
            degrade_hold_us: 10,
            recovery_hold_us: 20,
        };
        macro_rules! record {
            ($value:expr, $elapsed:expr) => {
                record_input_path_health($value, 300, $elapsed, policy, &mut window)
            };
        }

        record!(301, 0);
        record!(301, 1);
        assert_eq!(record!(301, 2), HealthTransition::None);
        assert_eq!(record!(301, 3), HealthTransition::None);
        assert_eq!(record!(300, 20), HealthTransition::EnteredDegraded);
        assert_eq!(window.state(), HealthState::Degraded);
        record!(300, 30);
        record!(300, 40);
        record!(300, 50);
        assert_eq!(record!(300, 70), HealthTransition::Recovered);
        assert_eq!(window.state(), HealthState::Healthy);
    }

    #[test]
    fn evicting_sample_uses_original_classification() {
        let mut window = HealthWindow::<4>::default();
        let policy = HealthWindowPolicy {
            minimum_samples: 1,
            bad_sample_count: 4,
            degrade_hold_us: 0,
            recovery_hold_us: 0,
        };
        record_input_path_health(500, 300, 0, policy, &mut window);
        assert_eq!(window.bad_count(), 1);
        record_input_path_health(500, 900, 1, policy, &mut window);
        record_input_path_health(500, 900, 2, policy, &mut window);
        record_input_path_health(500, 900, 3, policy, &mut window);
        record_input_path_health(500, 900, 4, policy, &mut window);
        assert_eq!(window.bad_count(), 0);
    }

    #[test]
    fn zero_budget_resets_tracker() {
        let mut window = HealthWindow::<4>::default();
        let policy = HealthWindowPolicy {
            minimum_samples: 1,
            bad_sample_count: 1,
            degrade_hold_us: 0,
            recovery_hold_us: 0,
        };
        record_input_path_health(501, 500, 0, policy, &mut window);
        assert_eq!(window.state(), HealthState::Degraded);
        window.reset();
        assert_eq!(window.bad_count(), 0);
        assert_eq!(window.sample_count(), 0);
    }

    #[test]
    fn typed_paths_keep_directional_counts() {
        let down = DispatchPath::DownOnly { down_count: 3 };
        let up = DispatchPath::UpOnly { up_count: 2 };
        let mixed = DispatchPath::Mixed {
            up_count: 2,
            down_count: 3,
        };
        assert_eq!(
            (down.down_count(), down.up_count(), down.event_count()),
            (3, 0, 3)
        );
        assert_eq!(
            (up.down_count(), up.up_count(), up.event_count()),
            (0, 2, 2)
        );
        assert_eq!(
            (mixed.down_count(), mixed.up_count(), mixed.event_count()),
            (3, 2, 5)
        );
    }

    #[test]
    fn mixed_budget_is_one_packet_not_two_directional_leads() {
        let estimator = SendLatencyEstimator::default();
        let options = DispatchHealthOptions::default();
        let mixed = build_dispatch_budget(
            &estimator,
            DispatchPath::Mixed {
                up_count: 2,
                down_count: 2,
            },
            LatencyClass::Hot,
            options,
            false,
        );
        let down = build_dispatch_budget(
            &estimator,
            DispatchPath::DownOnly { down_count: 2 },
            LatencyClass::Hot,
            options,
            false,
        );
        let up = build_dispatch_budget(
            &estimator,
            DispatchPath::UpOnly { up_count: 2 },
            LatencyClass::Hot,
            options,
            false,
        );
        assert!(mixed.send_warn_us < down.send_warn_us.saturating_add(up.send_warn_us));
    }
}

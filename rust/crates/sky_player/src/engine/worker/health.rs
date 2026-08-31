use super::{TrackedKeyState, WorkerTimingState};
use crate::engine::telemetry::{SharedMetrics, WorkerMetricsLocal};

pub(crate) const HEALTH_WINDOW_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DispatchHealthOptions {
    pub(crate) sendinput_warn_floor_us: u64,
    pub(crate) core_post_send_warn_us: u64,
    pub(crate) observer_warn_us: u64,
    pub(crate) wait_warn_us: u64,
    pub(crate) window_capacity: usize,
    pub(crate) bad_sample_count: usize,
    pub(crate) degrade_hold_us: u64,
    pub(crate) recovery_hold_us: u64,
}

impl Default for DispatchHealthOptions {
    fn default() -> Self {
        Self {
            sendinput_warn_floor_us: 300,
            core_post_send_warn_us: 5_000,
            observer_warn_us: 300,
            wait_warn_us: 300,
            window_capacity: HEALTH_WINDOW_CAPACITY,
            bad_sample_count: 4,
            degrade_hold_us: 1_000_000,
            recovery_hold_us: 2_000_000,
        }
    }
}

impl DispatchHealthOptions {
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
pub enum DispatchPath {
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
    pub(crate) event_count: usize,
    pub(crate) send_warn_us: u64,
    pub(crate) core_post_send_warn_us: u64,
}

pub(crate) fn build_dispatch_budget(
    path: DispatchPath,
    options: DispatchHealthOptions,
) -> FrozenDispatchBudget {
    let count = path.event_count();

    FrozenDispatchBudget {
        path,
        event_count: count,
        send_warn_us: options.sendinput_warn_floor_us,
        core_post_send_warn_us: options.core_post_send_warn_us,
    }
}

/// Fixed-capacity classification history for one performance signal.
///
/// The ring deliberately stores only the result of comparing an observation
/// with the budget frozen for that observation. Retaining raw durations would
/// allow a later threshold policy to reclassify history while it is being
/// evicted.
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

/// Record the only per-send timing evidence retained by the production
/// profile. Diagnostic profiles derive richer histograms and traces on the
/// deferred observer instead.
pub(crate) fn record_sendinput_pre_call_lateness(
    target_qpc: sky_dispatch_win32::clock::QpcTicks,
    started_qpc: sky_dispatch_win32::clock::QpcTicks,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
) -> Result<(), String> {
    let lateness_ticks = started_qpc
        .checked_duration_since(target_qpc)
        .map_err(|_| "SendInput pre-call preceded authored target".to_string())?;
    local_metrics.max_sendinput_pre_call_lateness_ticks = local_metrics
        .max_sendinput_pre_call_lateness_ticks
        .max(lateness_ticks.as_u64());
    if lateness_ticks >= timing.pre_call_10ms_ticks {
        local_metrics.pre_call_late_10ms = local_metrics.pre_call_late_10ms.saturating_add(1);
    }
    if lateness_ticks >= timing.pre_call_5ms_ticks {
        local_metrics.pre_call_late_5ms = local_metrics.pre_call_late_5ms.saturating_add(1);
    }
    if lateness_ticks >= timing.pre_call_2ms_ticks {
        local_metrics.pre_call_late_2ms = local_metrics.pre_call_late_2ms.saturating_add(1);
    }
    Ok(())
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

pub(crate) fn observe_wait_health(
    wake_error_us: u64,
    wait_warn_us: u64,
    elapsed_us: u64,
    policy: HealthWindowPolicy,
    window: &mut HealthWindow<HEALTH_WINDOW_CAPACITY>,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if wait_warn_us == 0 {
        window.reset();
    } else {
        let over_budget = wake_error_us > wait_warn_us;
        let _ = window.observe(over_budget, elapsed_us, policy);
        if over_budget {
            local_metrics.wait_degraded_samples =
                local_metrics.wait_degraded_samples.saturating_add(1);
        }
    }
    local_metrics.wait_path_degraded = window.is_degraded();
    local_metrics.input_path_degraded = local_metrics.sendinput_path_degraded
        || local_metrics.core_post_send_degraded
        || local_metrics.wait_path_degraded;
    local_metrics.wait_window_bad_count = window.bad_count() as u64;
    local_metrics.wait_window_sample_count = window.sample_count() as u64;
}

pub(crate) fn observe_dispatch_health(
    observation: DispatchHealthObservation,
    policy: HealthWindowPolicy,
    sendinput_window: &mut HealthWindow<HEALTH_WINDOW_CAPACITY>,
    core_post_send_window: &mut HealthWindow<HEALTH_WINDOW_CAPACITY>,
    local_metrics: &mut WorkerMetricsLocal,
) {
    let DispatchHealthObservation {
        send_duration_us,
        post_send_duration_us,
        post_send_metrics_available,
        path,
        send_warn_us,
        core_post_send_warn_us,
        elapsed_us,
    } = observation;
    let _ = record_input_path_health(
        send_duration_us,
        send_warn_us,
        elapsed_us,
        policy,
        sendinput_window,
    );
    if post_send_metrics_available {
        let _ = record_input_path_health(
            post_send_duration_us,
            core_post_send_warn_us,
            elapsed_us,
            policy,
            core_post_send_window,
        );
    }
    local_metrics.sendinput_path_degraded = sendinput_window.is_degraded();
    local_metrics.core_post_send_degraded = core_post_send_window.is_degraded();
    local_metrics.sendinput_window_bad_count = sendinput_window.bad_count() as u64;
    local_metrics.core_post_send_window_bad_count = core_post_send_window.bad_count() as u64;
    local_metrics.sendinput_window_sample_count = sendinput_window.sample_count() as u64;
    local_metrics.core_post_send_window_sample_count = core_post_send_window.sample_count() as u64;
    local_metrics.input_path_degraded = local_metrics.sendinput_path_degraded
        || local_metrics.core_post_send_degraded
        || local_metrics.wait_path_degraded;
    if post_send_metrics_available {
        local_metrics.core_post_send_max_us = local_metrics
            .core_post_send_max_us
            .max(post_send_duration_us);
    }
    local_metrics.dispatch_occupancy_max_us =
        local_metrics
            .dispatch_occupancy_max_us
            .max(if post_send_metrics_available {
                send_duration_us.saturating_add(post_send_duration_us)
            } else {
                send_duration_us
            });
    record_degraded_sample(
        send_duration_us,
        send_warn_us,
        &mut local_metrics.sendinput_degraded_samples,
    );
    if post_send_metrics_available {
        record_degraded_sample(
            post_send_duration_us,
            core_post_send_warn_us,
            &mut local_metrics.core_post_send_degraded_samples,
        );
    }
    if send_duration_us > send_warn_us {
        match path {
            DispatchPath::DownOnly { .. } => {
                local_metrics.send_down_degraded_samples =
                    local_metrics.send_down_degraded_samples.saturating_add(1);
            }
            DispatchPath::UpOnly { .. } => {
                local_metrics.send_up_degraded_samples =
                    local_metrics.send_up_degraded_samples.saturating_add(1);
            }
            DispatchPath::Mixed { .. } => {
                local_metrics.send_mixed_degraded_samples =
                    local_metrics.send_mixed_degraded_samples.saturating_add(1);
            }
        }
    }
}

pub(crate) fn observe_observer_health(
    observer_duration_us: u64,
    observer_warn_us: u64,
    elapsed_us: u64,
    policy: HealthWindowPolicy,
    observer_window: &mut HealthWindow<HEALTH_WINDOW_CAPACITY>,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if observer_warn_us == 0 {
        observer_window.reset();
    } else {
        let over_budget = observer_duration_us > observer_warn_us;
        let _ = observer_window.observe(over_budget, elapsed_us, policy);
        if over_budget {
            local_metrics.observer_degraded_samples =
                local_metrics.observer_degraded_samples.saturating_add(1);
        }
    }
    local_metrics.observer_degraded = observer_window.is_degraded();
    local_metrics.observer_window_bad_count = observer_window.bad_count() as u64;
    local_metrics.observer_window_sample_count = observer_window.sample_count() as u64;
    local_metrics.observer_duration_max_us = local_metrics
        .observer_duration_max_us
        .max(observer_duration_us);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DispatchHealthObservation {
    pub(crate) send_duration_us: u64,
    pub(crate) post_send_duration_us: u64,
    pub(crate) post_send_metrics_available: bool,
    pub(crate) path: DispatchPath,
    pub(crate) send_warn_us: u64,
    pub(crate) core_post_send_warn_us: u64,
    pub(crate) elapsed_us: u64,
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
        DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
        HealthState, HealthTransition, HealthWindow, HealthWindowPolicy, build_dispatch_budget,
        observe_dispatch_health, observe_observer_health, observe_wait_health,
        record_degraded_sample, record_input_path_health, record_sendinput_pre_call_lateness,
    };
    use crate::engine::telemetry::metrics::WorkerMetricsLocal;
    use crate::engine::worker::WorkerTimingState;
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    #[test]
    fn send_warning_budget_uses_fixed_floor() {
        let options = DispatchHealthOptions::default();
        assert_eq!(options.sendinput_warn_floor_us, 300);
    }

    #[test]
    fn sendinput_pre_call_buckets_compare_ticks_and_keep_public_max_lazy() {
        let mut timing = WorkerTimingState::create_test_timing();
        timing.pre_call_2ms_ticks = sky_dispatch_core::time::DurationTicks::from_raw(3);
        timing.pre_call_5ms_ticks = sky_dispatch_core::time::DurationTicks::from_raw(5);
        timing.pre_call_10ms_ticks = sky_dispatch_core::time::DurationTicks::from_raw(7);
        let mut metrics = WorkerMetricsLocal::default();

        record_sendinput_pre_call_lateness(
            QpcTicks::from_raw(10),
            QpcTicks::from_raw(13),
            &timing,
            &mut metrics,
        )
        .expect("valid lateness");

        assert_eq!(metrics.max_sendinput_pre_call_lateness_ticks, 3);
        assert_eq!(metrics.max_sendinput_pre_call_lateness_us, 0);
        assert_eq!(metrics.pre_call_late_2ms, 1);
        assert_eq!(metrics.pre_call_late_5ms, 0);
        assert_eq!(metrics.pre_call_late_10ms, 0);

        let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(1_000_000).unwrap());
        assert_eq!(
            clock.duration_to_us(sky_dispatch_core::time::DurationTicks::from_raw(
                metrics.max_sendinput_pre_call_lateness_ticks,
            )),
            Ok(3)
        );
    }

    #[test]
    fn health_paths_have_independent_default_thresholds() {
        let options = DispatchHealthOptions::default();
        assert_eq!(options.core_post_send_warn_us, 5_000);
        assert_eq!(options.observer_warn_us, 300);
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
        let options = DispatchHealthOptions::default();
        let mixed = build_dispatch_budget(
            DispatchPath::Mixed {
                up_count: 2,
                down_count: 2,
            },
            options,
        );
        let down = build_dispatch_budget(DispatchPath::DownOnly { down_count: 2 }, options);
        let up = build_dispatch_budget(DispatchPath::UpOnly { up_count: 2 }, options);
        assert_eq!(mixed.event_count, 4);
        assert_eq!(mixed.send_warn_us, down.send_warn_us);
        assert_eq!(mixed.send_warn_us, up.send_warn_us);
    }

    #[test]
    fn wait_health_observation_only_records_explicit_deadline_samples() {
        let mut window = HealthWindow::<{ HEALTH_WINDOW_CAPACITY }>::default();
        let mut metrics = WorkerMetricsLocal::default();
        let policy = HealthWindowPolicy {
            minimum_samples: 1,
            bad_sample_count: 1,
            degrade_hold_us: 0,
            recovery_hold_us: 0,
        };
        observe_wait_health(301, 300, 0, policy, &mut window, &mut metrics);
        assert_eq!(window.sample_count(), 1);
        assert_eq!(window.bad_count(), 1);
        assert_eq!(metrics.wait_degraded_samples, 1);

        // Interrupted and backend-failure paths return before this helper;
        // they therefore cannot manufacture a latency sample.
        let before = (
            window.sample_count(),
            window.bad_count(),
            window.is_degraded(),
        );
        assert_eq!(before, (1, 1, true));
    }

    #[test]
    fn health_domain_isolation_only_degrades_target_domain() {
        let policy = HealthWindowPolicy {
            minimum_samples: 1,
            bad_sample_count: 1,
            degrade_hold_us: 0,
            recovery_hold_us: 0,
        };

        // 1. Slow SendInput degrades sendinput_window only
        {
            let mut send_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut core_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut metrics = WorkerMetricsLocal::default();
            let obs = DispatchHealthObservation {
                send_duration_us: 1000,
                post_send_duration_us: 10,
                post_send_metrics_available: true,
                path: DispatchPath::DownOnly { down_count: 1 },
                send_warn_us: 300,
                core_post_send_warn_us: 300,
                elapsed_us: 10_000,
            };
            observe_dispatch_health(obs, policy, &mut send_win, &mut core_win, &mut metrics);
            assert!(send_win.is_degraded());
            assert!(!core_win.is_degraded());
            assert!(metrics.sendinput_path_degraded);
            assert!(!metrics.core_post_send_degraded);
            assert!(metrics.input_path_degraded);
        }

        // 2. Slow core commit degrades core_post_send_window only
        {
            let mut send_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut core_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut metrics = WorkerMetricsLocal::default();
            let obs = DispatchHealthObservation {
                send_duration_us: 100,
                post_send_duration_us: 1000,
                post_send_metrics_available: true,
                path: DispatchPath::DownOnly { down_count: 1 },
                send_warn_us: 300,
                core_post_send_warn_us: 300,
                elapsed_us: 10_000,
            };
            observe_dispatch_health(obs, policy, &mut send_win, &mut core_win, &mut metrics);
            assert!(!send_win.is_degraded());
            assert!(core_win.is_degraded());
            assert!(!metrics.sendinput_path_degraded);
            assert!(metrics.core_post_send_degraded);
            assert!(metrics.input_path_degraded);
        }

        // 3. Slow observer degrades observer_window only and DOES NOT set input_path_degraded
        {
            let mut obs_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut metrics = WorkerMetricsLocal::default();
            observe_observer_health(10_000, 5_000, 10_000, policy, &mut obs_win, &mut metrics);
            assert!(obs_win.is_degraded());
            assert!(metrics.observer_degraded);
            assert!(!metrics.sendinput_path_degraded);
            assert!(!metrics.core_post_send_degraded);
            assert!(!metrics.input_path_degraded);
        }

        // 4. Slow wake degrades wait_window only
        {
            let mut wait_win = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
            let mut metrics = WorkerMetricsLocal::default();
            observe_wait_health(1000, 300, 10_000, policy, &mut wait_win, &mut metrics);
            assert!(wait_win.is_degraded());
            assert!(metrics.wait_path_degraded);
            assert!(metrics.input_path_degraded);
            assert!(!metrics.sendinput_path_degraded);
            assert!(!metrics.core_post_send_degraded);
            assert!(!metrics.observer_degraded);
        }

        let mut send_window = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
        let mut core_window = HealthWindow::<HEALTH_WINDOW_CAPACITY>::default();
        let mut metrics = WorkerMetricsLocal::default();
        observe_dispatch_health(
            DispatchHealthObservation {
                send_duration_us: 100,
                post_send_duration_us: 10_000,
                post_send_metrics_available: false,
                path: DispatchPath::DownOnly { down_count: 1 },
                send_warn_us: 300,
                core_post_send_warn_us: 300,
                elapsed_us: 10_000,
            },
            policy,
            &mut send_window,
            &mut core_window,
            &mut metrics,
        );
        assert_eq!(core_window.sample_count(), 0);
        assert_eq!(metrics.core_post_send_degraded_samples, 0);
        assert_eq!(metrics.core_post_send_max_us, 0);
        assert_eq!(metrics.dispatch_occupancy_max_us, 100);
    }
}

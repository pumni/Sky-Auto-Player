use super::{
    DispatchProfile, NativeSessionOptions, TelemetryMode, TelemetryOptions, WaitOptions,
    WorkerConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EffectiveRuntimePolicy {
    pub(crate) strict_failure_policy: bool,
    pub(crate) observer_enabled: bool,
    pub(crate) production_wait: bool,
    pub(crate) wait_guard_mode: sky_dispatch_win32::wait::WaitGuardMode,
    pub(crate) telemetry_mode: TelemetryMode,
    pub(crate) telemetry_capacity: usize,
}

impl EffectiveRuntimePolicy {
    fn resolve(
        profile: DispatchProfile,
        strict_timing: bool,
        qualification_observer: bool,
        backend_is_production: bool,
        wait: &WaitOptions,
        telemetry: &TelemetryOptions,
    ) -> Self {
        let observer_enabled = observer_enabled_from_inputs(profile, qualification_observer);
        let (telemetry_mode, telemetry_capacity) = if observer_enabled {
            (telemetry.mode, telemetry.capacity)
        } else {
            (TelemetryMode::Off, 0)
        };

        Self {
            strict_failure_policy: strict_timing,
            observer_enabled,
            production_wait: wait.production_wait_policy(backend_is_production),
            wait_guard_mode: wait.guard_mode,
            telemetry_mode,
            telemetry_capacity,
        }
    }

    pub(crate) fn for_native(options: &NativeSessionOptions) -> Self {
        let qualification_observer = {
            #[cfg(any(test, feature = "test-support"))]
            {
                options.timing_observer_for_qualification
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
                false
            }
        };
        Self::resolve(
            options.profile,
            options.timing.strict_timing,
            qualification_observer,
            matches!(&options.backend, super::BackendConfig::Production),
            &options.wait,
            &options.telemetry,
        )
    }

    pub(crate) fn for_worker(config: &WorkerConfig) -> Self {
        let qualification_observer = {
            #[cfg(any(test, feature = "test-support"))]
            {
                config.timing_observer_for_qualification
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
                false
            }
        };
        Self::resolve(
            config.profile,
            config.timing.strict_timing,
            qualification_observer,
            matches!(&config.backend, super::BackendConfig::Production),
            &config.wait,
            &config.telemetry,
        )
    }
}

pub(super) fn observer_enabled_from_inputs(
    profile: DispatchProfile,
    qualification_observer: bool,
) -> bool {
    profile.observer_enabled() || qualification_observer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        BackendConfig, FaultInjectionScript, FocusOptions, PriorityOptions,
        StartupCalibrationPlacement, TestWaitPolicy, TimingOptions,
    };
    use sky_dispatch_win32::mmcss::PriorityMode;

    const REQUESTED_CAPACITY: usize = 64;

    fn wait_options(test_wait_policy: TestWaitPolicy) -> WaitOptions {
        WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: 0,
            guard_mode: sky_dispatch_win32::wait::WaitGuardMode::Frozen,
            test_spin_threshold_us: None,
            test_wait_policy,
        }
    }

    fn worker_config(
        profile: DispatchProfile,
        strict_timing: bool,
        qualification_observer: bool,
        backend_is_production: bool,
        test_wait_policy: TestWaitPolicy,
        telemetry_mode: TelemetryMode,
    ) -> WorkerConfig {
        WorkerConfig {
            backend: if backend_is_production {
                BackendConfig::Production
            } else {
                BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script: FaultInjectionScript::none(),
                }
            },
            profile,
            timing: TimingOptions {
                strict_timing,
                ..WorkerConfig::default().timing
            },
            wait: wait_options(test_wait_policy),
            telemetry: TelemetryOptions {
                mode: telemetry_mode,
                capacity: REQUESTED_CAPACITY,
            },
            timing_observer_for_qualification: qualification_observer,
            ..WorkerConfig::default()
        }
    }

    fn native_options(
        profile: DispatchProfile,
        strict_timing: bool,
        qualification_observer: bool,
        backend_is_production: bool,
        test_wait_policy: TestWaitPolicy,
        telemetry_mode: TelemetryMode,
    ) -> NativeSessionOptions {
        NativeSessionOptions {
            schedule: sky_dispatch_core::compile::compile_runtime_intents(&[], &[0x15])
                .expect("empty schedule with one allowed key"),
            backend: if backend_is_production {
                BackendConfig::Production
            } else {
                BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script: FaultInjectionScript::none(),
                }
            },
            profile,
            timing: TimingOptions {
                strict_timing,
                ..WorkerConfig::default().timing
            },
            focus: FocusOptions {
                require_focus: false,
                focus_restore_grace_us: 100_000,
            },
            wait: wait_options(test_wait_policy),
            telemetry: TelemetryOptions {
                mode: telemetry_mode,
                capacity: REQUESTED_CAPACITY,
            },
            priority: PriorityOptions {
                mode: PriorityMode::Off,
            },
            instrument_key_profile: None,
            startup_ordering_hook: None,
            startup_calibration_placement: StartupCalibrationPlacement::BeforeSchedulingGuards,
            timing_observer_for_qualification: qualification_observer,
            restore_race_hook: None,
            focus_pause_hook: None,
            timer_lifecycle_context: None,
            prepared_packet_ambiguity_mask: None,
            preflight_user_held_mask: None,
            modifier_key_state_query_for_test: None,
        }
    }

    #[test]
    fn six_profile_strict_pairs_keep_their_existing_projection() {
        let rows = [
            (DispatchProfile::Production, false, false),
            (DispatchProfile::Production, true, false),
            (DispatchProfile::StrictTimingDiagnostic, false, true),
            (DispatchProfile::StrictTimingDiagnostic, true, true),
            (DispatchProfile::MockTest, false, true),
            (DispatchProfile::MockTest, true, true),
        ];

        for (profile, strict_timing, observer_enabled) in rows {
            let config = worker_config(
                profile,
                strict_timing,
                false,
                true,
                TestWaitPolicy::LegacyTestWideSpin,
                TelemetryMode::Ring,
            );
            let policy = EffectiveRuntimePolicy::for_worker(&config);

            assert_eq!(policy.strict_failure_policy, strict_timing);
            assert_eq!(policy.observer_enabled, observer_enabled);
            assert!(policy.production_wait);
            if observer_enabled {
                assert_eq!(policy.telemetry_mode, TelemetryMode::Ring);
                assert_eq!(policy.telemetry_capacity, REQUESTED_CAPACITY);
            } else {
                assert_eq!(policy.telemetry_mode, TelemetryMode::Off);
                assert_eq!(policy.telemetry_capacity, 0);
            }
        }
    }

    #[test]
    fn qualification_observer_and_ring_off_modes_remain_independent() {
        for (qualification_observer, observer_enabled) in [(false, false), (true, true)] {
            for telemetry_mode in [TelemetryMode::Ring, TelemetryMode::Off] {
                let config = worker_config(
                    DispatchProfile::Production,
                    false,
                    qualification_observer,
                    true,
                    TestWaitPolicy::LegacyTestWideSpin,
                    telemetry_mode,
                );
                let policy = EffectiveRuntimePolicy::for_worker(&config);

                assert!(!policy.strict_failure_policy);
                assert_eq!(policy.observer_enabled, observer_enabled);
                assert!(policy.production_wait);
                assert_eq!(
                    policy.telemetry_mode,
                    if observer_enabled {
                        telemetry_mode
                    } else {
                        TelemetryMode::Off
                    }
                );
                assert_eq!(
                    policy.telemetry_capacity,
                    if observer_enabled {
                        REQUESTED_CAPACITY
                    } else {
                        0
                    }
                );
            }
        }
    }

    #[test]
    fn mock_backend_wait_policy_is_independent_of_profile() {
        let cases = [
            (true, TestWaitPolicy::LegacyTestWideSpin, true),
            (false, TestWaitPolicy::LegacyTestWideSpin, false),
            (false, TestWaitPolicy::ProductionCalibrated, true),
        ];

        for (backend_is_production, wait_policy, production_wait) in cases {
            let config = worker_config(
                DispatchProfile::Production,
                false,
                false,
                backend_is_production,
                wait_policy,
                TelemetryMode::Ring,
            );
            let policy = EffectiveRuntimePolicy::for_worker(&config);

            assert_eq!(policy.production_wait, production_wait);
            assert!(!policy.strict_failure_policy);
            assert!(!policy.observer_enabled);
            assert_eq!(policy.telemetry_mode, TelemetryMode::Off);
            assert_eq!(policy.telemetry_capacity, 0);
        }
    }

    #[test]
    fn native_and_worker_adapters_resolve_the_same_inputs() {
        let rows = [
            (DispatchProfile::Production, false),
            (DispatchProfile::Production, true),
            (DispatchProfile::StrictTimingDiagnostic, false),
            (DispatchProfile::StrictTimingDiagnostic, true),
            (DispatchProfile::MockTest, false),
            (DispatchProfile::MockTest, true),
        ];

        for (profile, strict_timing) in rows {
            for (backend_is_production, wait_policy) in [
                (true, TestWaitPolicy::LegacyTestWideSpin),
                (false, TestWaitPolicy::ProductionCalibrated),
            ] {
                let native = native_options(
                    profile,
                    strict_timing,
                    false,
                    backend_is_production,
                    wait_policy,
                    TelemetryMode::Ring,
                );
                let worker = worker_config(
                    profile,
                    strict_timing,
                    false,
                    backend_is_production,
                    wait_policy,
                    TelemetryMode::Ring,
                );

                assert_eq!(
                    EffectiveRuntimePolicy::for_native(&native),
                    EffectiveRuntimePolicy::for_worker(&worker)
                );
            }
        }

        let native = native_options(
            DispatchProfile::Production,
            false,
            true,
            true,
            TestWaitPolicy::LegacyTestWideSpin,
            TelemetryMode::Off,
        );
        let worker = worker_config(
            DispatchProfile::Production,
            false,
            true,
            true,
            TestWaitPolicy::LegacyTestWideSpin,
            TelemetryMode::Off,
        );
        assert_eq!(
            EffectiveRuntimePolicy::for_native(&native),
            EffectiveRuntimePolicy::for_worker(&worker)
        );
    }
}

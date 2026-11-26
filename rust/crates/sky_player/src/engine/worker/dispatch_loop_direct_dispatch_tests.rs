use super::super::{PreWaitAdmission, pre_wait_admission};
use super::support::ProductionDispatchTestHarness;
use crate::engine::EngineSnapshot;
use crate::engine::config::{BackendConfig, DispatchProfile};
use crate::engine::session::NativeDispatchSession;
use crate::engine::test_support::{FaultInjectionScript, MockSendInputCapture};
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::plan::build_onset_plan;
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalPacket};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn run_mock_worker_packets(
    chords: &[OnsetChordInput],
    scan_codes: &[u16],
    hold_us: u64,
    latency_base_us: u64,
    with_observer: bool,
) -> (Vec<PhysicalPacket>, EngineSnapshot) {
    let plan = build_onset_plan(chords, scan_codes, hold_us).expect("valid onset plan");
    let captures = Arc::new(Mutex::new(Vec::new()));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_input_capture = Some(Arc::clone(&captures));
    let mut options = crate::engine::tests::support::test_session_options(
        plan,
        1,
        BackendConfig::Mock {
            latency_base_us,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.timing_observer_for_qualification = with_observer;

    let session = NativeDispatchSession::new(options).expect("mock session admission");
    session.arm(0).expect("arm worker");
    assert!(
        session.join(Duration::from_secs(5)).expect("worker join"),
        "worker must complete within timeout"
    );
    let snapshot = session.snapshot();
    let packets = captures
        .lock()
        .expect("capture lock")
        .iter()
        .filter_map(|entry| match entry {
            MockSendInputCapture::Packet { packet, .. } => Some(*packet),
            MockSendInputCapture::SingleEmitter { .. } => None,
        })
        .collect();
    (packets, snapshot)
}

#[test]
fn direct_dispatch_worker_1_5_15_width_and_dense_same_key_repeat_i_eq_h() {
    let hold_us = 17_167;
    for width in [1_usize, 5, 15] {
        let keys = &PHYSICAL_INSTRUMENT_SCAN_CODES[..width];
        let full_mask = (1_u16 << width) - 1;
        // Same-key repeat at exact I = H (hold-only, no G).
        let chords = [
            OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: keys.iter().copied().collect(),
            },
            OnsetChordInput {
                source_action_index: 1,
                scheduled_us: hold_us,
                scan_codes: keys.iter().copied().collect(),
            },
        ];
        for with_observer in [false, true] {
            let (packets, snapshot) =
                run_mock_worker_packets(&chords, keys, hold_us, 0, with_observer);
            assert_eq!(
                snapshot.status, "finished",
                "width={width} obs={with_observer}"
            );
            assert_eq!(snapshot.final_release_obligation_mask, 0);
            assert_eq!(snapshot.generation_accounting.activated, (width * 2) as u64);
            assert_eq!(snapshot.generation_accounting.released, (width * 2) as u64);
            assert_eq!(snapshot.generation_accounting.cancelled, 0);
            assert_eq!(
                packets,
                vec![
                    PhysicalPacket::new(0, full_mask),
                    PhysicalPacket::new(full_mask, 0),
                    PhysicalPacket::new(0, full_mask),
                    PhysicalPacket::new(full_mask, 0),
                ],
                "exact I=H repeat must commit Up before next Down with no overlap (width={width}, obs={with_observer})"
            );
            for packet in &packets {
                assert_eq!(
                    packet.up_mask & packet.down_mask,
                    0,
                    "no overlapping up/down masks"
                );
            }
        }
    }
}

#[test]
fn direct_dispatch_worker_blocked_overlap_commits_up_before_repeat_down_on_late_wake() {
    // 25ms mock transport latency pushes completion + H well past Chord 1's authored onset,
    // exercising late wake bursts on an overlapping repeat and a disjoint chord.
    let hold_us = 17_167;
    let chords = [
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: hold_us,
            scan_codes: smallvec::smallvec![0x15],
        },
        OnsetChordInput {
            source_action_index: 2,
            scheduled_us: hold_us + 1_000,
            scan_codes: smallvec::smallvec![0x16],
        },
    ];
    let (packets, snapshot) =
        run_mock_worker_packets(&chords, &[0x15, 0x16], hold_us, 25_000, true);
    assert_eq!(snapshot.status, "finished");
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.generation_accounting.activated, 3);
    assert_eq!(snapshot.generation_accounting.released, 3);
    // Packet 0: Down 0x15 (bit 1).
    // Packet 1: Up 0x15 (bit 1) — repeat Down 0x15 is blocked while bit 1 is active!
    // Packet 2: Down 0x15 (bit 1) — unblocked immediately after Up commit.
    // Packet 3: Down 0x16 (bit 2) — disjoint from active bit 1, already overdue!
    // Remaining Ups release both bit 1 and bit 2 cleanly.
    assert_eq!(packets[0], PhysicalPacket::new(0, 0b01));
    assert_eq!(packets[1], PhysicalPacket::new(0b01, 0));
    assert_eq!(packets[2], PhysicalPacket::new(0, 0b01));
    assert_eq!(packets[3].down_mask, 0b10);
    for packet in &packets {
        assert_eq!(packet.up_mask & packet.down_mask, 0);
    }
}

#[test]
fn direct_dispatch_worker_paused_idle_performs_zero_periodic_wakes() {
    let chords = [
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: 500_000,
            scan_codes: smallvec::smallvec![0x16],
        },
    ];
    let plan = build_onset_plan(&chords, &[0x15, 0x16], 17_167).expect("valid pause plan");
    let mut options = crate::engine::tests::support::test_session_options(
        plan,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.timing_observer_for_qualification = true;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.arm(0).expect("arm worker");
    session.pause().expect("pause worker");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !session.snapshot_lite().is_paused && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(session.snapshot_lite().is_paused, "worker entered pause");

    let before = session.snapshot();
    std::thread::sleep(Duration::from_millis(80));
    let after = session.snapshot();

    assert_eq!(
        after.spin_observation_expected_samples, before.spin_observation_expected_samples,
        "clean paused worker must perform zero periodic timer wakes"
    );
    assert_eq!(
        after.wait_interrupted_count, before.wait_interrupted_count,
        "clean paused worker must not spuriously interrupt"
    );

    session.quit().expect("quit paused session");
    assert!(session.join(Duration::from_secs(2)).expect("join worker"));
    assert_eq!(session.snapshot().final_release_obligation_mask, 0);
}

#[test]
fn direct_dispatch_pre_wait_admission_rejects_controls_focus_and_invalid_target_without_staging() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();

    // Healthy -> Proceed
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        PreWaitAdmission::Proceed
    );

    // Focus loss -> Replan
    harness.config.focus.require_focus = true;
    harness.focus_active.store(false, Ordering::Release);
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        PreWaitAdmission::Replan
    );
    harness.focus_active.store(true, Ordering::Release);
    harness.config.focus.require_focus = false;

    // Manual pause -> Replan
    harness.desired_pause.store(true, Ordering::Release);
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        PreWaitAdmission::Replan
    );
    harness.desired_pause.store(false, Ordering::Release);

    // Quit, Skip, Panic -> Exit
    for control in ["quit", "skip", "panic"] {
        match control {
            "quit" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            _ => harness.panic_requested.store(true, Ordering::Release),
        }
        assert_eq!(
            harness.pre_wait_admission_for_test(),
            PreWaitAdmission::Exit,
            "{control} must exit pre-wait admission"
        );
        match control {
            "quit" => harness.quit_requested.store(false, Ordering::Release),
            "skip" => harness.skip_requested.store(false, Ordering::Release),
            _ => harness.panic_requested.store(false, Ordering::Release),
        }
    }

    // System suspend -> Exit with SystemSuspend trigger
    harness.system_power.notify(true, &harness.interrupt);
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        PreWaitAdmission::Exit
    );
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(super::super::super::control::TerminalTrigger::SystemSuspend)
    );

    // Invalid target -> Exit with terminal_error
    let mut inv = ProductionDispatchTestHarness::new_down_only();
    inv.target.invalidate_target();
    let result = pre_wait_admission(
        &inv.target,
        inv.config.focus.require_focus,
        &inv.focus_active,
        super::super::FinalControlSignals {
            quit_requested: &inv.quit_requested,
            skip_requested: &inv.skip_requested,
            panic_requested: &inv.panic_requested,
            desired_pause: &inv.desired_pause,
            supervisor_expired: &inv.supervisor_expired,
            system_power: Some(&inv.system_power),
        },
        &mut inv.runtime,
    );
    assert_eq!(result, PreWaitAdmission::Exit);
    assert!(inv.runtime.terminal_error.is_some());
}

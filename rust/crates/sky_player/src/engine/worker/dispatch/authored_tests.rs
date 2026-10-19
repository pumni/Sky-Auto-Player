use super::super::{PhysicalCommit, RecoveryDescriptor};
use super::*;
use sky_dispatch_core::coordinator::{PreparedAuthoredCommit, PreparedBatch};
use sky_dispatch_core::model::PhysicalPacketKind;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::input::{PhysicalPacket, PreparedPhysicalPacket};
use std::num::NonZeroU64;

#[test]
fn healthy_down_terminal_path_does_not_convert_ticks_to_microseconds() {
    let view = AuthoredBatchView {
        prepared_batch: PreparedBatch {
            index: 0,
            effective_scheduled_ticks: TimelineTicks::ZERO,
            packet_kind: PhysicalPacketKind::DownOnly,
            packet_batch_count: 1,
            packet_index: 0,
        },
        batch_source_action_index: 0,
        batch_intent_count: 1,
        batch_kind: ActionKind::Down,
        batch_scheduled_ticks: TimelineTicks::ZERO,
        authored_batch_scheduled_ticks: TimelineTicks::ZERO,
        conflict_mask: 0,
        dispatch_path: DispatchPath::DownOnly { down_count: 1 },
        packet_masks: PhysicalPacket::new(0, 0b001),
        prepared_packet: PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).unwrap(),
        recovery: RecoveryDescriptor::None,
        commit: PhysicalCommit::Authored(PreparedAuthoredCommit {
            frame: sky_dispatch_core::coordinator::PreparedAuthoredFrame {
                first_batch_index: 0,
                packet_index: 0,
                packet_batch_count: 1,
                authored_ticks: TimelineTicks::ZERO,
                immediate_up_mask: 0,
                deferred_up_mask: 0,
                down_mask: 0b001,
                stale_up_count: 0,
            },
            up_intents: smallvec::SmallVec::new(),
            down_intents: smallvec::SmallVec::new(),
            down_source_action_index: Some(0),
        }),
    };
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1).unwrap());
    let mut runtime = WorkerRuntime::default();

    let step = resolve_slo_terminal_step(
        false,
        false,
        false,
        qpc_clock,
        i64::MIN,
        &view,
        &mut runtime,
    );

    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn final_gate_precedes_the_authoritative_pre_call_boundary() {
    assert_final_gate_source_order(include_str!("authored.rs"));
}

#[test]
fn final_gate_source_order_accepts_lf_and_crlf_fixtures() {
    let lf_source = include_str!("authored.rs").replace("\r\n", "\n");
    let crlf_source = lf_source.replace('\n', "\r\n");
    let old_line_ending_sensitive_match = "lease_timeout_ticks,\n            qpc_clock";

    assert!(lf_source.contains(old_line_ending_sensitive_match));
    assert!(!crlf_source.contains(old_line_ending_sensitive_match));
    assert_final_gate_source_order(&lf_source);
    assert_final_gate_source_order(&crlf_source);
}

fn assert_final_gate_source_order(source: &str) {
    let normalized_source = source.replace("\r\n", "\n");
    let source = normalized_source.as_str();
    let finalizer = source
        .split("fn finalize_authored_down_admission")
        .nth(1)
        .expect("authored finalizer");
    let crossing = finalizer
        .find("let target_crossing_qpc")
        .expect("target crossing handoff");
    assert!(!finalizer.contains("wait_to_precision_boundary"));
    let control = finalizer
        .find("final_down_control_verdict")
        .expect("final control gate");
    let target = finalizer
        .find("final_down_target_admission")
        .expect("final target/focus gate");
    let modifier_guard = finalizer
        .find("final_down_modifier_verdict")
        .expect("fixed modifier guard");
    let post_modifier = finalizer
        .find("final_down_post_modifier_verdict")
        .expect("shared post-modifier authority policy");
    let pre_call = finalizer
        .find("let final_policy_qpc")
        .expect("final policy timestamp");
    assert!(crossing < control);
    assert!(control < target);
    assert!(target < modifier_guard);
    assert!(modifier_guard < post_modifier);
    assert!(post_modifier < pre_call);
    assert!(target < pre_call);

    let finalizer_body = finalizer
        .split("\nfn final_atomic_revalidation")
        .next()
        .expect("finalizer body");
    assert!(!finalizer_body.contains("foreground_window_matches"));
    assert!(!finalizer_body.contains("focus_matches_hwnd"));
    assert!(!finalizer_body.contains("prove_down_age"));
    assert!(finalizer_body.contains("lease_timeout_ticks"));
    let compact_finalizer_body: String = finalizer_body
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact_finalizer_body.contains("lease_timeout_ticks,qpc_clock"));
    assert!(!finalizer_body.contains("supervisor_heartbeat_ticks"));

    let admission = include_str!("../admission.rs");
    let post_modifier_policy = admission
        .split("pub(crate) fn final_down_post_modifier_verdict")
        .nth(1)
        .expect("shared post-modifier policy")
        .split("pub(crate) fn final_down_modifier_verdict")
        .next()
        .expect("shared post-modifier policy body");
    let post_modifier_authority = post_modifier_policy
        .find("final_down_atomic_revalidation")
        .expect("post-modifier target/owner revalidation");
    let lease_proof = post_modifier_policy
        .find("final_down_lease_age_proof")
        .expect("bounded final lease-age proof");
    let final_control = post_modifier_policy
        .find("final_down_control_verdict")
        .expect("post-modifier control revalidation");
    assert!(post_modifier_authority < lease_proof);
    assert!(lease_proof < final_control);

    let target_admission = admission
        .split("pub(crate) fn final_down_target_admission")
        .nth(1)
        .expect("final target admission");
    let target_admission_body = target_admission
        .split("/// Recheck only bounded atomic target")
        .next()
        .expect("target admission body");
    let atomic_focus = target_admission_body
        .find("focus_matches(target.require_focus")
        .expect("published focus hint check");
    let identity = target_admission_body
        .find("owner_identity_status(")
        .expect("generation-bound owner proof");
    let foreground = target_admission_body
        .find("foreground_window_owner_matches(")
        .expect("fresh foreground and owner proof");
    let post_focus_hook = target_admission_body
        .find("target.post_focus_race_hook")
        .expect("post-focus race seam");
    let final_atomic_recheck = target_admission_body
        .find("final_down_atomic_revalidation(")
        .expect("post-focus atomic revalidation");
    assert!(atomic_focus < identity);
    assert!(identity < foreground);
    assert!(foreground < post_focus_hook);
    assert!(post_focus_hook < final_atomic_recheck);

    let final_atomic = admission
        .split("pub(crate) fn final_down_atomic_revalidation")
        .nth(1)
        .expect("atomic target and owner revalidation");
    let final_atomic_body = final_atomic
        .split("/// The shared post-modifier authority gate")
        .next()
        .expect("atomic revalidation body");
    let atomic_target = final_atomic_body
        .find("target_stamp_still_current(")
        .expect("final target stamp check");
    let atomic_focus = final_atomic_body
        .find("focus_matches(require_focus")
        .expect("final focus hint check");
    let atomic_owner = final_atomic_body
        .find("owner_identity_status(")
        .expect("final owner status check");
    assert!(atomic_target < atomic_focus);
    assert!(atomic_focus < atomic_owner);

    let sender = source
        .split("fn record_down_send_outcome")
        .nth(1)
        .expect("authored sender handoff");
    assert!(sender.contains("send_prepared_physical_packet_at_final_boundary"));
    assert!(!sender.contains("latest_start"));
}

#[test]
fn final_gate_rejection_counters_are_worker_local_and_reason_specific() {
    let mut metrics = WorkerMetricsLocal::default();
    for reason in [
        crate::engine::worker::FinalGateRejection::Control,
        crate::engine::worker::FinalGateRejection::Target,
        crate::engine::worker::FinalGateRejection::Focus,
    ] {
        crate::engine::worker::record_final_gate_rejection(&mut metrics, reason);
    }
    assert_eq!(metrics.final_gate_control_rejections, 1);
    assert_eq!(metrics.final_gate_target_changes, 1);
    assert_eq!(metrics.final_gate_focus_losses, 1);
    assert_eq!(metrics.final_gate_lease_expirations, 0);
    assert_eq!(metrics.final_sender_window_expirations, 0);
}

#[test]
fn anchored_target_math_supports_explicit_offset() {
    let anchor = QpcTicks::from_raw(10_000);
    let lead = DurationTicks::from_raw(500);
    for (scheduled, expected_target) in [
        (0, 9_500),
        (100, 9_600),
        (499, 9_999),
        (500, 10_000),
        (501, 10_001),
    ] {
        let target = super::super::super::anchored_dispatch_target_ticks_typed(
            QpcTicks::from_raw(9_500),
            anchor,
            TimelineTicks::from_raw(scheduled),
            lead,
        )
        .expect("startup target");
        assert_eq!(target, QpcTicks::from_raw(expected_target));
    }
}

#[test]
fn authored_sender_handoff_uses_one_canonical_prepared_sender() {
    let source = include_str!("authored.rs");
    let sender = source
        .split("fn record_down_send_outcome")
        .nth(1)
        .expect("authored sender handoff");
    assert!(sender.contains("send_prepared_physical_packet_at_final_boundary"));
    assert!(!sender.contains("latest_start"));
}

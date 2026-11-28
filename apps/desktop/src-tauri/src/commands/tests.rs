use super::{
    AppearanceMode, DispatchModelScenarioDto, DispatchModelStatus, GenerationTimingAdvisoryDto,
    PlaybackCommandAckDto, PlaybackConfigDto, PlaybackDecision, PlaybackPlanVariantDto,
    SettingsDto, SettingsPatch, TimingAdvisoryStatus,
};
use serde_json::json;

#[test]
fn playback_command_ack_is_typed_and_rejects_unknown_fields() {
    let value = serde_json::from_value::<PlaybackCommandAckDto>(json!({
        "accepted": true,
        "session_id": "a".repeat(32),
        "state": "playing",
        "pending_command": "pause",
        "reason": null,
    }))
    .expect("valid playback acknowledgement");
    assert!(value.accepted);

    let unknown = serde_json::from_value::<PlaybackCommandAckDto>(json!({
        "accepted": true,
        "session_id": "a".repeat(32),
        "state": "playing",
        "pending_command": null,
        "reason": null,
        "extra": true,
    }));
    assert!(unknown.is_err());
}

#[test]
fn settings_patch_accepts_legacy_theme_input_but_serializes_palette() {
    let patch: SettingsPatch =
        serde_json::from_value(json!({"theme": "slate"})).expect("legacy patch input");
    assert_eq!(patch.palette.as_deref(), Some("slate"));
    let serialized = serde_json::to_value(&patch).expect("serialize canonical patch");
    assert_eq!(serialized["palette"], "slate");
    assert!(serialized.get("theme").is_none());
}

#[test]
fn settings_patch_accepts_canonical_palette_input() {
    let patch: SettingsPatch =
        serde_json::from_value(json!({"palette": "classic"})).expect("canonical patch input");
    assert_eq!(patch.palette.as_deref(), Some("classic"));
}

#[test]
fn appearance_mode_uses_only_the_generated_wire_values() {
    for (wire, expected) in [
        ("system", AppearanceMode::System),
        ("light", AppearanceMode::Light),
        ("dark", AppearanceMode::Dark),
    ] {
        let patch: SettingsPatch = serde_json::from_value(json!({"appearanceMode": wire}))
            .expect("valid appearance patch");
        assert_eq!(patch.appearance_mode, Some(expected));
        let encoded = serde_json::to_value(&patch).expect("serialize appearance patch");
        assert_eq!(encoded["appearanceMode"], wire);
    }

    assert!(
        serde_json::from_value::<SettingsPatch>(json!({
            "appearanceMode": "sepia",
            "telemetryEnabled": true
        }))
        .is_err()
    );
}

#[test]
fn settings_patch_rejects_unknown_fields() {
    let unknown = serde_json::from_value::<SettingsPatch>(json!({
        "palette": "aurora",
        "unexpected": true,
    }));
    assert!(unknown.is_err());
}

#[test]
fn settings_patch_rejects_ambiguous_palette_and_theme_input() {
    let duplicate = serde_json::from_value::<SettingsPatch>(json!({
        "palette": "classic",
        "theme": "slate",
    }));
    assert!(duplicate.is_err());
}

#[test]
fn settings_patch_accepts_empty_and_partial_input() {
    let empty: SettingsPatch = serde_json::from_value(json!({})).expect("empty patch input");
    assert_eq!(empty.palette, None);
    assert_eq!(empty.telemetry_enabled, None);

    let partial: SettingsPatch =
        serde_json::from_value(json!({"verboseHud": true})).expect("partial patch input");
    assert_eq!(partial.palette, None);
    assert_eq!(partial.verbose_hud, Some(true));
}

#[test]
fn settings_dto_serializes_canonical_palette_field() {
    let dto = SettingsDto {
        appearance_mode: super::AppearanceMode::Dark,
        palette: "classic".into(),
        ui_background_mode: "transparent".into(),
        playback_defaults: super::PlaybackDefaultsDto {
            hold_frames: 1.0,
            tempo_scale: 1.0,
            fps: 60,
            timing_margin_us: 500,
            dry_run: false,
        },
        auto_play: true,
        telemetry_enabled: false,
        verbose_hud: false,
        update_preferences: super::UpdatePreferencesDto {
            auto_check: true,
            channel: super::UpdateChannel::Stable,
            skip_version: String::new(),
            check_interval_s: 86_400,
            last_check_ts: 0,
            last_error_ts: 0,
        },
    };
    let serialized = serde_json::to_value(dto).expect("serialize settings DTO");
    assert_eq!(serialized["appearance_mode"], "dark");
    assert_eq!(serialized["palette"], "classic");
    assert!(serialized.get("theme").is_none());
}

#[test]
fn playback_variant_advisory_is_optional_and_has_a_stable_compact_wire_shape() {
    let config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: false,
    };
    let absent = PlaybackPlanVariantDto {
        decision: PlaybackDecision::Proceed,
        config: config.clone(),
        plan_fingerprint: "plan-a".into(),
        timing_advisory: None,
    };
    let absent_json = serde_json::to_value(&absent).expect("serialize absent advisory");
    assert!(absent_json.get("timing_advisory").is_none());
    let missing: PlaybackPlanVariantDto = serde_json::from_value(json!({
        "decision": "proceed",
        "config": {
            "hold_frames": 1.0,
            "timing_margin_us": 800,
            "tempo_scale": 1.0,
            "fps": 60,
            "dry_run": false
        },
        "plan_fingerprint": "plan-a"
    }))
    .expect("decode older variant without advisory");
    assert_eq!(missing.timing_advisory, None);
    let null: PlaybackPlanVariantDto = serde_json::from_value(json!({
        "decision": "proceed",
        "config": {
            "hold_frames": 1.0,
            "timing_margin_us": 800,
            "tempo_scale": 1.0,
            "fps": 60,
            "dry_run": false
        },
        "plan_fingerprint": "plan-a",
        "timing_advisory": null
    }))
    .expect("decode explicit null advisory");
    assert_eq!(null.timing_advisory, None);

    let present = PlaybackPlanVariantDto {
            decision: PlaybackDecision::Proceed,
            config,
            plan_fingerprint: "plan-a".into(),
            timing_advisory: Some(GenerationTimingAdvisoryDto {
                status: TimingAdvisoryStatus::Analyzed,
                plan_fingerprint: "plan-a".into(),
                explanation: "1 of 1 same-key repeat(s) have zero nominal scheduling slack (I = H; minimum slack 0 µs).".into(),
                total_authored_timestamp_group_count: Some(3),
                down_generation_count: Some(2),
                unmatched_up_count: Some(0),
                generation_limit: 50_000,
                work_limit_units: 3_000_000,
                working_memory_limit_bytes: 67_108_864,
                min_same_key_slack_us: Some(0),
                same_key_repeat_count: 1,
                zero_slack_repeat_count: 1,
                effective_hold_floor_h_us: Some(17_467),
                dispatch_model_status: DispatchModelStatus::ModeledDisplacement,
                dispatch_model_explanation: "Reference illustrative model: 1 late Down (max 31 µs), 2 H-floor Up slots (max 13 µs).".into(),
                model_cost_disclaimer: "Illustrative modeled cost only — not measured SendInput latency or a predictive gameplay guarantee.".into(),
                scenarios: vec![DispatchModelScenarioDto {
                    scenario_id: "reference".into(),
                    label: "Reference (illustrative modeled cost: 10 µs + 3 µs/event + 5 µs)".into(),
                    estimated_sender_setup_us: 10,
                    estimated_per_input_us: 3,
                    estimated_post_send_us: 5,
                    modeled_send_count: 4,
                    modeled_down_packet_count: 2,
                    modeled_up_packet_count: 2,
                    modeled_late_down_count: 1,
                    max_modeled_down_lateness_us: 31,
                    modeled_key_readiness_displaced_down_count: 1,
                    max_modeled_key_readiness_displacement_us: 31,
                    modeled_cursor_order_displaced_down_count: 0,
                    max_modeled_cursor_order_displacement_us: 0,
                    modeled_worker_occupancy_displaced_down_count: 0,
                    max_modeled_worker_occupancy_residual_us: 0,
                    modeled_up_slot_count: 2,
                    modeled_h_floor_displaced_up_slot_count: 2,
                    max_modeled_h_floor_displacement_us: 13,
                    modeled_late_up_packet_count: 2,
                    max_modeled_up_lateness_us: 44,
                    max_modeled_up_pre_call_above_floor_us: 0,
                }],
                dry_run_is_hypothetical: false,
            }),
        };
    let scenario_json = json!({
        "scenario_id": "reference",
        "label": "Reference (illustrative modeled cost: 10 µs + 3 µs/event + 5 µs)",
        "estimated_sender_setup_us": 10,
        "estimated_per_input_us": 3,
        "estimated_post_send_us": 5,
        "modeled_send_count": 4,
        "modeled_down_packet_count": 2,
        "modeled_up_packet_count": 2,
        "modeled_late_down_count": 1,
        "max_modeled_down_lateness_us": 31,
        "modeled_key_readiness_displaced_down_count": 1,
        "max_modeled_key_readiness_displacement_us": 31,
        "modeled_cursor_order_displaced_down_count": 0,
        "max_modeled_cursor_order_displacement_us": 0,
        "modeled_worker_occupancy_displaced_down_count": 0,
        "max_modeled_worker_occupancy_residual_us": 0,
        "modeled_up_slot_count": 2,
        "modeled_h_floor_displaced_up_slot_count": 2,
        "max_modeled_h_floor_displacement_us": 13,
        "modeled_late_up_packet_count": 2,
        "max_modeled_up_lateness_us": 44,
        "max_modeled_up_pre_call_above_floor_us": 0
    });
    assert_eq!(
        serde_json::to_value(present).expect("serialize modeled advisory"),
        json!({
            "decision": "proceed",
            "config": {
                "hold_frames": 1.0,
                "timing_margin_us": 800,
                "tempo_scale": 1.0,
                "fps": 60,
                "dry_run": false
            },
            "plan_fingerprint": "plan-a",
            "timing_advisory": {
                "status": "analyzed",
                "plan_fingerprint": "plan-a",
                "explanation": "1 of 1 same-key repeat(s) have zero nominal scheduling slack (I = H; minimum slack 0 µs).",
                "total_authored_timestamp_group_count": 3,
                "down_generation_count": 2,
                "unmatched_up_count": 0,
                "generation_limit": 50_000,
                "work_limit_units": 3_000_000,
                "working_memory_limit_bytes": 67_108_864,
                "min_same_key_slack_us": 0,
                "same_key_repeat_count": 1,
                "zero_slack_repeat_count": 1,
                "effective_hold_floor_h_us": 17_467,
                "dispatch_model_status": "modeled_displacement",
                "dispatch_model_explanation": "Reference illustrative model: 1 late Down (max 31 µs), 2 H-floor Up slots (max 13 µs).",
                "model_cost_disclaimer": "Illustrative modeled cost only — not measured SendInput latency or a predictive gameplay guarantee.",
                "scenarios": [scenario_json],
                "dry_run_is_hypothetical": false
            }
        })
    );
    assert_eq!(
        serde_json::to_value(TimingAdvisoryStatus::Analyzed).expect("serialize analyzed status"),
        json!("analyzed")
    );
    assert_eq!(
        serde_json::to_value(DispatchModelStatus::NoModeledDisplacement)
            .expect("serialize execution-model status"),
        json!("no_modeled_displacement")
    );
}

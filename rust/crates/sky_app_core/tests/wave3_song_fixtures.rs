use serde_json::Value;
use sky_app_core::song::{
    ActionKind, analyze_schedule_with_context, build_schedule_with_policy, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/wave3/song_planning.json"
    ))
    .expect("valid committed Wave 3 song fixture")
}

#[test]
fn parser_cases_are_generated_by_the_current_python_oracle() {
    let raw = fixture();
    let cases = raw["parser_cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for case in cases {
        let bytes = serde_json::to_vec(&case["raw"]).unwrap();
        match case["status"].as_str().unwrap() {
            "ok" => {
                let expected = &case["song"];
                let actual = parse_song_json(&bytes, "fixture").expect("valid oracle case");
                assert_eq!(actual.name, expected["name"].as_str().unwrap(), "{case}");
                let expected_notes = expected["notes"].as_array().unwrap();
                assert_eq!(actual.notes.len(), expected_notes.len(), "{case}");
                for (actual, expected) in actual.notes.iter().zip(expected_notes) {
                    assert_eq!(
                        actual.time_ms,
                        expected["time_ms"].as_i64().unwrap(),
                        "{case}"
                    );
                    assert_eq!(actual.key, expected["key"].as_str().unwrap(), "{case}");
                }
            }
            "error" => assert!(parse_song_json(&bytes, "fixture").is_err(), "{case}"),
            other => panic!("unexpected parser status {other}"),
        }
    }
}

#[test]
fn schedule_and_risk_cases_match_python_oracle_fields() {
    let raw = fixture();
    let cases = raw["schedule_cases"].as_array().unwrap();
    assert!(cases.len() >= 4);
    for case in cases {
        let bytes = serde_json::to_vec(&case["raw"]).unwrap();
        let song = parse_song_json(&bytes, "fixture").expect("valid schedule fixture");
        let policy = MaterializedTimingPolicy::from_calibration(
            case["fps"].as_u64().unwrap() as u16,
            case["hold_frames"].as_f64().unwrap(),
            case["transport_margin_us"].as_u64().unwrap(),
            case["transport_margin_source"].as_str().unwrap(),
        )
        .expect("timing policy oracle case");
        let schedule =
            build_schedule_with_policy(&song, case["tempo_scale"].as_f64().unwrap(), &policy)
                .expect("schedule oracle case");
        let expected = &case["schedule"];
        let actual = serde_json::to_value(&schedule).unwrap();
        for field in [
            "actions",
            "source_duration_us",
            "playback_duration_us",
            "duration_us",
            "note_count",
            "deduplicated_note_count",
            "duplicate_note_count",
            "compressed_holds",
            "impossible_same_key_repeats",
            "risky_same_key_repeats",
            "max_polyphony",
            "shortest_same_key_interval_us",
            "min_same_key_up_gap_us",
            "recommended_hold_frames",
            "recommended_tempo_scale",
        ] {
            assert_eq!(
                actual[field], expected[field],
                "schedule field {field}: {case}"
            );
        }

        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            case["hold_frames"].as_f64().unwrap(),
            case["tempo_scale"].as_f64().unwrap(),
        );
        let expected_risk = &case["risk"];
        let actual_risk = serde_json::to_value(&risk).unwrap();
        for field in [
            "severity",
            "reason",
            "recommendations",
            "dense_clusters",
            "max_polyphony",
            "max_chord_size",
            "chords_count",
            "timing_stress_rate",
            "min_any_note_gap_us",
            "min_same_key_gap_us",
            "suggested_hold_frames",
            "suggested_tempo_scale",
        ] {
            assert_eq!(
                actual_risk[field], expected_risk[field],
                "risk field {field}: {case}"
            );
        }
    }
}

#[test]
fn action_kind_wire_names_remain_stable() {
    assert_eq!(serde_json::to_value(ActionKind::Down).unwrap(), "down");
    assert_eq!(serde_json::to_value(ActionKind::Up).unwrap(), "up");
}

#[test]
fn calibrated_policy_cases_match_the_python_production_resolver() {
    let raw = fixture();
    let cases = raw["timing_policy_cases"]
        .as_array()
        .expect("timing policy oracle cases");
    assert!(cases.len() >= 4);
    for case in cases {
        let policy = MaterializedTimingPolicy::from_calibration(
            case["fps"].as_u64().unwrap() as u16,
            case["hold_frames"].as_f64().unwrap(),
            case["transport_margin_us"].as_u64().unwrap(),
            case["transport_margin_source"].as_str().unwrap(),
        )
        .expect("materialized policy oracle case");
        assert_eq!(
            policy.frame_us,
            case["frame_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.frame_base_hold_us,
            case["frame_base_hold_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.down_late_grace_us,
            case["down_late_grace_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.min_hold_us,
            case["min_hold_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.min_release_gap_us,
            case["min_release_gap_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.focus_restore_grace_us,
            case["focus_restore_grace_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.transport_margin_source,
            case["transport_margin_source"].as_str().unwrap(),
            "{case}"
        );
    }
}

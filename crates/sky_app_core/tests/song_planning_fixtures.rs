use serde_json::{Value, json};
use sky_app_core::song::{
    ScheduleMetadata, analyze_schedule_with_context, build_schedule_with_policy, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/song_planning/schedule_contract_cases.json"
    ))
    .expect("valid committed schedule contract fixture")
}

fn assert_json_field(actual: &Value, expected: &Value, field: &str, context: &Value) {
    let actual_value = &actual[field];
    let expected_value = &expected[field];
    if let (Some(actual_number), Some(expected_number)) =
        (actual_value.as_f64(), expected_value.as_f64())
    {
        assert_eq!(actual_number, expected_number, "{field}: {context}");
    } else {
        assert_eq!(actual_value, expected_value, "{field}: {context}");
    }
}

/// Derive the historical paired Down/Up actions array from an onset-only
/// `ScheduleMetadata` so the immutable `schedule_contract_cases.json` fixture
/// can be verified without modifying historical evidence.
fn derive_historical_actions_json(schedule: &ScheduleMetadata) -> Value {
    let mut flat = Vec::new();
    for chord in &schedule.chords {
        for &scan in &chord.scan_codes {
            flat.push((chord.at_us, scan));
        }
    }
    let mut has_next = vec![false; flat.len()];
    let mut seen_later = HashMap::<u16, u64>::new();
    for (idx, &(at_us, scan)) in flat.iter().enumerate().rev() {
        has_next[idx] = seen_later.contains_key(&scan);
        seen_later.insert(scan, at_us);
    }
    // Sort key: (at_us, is_down) so Up precedes Down at equal timestamps.
    let mut grouped = BTreeMap::<(u64, bool), (Vec<u16>, BTreeSet<String>)>::new();
    for (idx, &(at_us, scan)) in flat.iter().enumerate() {
        let down = grouped.entry((at_us, true)).or_default();
        if !down.0.contains(&scan) {
            down.0.push(scan);
        }
        down.1.insert("onset".into());

        let up_at = at_us + schedule.hold_us;
        let up = grouped.entry((up_at, false)).or_default();
        if !up.0.contains(&scan) {
            up.0.push(scan);
        }
        up.1.insert(if has_next[idx] {
            "repeat_release".into()
        } else {
            "release".into()
        });
    }
    Value::Array(
        grouped
            .into_iter()
            .map(|((at_us, is_down), (scan_codes, reasons))| {
                let reason = if reasons.len() == 1 {
                    reasons.into_iter().next().unwrap_or_default()
                } else {
                    "mixed".into()
                };
                json!({
                    "at_us": at_us,
                    "scan_codes": scan_codes,
                    "kind": if is_down { "down" } else { "up" },
                    "reason": reason,
                })
            })
            .collect(),
    )
}

#[test]
fn parser_cases_preserve_the_checked_in_legacy_contract() {
    let raw = fixture();
    let cases = raw["parser_cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for case in cases {
        let bytes = serde_json::to_vec(&case["raw"]).unwrap();
        match case["status"].as_str().unwrap() {
            "ok" => {
                let expected = &case["song"];
                let actual = parse_song_json(&bytes, "fixture").expect("valid fixture case");
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
fn schedule_and_risk_cases_match_recorded_contract_fields() {
    let raw = fixture();
    let cases = raw["schedule_cases"].as_array().unwrap();
    assert!(cases.len() >= 4);
    for case in cases {
        let bytes = serde_json::to_vec(&case["raw"]).unwrap();
        let song = parse_song_json(&bytes, "fixture").expect("valid schedule fixture");
        let policy = MaterializedTimingPolicy::from_user_margin(
            case["fps"].as_u64().unwrap() as u16,
            case["hold_frames"].as_f64().unwrap(),
            case["timing_margin_us"].as_u64().unwrap(),
        )
        .expect("timing policy fixture case");
        let schedule =
            build_schedule_with_policy(&song, case["tempo_scale"].as_f64().unwrap(), &policy)
                .expect("schedule fixture case");
        let expected = &case["schedule"];
        let mut actual = serde_json::to_value(&schedule).unwrap();
        actual["actions"] = derive_historical_actions_json(&schedule);
        for field in [
            "actions",
            "source_duration_us",
            "playback_duration_us",
            "duration_us",
            "note_count",
            "deduplicated_note_count",
            "duplicate_note_count",
            "impossible_same_key_repeats",
            "max_polyphony",
            "shortest_same_key_interval_us",
            "min_same_key_slack_us",
            "same_key_repeat_count",
            "zero_slack_repeat_count",
            "recommended_hold_frames",
            "recommended_tempo_scale",
        ] {
            assert_json_field(&actual, expected, field, case);
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
            "min_same_key_slack_us",
            "same_key_repeat_count",
            "zero_slack_repeat_count",
            "min_any_note_gap_us",
            "min_same_key_gap_us",
            "suggested_hold_frames",
            "suggested_tempo_scale",
        ] {
            assert_json_field(&actual_risk, expected_risk, field, case);
        }
    }
}

#[test]
fn timing_policy_cases_match_the_authored_timing_contract() {
    let raw = fixture();
    let cases = raw["timing_policy_cases"]
        .as_array()
        .expect("timing policy fixture cases");
    assert!(cases.len() >= 4);
    for case in cases {
        let policy = MaterializedTimingPolicy::from_user_margin(
            case["fps"].as_u64().unwrap() as u16,
            case["hold_frames"].as_f64().unwrap(),
            case["timing_margin_us"].as_u64().unwrap(),
        )
        .expect("materialized policy fixture case");
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
            policy.min_hold_us,
            case["min_hold_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.focus_restore_grace_us,
            case["focus_restore_grace_us"].as_u64().unwrap(),
            "{case}"
        );
        assert_eq!(
            policy.timing_margin_us,
            case["timing_margin_us"].as_u64().unwrap()
        );
    }
}

#[test]
fn planner_invariants_across_boundary_tempo_scales() {
    let raw = fixture();
    let cases = raw["schedule_cases"].as_array().unwrap();
    let first_case = &cases[0];
    let bytes = serde_json::to_vec(&first_case["raw"]).unwrap();
    let song = parse_song_json(&bytes, "boundary-test").expect("valid test song");

    let policy =
        MaterializedTimingPolicy::from_user_margin(60, 1.0, 500).expect("valid timing policy");

    for tempo in [0.25, 0.5, 1.0, 1.5, 2.0] {
        let schedule = build_schedule_with_policy(&song, tempo, &policy)
            .unwrap_or_else(|e| panic!("failed to build schedule for tempo {tempo}: {e:?}"));

        assert_eq!(schedule.hold_us, policy.min_hold_us);
        let mut prev_us = None;
        for chord in &schedule.chords {
            if let Some(prev) = prev_us {
                assert!(
                    chord.at_us > prev,
                    "chord at {} did not strictly advance past {} for tempo {}",
                    chord.at_us,
                    prev,
                    tempo
                );
            }
            assert!(!chord.scan_codes.is_empty());
            prev_us = Some(chord.at_us);
        }
    }
}

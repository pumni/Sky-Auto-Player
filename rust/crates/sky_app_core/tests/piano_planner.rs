use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sky_app_core::song::{
    ActionKind, ConversionMode, LegacyCompatibilityPlan, PianoPlannerOptions, PlanStatus,
    PlannerError, SKY_SCAN_CODES, SongDecodeError, SongPlan, analyze_schedule_with_notes,
    build_schedule_with_policy, build_song_plan, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::BTreeSet;

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("planner fixture")
}

fn planner_cases() -> Value {
    serde_json::from_slice(&fixture("song_planning/piano_planner_cases.json"))
        .expect("valid Piano planner fixture")
}

fn policy() -> MaterializedTimingPolicy {
    MaterializedTimingPolicy::from_user_margin(60, 1.0, 0).expect("valid fixed policy")
}

fn run_case(case: &Value) -> sky_app_core::song::PianoPlan {
    let bytes = serde_json::to_vec(&case["sheet"]).unwrap();
    let options: PianoPlannerOptions = serde_json::from_value(json!({
        "mode": case["mode"],
        "target_pitch": case["target_pitch"],
        "tempo_scale": case["tempo_scale"]
    }))
    .expect("valid planner options");
    match build_song_plan(&bytes, "planner fixture", &options, &policy()).unwrap() {
        SongPlan::PianoConversion(plan) => plan,
        SongPlan::LegacyCompatibility(_) => panic!("versioned fixture took legacy path"),
    }
}

fn schedule_fingerprint<T: serde::Serialize>(value: &T) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn piano_planner_golden_cases_cover_pitch_mapping_modes_and_losses() {
    let cases = planner_cases();
    let cases = cases["cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for case in cases {
        let plan = run_case(case);
        if case["mode"].is_null() {
            assert_eq!(plan.mode, ConversionMode::MergePlayable);
        }
        assert_eq!(
            serde_json::to_value(plan.status).unwrap(),
            case["status"],
            "status: {}",
            case["name"]
        );
        let events = plan
            .events
            .iter()
            .map(|event| {
                json!({
                    "onset_us": event.onset_us,
                    "scheduled_onset_us": event.scheduled_onset_us,
                    "button_index": event.button_index,
                    "scan_code": event.scan_code,
                    "provenance_count": event.provenance.len()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            Value::Array(events),
            case["events"],
            "events: {}",
            case["name"]
        );

        if let Some(expected) = case.get("expected_target_pitch") {
            assert_eq!(
                serde_json::to_value(plan.target_pitch).unwrap(),
                *expected,
                "target pitch: {}",
                case["name"]
            );
        }
        if let Some(expected) = case.get("diagnostics") {
            let actual = serde_json::to_value(&plan.diagnostics).unwrap();
            for (field, expected_value) in expected.as_object().unwrap() {
                assert_eq!(actual[field], *expected_value, "{field}: {}", case["name"]);
            }
        }
        if let Some(expected) = case.get("loss_reasons") {
            let actual = plan
                .diagnostics
                .losses
                .iter()
                .map(|loss| {
                    serde_json::to_value(loss.reason)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<BTreeSet<_>>();
            let expected = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|reason| reason.as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected, "losses: {}", case["name"]);
        }
        if !plan.events.is_empty() {
            let schedule = plan
                .schedule
                .as_ref()
                .expect("playable events have schedule");
            assert_eq!(schedule.duplicate_note_count, 0, "{}", case["name"]);
            let mut emitted = BTreeSet::new();
            for action in &schedule.actions {
                if action.kind == ActionKind::Down {
                    for scan_code in &action.scan_codes {
                        assert!(
                            emitted.insert((action.at_us, *scan_code)),
                            "duplicate scan/time in {}",
                            case["name"]
                        );
                    }
                }
            }
            let actual_pitch = serde_json::to_value(plan.target_pitch).unwrap();
            if let Some(expected_pitch) = case.get("expected_target_pitch") {
                assert_eq!(actual_pitch, *expected_pitch, "{}", case["name"]);
            }
            if plan.diagnostics.authored_sustain_lost_count > 0 {
                let down = schedule
                    .actions
                    .iter()
                    .find(|action| action.kind == ActionKind::Down)
                    .unwrap();
                let up = schedule
                    .actions
                    .iter()
                    .find(|action| action.kind == ActionKind::Up)
                    .unwrap();
                assert_eq!(
                    up.at_us - down.at_us,
                    policy().min_hold_us,
                    "authored musical sustain must not redefine Piano's physical hold"
                );
            }
            let onset_groups = plan
                .events
                .iter()
                .fold(BTreeSet::new(), |mut groups, event| {
                    groups.insert(event.scheduled_onset_us);
                    groups
                });
            for onset in onset_groups {
                let chord_size = plan
                    .events
                    .iter()
                    .filter(|event| event.scheduled_onset_us == onset)
                    .count();
                if chord_size > 1 {
                    let down = schedule
                        .actions
                        .iter()
                        .find(|action| action.kind == ActionKind::Down && action.at_us == onset)
                        .expect("same-onset notes share one Down action");
                    assert_eq!(down.scan_codes.len(), chord_size);
                }
            }
        } else {
            assert!(plan.schedule.is_none(), "{}", case["name"]);
            assert_eq!(plan.status, PlanStatus::Unsupported);
        }
    }
}

#[test]
fn legacy_source_keeps_the_original_parser_and_schedule_path() {
    let bytes = fixture("song_decoding/legacy-flat.json");
    let expected_song = parse_song_json(&bytes, "legacy fallback").unwrap();
    let expected_schedule = build_schedule_with_policy(&expected_song, 0.75, &policy()).unwrap();
    let expected_risk = analyze_schedule_with_notes(&expected_schedule, &expected_song.notes);
    let options = PianoPlannerOptions {
        tempo_scale: 0.75,
        ..PianoPlannerOptions::default()
    };
    let plan = build_song_plan(&bytes, "legacy fallback", &options, &policy()).unwrap();
    let SongPlan::LegacyCompatibility(LegacyCompatibilityPlan {
        status,
        song,
        schedule,
        physical_risk,
    }) = plan
    else {
        panic!("flat legacy source did not use compatibility path")
    };
    assert_eq!(song, expected_song);
    assert_eq!(status, PlanStatus::Ready);
    assert_eq!(schedule, expected_schedule);
    assert_eq!(physical_risk, expected_risk);
}

#[test]
fn layer_expansion_is_counted_separately_from_serialized_source_tuples() {
    let bytes = fixture("song_decoding/composed-v1.json");
    let options = PianoPlannerOptions::default();
    let plan = build_song_plan(&bytes, "layered", &options, &policy()).unwrap();
    let SongPlan::PianoConversion(plan) = plan else {
        panic!("composed source did not use conversion path")
    };
    assert_eq!(plan.diagnostics.source_tuple_count, 2);
    assert_eq!(plan.diagnostics.decoded_event_count, 3);
    assert_eq!(
        plan.diagnostics.decoded_event_count,
        plan.diagnostics.selected_source_event_count + plan.diagnostics.omitted_source_event_count
    );
    assert_eq!(
        plan.diagnostics.selected_source_event_count,
        plan.diagnostics.resolved_event_count
            + plan.diagnostics.source_unpitched_count
            + plan.diagnostics.source_unresolved_count
    );
    assert_eq!(
        plan.diagnostics.resolved_event_count,
        plan.diagnostics.mapped_source_event_count + plan.diagnostics.target_unplayable_count
    );
    assert_eq!(
        plan.diagnostics.mapped_source_event_count,
        plan.diagnostics.mapped_onset_count + plan.diagnostics.merged_duplicate_count
    );
}

#[test]
fn decoder_warnings_remain_separate_from_physical_risk() {
    let bytes = fixture("song_decoding/legacy-flat.json");
    let source = sky_app_core::song::decode_song_json(&bytes, "flat canonical").unwrap();
    assert!(!source.diagnostics.warnings.is_empty());
    let plan = sky_app_core::song::plan_canonical_song(
        &source,
        &PianoPlannerOptions::default(),
        &policy(),
    )
    .unwrap();
    assert_eq!(
        plan.diagnostics.source_warnings,
        source.diagnostics.warnings
    );
    assert!(plan.physical_risk.is_some());
}

#[test]
fn same_key_repeat_conflicts_are_preserved_for_existing_risk_gates() {
    let sheet = json!({
        "type": "recorded", "version": 4,
        "data": {"appName": "Sky"},
        "tracks": [{
            "instrument": {"name": "Piano"},
            "notes": [[72, 0], [72, 1]]
        }]
    });
    let bytes = serde_json::to_vec(&sheet).unwrap();
    let options = PianoPlannerOptions::default();
    let plan = build_song_plan(&bytes, "repeat", &options, &policy()).unwrap();
    let SongPlan::PianoConversion(plan) = plan else {
        panic!("recorded source did not use conversion path")
    };
    assert_eq!(plan.events.len(), 2);
    assert_eq!(plan.events[0].onset_us, 0);
    assert_eq!(plan.events[1].onset_us, 1_000);
    assert_eq!(plan.events[0].scheduled_onset_us, 0);
    assert_eq!(plan.events[1].scheduled_onset_us, 1_000);
    let schedule = plan.schedule.as_ref().unwrap();
    assert_eq!(schedule.impossible_same_key_repeats, 1);
    assert_eq!(plan.diagnostics.physical_timing_conflict_count, 1);
    assert_eq!(plan.physical_risk.as_ref().unwrap().severity, "high");
    let downs = schedule
        .actions
        .iter()
        .filter(|action| action.kind == ActionKind::Down)
        .map(|action| action.at_us)
        .collect::<Vec<_>>();
    assert_eq!(downs, [0, 1_000]);
    assert_eq!(plan.status, PlanStatus::Ready);
}

#[test]
fn tempo_rounding_merges_same_key_schedule_collisions_with_provenance() {
    let sheet = json!({
        "type": "recorded", "version": 4,
        "data": {"appName": "Sky"},
        "tracks": [{
            "instrument": {"name": "Piano"},
            "notes": [[72, 0], [72, 1]]
        }]
    });
    let options = PianoPlannerOptions {
        tempo_scale: 1_000_000.0,
        ..PianoPlannerOptions::default()
    };
    let plan = build_song_plan(
        &serde_json::to_vec(&sheet).unwrap(),
        "rounding collision",
        &options,
        &policy(),
    )
    .unwrap();
    let SongPlan::PianoConversion(plan) = plan else {
        panic!("recorded source did not use conversion path")
    };
    assert_eq!(plan.events.len(), 1);
    assert_eq!(plan.events[0].scheduled_onset_us, 0);
    assert_eq!(plan.events[0].provenance.len(), 2);
    assert_eq!(plan.diagnostics.mapped_source_event_count, 2);
    assert_eq!(plan.diagnostics.mapped_onset_count, 1);
    assert_eq!(plan.diagnostics.merged_duplicate_count, 1);
    assert_eq!(plan.schedule.as_ref().unwrap().duplicate_note_count, 0);
}

#[test]
fn full_fifteen_key_chord_keeps_all_simultaneous_scan_codes() {
    let note_numbers = [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84];
    let notes = note_numbers
        .iter()
        .map(|number| json!([0, number]))
        .collect::<Vec<_>>();
    let sheet = json!({
        "type": "composed", "version": 5, "bpm": 120, "pitch": "C",
        "data": {"appName": "Sky"}, "columnTempos": [0],
        "tracks": [{"instrument": {"name": "Piano"}, "notes": notes}]
    });
    let options = PianoPlannerOptions::default();
    let plan = build_song_plan(
        &serde_json::to_vec(&sheet).unwrap(),
        "full chord",
        &options,
        &policy(),
    )
    .unwrap();
    let SongPlan::PianoConversion(plan) = plan else {
        panic!("composed source did not use conversion path")
    };
    assert_eq!(plan.events.len(), 15);
    assert_eq!(
        plan.events
            .iter()
            .map(|event| event.button_index)
            .collect::<Vec<_>>(),
        (0..15).collect::<Vec<_>>()
    );
    assert!(plan.events.iter().all(|event| event.onset_us == 100_000));
    let down = plan
        .schedule
        .as_ref()
        .unwrap()
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::Down)
        .unwrap();
    assert_eq!(down.at_us, 100_000);
    assert_eq!(down.scan_codes, SKY_SCAN_CODES);
    assert_eq!(plan.physical_risk.as_ref().unwrap().severity, "medium");
}

#[test]
fn planner_output_and_schedule_fingerprint_ignore_canonical_vector_order() {
    let bytes = fixture("song_decoding/composed-v1.json");
    let source = sky_app_core::song::decode_song_json(&bytes, "stable").unwrap();
    let options = PianoPlannerOptions::default();
    let first = sky_app_core::song::plan_canonical_song(&source, &options, &policy()).unwrap();
    let first_hash = schedule_fingerprint(first.schedule.as_ref().unwrap());

    let mut reversed = source.clone();
    reversed.events.reverse();
    let second = sky_app_core::song::plan_canonical_song(&reversed, &options, &policy()).unwrap();
    let second_hash = schedule_fingerprint(second.schedule.as_ref().unwrap());
    assert_eq!(first.events, second.events);
    assert_eq!(first_hash, second_hash);
    assert_eq!(
        first_hash,
        schedule_fingerprint(first.schedule.as_ref().unwrap())
    );
}

#[test]
fn malformed_versioned_sources_and_bad_times_do_not_fall_back_to_song_notes() {
    let malformed = json!({
        "type": "composed", "version": 5, "bpm": 120,
        "data": {"appName": "Sky"}, "columnTempos": [0],
        "tracks": [{"instrument": {"name": "Piano"}, "notes": [[0, 72, 0]]}],
        "songNotes": [{"key": "Key0", "time": 0}]
    });
    let options = PianoPlannerOptions::default();
    assert!(matches!(
        build_song_plan(
            &serde_json::to_vec(&malformed).unwrap(),
            "bad",
            &options,
            &policy()
        ),
        Err(PlannerError::Source(SongDecodeError::InvalidField { .. }))
    ));

    let bad_time = json!({
        "type": "recorded", "version": 4,
        "data": {"appName": "Sky"},
        "tracks": [{"instrument": {"name": "Piano"}, "notes": [[72, -1]]}]
    });
    assert!(matches!(
        build_song_plan(
            &serde_json::to_vec(&bad_time).unwrap(),
            "bad time",
            &options,
            &policy()
        ),
        Err(PlannerError::Source(SongDecodeError::InvalidField { .. }))
    ));
}

#[test]
fn track_selection_rejects_unknown_ids() {
    let bytes = fixture("song_decoding/composed-v5.json");
    let options = PianoPlannerOptions {
        mode: Some(ConversionMode::SingleTrack { track_id: 4 }),
        ..PianoPlannerOptions::default()
    };
    assert!(matches!(
        build_song_plan(&bytes, "selection", &options, &policy()),
        Err(PlannerError::InvalidTrackSelection { track_id: 4 })
    ));
}

#[test]
fn duplicated_track_ids_are_rejected_even_when_instrument_names_are_valid() {
    let bytes = fixture("song_decoding/composed-v5.json");
    let mut source = sky_app_core::song::decode_song_json(&bytes, "ambiguous id").unwrap();
    source.tracks.push(source.tracks[0].clone());
    assert!(matches!(
        sky_app_core::song::plan_canonical_song(
            &source,
            &PianoPlannerOptions::default(),
            &policy()
        ),
        Err(PlannerError::AmbiguousTrackId { track_id: 0 })
    ));
}

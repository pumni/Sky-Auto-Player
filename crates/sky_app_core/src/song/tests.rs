use super::*;

#[test]
fn parser_accepts_legacy_aliases_and_stably_sorts() {
    let song = parse_song_json(
        br#"{"name":"Demo","songNotes":[{"time":20,"key":"1Key1"},{"time":"10","key":"Key0"}]}"#,
        "fallback",
    )
    .unwrap();
    assert_eq!(song.notes[0].time_ms, 10);
    assert_eq!(scan_code_for_key(&song.notes[1].key), Some(0x16));
}

#[test]
fn schedule_preserves_chords_and_deduplicates_same_key_timestamp() {
    let song = Song {
        name: "Demo".into(),
        notes: vec![
            Note {
                time_ms: 0,
                key: "Key0".into(),
                source_index: 0,
            },
            Note {
                time_ms: 0,
                key: "Key1".into(),
                source_index: 1,
            },
            Note {
                time_ms: 0,
                key: "Key1".into(),
                source_index: 2,
            },
        ],
    };
    let schedule = build_schedule(&song, 1.0, 1.0, 60).unwrap();
    assert_eq!(schedule.duplicate_note_count, 1);
    assert_eq!(schedule.chords[0].scan_codes, vec![0x15, 0x16]);
}

#[test]
fn schedule_preserves_authored_chord_scan_order() {
    let song = Song {
        name: "Demo".into(),
        notes: vec![
            Note {
                time_ms: 0,
                key: "Key1".into(),
                source_index: 0,
            },
            Note {
                time_ms: 0,
                key: "Key0".into(),
                source_index: 1,
            },
        ],
    };
    let schedule = build_schedule(&song, 1.0, 1.0, 60).unwrap();
    assert_eq!(schedule.chords[0].scan_codes, vec![0x16, 0x15]);
}

#[test]
fn same_key_repeat_requires_only_the_hold_floor_after_up() {
    let song = Song {
        name: "Repeated key".into(),
        notes: vec![
            Note {
                time_ms: 0,
                key: "Key0".into(),
                source_index: 0,
            },
            Note {
                time_ms: 35,
                key: "Key0".into(),
                source_index: 1,
            },
        ],
    };
    let margin_800 = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 800)
        .expect("valid margin");
    let margin_900 = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 900)
        .expect("valid margin");
    let feasible = build_schedule_with_policy(&song, 1.0, &margin_800).expect("schedule");
    assert_eq!(feasible.impossible_same_key_repeats, 0);
    assert_eq!(feasible.hold_us, margin_800.min_hold_us);

    let margin_change = build_schedule_with_policy(&song, 1.0, &margin_900).expect("schedule");
    assert_eq!(margin_change.impossible_same_key_repeats, 0);
    assert_eq!(margin_change.hold_us, margin_900.min_hold_us);
}

#[test]
fn repeat_up_is_down_plus_h_and_equal_deadlines_order_up_before_down() {
    let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid policy");
    let h = policy.min_hold_us;
    for (interval, conflicts, min_slack, repeat_count, zero_slack_count) in [
        (h - 1, 1, None, 0, 0),
        (h, 0, Some(0), 1, 1),
        (h + 1, 0, Some(1), 1, 0),
    ] {
        let schedule = build_schedule_from_scaled_onsets(
            vec![(0, SKY_SCAN_CODES[0], 0), (interval, SKY_SCAN_CODES[0], 1)],
            1.0,
            &policy,
        )
        .expect("schedule timestamps fit");
        assert_eq!(
            schedule.impossible_same_key_repeats, conflicts,
            "interval={interval}"
        );
        assert_eq!(schedule.hold_us, h, "interval={interval}");
        assert_eq!(schedule.chords.len(), 2, "interval={interval}");
        assert_eq!(schedule.chords[0].at_us, 0, "interval={interval}");
        assert_eq!(schedule.chords[1].at_us, interval, "interval={interval}");
        assert_eq!(schedule.min_same_key_slack_us, min_slack);
        assert_eq!(schedule.same_key_repeat_count, repeat_count);
        assert_eq!(schedule.zero_slack_repeat_count, zero_slack_count);
        if interval == h {
            assert_eq!(schedule.max_polyphony, 1);
            let risk = analyze_schedule(&schedule);
            // Zero nominal slack is diagnostic only: Hold-only allows I == H
            // without elevating severity or gating admission.
            assert_eq!(risk.severity, "low");
            assert_eq!(risk.min_same_key_slack_us, Some(0));
            assert_eq!(risk.same_key_repeat_count, 1);
            assert_eq!(risk.zero_slack_repeat_count, 1);
        }
    }
}

#[test]
fn nominal_same_key_slack_is_diagnostic_only_without_frame_threshold() {
    let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid policy");
    let mut notes = Vec::new();
    let mut source_index = 0;
    for cluster in 0..3usize {
        let cluster_start_ms = i64::try_from(cluster * 200).expect("test time range");
        notes.push(Note {
            time_ms: cluster_start_ms,
            key: KEY_NAMES[0].into(),
            source_index,
        });
        source_index += 1;
        notes.push(Note {
            time_ms: cluster_start_ms + 17,
            key: KEY_NAMES[0].into(),
            source_index,
        });
        source_index += 1;
    }
    let schedule = build_schedule_with_policy(
        &Song {
            name: "Small positive nominal slack".into(),
            notes,
        },
        1.0,
        &policy,
    )
    .expect("feasible schedule");
    assert_eq!(schedule.impossible_same_key_repeats, 0);
    assert_eq!(schedule.same_key_repeat_count, 5);
    assert_eq!(schedule.zero_slack_repeat_count, 0);
    assert_eq!(
        schedule.min_same_key_slack_us,
        Some(17_000 - policy.min_hold_us)
    );

    let risk = analyze_schedule(&schedule);
    assert_eq!(risk.severity, "low");
    assert_eq!(risk.min_same_key_slack_us, schedule.min_same_key_slack_us);
    assert_eq!(risk.same_key_repeat_count, 5);
    assert_eq!(risk.zero_slack_repeat_count, 0);
}

#[test]
fn empty_song_has_the_materialized_hold_duration() {
    let schedule = build_schedule(
        &Song {
            name: "Empty".into(),
            notes: Vec::new(),
        },
        1.0,
        1.0,
        60,
    )
    .expect("empty songs produce an empty schedule");
    assert!(schedule.chords.is_empty());
    assert_eq!(schedule.source_duration_us, 17_167);
}

#[test]
fn risk_reports_dense_cluster() {
    let song = Song {
        name: "Dense".into(),
        notes: (0usize..7)
            .map(|index| Note {
                time_ms: index as i64,
                key: KEY_NAMES[index].into(),
                source_index: index,
            })
            .collect(),
    };
    let report = analyze_schedule(&build_schedule(&song, 1.0, 1.0, 60).unwrap());
    assert_eq!(report.dense_clusters.len(), 1);
}

#[test]
fn legacy_timestamp_coercion_preserves_documented_json_values() {
    let song = parse_song_json(
            br#"{"songNotes":[{"time":true,"key":"Key0"},{"time":2.9,"key":"Key1"},{"time":" 3 ","key":"Key2"}]}"#,
            "fallback",
        )
        .expect("legacy timestamp forms remain accepted");
    assert_eq!(
        song.notes
            .iter()
            .map(|note| note.time_ms)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    let invalid_time = parse_song_json(
        br#"{"songNotes":[{"time":9223372036854775808,"key":"Key0"}]}"#,
        "fallback",
    );
    assert!(matches!(
        invalid_time,
        Err(SongError::InvalidTime { index: 0 })
    ));
}

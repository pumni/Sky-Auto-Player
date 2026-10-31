use serde_json::{Value, json};
use sky_app_core::song::{
    CanonicalPitch, Pitchability, RawNoteIdentity, SongDecodeError, SongFormat, SourceGame,
    decode_song_json, detect_song_format,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../tests/fixtures/song_decoding/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("source format fixture")
}

fn decode_fixture(name: &str) -> sky_app_core::song::CanonicalSong {
    let bytes = fixture(name);
    decode_song_json(&bytes, name).expect("valid source format fixture")
}

fn note_number(event: &sky_app_core::song::CanonicalEvent) -> (i32, Pitchability) {
    match event.pitch {
        CanonicalPitch::NoteNumber {
            number,
            source_pitchability,
        } => (number, source_pitchability),
        CanonicalPitch::SkyKeyIndex(index) => panic!("expected note number, got Sky key {index}"),
    }
}

#[test]
fn flat_keys_preserve_layer_identity_and_stable_timing() {
    let bytes = fixture("legacy-flat.json");
    assert_eq!(detect_song_format(&bytes).unwrap(), SongFormat::LegacyFlat);
    let song = decode_song_json(&bytes, "fallback").unwrap();

    assert_eq!(song.source_game, SourceGame::Sky);
    assert_eq!(song.source_pitch, "A");
    assert_eq!(song.events.len(), 4);
    assert_eq!(song.diagnostics.source_note_count, 3);
    assert_eq!(song.diagnostics.source_track_count, 2);
    assert_eq!(song.diagnostics.potential_physical_key_merge_count, 2);
    assert_eq!(song.events[0].onset_us, 0);
    assert_eq!(song.events[0].pitch, CanonicalPitch::SkyKeyIndex(3));
    assert_eq!(song.events[1].onset_us, 2_500);
    assert_eq!(song.events[1].track_index, 1);
    assert_eq!(song.events[1].layer_mask, Some(2));
    assert_eq!(song.events[2].track_index, 0);
    assert_eq!(song.events[3].track_index, 1);
}

#[test]
fn official_historical_sky_headers_select_authoritative_v1_sources() {
    let composed_bytes = fixture("historical-composed-v1.json");
    assert_eq!(
        detect_song_format(&composed_bytes).unwrap(),
        SongFormat::Composed { version: 1 }
    );
    let composed = decode_song_json(&composed_bytes, "historical composed").unwrap();
    assert_eq!(composed.source_game, SourceGame::Sky);
    assert_eq!(composed.tracks.len(), 4);
    assert_eq!(composed.events.len(), 1);
    assert_eq!(composed.events[0].track_index, 0);
    assert_eq!(composed.events[0].onset_us, 100_000);

    let recorded_bytes = fixture("historical-recorded-v1.json");
    assert_eq!(
        detect_song_format(&recorded_bytes).unwrap(),
        SongFormat::Recorded { version: 1 }
    );
    let recorded = decode_song_json(&recorded_bytes, "historical recorded").unwrap();
    assert_eq!(recorded.source_game, SourceGame::Sky);
    assert_eq!(recorded.tracks.len(), 1);
    assert_eq!(recorded.tracks[0].instrument.as_deref(), Some("Piano"));
    assert_eq!(recorded.events.len(), 1);
    assert_eq!(recorded.events[0].track_index, 0);
    assert_eq!(recorded.events[0].onset_us, 909_000);
}

#[test]
fn composed_v1_through_v3_decode_their_authoritative_layer_encodings() {
    let v1 = decode_fixture("composed-v1.json");
    assert_eq!(v1.format, SongFormat::Composed { version: 1 });
    assert_eq!(v1.events.len(), 3);
    assert_eq!(v1.events[0].track_index, 1);
    assert_eq!(note_number(&v1.events[0]).0, 48);
    assert_eq!(v1.events[0].layer_mask, Some(2));
    assert_eq!(v1.events[0].onset_us, 100_000);
    assert_eq!(v1.events[1].onset_us, 600_000);
    assert_eq!(note_number(&v1.events[1]).0, 62);
    assert_eq!(note_number(&v1.events[2]).0, 50);

    let v2 = decode_fixture("composed-v2.json");
    assert_eq!(v2.format, SongFormat::Composed { version: 2 });
    assert_eq!(v2.events.len(), 3);
    assert_eq!(v2.events[0].track_index, 1);
    assert_eq!(note_number(&v2.events[0]).0, 36);
    assert_eq!(v2.events[0].onset_us, 100_000);
    assert_eq!(v2.events[1].onset_us, 600_000);
    assert_eq!(v2.events[1].layer_mask, Some(3));

    let v3 = decode_fixture("composed-v3.json");
    assert_eq!(v3.format, SongFormat::Composed { version: 3 });
    assert_eq!(v3.tracks[1].effective_pitch.as_deref(), Some("D"));
    assert_eq!(v3.events.len(), 1);
    assert_eq!(v3.events[0].onset_us, 100_000);
    assert_eq!(note_number(&v3.events[0]).0, 52);
}

#[test]
fn composed_v4_converts_nominal_pitch_and_only_spans_longer_than_one_column_sustain() {
    let v4 = decode_fixture("composed-v4.json");
    assert_eq!(v4.format, SongFormat::Composed { version: 4 });
    assert_eq!(v4.events.len(), 3);
    assert_eq!(v4.events[0].raw_identity, RawNoteIdentity::NominalId(72));
    assert_eq!(note_number(&v4.events[0]), (50, Pitchability::Pitched));
    assert_eq!(v4.events[0].onset_us, 100_000);
    assert_eq!(v4.events[0].duration_us, 750_000);
    assert_eq!(v4.events[0].span_columns, Some(2));
    assert_eq!(v4.events[1].onset_us, 600_000);
    assert_eq!(v4.events[1].duration_us, 0);
    assert_eq!(v4.events[1].span_columns, Some(1));
    assert_eq!(v4.events[2].onset_us, 850_000);
    assert_eq!(v4.events[2].duration_us, 0);
    assert_eq!(v4.diagnostics.authored_sustain_count, 1);

    let v5 = decode_fixture("composed-v5.json");
    assert_eq!(v5.format, SongFormat::Composed { version: 5 });
    assert_eq!(
        v5.events[0].raw_identity,
        RawNoteIdentity::AbsoluteNoteNumber(50)
    );
    assert_eq!(note_number(&v5.events[0]), (50, Pitchability::Pitched));
    assert_eq!(v5.events[0].onset_us, 100_000);
    assert_eq!(v5.events[0].duration_us, 0);
    assert_eq!(v5.events[0].span_columns, Some(1));
    assert_eq!(v5.events[1].onset_us, 600_000);
    assert_eq!(v5.events[1].duration_us, 0);
}

#[test]
fn source_instrument_tables_keep_unpitched_notes_and_special_transpositions() {
    let sheet = json!({
        "type": "composed", "version": 4, "bpm": 120, "pitch": "C",
        "data": {"appName": "Genshin"}, "columnTempos": [0],
        "tracks": [{
            "instrument": {"name": "Ukulele", "pitch": ""},
            "notes": [[0, 72]]
        }]
    });
    let song = decode_song_json(&serde_json::to_vec(&sheet).unwrap(), "table").unwrap();
    assert_eq!(note_number(&song.events[0]), (84, Pitchability::Unpitched));
}

#[test]
fn recorded_v1_through_v4_keep_their_layer_time_pitch_and_duration_semantics() {
    let v1 = decode_fixture("recorded-v1.json");
    assert_eq!(v1.format, SongFormat::Recorded { version: 1 });
    assert_eq!(v1.events.len(), 4);
    assert_eq!(v1.events[0].track_index, 1);
    assert_eq!(v1.events[0].onset_us, 10_000);
    assert_eq!(note_number(&v1.events[0]).0, 50);
    assert_eq!(v1.events[0].layer_mask, Some(2));
    assert_eq!(v1.events[1].track_index, 4);
    assert_eq!(v1.events[1].layer_mask, Some(16));
    assert_eq!(note_number(&v1.events[1]).0, 38);
    assert_eq!(v1.events[2].layer_mask, Some(1));
    assert_eq!(v1.events[3].layer_mask, Some(1));

    let v2 = decode_fixture("recorded-v2.json");
    assert_eq!(v2.format, SongFormat::Recorded { version: 2 });
    assert_eq!(v2.source_game, SourceGame::Genshin);
    assert_eq!(v2.events.len(), 2);
    assert_eq!(v2.events[0].onset_us, 25_000);
    assert_eq!(note_number(&v2.events[0]).0, 74);
    assert_eq!(note_number(&v2.events[1]).0, 73);

    let v3 = decode_fixture("recorded-v3.json");
    assert_eq!(v3.format, SongFormat::Recorded { version: 3 });
    assert_eq!(v3.events[0].raw_identity, RawNoteIdentity::NominalId(72));
    assert_eq!(note_number(&v3.events[0]).0, 72);
    assert_eq!(v3.events[0].onset_us, 12_500);
    assert_eq!(v3.events[0].duration_us, 20_000);

    let v4 = decode_fixture("recorded-v4.json");
    assert_eq!(v4.format, SongFormat::Recorded { version: 4 });
    assert_eq!(
        v4.events[0].raw_identity,
        RawNoteIdentity::AbsoluteNoteNumber(73)
    );
    assert_eq!(note_number(&v4.events[0]), (73, Pitchability::Pitched));
    assert_eq!(v4.events[0].duration_us, 20_000);
}

#[test]
fn versioned_authority_and_container_validation_fail_closed() {
    let legacy = br#"{"songNotes":[{"key":"Key0","time":0}]}"#;
    let invalid_version =
        br#"{"type":"composed","version":6,"songNotes":[{"key":"Key0","time":0}]}"#;
    assert!(matches!(
        detect_song_format(invalid_version),
        Err(SongDecodeError::UnsupportedVersion { .. })
    ));
    assert!(matches!(
        decode_song_json(b"[]", "x"),
        Err(SongDecodeError::EmptySongArray)
    ));
    assert!(matches!(
        decode_song_json(b"[{},{}]", "x"),
        Err(SongDecodeError::MultipleSongsUnsupported { count: 2 })
    ));
    assert!(decode_song_json(b"[{\"songNotes\":[]}]", "x").is_ok());
    assert!(matches!(
        decode_song_json(
            br#"{"type":"recorded","version":4,"isEncrypted":true}"#,
            "x"
        ),
        Err(SongDecodeError::Encrypted)
    ));
    assert!(matches!(
        decode_song_json(
            br#"{"type":"recorded","version":4,"data":{"isEncrypted":true}}"#,
            "x"
        ),
        Err(SongDecodeError::Encrypted)
    ));
    assert!(matches!(
        decode_song_json(
            br#"{"type":"composed","version":1,"bpm":120,"instruments":[],"columns":[]}"#,
            "x"
        ),
        Err(SongDecodeError::AmbiguousSourceGame)
    ));
    assert!(matches!(
        decode_song_json(legacy, "x"),
        Ok(song) if song.format == SongFormat::LegacyFlat
    ));

    let contradictory_flag = br#"{"type":"composed","version":1,"data":{"appName":"Sky","isComposedVersion":false},"instruments":[],"columns":[]}"#;
    assert!(matches!(
        detect_song_format(contradictory_flag),
        Err(SongDecodeError::UnsupportedFormat { .. })
    ));
    let historical_wrong_container =
        br#"{"data":{"appName":"Sky","isComposedVersion":true},"notes":[]}"#;
    assert!(matches!(
        detect_song_format(historical_wrong_container),
        Err(SongDecodeError::MissingField { path }) if path == "version"
    ));
    let versionless_modern =
        br#"{"data":{"appName":"Sky","isComposedVersion":true},"columnTempos":[],"tracks":[]}"#;
    assert!(matches!(
        detect_song_format(versionless_modern),
        Err(SongDecodeError::MissingField { path }) if path == "version"
    ));
    let missing_type = br#"{"version":1,"data":{"appName":"Sky","isComposedVersion":true},"columns":[],"instruments":[]}"#;
    assert_eq!(
        detect_song_format(missing_type).unwrap(),
        SongFormat::Composed { version: 1 }
    );
    let type_shape_conflict =
        br#"{"type":"recorded","version":1,"data":{"appName":"Sky"},"columns":[]}"#;
    assert!(matches!(
        detect_song_format(type_shape_conflict),
        Err(SongDecodeError::UnsupportedFormat { .. })
    ));

    let unknown_instrument = json!({
        "type": "recorded", "version": 4, "data": {"appName": "Sky"},
        "tracks": [{"instrument": {"name": "FutureInstrument"}, "notes": []}]
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&unknown_instrument).unwrap(), "x"),
        Err(SongDecodeError::UnsupportedInstrument { .. })
    ));

    let multiple_notes = json!({
        "type": "composed", "version": 5, "bpm": 120,
        "data": {"appName": "Sky"}, "columnTempos": [0],
        "tracks": [{"instrument": {"name": "Piano"}, "notes": [[0, 60, 0]]}]
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&multiple_notes).unwrap(), "x"),
        Err(SongDecodeError::InvalidField { .. })
    ));

    let oversized = vec![b' '; sky_app_core::song::MAX_SHEET_BYTES + 1];
    assert!(matches!(
        decode_song_json(&oversized, "x"),
        Err(SongDecodeError::PayloadTooLarge { .. })
    ));
}

#[test]
fn legacy_layer_masks_reject_tracks_outside_declared_rosters() {
    let invalid_sheets = [
        json!({
            "type": "composed", "version": 1, "bpm": 120,
            "data": {"appName": "Sky"}, "instruments": ["Piano"],
            "columns": [[0, [[0, "01"]]]]
        }),
        json!({
            "type": "composed", "version": 1, "bpm": 120,
            "data": {"appName": "Sky"}, "columns": [[0, [[0, "1"]]]]
        }),
        json!({
            "type": "composed", "version": 2, "bpm": 120,
            "data": {"appName": "Sky"}, "instruments": ["Piano"],
            "columns": [[0, [[0, "2"]]]]
        }),
        json!({
            "type": "composed", "version": 3, "bpm": 120,
            "data": {"appName": "Sky"},
            "instruments": [{"name": "Piano"}], "columns": [[0, [[0, "2"]]]]
        }),
        json!({
            "type": "recorded", "version": 1,
            "data": {"appName": "Sky"}, "instruments": [{"name": "Piano"}],
            "notes": [[0, 0, 2]]
        }),
        json!({
            "type": "recorded", "version": 2,
            "data": {"appName": "Sky"}, "instruments": [{"name": "Piano"}],
            "notes": [[0, 0, "2"]]
        }),
    ];

    for (index, value) in invalid_sheets.into_iter().enumerate() {
        let result = decode_song_json(&serde_json::to_vec(&value).unwrap(), "x");
        assert!(
            matches!(
                &result,
                Err(SongDecodeError::InvalidField { .. } | SongDecodeError::MissingField { .. })
            ),
            "undeclared track case {index} should fail, got {result:?}"
        );
    }
}

#[test]
fn recorded_v1_v2_absent_rosters_allow_only_the_single_default_track() {
    let recorded_v2 = json!({
        "type": "recorded", "version": 2,
        "data": {"appName": "Sky"}, "notes": [[9, 909, "1"]]
    });
    let song = decode_song_json(&serde_json::to_vec(&recorded_v2).unwrap(), "implicit v2").unwrap();
    assert_eq!(song.tracks.len(), 1);
    assert_eq!(song.tracks[0].instrument.as_deref(), Some("Piano"));
    assert_eq!(song.events.len(), 1);
    assert_eq!(song.events[0].track_index, 0);

    for (version, mask) in [(1, json!(2)), (2, json!("2"))] {
        let out_of_roster = json!({
            "type": "recorded", "version": version,
            "data": {"appName": "Sky"}, "notes": [[9, 909, mask]]
        });
        assert!(matches!(
            decode_song_json(
                &serde_json::to_vec(&out_of_roster).unwrap(),
                "implicit out of roster"
            ),
            Err(SongDecodeError::InvalidField { .. })
        ));
    }

    let explicit_empty = json!({
        "type": "recorded", "version": 1,
        "data": {"appName": "Sky"}, "instruments": [], "notes": [[9, 909, 1]]
    });
    assert!(matches!(
        decode_song_json(
            &serde_json::to_vec(&explicit_empty).unwrap(),
            "empty roster"
        ),
        Err(SongDecodeError::InvalidField { .. })
    ));

    let explicit_underspecified = json!({
        "type": "recorded", "version": 1,
        "data": {"appName": "Sky"}, "instruments": [{}], "notes": [[9, 909, 1]]
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&explicit_underspecified).unwrap(), "underspecified roster"),
        Err(SongDecodeError::MissingField { path }) if path == "instruments[0].name"
    ));
}

#[test]
fn composed_timing_rounds_millisecond_boundaries_with_long_compositions() {
    let columns = vec![json!(0); 10_000];
    let value: Value = json!({
        "type": "composed", "version": 4, "bpm": 137,
        "data": {"appName": "Sky"}, "columnTempos": columns,
        "tracks": [{"instrument": {"name": "Piano"}, "notes": [[0, 60, 10_000], [9_999, 62]]}]
    });
    let song = decode_song_json(&serde_json::to_vec(&value).unwrap(), "timing").unwrap();
    assert_eq!(song.events[0].onset_us, 100_000);
    assert_eq!(song.events[0].duration_us, 4_379_562_000);
    assert_eq!(song.events[1].onset_us, 4_379_224_000);
    assert_eq!(song.events[1].duration_us, 0);
}

#[test]
fn decoder_enforces_source_track_note_and_column_limits() {
    let tracks = vec![
        json!({ "instrument": {"name": "Piano"}, "notes": [] });
        sky_app_core::song::MAX_SOURCE_TRACKS + 1
    ];
    let too_many_tracks = json!({
        "type": "recorded", "version": 4,
        "data": {"appName": "Sky"}, "tracks": tracks
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&too_many_tracks).unwrap(), "x"),
        Err(SongDecodeError::ResourceLimit { .. })
    ));

    let notes = vec![json!({ "key": "Key0", "time": 0 }); sky_app_core::song::MAX_SOURCE_NOTES + 1];
    let too_many_notes = json!({ "songNotes": notes });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&too_many_notes).unwrap(), "x"),
        Err(SongDecodeError::ResourceLimit { .. })
    ));

    let tempos = vec![json!(0); sky_app_core::song::MAX_SOURCE_COLUMNS + 1];
    let too_many_columns = json!({
        "type": "composed", "version": 4, "bpm": 120,
        "data": {"appName": "Sky"}, "columnTempos": tempos,
        "tracks": []
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&too_many_columns).unwrap(), "x"),
        Err(SongDecodeError::ResourceLimit { .. })
    ));

    let too_many_expanded_events = json!({
        "songNotes": vec![
            json!({ "key": "Key0", "time": 0, "l": 65_535 });
            sky_app_core::song::MAX_SOURCE_NOTES / 16 + 1
        ]
    });
    assert!(matches!(
        decode_song_json(&serde_json::to_vec(&too_many_expanded_events).unwrap(), "x"),
        Err(SongDecodeError::ResourceLimit { .. })
    ));
}

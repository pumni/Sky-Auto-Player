use serde::Deserialize;
use sha2::{Digest, Sha256};
use sky_app_core::song::{build_schedule_with_policy, parse_song_json};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const EXPECTED_HISTORICAL_SONG_COUNT: usize = 106;

#[derive(Deserialize)]
struct BaselineFingerprint {
    input_sha256: String,
    schedule_sha256: String,
    note_count: usize,
    action_count: usize,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn historical_source_fixture(relative: &str) -> Option<&'static [u8]> {
    match relative {
        "Only_LeeHi.json" => Some(include_bytes!(
            "../../../../tests/fixtures/song_planning/historical_sources/Only_LeeHi.json"
        )),
        "Stay with me Goblin.json" => Some(include_bytes!(
            "../../../../tests/fixtures/song_planning/historical_sources/Stay with me Goblin.json"
        )),
        _ => None,
    }
}

#[test]
fn historical_songs_match_the_pinned_current_schedule() {
    let expected: BTreeMap<String, BaselineFingerprint> = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/song_planning/generation_schedule_fingerprints.v2.json"
    ))
    .expect("valid pinned generation schedule fingerprints v2");
    assert_eq!(expected.len(), EXPECTED_HISTORICAL_SONG_COUNT);

    let songs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../songs")
        .canonicalize()
        .expect("repository songs directory");
    // Frozen representative settings: 60 FPS, one hold frame, zero user margin, no tempo scaling.
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid frozen compatibility timing policy");
    for (relative, baseline) in &expected {
        let path = songs_dir.join(relative);
        // Replaced active sources retain their original bytes in immutable
        // fixtures. Every unchanged historical source is still read from songs/.
        let bytes = historical_source_fixture(relative)
            .map(<[u8]>::to_vec)
            .unwrap_or_else(|| fs::read(&path).expect("read built-in song"));
        assert_eq!(
            sha256_hex(&bytes),
            baseline.input_sha256,
            "asset changed: {relative}"
        );

        let fallback = path.file_stem().unwrap().to_string_lossy();
        let song = parse_song_json(&bytes, &fallback)
            .unwrap_or_else(|error| panic!("legacy parse failed for {relative}: {error}"));
        let schedule = build_schedule_with_policy(&song, 1.0, &policy)
            .unwrap_or_else(|error| panic!("legacy schedule failed for {relative}: {error}"));
        assert_eq!(
            song.notes.len(),
            baseline.note_count,
            "note count: {relative}"
        );
        assert_eq!(
            schedule.actions.len(),
            baseline.action_count,
            "action count: {relative}"
        );
        assert_eq!(
            sha256_hex(&serde_json::to_vec(&schedule).expect("serialize schedule")),
            baseline.schedule_sha256,
            "pinned schedule changed: {relative}"
        );
    }
}

#[test]
fn legacy_schedule_fingerprint_v1_is_preserved_as_a_historical_record() {
    let legacy = include_bytes!(
        "../../../../tests/fixtures/song_planning/legacy_schedule_fingerprints.json"
    );
    let expected_hash = "dbc270ffc3f2507e0d00fec1e483ccdd2583be3cf3fd6477ef40834bdb3b62be";
    assert_eq!(sha256_hex(legacy), expected_hash);
    let fingerprints: BTreeMap<String, BaselineFingerprint> =
        serde_json::from_slice(legacy).expect("valid immutable schedule fingerprints v1");
    assert_eq!(fingerprints.len(), EXPECTED_HISTORICAL_SONG_COUNT);
}

#[test]
fn wide_gap_schedule_projection_matches_the_legacy_release_schedule() {
    let expected: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/song_planning/wide_gap_legacy_projection_v1.json"
    ))
    .expect("valid wide-gap legacy projection");
    let song = sky_app_core::song::Song {
        name: "Wide-gap repeat".into(),
        notes: vec![
            sky_app_core::song::Note {
                time_ms: 0,
                key: "Key0".into(),
                source_index: 0,
            },
            sky_app_core::song::Note {
                time_ms: 1_000,
                key: "Key0".into(),
                source_index: 1,
            },
        ],
    };
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid frozen compatibility timing policy");
    let schedule = build_schedule_with_policy(&song, 1.0, &policy).expect("wide-gap schedule");
    let projection = serde_json::json!({
        "actions": schedule.actions.iter().map(|action| serde_json::json!({
            "at_us": action.at_us,
            "kind": action.kind,
            "scan_codes": action.scan_codes,
        })).collect::<Vec<_>>(),
    });
    assert_eq!(projection, expected);
}

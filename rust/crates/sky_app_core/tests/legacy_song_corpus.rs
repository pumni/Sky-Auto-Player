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
fn frozen_historical_songs_match_the_pinned_legacy_schedule_baseline() {
    let expected: BTreeMap<String, BaselineFingerprint> = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/song_planning/legacy_schedule_fingerprints.json"
    ))
    .expect("valid pinned legacy schedule fingerprints");
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
            "legacy schedule changed: {relative}"
        );
    }
}

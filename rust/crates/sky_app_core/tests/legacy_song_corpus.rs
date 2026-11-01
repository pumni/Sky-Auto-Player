use serde::Deserialize;
use sha2::{Digest, Sha256};
use sky_app_core::song::{build_schedule_with_policy, parse_song_json};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const EXPECTED_SONG_COUNT: usize = 106;

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

#[test]
fn all_builtin_songs_match_the_pinned_legacy_schedule_baseline() {
    let expected: BTreeMap<String, BaselineFingerprint> = serde_json::from_slice(include_bytes!(
        "../../../../tests/fixtures/song_planning/legacy_schedule_fingerprints.json"
    ))
    .expect("valid pinned legacy schedule fingerprints");
    assert_eq!(expected.len(), EXPECTED_SONG_COUNT);

    let songs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../songs")
        .canonicalize()
        .expect("repository songs directory");
    let mut paths = fs::read_dir(&songs_dir)
        .expect("read songs directory")
        .map(|entry| entry.expect("read song entry").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "json" | "skysheet" | "txt"
                    )
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths.len(), EXPECTED_SONG_COUNT);

    // Frozen representative settings: 60 FPS, one hold frame, zero user margin, no tempo scaling.
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid frozen compatibility timing policy");
    for path in paths {
        let bytes = fs::read(&path).expect("read built-in song");
        let relative = path
            .strip_prefix(&songs_dir)
            .expect("song is directly inside songs")
            .to_string_lossy()
            .replace('\\', "/");
        let baseline = expected
            .get(&relative)
            .unwrap_or_else(|| panic!("missing baseline fingerprint for {relative}"));
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

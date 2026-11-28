use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sky_app_core::song::{ScheduleMetadata, build_schedule_with_policy, parse_song_json};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum HistoricalActionKind {
    Down,
    Up,
}

#[derive(Serialize)]
struct HistoricalKeyAction {
    at_us: u64,
    scan_codes: Vec<u16>,
    kind: HistoricalActionKind,
    reason: String,
}

#[derive(Serialize)]
struct HistoricalScheduleProjection<'a> {
    actions: Vec<HistoricalKeyAction>,
    source_duration_us: u64,
    playback_duration_us: u64,
    duration_us: u64,
    note_count: usize,
    deduplicated_note_count: usize,
    duplicate_note_count: usize,
    impossible_same_key_repeats: usize,
    max_polyphony: usize,
    shortest_same_key_interval_us: Option<u64>,
    min_same_key_up_gap_us: Option<u64>,
    short_up_gap_repeat_count: usize,
    recommended_hold_frames: Option<f64>,
    recommended_tempo_scale: Option<f64>,
    warnings: &'a [String],
}

/// Derive the historical paired-action JSON projection from the canonical
/// onset-only schedule solely to verify exact equivalence against the
/// immutable 106-song v2 fingerprint fixture.
fn derive_historical_projection(
    schedule: &ScheduleMetadata,
    frame_us: u64,
) -> HistoricalScheduleProjection<'_> {
    let mut flat = Vec::new();
    for chord in &schedule.chords {
        for &scan in &chord.scan_codes {
            flat.push((chord.at_us, scan));
        }
    }
    let mut has_next = vec![false; flat.len()];
    let mut seen_later = HashMap::<u16, u64>::new();
    let mut min_same_key_up_gap_us: Option<u64> = None;
    let mut short_up_gap_repeat_count = 0usize;
    for (idx, &(at_us, scan)) in flat.iter().enumerate().rev() {
        if let Some(&next_at) = seen_later.get(&scan) {
            has_next[idx] = true;
            let interval = next_at.saturating_sub(at_us);
            let gap = interval.saturating_sub(schedule.hold_us);
            min_same_key_up_gap_us =
                Some(min_same_key_up_gap_us.map_or(gap, |current| current.min(gap)));
            if interval >= schedule.hold_us && interval - schedule.hold_us < frame_us {
                short_up_gap_repeat_count += 1;
            }
        }
        seen_later.insert(scan, at_us);
    }

    let mut grouped = BTreeMap::<(u64, HistoricalActionKind), (Vec<u16>, BTreeSet<String>)>::new();
    for (idx, &(at_us, scan)) in flat.iter().enumerate() {
        let down = grouped
            .entry((at_us, HistoricalActionKind::Down))
            .or_default();
        if !down.0.contains(&scan) {
            down.0.push(scan);
        }
        down.1.insert("onset".into());

        let up_at = at_us + schedule.hold_us;
        let up = grouped
            .entry((up_at, HistoricalActionKind::Up))
            .or_default();
        if !up.0.contains(&scan) {
            up.0.push(scan);
        }
        up.1.insert(if has_next[idx] {
            "repeat_release".into()
        } else {
            "release".into()
        });
    }
    let mut actions = grouped
        .into_iter()
        .map(
            |((at_us, kind), (scan_codes, reasons))| HistoricalKeyAction {
                at_us,
                scan_codes,
                kind,
                reason: if reasons.len() == 1 {
                    reasons.into_iter().next().unwrap_or_default()
                } else {
                    "mixed".into()
                },
            },
        )
        .collect::<Vec<_>>();
    actions.sort_by_key(|action| {
        (
            action.at_us,
            matches!(action.kind, HistoricalActionKind::Down),
        )
    });

    HistoricalScheduleProjection {
        actions,
        source_duration_us: schedule.source_duration_us,
        playback_duration_us: schedule.playback_duration_us,
        duration_us: schedule.duration_us,
        note_count: schedule.note_count,
        deduplicated_note_count: schedule.deduplicated_note_count,
        duplicate_note_count: schedule.duplicate_note_count,
        impossible_same_key_repeats: schedule.impossible_same_key_repeats,
        max_polyphony: schedule.max_polyphony,
        shortest_same_key_interval_us: schedule.shortest_same_key_interval_us,
        min_same_key_up_gap_us,
        short_up_gap_repeat_count,
        recommended_hold_frames: schedule.recommended_hold_frames,
        recommended_tempo_scale: schedule.recommended_tempo_scale,
        warnings: &schedule.warnings,
    }
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
            "../../../tests/fixtures/song_planning/historical_sources/Only_LeeHi.json"
        )),
        "Stay with me Goblin.json" => Some(include_bytes!(
            "../../../tests/fixtures/song_planning/historical_sources/Stay with me Goblin.json"
        )),
        _ => None,
    }
}

#[test]
fn historical_songs_match_the_pinned_current_schedule() {
    let expected: BTreeMap<String, BaselineFingerprint> = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/song_planning/generation_schedule_fingerprints.v2.json"
    ))
    .expect("valid pinned generation schedule fingerprints v2");
    assert_eq!(expected.len(), EXPECTED_HISTORICAL_SONG_COUNT);

    let songs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../songs")
        .canonicalize()
        .expect("repository songs directory");
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .expect("valid frozen compatibility timing policy");
    for (relative, baseline) in &expected {
        let path = songs_dir.join(relative);
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
        let projected = derive_historical_projection(&schedule, policy.frame_us);
        assert_eq!(
            song.notes.len(),
            baseline.note_count,
            "note count: {relative}"
        );
        assert_eq!(
            projected.actions.len(),
            baseline.action_count,
            "action count: {relative}"
        );
        assert_eq!(
            sha256_hex(&serde_json::to_vec(&projected).expect("serialize projected schedule")),
            baseline.schedule_sha256,
            "pinned schedule changed: {relative}"
        );
    }
}

#[test]
fn legacy_schedule_fingerprint_v1_is_preserved_as_a_historical_record() {
    let legacy =
        include_bytes!("../../../tests/fixtures/song_planning/legacy_schedule_fingerprints.json");
    let expected_hash = "dbc270ffc3f2507e0d00fec1e483ccdd2583be3cf3fd6477ef40834bdb3b62be";
    assert_eq!(sha256_hex(legacy), expected_hash);
    let fingerprints: BTreeMap<String, BaselineFingerprint> =
        serde_json::from_slice(legacy).expect("valid immutable schedule fingerprints v1");
    assert_eq!(fingerprints.len(), EXPECTED_HISTORICAL_SONG_COUNT);
}

#[test]
fn wide_gap_schedule_projection_matches_the_legacy_release_schedule() {
    let expected: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/song_planning/wide_gap_legacy_projection_v1.json"
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
    let projected = derive_historical_projection(&schedule, policy.frame_us);
    let projection = serde_json::json!({
        "actions": projected.actions.iter().map(|action| serde_json::json!({
            "at_us": action.at_us,
            "kind": action.kind,
            "scan_codes": action.scan_codes,
        })).collect::<Vec<_>>(),
    });
    assert_eq!(projection, expected);
}

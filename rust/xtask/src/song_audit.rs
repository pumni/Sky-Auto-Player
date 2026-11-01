use crate::{Result, builtin_catalog};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sky_app_core::song::{
    CanonicalPitch, CanonicalSong, PianoPlannerOptions, PlanStatus, SongFormat, SongPlan,
    build_song_plan, decode_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const INVENTORY_SCHEMA_VERSION: u32 = 1;
const DEFAULT_OUTPUT_DIR: &str = "docs/song-curation";
const FAST_REPEAT_THRESHOLD_US: u64 = 16_667;
const C_KEY_SEMITONES: [i16; 15] = [0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19, 21, 23, 24];

#[derive(Debug, Clone, Serialize)]
struct AuditMetrics {
    source_title_metadata_field: Option<String>,
    source_title_metadata: Option<String>,
    source_author_metadata_field: Option<String>,
    source_author_metadata: Option<String>,
    source_format: String,
    source_schema_version: Option<u8>,
    source_game: String,
    source_pitch: String,
    source_track_count: usize,
    source_instruments: Vec<String>,
    parsing_status: String,
    raw_note_count: usize,
    authored_source_event_count: usize,
    decoded_event_count: usize,
    authored_distinct_onset_count: usize,
    unique_physical_time_key_count: usize,
    physical_collision_group_count: usize,
    physical_collision_extra_event_count: usize,
    source_layer_collision_extra_event_count: usize,
    first_note_us: Option<u64>,
    last_note_us: Option<u64>,
    active_duration_us: Option<u64>,
    active_seconds: Option<f64>,
    onsets_per_second: Option<f64>,
    maximum_rest_us: Option<u64>,
    max_polyphony: Option<usize>,
    physical_onset_count: usize,
    chord_onset_count: usize,
    chord_onset_ratio: Option<f64>,
    max_chord_size: Option<usize>,
    average_chord_size: Option<f64>,
    fast_repeat_count: usize,
    fast_repeat_threshold_us: u64,
    plan_status: String,
    plan_error: Option<String>,
    plan_loss_count: usize,
    omitted_source_event_count: usize,
    target_unplayable_count: usize,
    source_unpitched_count: usize,
    source_unresolved_count: usize,
    multi_track_lossy_in_15_key_planner: Option<bool>,
    physical_risk_severity: Option<String>,
    physical_risk_reason: Option<String>,
    impossible_same_key_repeat_count: Option<usize>,
    risky_same_key_repeat_count: Option<usize>,
    output_physical_note_count: Option<usize>,
    output_action_count: Option<usize>,
    schedule_duration_us: Option<u64>,
    authored_sustain_count: usize,
    decoder_warnings: Vec<String>,
    planner_loss_reasons: Vec<String>,
    schedule_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct SheetAnalysis {
    metrics: AuditMetrics,
    physical_notes: Vec<(u64, u8)>,
    melody: Vec<i16>,
    interval_sketch: Vec<i16>,
    normalized_onset_positions: Vec<u16>,
    normalized_inter_onset_ratios: Vec<u32>,
    phrase_break_indices: Vec<usize>,
    relative_time_quartile_coverage: [bool; 4],
}

#[derive(Debug, Serialize)]
struct InventorySong {
    id: String,
    path: String,
    manifest_title: String,
    manifest_sha256: String,
    #[serde(flatten)]
    metrics: AuditMetrics,
}

#[derive(Debug, Serialize)]
struct InventoryDocument {
    schema_version: u32,
    manifest_sha256: String,
    active_song_count: usize,
    songs: Vec<InventorySong>,
}

#[derive(Debug, Serialize)]
struct ComparisonDocument {
    schema_version: u32,
    current: ComparisonSheet,
    candidate: ComparisonSheet,
    comparison: ComparisonSignals,
    limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ComparisonSheet {
    input_path: String,
    sha256: String,
    #[serde(flatten)]
    metrics: AuditMetrics,
    normalized_melody_semitones: Vec<i16>,
    normalized_interval_sketch: Vec<i16>,
    normalized_onset_positions: Vec<u16>,
    normalized_inter_onset_ratios: Vec<u32>,
    phrase_break_indices: Vec<usize>,
    relative_time_quartile_coverage: [bool; 4],
}

#[derive(Debug, Serialize)]
struct ComparisonSignals {
    interval_sequence_similarity: f64,
    normalized_timing_note_overlap: Option<f64>,
    initial_onset_offset_candidate_minus_current_us: Option<i64>,
    initial_offset_aligned_time_key_overlap: Option<f64>,
    current_events_subset_of_candidate_after_initial_offset: bool,
    candidate_events_subset_of_current_after_initial_offset: bool,
    active_duration_ratio_candidate_over_current: Option<f64>,
    source_pitch_matches: Option<bool>,
    aligned_melody_button_offset_semitones: Option<i16>,
    chord_onset_ratio_difference: Option<f64>,
    max_chord_size_difference: Option<i64>,
    composition_section_completeness: String,
    relative_time_quartiles_with_notes: usize,
    source_identity_confidence: String,
    structural_signal_confidence: String,
    musical_fidelity_confidence: String,
    listening_required: bool,
    verdict: String,
}

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("inventory") => run_inventory(root, &args[1..]),
        Some("compare") => run_compare(&args[1..]),
        Some(other) => Err(format!(
            "unknown song-audit operation `{other}`; expected inventory or compare"
        )
        .into()),
        None => Err("song-audit requires inventory or compare".into()),
    }
}

fn run_inventory(root: &Path, args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let output_dir =
        option_path(&options, "output-dir").unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT_DIR));
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        root.join(output_dir)
    };

    let manifest_path = root.join("builtin-songs/manifest.json");
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let (manifest, sources) = builtin_catalog::verified_sources(root)?;
    let mut songs = Vec::with_capacity(sources.len());
    for source in sources {
        let fallback = Path::new(&source.path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&source.title);
        let analysis = analyze_sheet(&source.bytes, fallback)?;
        songs.push(InventorySong {
            id: source.id,
            path: source.path,
            manifest_title: source.title,
            manifest_sha256: source.sha256,
            metrics: analysis.metrics,
        });
    }
    songs.sort_by(|left, right| compare_catalog_paths(&left.path, &right.path));

    let document = InventoryDocument {
        schema_version: INVENTORY_SCHEMA_VERSION,
        manifest_sha256: manifest_sha256.clone(),
        active_song_count: manifest.songs.len(),
        songs,
    };
    if document.songs.len() != document.active_song_count {
        return Err(format!(
            "catalog audit coverage mismatch: manifest={}, audited={}",
            document.active_song_count,
            document.songs.len()
        )
        .into());
    }

    fs::create_dir_all(&output_dir)?;
    let json = json_with_newline(&document)?;
    let csv = inventory_csv(&document.songs);
    let summary = inventory_summary(&document);
    let json_path = output_dir.join("inventory.json");
    let csv_path = output_dir.join("inventory.csv");
    let summary_path = output_dir.join("inventory-summary.md");
    fs::write(&json_path, &json)?;
    fs::write(&csv_path, &csv)?;
    fs::write(&summary_path, &summary)?;
    println!(
        "Song audit inventory: PASS (active={}, audited={}, manifest_sha256={}, inventory_json_sha256={}, inventory_csv_sha256={})",
        document.active_song_count,
        document.songs.len(),
        manifest_sha256,
        sha256_hex(&json),
        sha256_hex(csv.as_bytes()),
    );
    println!(
        "Artifacts: {}, {}, {}",
        json_path.display(),
        csv_path.display(),
        summary_path.display()
    );
    Ok(())
}

fn run_compare(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let current_path = required_path(&options, "current")?;
    let candidate_path = required_path(&options, "candidate")?;
    let current_bytes = fs::read(&current_path)?;
    let candidate_bytes = fs::read(&candidate_path)?;
    let current_fallback = file_stem(&current_path);
    let candidate_fallback = file_stem(&candidate_path);
    let current = analyze_sheet(&current_bytes, &current_fallback)?;
    let candidate = analyze_sheet(&candidate_bytes, &candidate_fallback)?;
    let document = comparison_document(
        &current_path,
        &current_bytes,
        current,
        &candidate_path,
        &candidate_bytes,
        candidate,
    );
    let bytes = json_with_newline(&document)?;
    match option_path(&options, "output") {
        Some(path) => {
            fs::write(&path, &bytes)?;
            println!(
                "Candidate comparison: PASS (current_sha256={}, candidate_sha256={}, report_sha256={}) -> {}",
                sha256_hex(&current_bytes),
                sha256_hex(&candidate_bytes),
                sha256_hex(&bytes),
                path.display()
            );
        }
        None => print!("{}", String::from_utf8(bytes)?),
    }
    Ok(())
}

fn analyze_sheet(bytes: &[u8], fallback_name: &str) -> Result<SheetAnalysis> {
    let source = decode_song_json(bytes, fallback_name)
        .map_err(|error| format!("strict source decode failed: {error}"))?;
    let raw = raw_song_object(bytes)?;
    let plan = build_song_plan(
        bytes,
        fallback_name,
        &PianoPlannerOptions::default(),
        &audit_policy()?,
    );

    let mut unmerged = Vec::new();
    let mut schedule_action_count = None;
    let mut schedule_note_count = None;
    let mut schedule_duration_us = None;
    let mut impossible_repeats = None;
    let mut risky_repeats = None;
    let mut risk_severity = None;
    let mut risk_reason = None;
    let mut max_polyphony = None;
    let mut schedule_warnings = Vec::new();
    let plan_status;
    let mut plan_error = None;
    let mut plan_loss_count = 0;
    let mut omitted_events = 0;
    let mut target_unplayable = 0;
    let mut source_unpitched = 0;
    let mut source_unresolved = 0;
    let mut planner_loss_reasons = Vec::new();
    let mut multi_track_lossy = None;

    match plan {
        Ok(SongPlan::LegacyCompatibility(legacy)) => {
            plan_status = "ready_legacy_compatibility".to_owned();
            schedule_action_count = Some(legacy.schedule.actions.len());
            schedule_note_count = Some(legacy.schedule.note_count);
            schedule_duration_us = Some(legacy.schedule.duration_us);
            impossible_repeats = Some(legacy.schedule.impossible_same_key_repeats);
            risky_repeats = Some(legacy.schedule.risky_same_key_repeats);
            risk_severity = Some(legacy.physical_risk.severity.clone());
            risk_reason = Some(legacy.physical_risk.reason.clone());
            max_polyphony = Some(legacy.physical_risk.max_polyphony);
            schedule_warnings = legacy.schedule.warnings.clone();
            for event in &source.events {
                if let CanonicalPitch::SkyKeyIndex(button) = &event.pitch {
                    unmerged.push((event.onset_us, *button));
                }
            }
        }
        Ok(SongPlan::PianoConversion(piano)) => {
            plan_status = plan_status_name(piano.status).to_owned();
            plan_loss_count = piano.diagnostics.losses.len();
            omitted_events = piano.diagnostics.omitted_source_event_count;
            target_unplayable = piano.diagnostics.target_unplayable_count;
            source_unpitched = piano.diagnostics.source_unpitched_count;
            source_unresolved = piano.diagnostics.source_unresolved_count;
            planner_loss_reasons = piano
                .diagnostics
                .losses
                .iter()
                .map(|loss| format!("{:?}", loss.reason))
                .collect();
            if piano.source_tracks.len() > 1 {
                multi_track_lossy = Some(
                    omitted_events > 0
                        || target_unplayable > 0
                        || source_unpitched > 0
                        || source_unresolved > 0,
                );
            }
            if let Some(schedule) = &piano.schedule {
                schedule_action_count = Some(schedule.actions.len());
                schedule_note_count = Some(schedule.note_count);
                schedule_duration_us = Some(schedule.duration_us);
                impossible_repeats = Some(schedule.impossible_same_key_repeats);
                risky_repeats = Some(schedule.risky_same_key_repeats);
                schedule_warnings = schedule.warnings.clone();
            }
            if let Some(risk) = &piano.physical_risk {
                risk_severity = Some(risk.severity.clone());
                risk_reason = Some(risk.reason.clone());
                max_polyphony = Some(risk.max_polyphony);
            }
            for onset in &piano.events {
                for _ in &onset.provenance {
                    unmerged.push((onset.scheduled_onset_us, onset.button_index));
                }
            }
        }
        Err(error) => {
            plan_error = Some(error.to_string());
            plan_status = "unavailable".to_owned();
        }
    }

    let physical_notes = unmerged.iter().copied().collect::<BTreeSet<_>>();
    let mut physical_notes = physical_notes.iter().copied().collect::<Vec<_>>();
    physical_notes.sort_unstable();
    unmerged.sort_unstable();
    let mut pair_counts = BTreeMap::<(u64, u8), usize>::new();
    for pair in &unmerged {
        *pair_counts.entry(*pair).or_default() += 1;
    }
    let physical_collision_group_count = pair_counts.values().filter(|count| **count > 1).count();
    let physical_collision_extra_event_count = pair_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    let source_layer_collision_extra_event_count = source_layer_collisions(&source);

    let authored_onsets = source
        .events
        .iter()
        .map(|event| event.onset_us)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let (first_note_us, last_note_us, active_duration_us) = match (
        source.events.iter().map(|event| event.onset_us).min(),
        source.events.iter().map(|event| event.onset_us).max(),
    ) {
        (Some(first), Some(last)) => (Some(first), Some(last), Some(last - first)),
        _ => (None, None, None),
    };
    let active_seconds = active_duration_us.map(|duration| duration as f64 / 1_000_000.0);
    let onsets_per_second = active_duration_us
        .filter(|duration| *duration > 0)
        .map(|duration| authored_onsets.len() as f64 / (duration as f64 / 1_000_000.0));
    let maximum_rest_us = authored_onsets
        .windows(2)
        .map(|window| window[1] - window[0])
        .max();
    let onset_groups = group_physical_onsets(&physical_notes);
    let chord_onset_count = onset_groups.values().filter(|keys| keys.len() > 1).count();
    let physical_onset_count = onset_groups.len();
    let max_chord_size = onset_groups.values().map(BTreeSet::len).max();
    let average_chord_size = (!onset_groups.is_empty())
        .then(|| physical_notes.len() as f64 / physical_onset_count as f64);
    let chord_onset_ratio =
        (!onset_groups.is_empty()).then(|| chord_onset_count as f64 / physical_onset_count as f64);
    let fast_repeat_count = count_fast_repeats(&physical_notes);
    let (source_schema_version, source_format) = format_fields(source.format);
    let source_instruments = source
        .tracks
        .iter()
        .filter_map(|track| track.instrument.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let source_title_metadata = metadata_entry(&raw, &["title", "name", "songName"]);
    let source_author_metadata = metadata_entry(
        &raw,
        &["author", "arranger", "creator", "composer", "artist", "by"],
    );
    let mut decoder_warnings = source
        .diagnostics
        .warnings
        .iter()
        .map(|warning| format!("{:?}", warning))
        .collect::<Vec<_>>();
    decoder_warnings.sort();
    planner_loss_reasons.sort();
    schedule_warnings.sort();

    let metrics = AuditMetrics {
        source_title_metadata_field: source_title_metadata.as_ref().map(|entry| entry.0.clone()),
        source_title_metadata: source_title_metadata.map(|entry| entry.1),
        source_author_metadata_field: source_author_metadata.as_ref().map(|entry| entry.0.clone()),
        source_author_metadata: source_author_metadata.map(|entry| entry.1),
        source_format,
        source_schema_version,
        source_game: format!("{:?}", source.source_game),
        source_pitch: source.source_pitch.clone(),
        source_track_count: source.diagnostics.source_track_count,
        source_instruments,
        parsing_status: "pass".into(),
        raw_note_count: source.diagnostics.source_note_count,
        authored_source_event_count: source.diagnostics.source_note_count,
        decoded_event_count: source.diagnostics.decoded_event_count,
        authored_distinct_onset_count: authored_onsets.len(),
        unique_physical_time_key_count: physical_notes.len(),
        physical_collision_group_count,
        physical_collision_extra_event_count,
        source_layer_collision_extra_event_count,
        first_note_us,
        last_note_us,
        active_duration_us,
        active_seconds,
        onsets_per_second,
        maximum_rest_us,
        max_polyphony,
        physical_onset_count,
        chord_onset_count,
        chord_onset_ratio,
        max_chord_size,
        average_chord_size,
        fast_repeat_count,
        fast_repeat_threshold_us: FAST_REPEAT_THRESHOLD_US,
        plan_status,
        plan_error,
        plan_loss_count,
        omitted_source_event_count: omitted_events,
        target_unplayable_count: target_unplayable,
        source_unpitched_count: source_unpitched,
        source_unresolved_count: source_unresolved,
        multi_track_lossy_in_15_key_planner: multi_track_lossy,
        physical_risk_severity: risk_severity,
        physical_risk_reason: risk_reason,
        impossible_same_key_repeat_count: impossible_repeats,
        risky_same_key_repeat_count: risky_repeats,
        output_physical_note_count: schedule_note_count,
        output_action_count: schedule_action_count,
        schedule_duration_us,
        authored_sustain_count: source.diagnostics.authored_sustain_count,
        decoder_warnings,
        planner_loss_reasons,
        schedule_warnings,
    };
    let melody = melody_sketch(&physical_notes, &source.source_pitch);
    let interval_sketch = melody
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let physical_onsets = group_physical_onsets(&physical_notes)
        .into_keys()
        .collect::<Vec<_>>();
    let normalized_onset_positions = normalized_onset_positions(&physical_onsets);
    let normalized_inter_onset_ratios = normalized_inter_onset_ratios(&physical_onsets);
    let phrase_break_indices = phrase_breaks(&authored_onsets);
    let relative_time_quartile_coverage = time_quartiles(&physical_notes);

    Ok(SheetAnalysis {
        metrics,
        physical_notes,
        melody,
        interval_sketch,
        normalized_onset_positions,
        normalized_inter_onset_ratios,
        phrase_break_indices,
        relative_time_quartile_coverage,
    })
}

fn compare_catalog_paths(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| left.cmp(right))
}

fn source_layer_collisions(source: &CanonicalSong) -> usize {
    if source.format != SongFormat::LegacyFlat {
        return 0;
    }
    let mut groups = BTreeMap::<(u64, u8), BTreeMap<usize, BTreeSet<usize>>>::new();
    for event in &source.events {
        if let CanonicalPitch::SkyKeyIndex(button) = &event.pitch {
            groups
                .entry((event.onset_us, *button))
                .or_default()
                .entry(event.source_index)
                .or_default()
                .insert(event.track_index);
        }
    }
    groups
        .values()
        .flat_map(BTreeMap::values)
        .map(|tracks| tracks.len().saturating_sub(1))
        .sum()
}

fn group_physical_onsets(notes: &[(u64, u8)]) -> BTreeMap<u64, BTreeSet<u8>> {
    let mut groups = BTreeMap::<u64, BTreeSet<u8>>::new();
    for (time, button) in notes {
        groups.entry(*time).or_default().insert(*button);
    }
    groups
}

fn count_fast_repeats(notes: &[(u64, u8)]) -> usize {
    let mut by_button = BTreeMap::<u8, Vec<u64>>::new();
    for (time, button) in notes {
        by_button.entry(*button).or_default().push(*time);
    }
    by_button
        .values_mut()
        .map(|times| {
            times.sort_unstable();
            times
                .windows(2)
                .filter(|pair| pair[1] - pair[0] < FAST_REPEAT_THRESHOLD_US)
                .count()
        })
        .sum()
}

fn melody_sketch(notes: &[(u64, u8)], source_pitch: &str) -> Vec<i16> {
    let groups = group_physical_onsets(notes);
    let offset = pitch_offset(source_pitch);
    groups
        .values()
        .filter_map(|keys| {
            keys.iter()
                .next_back()
                .map(|button| C_KEY_SEMITONES[usize::from(*button)] + offset)
        })
        .collect()
}

fn phrase_breaks(onsets: &[u64]) -> Vec<usize> {
    if onsets.len() < 3 {
        return Vec::new();
    }
    let mut intervals = onsets
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    let median = intervals[intervals.len() / 2];
    onsets
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            (median > 0 && pair[1] - pair[0] >= median.saturating_mul(4)).then_some(index + 1)
        })
        .collect()
}

fn normalized_onset_positions(onsets: &[u64]) -> Vec<u16> {
    let Some(first) = onsets.first().copied() else {
        return Vec::new();
    };
    let last = onsets.last().copied().unwrap_or(first);
    let span = last.saturating_sub(first);
    onsets
        .iter()
        .map(|time| {
            if span == 0 {
                0
            } else {
                ((time.saturating_sub(first) as u128 * 10_000) / span as u128) as u16
            }
        })
        .collect()
}

fn normalized_inter_onset_ratios(onsets: &[u64]) -> Vec<u32> {
    let mut intervals = onsets
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .filter(|interval| *interval > 0)
        .collect::<Vec<_>>();
    if intervals.is_empty() {
        return Vec::new();
    }
    intervals.sort_unstable();
    let median = intervals[intervals.len() / 2];
    onsets
        .windows(2)
        .map(|pair| {
            (((pair[1].saturating_sub(pair[0])) as u128 * 1_000) / median as u128)
                .min(u128::from(u32::MAX)) as u32
        })
        .collect()
}

fn time_quartiles(notes: &[(u64, u8)]) -> [bool; 4] {
    let Some(first) = notes.iter().map(|(time, _)| *time).min() else {
        return [false; 4];
    };
    let last = notes.iter().map(|(time, _)| *time).max().unwrap_or(first);
    let span = last.saturating_sub(first);
    let mut result = [false; 4];
    for (time, _) in notes {
        let index = if span == 0 {
            0
        } else {
            ((time.saturating_sub(first) as u128 * 4) / span as u128).min(3) as usize
        };
        result[index] = true;
    }
    result
}

fn comparison_document(
    current_path: &Path,
    current_bytes: &[u8],
    current: SheetAnalysis,
    candidate_path: &Path,
    candidate_bytes: &[u8],
    candidate: SheetAnalysis,
) -> ComparisonDocument {
    let interval_sequence_similarity =
        sequence_similarity(&current.interval_sketch, &candidate.interval_sketch);
    let note_overlap = normalized_note_overlap(&current, &candidate);
    let initial_offset_alignment = initial_offset_alignment(&current, &candidate);
    let duration_ratio = match (
        current.metrics.active_duration_us,
        candidate.metrics.active_duration_us,
    ) {
        (Some(a), Some(b)) if a > 0 => Some(b as f64 / a as f64),
        (Some(0), Some(0)) => Some(1.0),
        _ => None,
    };
    let source_pitch_matches = Some(current.metrics.source_pitch == candidate.metrics.source_pitch);
    let aligned_melody_button_offset_semitones =
        aligned_pitch_offset(&current, &candidate, interval_sequence_similarity);
    let chord_onset_ratio_difference = match (
        current.metrics.chord_onset_ratio,
        candidate.metrics.chord_onset_ratio,
    ) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let max_chord_size_difference = match (
        current.metrics.max_chord_size,
        candidate.metrics.max_chord_size,
    ) {
        (Some(a), Some(b)) => Some(b as i64 - a as i64),
        _ => None,
    };
    let relative_time_quartiles_with_notes = candidate
        .relative_time_quartile_coverage
        .iter()
        .filter(|covered| **covered)
        .count();
    ComparisonDocument {
        schema_version: INVENTORY_SCHEMA_VERSION,
        current: comparison_sheet(current_path, current_bytes, current),
        candidate: comparison_sheet(candidate_path, candidate_bytes, candidate),
        comparison: ComparisonSignals {
            interval_sequence_similarity,
            normalized_timing_note_overlap: note_overlap,
            initial_onset_offset_candidate_minus_current_us: initial_offset_alignment.offset_us,
            initial_offset_aligned_time_key_overlap: initial_offset_alignment.overlap,
            current_events_subset_of_candidate_after_initial_offset: initial_offset_alignment
                .current_subset,
            candidate_events_subset_of_current_after_initial_offset: initial_offset_alignment
                .candidate_subset,
            active_duration_ratio_candidate_over_current: duration_ratio,
            source_pitch_matches,
            aligned_melody_button_offset_semitones,
            chord_onset_ratio_difference,
            max_chord_size_difference,
            composition_section_completeness: "not_measured_human_listening_required".into(),
            relative_time_quartiles_with_notes,
            source_identity_confidence: "high_for_input_bytes_sha256".into(),
            structural_signal_confidence: "screening_only_not_acoustic".into(),
            musical_fidelity_confidence: "not_assessed".into(),
            listening_required: true,
            verdict: "screening_signals_only_no_song_selection".into(),
        },
        limitations: vec![
            "The melody sketch selects the highest physical key at each onset; it is a proxy, not a verified lead voice.".into(),
            "Intervals, timing overlap, chord density, pitch and equal-time quartiles are structural screening signals, not acoustic-fidelity claims.".into(),
            "Quarter coverage does not identify intro, verse, chorus or outro; a person must inspect the same named sections and timestamps.".into(),
            "No official commercial audio is downloaded, embedded or compared by this tool.".into(),
        ],
    }
}

fn comparison_sheet(path: &Path, bytes: &[u8], analysis: SheetAnalysis) -> ComparisonSheet {
    ComparisonSheet {
        input_path: path.to_string_lossy().replace('\\', "/"),
        sha256: sha256_hex(bytes),
        metrics: analysis.metrics,
        normalized_melody_semitones: analysis.melody,
        normalized_interval_sketch: analysis.interval_sketch,
        normalized_onset_positions: analysis.normalized_onset_positions,
        normalized_inter_onset_ratios: analysis.normalized_inter_onset_ratios,
        phrase_break_indices: analysis.phrase_break_indices,
        relative_time_quartile_coverage: analysis.relative_time_quartile_coverage,
    }
}

fn normalized_note_overlap(current: &SheetAnalysis, candidate: &SheetAnalysis) -> Option<f64> {
    if current.physical_notes.is_empty() || candidate.physical_notes.is_empty() {
        return None;
    }
    let current_first = current
        .physical_notes
        .first()
        .map(|pair| pair.0)
        .unwrap_or(0);
    let candidate_first = candidate
        .physical_notes
        .first()
        .map(|pair| pair.0)
        .unwrap_or(0);
    let current_span = current
        .physical_notes
        .last()
        .map(|pair| pair.0.saturating_sub(current_first))
        .unwrap_or(0);
    let candidate_span = candidate
        .physical_notes
        .last()
        .map(|pair| pair.0.saturating_sub(candidate_first))
        .unwrap_or(0);
    let current_positions = current
        .physical_notes
        .iter()
        .map(|(time, button)| {
            (
                relative_position(*time, current_first, current_span),
                *button,
            )
        })
        .collect::<Vec<_>>();
    let candidate_positions = candidate
        .physical_notes
        .iter()
        .map(|(time, button)| {
            (
                relative_position(*time, candidate_first, candidate_span),
                *button,
            )
        })
        .collect::<Vec<_>>();
    let mut matched = BTreeSet::new();
    let mut hits = 0usize;
    for (time, button) in &current_positions {
        if let Some((index, _)) = candidate_positions
            .iter()
            .enumerate()
            .filter(|(index, (candidate_time, candidate_button))| {
                !matched.contains(index)
                    && candidate_button == button
                    && candidate_time.abs_diff(*time) <= 200
            })
            .min_by_key(|(_, (candidate_time, _))| candidate_time.abs_diff(*time))
        {
            matched.insert(index);
            hits += 1;
        }
    }
    let precision = hits as f64 / candidate_positions.len() as f64;
    let recall = hits as f64 / current_positions.len() as f64;
    Some(if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    })
}

fn relative_position(time: u64, first: u64, span: u64) -> u64 {
    if span == 0 {
        0
    } else {
        (((time.saturating_sub(first)) as u128 * 10_000) / span as u128) as u64
    }
}

struct InitialOffsetAlignment {
    offset_us: Option<i64>,
    overlap: Option<f64>,
    current_subset: bool,
    candidate_subset: bool,
}

fn initial_offset_alignment(
    current: &SheetAnalysis,
    candidate: &SheetAnalysis,
) -> InitialOffsetAlignment {
    let first_time =
        |analysis: &SheetAnalysis| analysis.physical_notes.iter().map(|(time, _)| *time).min();
    let Some(current_first) = first_time(current) else {
        return InitialOffsetAlignment {
            offset_us: None,
            overlap: None,
            current_subset: false,
            candidate_subset: false,
        };
    };
    let Some(candidate_first) = first_time(candidate) else {
        return InitialOffsetAlignment {
            offset_us: None,
            overlap: None,
            current_subset: false,
            candidate_subset: false,
        };
    };
    let offset = i64::try_from(i128::from(candidate_first) - i128::from(current_first)).ok();
    let relative_events = |analysis: &SheetAnalysis, first: u64| {
        analysis
            .physical_notes
            .iter()
            .map(|(time, button)| (time.saturating_sub(first), *button))
            .collect::<BTreeSet<_>>()
    };
    let current_events = relative_events(current, current_first);
    let candidate_events = relative_events(candidate, candidate_first);
    let intersection = current_events.intersection(&candidate_events).count();
    let precision = intersection as f64 / candidate_events.len() as f64;
    let recall = intersection as f64 / current_events.len() as f64;
    let overlap = if precision + recall == 0.0 {
        Some(0.0)
    } else {
        Some(2.0 * precision * recall / (precision + recall))
    };
    InitialOffsetAlignment {
        offset_us: offset,
        overlap,
        current_subset: current_events.is_subset(&candidate_events),
        candidate_subset: candidate_events.is_subset(&current_events),
    }
}

fn aligned_pitch_offset(
    current: &SheetAnalysis,
    candidate: &SheetAnalysis,
    similarity: f64,
) -> Option<i16> {
    if similarity < 0.9
        || current.melody.len() < 2
        || current.melody.len() != candidate.melody.len()
    {
        return None;
    }
    let mut offsets = current
        .melody
        .iter()
        .zip(&candidate.melody)
        .map(|(current, candidate)| candidate - current);
    let first = offsets.next()?;
    offsets.all(|offset| offset == first).then_some(first)
}

fn sequence_similarity(left: &[i16], right: &[i16]) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    let mut row = (0..=right.len()).collect::<Vec<usize>>();
    for (left_index, left_value) in left.iter().enumerate() {
        let mut next = vec![left_index + 1; right.len() + 1];
        for (right_index, right_value) in right.iter().enumerate() {
            let substitution = row[right_index] + usize::from(left_value != right_value);
            next[right_index + 1] = substitution
                .min(next[right_index] + 1)
                .min(row[right_index + 1] + 1);
        }
        row = next;
    }
    let distance = row[right.len()];
    let denominator = left.len().max(right.len()).max(1);
    1.0 - distance as f64 / denominator as f64
}

fn inventory_csv(songs: &[InventorySong]) -> String {
    let headers = [
        "id",
        "path",
        "manifest_title",
        "manifest_sha256",
        "source_title_metadata_field",
        "source_author_metadata_field",
        "source_format",
        "source_schema_version",
        "source_game",
        "source_pitch",
        "source_track_count",
        "source_instruments",
        "source_title_metadata",
        "source_author_metadata",
        "parsing_status",
        "raw_note_count",
        "authored_source_event_count",
        "decoded_event_count",
        "authored_distinct_onset_count",
        "unique_physical_time_key_count",
        "physical_collision_group_count",
        "physical_collision_extra_event_count",
        "source_layer_collision_extra_event_count",
        "first_note_us",
        "last_note_us",
        "active_duration_us",
        "active_seconds",
        "onsets_per_second",
        "maximum_rest_us",
        "max_polyphony",
        "physical_onset_count",
        "chord_onset_count",
        "chord_onset_ratio",
        "max_chord_size",
        "average_chord_size",
        "fast_repeat_count",
        "fast_repeat_threshold_us",
        "plan_status",
        "plan_error",
        "plan_loss_count",
        "omitted_source_event_count",
        "target_unplayable_count",
        "source_unpitched_count",
        "source_unresolved_count",
        "multi_track_lossy_in_15_key_planner",
        "physical_risk_severity",
        "physical_risk_reason",
        "impossible_same_key_repeat_count",
        "risky_same_key_repeat_count",
        "output_physical_note_count",
        "output_action_count",
        "schedule_duration_us",
        "authored_sustain_count",
        "decoder_warnings",
        "planner_loss_reasons",
        "schedule_warnings",
    ];
    let mut output = csv_record(headers.iter().map(|value| (*value).to_owned()));
    for song in songs {
        let m = &song.metrics;
        let values = vec![
            song.id.clone(),
            song.path.clone(),
            song.manifest_title.clone(),
            song.manifest_sha256.clone(),
            display_option(m.source_title_metadata_field.as_deref()),
            display_option(m.source_author_metadata_field.as_deref()),
            m.source_format.clone(),
            display_option(m.source_schema_version),
            m.source_game.clone(),
            m.source_pitch.clone(),
            m.source_track_count.to_string(),
            json_cell(&m.source_instruments),
            display_option(m.source_title_metadata.as_deref()),
            display_option(m.source_author_metadata.as_deref()),
            m.parsing_status.clone(),
            m.raw_note_count.to_string(),
            m.authored_source_event_count.to_string(),
            m.decoded_event_count.to_string(),
            m.authored_distinct_onset_count.to_string(),
            m.unique_physical_time_key_count.to_string(),
            m.physical_collision_group_count.to_string(),
            m.physical_collision_extra_event_count.to_string(),
            m.source_layer_collision_extra_event_count.to_string(),
            display_option(m.first_note_us),
            display_option(m.last_note_us),
            display_option(m.active_duration_us),
            display_option(m.active_seconds),
            display_option(m.onsets_per_second),
            display_option(m.maximum_rest_us),
            display_option(m.max_polyphony),
            m.physical_onset_count.to_string(),
            m.chord_onset_count.to_string(),
            display_option(m.chord_onset_ratio),
            display_option(m.max_chord_size),
            display_option(m.average_chord_size),
            m.fast_repeat_count.to_string(),
            m.fast_repeat_threshold_us.to_string(),
            m.plan_status.clone(),
            display_option(m.plan_error.as_deref()),
            m.plan_loss_count.to_string(),
            m.omitted_source_event_count.to_string(),
            m.target_unplayable_count.to_string(),
            m.source_unpitched_count.to_string(),
            m.source_unresolved_count.to_string(),
            display_option(m.multi_track_lossy_in_15_key_planner),
            display_option(m.physical_risk_severity.as_deref()),
            display_option(m.physical_risk_reason.as_deref()),
            display_option(m.impossible_same_key_repeat_count),
            display_option(m.risky_same_key_repeat_count),
            display_option(m.output_physical_note_count),
            display_option(m.output_action_count),
            display_option(m.schedule_duration_us),
            m.authored_sustain_count.to_string(),
            json_cell(&m.decoder_warnings),
            json_cell(&m.planner_loss_reasons),
            json_cell(&m.schedule_warnings),
        ];
        output.push_str(&csv_record(values));
    }
    output
}

fn inventory_summary(document: &InventoryDocument) -> String {
    let mut formats = BTreeMap::<String, usize>::new();
    let mut plan_statuses = BTreeMap::<String, usize>::new();
    let mut under_45 = Vec::new();
    let mut from_45_to_75 = Vec::new();
    let mut low_density = Vec::new();
    let mut long_rests = Vec::new();
    let mut collision_rows = Vec::new();
    let mut parsed_count = 0;
    let mut colliding_file_count = 0;
    let mut collision_extra_count = 0;
    let mut source_layer_collision_extra_count = 0;
    for song in &document.songs {
        *formats
            .entry(song.metrics.source_format.clone())
            .or_default() += 1;
        if song.metrics.parsing_status == "pass" {
            parsed_count += 1;
        }
        *plan_statuses
            .entry(song.metrics.plan_status.clone())
            .or_default() += 1;
        if let Some(duration) = song.metrics.schedule_duration_us {
            if duration < 45_000_000 {
                under_45.push(song.path.as_str());
            } else if duration <= 75_000_000 {
                from_45_to_75.push(song.path.as_str());
            }
        }
        if song
            .metrics
            .onsets_per_second
            .is_some_and(|density| density < 2.0)
        {
            low_density.push(song.path.as_str());
        }
        if song
            .metrics
            .maximum_rest_us
            .is_some_and(|rest| rest >= 5_000_000)
        {
            long_rests.push(song.path.as_str());
        }
        if song.metrics.physical_collision_extra_event_count > 0 {
            colliding_file_count += 1;
            collision_extra_count += song.metrics.physical_collision_extra_event_count;
            source_layer_collision_extra_count +=
                song.metrics.source_layer_collision_extra_event_count;
            collision_rows.push(format!(
                "{}: {} extra physical event(s), {} from same-source layer expansion",
                song.path,
                song.metrics.physical_collision_extra_event_count,
                song.metrics.source_layer_collision_extra_event_count
            ));
        }
    }
    let mut output = format!(
        "# Built-in song inventory summary\n\n- Active manifest entries audited: {}\n- Manifest SHA-256: `{}`\n- Generated with: `cargo xtask song-audit inventory`\n- Runtime/parser policy: existing source decoder, planner and frozen 60 FPS / one-frame / zero-margin policy.\n\n",
        document.active_song_count, document.manifest_sha256
    );
    output.push_str("## Schema counts\n\n| Schema | Songs |\n| --- | ---: |\n");
    for (format, count) in formats {
        output.push_str(&format!("| {format} | {count} |\n"));
    }
    output.push_str("\n## Validation coverage\n\n");
    output.push_str(&format!(
        "- Strict source decoding: {parsed_count}/{} passed.\n",
        document.active_song_count
    ));
    for (status, count) in plan_statuses {
        output.push_str(&format!("- Planner status `{status}`: {count}.\n"));
    }
    output.push_str("\n## Screening flags\n\nThese filters only surface measurements. They do not rate musical quality.\n\n");
    append_paths(&mut output, "Under 45 seconds", &under_45);
    append_paths(&mut output, "45–75 seconds", &from_45_to_75);
    append_paths(
        &mut output,
        "Below 2 authored onsets per active second",
        &low_density,
    );
    append_paths(
        &mut output,
        "Maximum authored rest of at least 5 seconds",
        &long_rests,
    );
    output.push_str(&format!(
        "## Physical-key collision screening\n\n- Files with extra physical time/key events: {colliding_file_count}\n- Extra physical events after deduplication: {collision_extra_count}\n- Of those, extras from same-source layer expansion: {source_layer_collision_extra_count}\n\n"
    ));
    if collision_rows.is_empty() {
        output.push_str("No duplicate physical time/key pairs were measured.\n");
    } else {
        for row in collision_rows {
            output.push_str(&format!("- {row}\n"));
        }
    }
    output.push_str("\nSee `README.md` for metric definitions and limitations.\n");
    output
}

fn append_paths(output: &mut String, title: &str, paths: &[&str]) {
    output.push_str(&format!("### {title} ({})\n\n", paths.len()));
    if paths.is_empty() {
        output.push_str("None.\n\n");
    } else {
        for path in paths {
            output.push_str(&format!("- `{path}`\n"));
        }
        output.push('\n');
    }
}

fn csv_record(values: impl IntoIterator<Item = String>) -> String {
    let mut line = values
        .into_iter()
        .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

fn json_cell<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn display_option<T: ToString>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn format_fields(format: SongFormat) -> (Option<u8>, String) {
    match format {
        SongFormat::LegacyFlat => (None, "legacy_flat".into()),
        SongFormat::Composed { version } => (Some(version), format!("composed_v{version}")),
        SongFormat::Recorded { version } => (Some(version), format!("recorded_v{version}")),
    }
}

fn plan_status_name(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Ready => "ready",
        PlanStatus::RequiresConfirmation => "requires_confirmation",
        PlanStatus::Unsupported => "unsupported",
    }
}

fn audit_policy() -> Result<MaterializedTimingPolicy> {
    MaterializedTimingPolicy::from_user_margin(60, 1.0, 0).map_err(|error| error.into())
}

fn raw_song_object(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes)?;
    match value {
        Value::Array(items) if items.len() == 1 => Ok(items.into_iter().next().expect("one item")),
        Value::Array(_) => Err("sheet must contain exactly one source object".into()),
        object @ Value::Object(_) => Ok(object),
        _ => Err("sheet root must be an object".into()),
    }
}

fn metadata_entry(value: &Value, keys: &[&str]) -> Option<(String, String)> {
    let object = value.as_object()?;
    let data = object.get("data").and_then(Value::as_object);
    let metadata = object.get("metadata").and_then(Value::as_object);
    keys.iter()
        .filter_map(|key| {
            object
                .get(*key)
                .or_else(|| data.and_then(|values| values.get(*key)))
                .or_else(|| metadata.and_then(|values| values.get(*key)))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).to_owned(), value.to_owned()))
        })
        .next()
}

fn pitch_offset(pitch: &str) -> i16 {
    match pitch {
        "Db" => 1,
        "D" => 2,
        "Eb" => 3,
        "E" => 4,
        "F" => 5,
        "Gb" => 6,
        "G" => 7,
        "Ab" => 8,
        "A" => 9,
        "Bb" => 10,
        "B" => 11,
        _ => 0,
    }
}

fn parse_options(args: &[String]) -> Result<BTreeMap<String, String>> {
    let mut options = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let key = args[index].strip_prefix("--").ok_or_else(|| {
            format!(
                "expected an option beginning with --, got `{}`",
                args[index]
            )
        })?;
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{key}"))?
            .clone();
        index += 1;
        if options.insert(key.to_owned(), value).is_some() {
            return Err(format!("option --{key} was specified more than once").into());
        }
    }
    Ok(options)
}

fn required_path(options: &BTreeMap<String, String>, key: &str) -> Result<PathBuf> {
    options
        .get(key)
        .map(PathBuf::from)
        .ok_or_else(|| format!("song-audit compare requires --{key} <path>").into())
}

fn option_path(options: &BTreeMap<String, String>, key: &str) -> Option<PathBuf> {
    options.get(key).map(PathBuf::from)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("song")
        .to_owned()
}

fn json_with_newline(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_collisions_and_zero_duration_are_reported_without_dropping_source_events() {
        let bytes = br#"{"name":"Zero","author":"Arranger","songNotes":[{"time":0,"key":"Key0","l":3},{"time":0,"key":"Key0"}]}"#;
        let analysis = analyze_sheet(bytes, "zero").expect("valid flat sheet");
        assert_eq!(analysis.metrics.raw_note_count, 2);
        assert_eq!(analysis.metrics.decoded_event_count, 3);
        assert_eq!(analysis.metrics.authored_distinct_onset_count, 1);
        assert_eq!(analysis.metrics.unique_physical_time_key_count, 1);
        assert_eq!(analysis.metrics.physical_collision_extra_event_count, 2);
        assert_eq!(analysis.metrics.source_layer_collision_extra_event_count, 1);
        assert_eq!(analysis.metrics.active_duration_us, Some(0));
        assert_eq!(analysis.metrics.onsets_per_second, None);
        assert_eq!(analysis.metrics.maximum_rest_us, None);
        assert_eq!(
            analysis.metrics.source_author_metadata.as_deref(),
            Some("Arranger")
        );
        assert_eq!(
            analysis.metrics.source_author_metadata_field.as_deref(),
            Some("author")
        );
        assert_eq!(analysis.normalized_onset_positions, [0]);
        assert!(analysis.normalized_inter_onset_ratios.is_empty());
    }

    #[test]
    fn multi_track_planner_metrics_keep_authored_and_physical_counts_separate() {
        let value = serde_json::json!({
            "type": "composed", "version": 4, "bpm": 120, "pitch": "C",
            "data": {"appName": "Sky"}, "columnTempos": [0, 0],
            "tracks": [
                {"instrument": {"name": "Piano"}, "notes": [[0, 60], [1, 62]]},
                {"instrument": {"name": "Piano"}, "notes": [[0, 60], [1, 72]]}
            ]
        });
        let bytes = serde_json::to_vec(&value).unwrap();
        let analysis = analyze_sheet(&bytes, "tracks").expect("valid multi-track sheet");
        assert_eq!(analysis.metrics.source_track_count, 2);
        assert_eq!(analysis.metrics.raw_note_count, 4);
        assert_eq!(analysis.metrics.decoded_event_count, 4);
        assert!(analysis.metrics.unique_physical_time_key_count <= 4);
        assert_eq!(
            analysis.metrics.multi_track_lossy_in_15_key_planner,
            Some(false)
        );
        assert!(analysis.metrics.output_action_count.unwrap() > 0);
    }

    #[test]
    fn negative_timestamps_fail_strict_audit_parsing() {
        let bytes = br#"{"songNotes":[{"time":-1,"key":"Key0"}]}"#;
        assert!(analyze_sheet(bytes, "negative").is_err());
    }

    #[test]
    fn sequence_metrics_and_serialized_comparisons_are_deterministic() {
        assert_eq!(sequence_similarity(&[2, -2, 1], &[2, -2, 1]), 1.0);
        assert!((sequence_similarity(&[2, -2, 1], &[5, -2, 1]) - 2.0 / 3.0).abs() < 1e-12);
        let bytes = br#"{"name":"Stable","songNotes":[{"time":100,"key":"Key0"},{"time":200,"key":"Key1"}]}"#;
        let current = analyze_sheet(bytes, "stable").unwrap();
        let candidate = analyze_sheet(bytes, "stable").unwrap();
        let first = comparison_document(
            Path::new("current.json"),
            bytes,
            current,
            Path::new("candidate.json"),
            bytes,
            candidate,
        );
        let current = analyze_sheet(bytes, "stable").unwrap();
        let candidate = analyze_sheet(bytes, "stable").unwrap();
        let second = comparison_document(
            Path::new("current.json"),
            bytes,
            current,
            Path::new("candidate.json"),
            bytes,
            candidate,
        );
        assert_eq!(
            json_with_newline(&first).unwrap(),
            json_with_newline(&second).unwrap()
        );
        assert!(
            first
                .comparison
                .current_events_subset_of_candidate_after_initial_offset
        );
    }

    #[test]
    fn inventory_paths_sort_case_folded_then_by_original_spelling() {
        let mut paths = vec!["sheets/b.json", "sheets/a.json", "sheets/A.json"];
        paths.sort_by(|left, right| compare_catalog_paths(left, right));
        assert_eq!(paths, ["sheets/A.json", "sheets/a.json", "sheets/b.json"]);
    }
}

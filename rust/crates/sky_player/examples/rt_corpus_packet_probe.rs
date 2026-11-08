//! Offline packet occupancy of verified built-in songs, using the production
//! song planner, runtime compiler and test-support prepared-stream builder.
//! This executable never invokes SendInput or starts a playback worker.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sky_app_core::catalog::BuiltinCatalogManifest;
use sky_app_core::song::{
    ActionKind as SongActionKind, KeyAction, PianoPlannerOptions, PlanStatus, SKY_SCAN_CODES,
    ScheduleMetadata, SongPlan, build_song_plan,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_player::engine::dispatch_primitives::ProductionDispatchTestHarness;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA_VERSION: u32 = 1;
const QPC_FREQUENCY_HZ: u64 = 1_000_000;
const PHYSICAL_PACKET_SLOTS: usize = sky_dispatch_win32::input::MAX_PACKET_EVENTS;

fn main() {
    if let Err(message) = run() {
        eprintln!("rt_corpus_packet_probe: {message}");
        std::process::exit(1);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn root_from_manifest_dir() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../")
        .canonicalize()
        .map_err(|error| format!("repository root: {error}"))
}

fn verified_source_path(root: &Path, source: &str) -> Result<PathBuf, String> {
    let filename = source
        .strip_prefix("sheets/")
        .ok_or_else(|| format!("unexpected manifest path prefix: {source}"))?;
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err(format!("manifest source is not a top-level song: {source}"));
    }
    Ok(root.join("songs").join(filename))
}

fn runtime_actions(actions: &[KeyAction]) -> Result<Vec<KeyActionInput>, String> {
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            Ok(KeyActionInput {
                source_action_index: u32::try_from(index)
                    .map_err(|_| "authored source index exceeds u32".to_string())?,
                kind: match action.kind {
                    SongActionKind::Down => ActionKind::Down,
                    SongActionKind::Up => ActionKind::Up,
                },
                scheduled_us: action.at_us,
                scan_codes: action.scan_codes.iter().copied().collect(),
                reason: action.reason.as_str().into(),
            })
        })
        .collect()
}

fn rejected(outcome: &str, detail: String, plan_status: &str) -> Value {
    json!({
        "outcome":outcome,
        "plan_status":plan_status,
        "reason":detail,
        "packet_histogram":null
    })
}

fn characterize(bytes: &[u8], fallback: &str, policy: &MaterializedTimingPolicy) -> Value {
    let plan = match build_song_plan(bytes, fallback, &PianoPlannerOptions::default(), policy) {
        Ok(plan) => plan,
        Err(error) => return rejected("planner_rejected", error.to_string(), "error"),
    };
    let (plan_status, schedule) = match plan {
        SongPlan::LegacyCompatibility(legacy) => ("ready_legacy_compatibility", legacy.schedule),
        SongPlan::PianoConversion(piano) => {
            let status = match piano.status {
                PlanStatus::Ready => "ready",
                PlanStatus::RequiresConfirmation => "requires_confirmation",
                PlanStatus::Unsupported => "unsupported",
            };
            if piano.status != PlanStatus::Ready {
                return rejected(
                    if piano.status == PlanStatus::Unsupported {
                        "unsupported"
                    } else {
                        "requires_confirmation"
                    },
                    "conversion plan is not automatically authorized for dispatch".into(),
                    status,
                );
            }
            let Some(schedule) = piano.schedule else {
                return rejected(
                    "planner_rejected",
                    "ready plan has no schedule".into(),
                    status,
                );
            };
            (status, schedule)
        }
    };
    characterize_schedule(&schedule, plan_status, policy)
}

fn characterize_schedule(
    schedule: &ScheduleMetadata,
    plan_status: &str,
    policy: &MaterializedTimingPolicy,
) -> Value {
    let actions = match runtime_actions(&schedule.actions) {
        Ok(actions) => actions,
        Err(error) => return rejected("action_mapping_rejected", error, plan_status),
    };
    let compiled = match compile_runtime_intents(&actions, &SKY_SCAN_CODES) {
        Ok(compiled) => compiled,
        Err(error) => return rejected("compiler_rejected", error.to_string(), plan_status),
    };
    let compiled_packet_count = compiled.packets.len();
    let compiled_mixed_boundaries = compiled
        .packets
        .iter()
        .filter(|packet| packet.up_mask != 0 && packet.down_mask != 0)
        .count();

    // The same test-support adapter used by the startup probe: production
    // coordinator simulation and actual prebuilt Win32 INPUT packet layout.
    // No send, foreground/window handle, real key events or RT worker.
    let builder = match ProductionDispatchTestHarness::new_prepared_startup_builder_for_test(
        compiled,
        QPC_FREQUENCY_HZ,
        policy.min_hold_us,
    ) {
        Ok(builder) => builder,
        Err(error) => return rejected("coordinator_rejected", error, plan_status),
    };
    #[cfg(windows)]
    let (prepared, compact, copy_ns, verify_ns) =
        match builder.prepare_with_compact_arena_measurement() {
            Ok(evidence) => evidence,
            Err(error) => return rejected("preparation_rejected", error, plan_status),
        };
    #[cfg(not(windows))]
    let prepared = match builder.prepare() {
        Ok(prepared) => prepared,
        Err(error) => return rejected("preparation_rejected", error, plan_status),
    };
    let histogram = prepared.physical_input_event_histogram;
    let histogram_count = histogram.iter().sum::<usize>();
    if histogram_count != prepared.physical_entry_count || histogram[0] != 0 {
        return rejected(
            "invariant_rejected",
            format!(
                "histogram count {histogram_count}, physical count {}, zero bin {}",
                prepared.physical_entry_count, histogram[0]
            ),
            plan_status,
        );
    }
    let input_events = histogram
        .iter()
        .enumerate()
        .map(|(events, count)| events * count)
        .sum::<usize>();
    let possible_slots = prepared.physical_entry_count * PHYSICAL_PACKET_SLOTS;
    if input_events > possible_slots {
        return rejected(
            "invariant_rejected",
            "INPUT events exceed packet slots".into(),
            plan_status,
        );
    }
    let mut report = json!({
        "outcome":"prepared",
        "plan_status":plan_status,
        "authored_action_count":actions.len(),
        "compiler_packet_count":compiled_packet_count,
        "compiler_mixed_boundary_count":compiled_mixed_boundaries,
        "prepared_entry_count":prepared.prepared_entry_count,
        "physical_packet_count":prepared.physical_entry_count,
        "metadata_entry_count":prepared.metadata_entry_count,
        "packet_histogram":histogram,
        "input_events":input_events,
        "possible_slots":possible_slots,
        "slot_utilization_percent":if possible_slots == 0 {
            0.0
        } else {
            100.0 * input_events as f64 / possible_slots as f64
        },
        "entry_size_bytes":prepared.prepared_entry_type_size_bytes,
        "reserved_entry_bytes":prepared.prepared_entry_allocation.1
    });
    #[cfg(windows)]
    {
        report["compact_arena_study"] = json!({
            "packet_count": compact.packet_count,
            "input_event_count": compact.event_count,
            "inline_packet_component_bytes": compact.inline_packet_component_bytes,
            "compact_arena_reserved_bytes": compact.total_arena_reserved_bytes,
            "descriptor_size_bytes": compact.descriptor_size,
            "input_size_bytes": compact.input_size,
            "descriptor_reserved_bytes": compact.descriptor_reserved_bytes,
            "input_reserved_bytes": compact.input_reserved_bytes,
            "offline_copy_build_ns": copy_ns,
            "offline_structural_verify_ns": verify_ns,
            "scope": "isolated_packet_component_copy_NOT_RSS_or_production_startup"
        });
    }
    report
}

fn git_head(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("git HEAD: {error}"))?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("invalid git revision: {error}"))
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let output = match (args.next(), args.next(), args.next()) {
        (Some(option), Some(path), None) if option == "--output" => PathBuf::from(path),
        _ => {
            return Err(
                "usage: rt_corpus_packet_probe --output <TEMP/real-song-packets.json>".into(),
            );
        }
    };
    let root = root_from_manifest_dir()?;
    let manifest_bytes = fs::read(root.join("builtin-songs/manifest.json"))
        .map_err(|error| format!("read manifest: {error}"))?;
    let manifest: BuiltinCatalogManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("decode manifest: {error}"))?;
    manifest
        .validate()
        .map_err(|error| format!("invalid manifest: {error}"))?;
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
        .map_err(|error| format!("frozen policy: {error}"))?;

    let mut songs = Vec::with_capacity(manifest.songs.len());
    let mut counts = BTreeMap::<String, usize>::new();
    let mut aggregate_histogram = [0u64; PHYSICAL_PACKET_SLOTS + 1];
    let mut prepared_input_events = 0u64;
    let mut prepared_physical_packets = 0u64;
    #[cfg(windows)]
    let mut inline_packet_component_bytes = 0u64;
    #[cfg(windows)]
    let mut compact_arena_reserved_bytes = 0u64;
    #[cfg(windows)]
    let mut total_offline_copy_ns = 0u64;
    #[cfg(windows)]
    let mut total_offline_verify_ns = 0u64;

    for song in &manifest.songs {
        let path = verified_source_path(&root, &song.path)?;
        let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", song.path))?;
        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != song.sha256 {
            return Err(format!("manifest SHA-256 mismatch for {}", song.path));
        }
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&song.title);
        let evidence = characterize(&bytes, fallback, &policy);
        let outcome = evidence["outcome"].as_str().unwrap_or("invariant_rejected");
        *counts.entry(outcome.to_string()).or_default() += 1;
        if outcome == "prepared" {
            let histogram = evidence["packet_histogram"]
                .as_array()
                .ok_or_else(|| format!("prepared histogram missing for {}", song.path))?;
            if histogram.len() != aggregate_histogram.len() {
                return Err(format!("invalid histogram length for {}", song.path));
            }
            for (bin, count) in aggregate_histogram.iter_mut().zip(histogram) {
                *bin += count
                    .as_u64()
                    .ok_or_else(|| "invalid histogram count".to_string())?;
            }
            prepared_input_events += evidence["input_events"]
                .as_u64()
                .ok_or_else(|| "missing input event count".to_string())?;
            prepared_physical_packets += evidence["physical_packet_count"]
                .as_u64()
                .ok_or_else(|| "missing physical packet count".to_string())?;
            #[cfg(windows)]
            {
                let arena = &evidence["compact_arena_study"];
                let read = |field: &str| -> Result<u64, String> {
                    arena[field]
                        .as_u64()
                        .ok_or_else(|| format!("missing arena {field} for {}", song.path))
                };
                if read("packet_count")?
                    != evidence["physical_packet_count"].as_u64().unwrap_or(u64::MAX)
                    || read("input_event_count")?
                        != evidence["input_events"].as_u64().unwrap_or(u64::MAX)
                {
                    return Err(format!("arena/stream counters disagree for {}", song.path));
                }
                inline_packet_component_bytes += read("inline_packet_component_bytes")?;
                compact_arena_reserved_bytes += read("compact_arena_reserved_bytes")?;
                total_offline_copy_ns += read("offline_copy_build_ns")?;
                total_offline_verify_ns += read("offline_structural_verify_ns")?;
            }
        }
        songs.push(json!({
            "id":song.id,
            "path":song.path,
            "manifest_title":song.title,
            "sha256":actual_sha256,
            "evidence":evidence
        }));
    }
    let accounted = counts.values().sum::<usize>();
    if accounted != manifest.songs.len() {
        return Err("some manifest songs were silently omitted".into());
    }
    let slots = prepared_physical_packets * PHYSICAL_PACKET_SLOTS as u64;
    let mut report = json!({
        "schema_version":SCHEMA_VERSION,
        "status":"OFFLINE_CORPUS_CHARACTERIZATION",
        "source_revision":git_head(&root)?,
        "manifest_sha256":sha256_hex(&manifest_bytes),
        "manifest_song_count":manifest.songs.len(),
        "accounted_song_count":accounted,
        "outcome_counts":counts,
        "timing_policy":{"fps":policy.fps,"hold_frames":policy.hold_frames,
            "timing_margin_us":policy.timing_margin_us,"min_hold_us":policy.min_hold_us},
        "packet_capacity":PHYSICAL_PACKET_SLOTS,
        "prepared_physical_packet_count":prepared_physical_packets,
        "prepared_input_event_count":prepared_input_events,
        "prepared_possible_slots":slots,
        "prepared_slot_utilization_percent":if slots == 0 {
            0.0
        } else {
            100.0 * prepared_input_events as f64 / slots as f64
        },
        "prepared_packet_histogram":aggregate_histogram,
        "songs":songs,
        "limitations":"Offline built-in corpus, only successfully prepared songs aggregated. Source/planner/compiler/preparation exclusions are individually reported. No SendInput, receiver, CPU, RSS or latency measurements."
    });
    #[cfg(windows)]
    {
        report["compact_arena_component_study"] = json!({
            "inline_packet_component_bytes": inline_packet_component_bytes,
            "compact_arena_reserved_bytes": compact_arena_reserved_bytes,
            "total_offline_copy_ns_unpaired": total_offline_copy_ns,
            "total_offline_structural_verify_ns_unpaired": total_offline_verify_ns,
            "warning": "Offline extra copy of materialized packets, not a production replacement; cannot claim RSS, startup speedup or realtime dispatch improvement."
        });
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create output directory: {error}"))?;
    }
    let mut text =
        serde_json::to_string_pretty(&report).map_err(|error| format!("encode report: {error}"))?;
    text.push('\n');
    fs::write(&output, text).map_err(|error| format!("write output: {error}"))?;
    println!(
        "Corpus packet evidence: manifest={} accounted={} prepared={} packets={} input_events={} output={}",
        manifest.songs.len(),
        accounted,
        counts.get("prepared").copied().unwrap_or(0),
        prepared_physical_packets,
        prepared_input_events,
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_mapping_is_top_level_only() {
        let root = Path::new("corpus");
        assert_eq!(
            verified_source_path(root, "sheets/piano.json").unwrap(),
            root.join("songs/piano.json")
        );
        for source in [
            "../piano.json",
            "sheets/../piano.json",
            "sheets/folder/x.json",
            "songs/x.json",
        ] {
            assert!(verified_source_path(root, source).is_err(), "{source}");
        }
    }

    #[test]
    fn native_action_mapping_retains_canonical_mixed_boundary() {
        let actions = vec![
            KeyAction {
                at_us: 0,
                kind: SongActionKind::Down,
                scan_codes: vec![0x15],
                reason: "onset".into(),
            },
            KeyAction {
                at_us: 20_000,
                kind: SongActionKind::Up,
                scan_codes: vec![0x15],
                reason: "release".into(),
            },
            KeyAction {
                at_us: 20_000,
                kind: SongActionKind::Down,
                scan_codes: vec![0x16],
                reason: "onset".into(),
            },
            KeyAction {
                at_us: 40_000,
                kind: SongActionKind::Up,
                scan_codes: vec![0x16],
                reason: "release".into(),
            },
        ];
        let inputs = runtime_actions(&actions).expect("native action mapping");
        let compiled =
            compile_runtime_intents(&inputs, &SKY_SCAN_CODES).expect("valid mixed schedule");
        assert_eq!(compiled.packets.len(), 3);
        assert_eq!(compiled.packets[1].up_mask, 1);
        assert_eq!(compiled.packets[1].down_mask, 2);
    }
}

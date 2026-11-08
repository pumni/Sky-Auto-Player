//! Windows-only, offline, mutually-exclusive prepared-storage process trial.
//! No playback worker, keyboard injection, SendInput, window focus or sender.
//! One process owns ONE inline, legacy compact or mask-reserved compact stream.
//! The third variant is an offline-only capacity experiment for issue #640.

#[cfg(not(windows))]
fn main() {
    eprintln!("rt_prepared_process_probe only supports Windows");
    std::process::exit(2);
}

#[cfg(windows)]
mod windows_trial {
    use serde::Serialize;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use sky_app_core::catalog::BuiltinCatalogManifest;
    use sky_app_core::song::{
        ActionKind as SongActionKind, KeyAction, PianoPlannerOptions, PlanStatus, SKY_SCAN_CODES,
        SongPlan, build_song_plan,
    };
    use sky_app_core::timing::MaterializedTimingPolicy;
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput, RuntimeSchedule};
    use sky_player::engine::dispatch_primitives::{
        HeldPreparedStudyEvidence, ProductionDispatchTestHarness,
    };
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::collections::BTreeMap;
    use std::fs;
    use std::mem::size_of;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    struct CountedSystemAllocator;

    static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    static ALLOC_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
    static DEALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    // Bookkeeping of requested *layout* bytes freed, not actual OS heap
    // decommit or process private commit. Count old realloc layouts only on
    // successful realloc, and dealloc layouts at their normal release point.
    static RELEASED_LAYOUT_BYTES: AtomicU64 = AtomicU64::new(0);

    // SAFETY: The system allocator preserves all GlobalAlloc contracts.
    // Atomics only record successful Rust allocator calls and never allocate.
    unsafe impl GlobalAlloc for CountedSystemAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
                ALLOC_REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
                ALLOC_REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let next = unsafe { System.realloc(ptr, layout, new_size) };
            if !next.is_null() {
                ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
                ALLOC_REQUESTED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
                RELEASED_LAYOUT_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            }
            next
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            DEALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            RELEASED_LAYOUT_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            unsafe { System.dealloc(ptr, layout) };
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountedSystemAllocator = CountedSystemAllocator;

    #[derive(Clone, Copy, Debug, Serialize)]
    struct AllocatorSnapshot {
        alloc_or_realloc_calls: u64,
        requested_bytes: u64,
        dealloc_calls: u64,
        released_layout_bytes: u64,
    }

    fn allocator_snapshot() -> AllocatorSnapshot {
        AllocatorSnapshot {
            alloc_or_realloc_calls: ALLOC_CALLS.load(Ordering::Relaxed),
            requested_bytes: ALLOC_REQUESTED_BYTES.load(Ordering::Relaxed),
            dealloc_calls: DEALLOC_CALLS.load(Ordering::Relaxed),
            released_layout_bytes: RELEASED_LAYOUT_BYTES.load(Ordering::Relaxed),
        }
    }

    fn rust_net_requested_layout_delta(
        before: AllocatorSnapshot,
        after: AllocatorSnapshot,
    ) -> i128 {
        (i128::from(after.requested_bytes) - i128::from(before.requested_bytes))
            - (i128::from(after.released_layout_bytes) - i128::from(before.released_layout_bytes))
    }

    // Keep the raw Win32 snapshots allocation-free until ALL phase samples
    // have been captured. Constructing serde_json::Value here would change
    // Rust heap counters before the following stage was observed.
    #[derive(Clone, Copy, Debug, Serialize)]
    struct ProcessMemorySnapshot {
        working_set_bytes: usize,
        peak_working_set_bytes_since_process_start: usize,
        private_commit_bytes: usize,
        peak_private_commit_bytes_since_process_start: usize,
        page_fault_count_since_process_start: u32,
    }

    fn memory_snapshot() -> Result<ProcessMemorySnapshot, String> {
        // SAFETY: zeroed is valid for a plain Win32 output struct; cb is
        // initialized to its complete extended size before the OS call.
        let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        let cb = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>())
            .map_err(|_| "Win32 memory counters size overflow".to_string())?;
        memory.cb = cb;
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle. The cast
        // points to the common initial PROCESS_MEMORY_COUNTERS prefix; cb
        // supplies the larger EX size, which Windows explicitly supports.
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
                cb,
            )
        };
        if ok == 0 {
            return Err(format!(
                "GetProcessMemoryInfo failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(ProcessMemorySnapshot {
            working_set_bytes: memory.WorkingSetSize,
            peak_working_set_bytes_since_process_start: memory.PeakWorkingSetSize,
            private_commit_bytes: memory.PrivateUsage,
            peak_private_commit_bytes_since_process_start: memory.PeakPagefileUsage,
            page_fault_count_since_process_start: memory.PageFaultCount,
        })
    }

    struct Options {
        variant: String,
        song_id: Option<String>,
        synthetic_generations: Option<usize>,
        width: usize,
        warmup: bool,
        output: PathBuf,
    }

    fn parse_options() -> Result<Options, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        if !raw.len().is_multiple_of(2) {
            return Err("expected --name value pairs".into());
        }
        let mut values = BTreeMap::new();
        for pair in raw.as_chunks::<2>().0 {
            let name = pair[0].as_str();
            if !matches!(
                name,
                "--variant"
                    | "--song-id"
                    | "--synthetic-generations"
                    | "--width"
                    | "--warmup"
                    | "--output"
            ) || values.insert(name, pair[1].clone()).is_some()
            {
                return Err(format!("unknown or repeated flag: {name}"));
            }
        }
        let variant = values
            .get("--variant")
            .ok_or_else(|| "missing --variant".to_string())?
            .clone();
        if !matches!(variant.as_str(), "inline" | "compact" | "compact-masks") {
            return Err("--variant must be inline, compact or compact-masks".into());
        }
        let song_id = values.get("--song-id").cloned();
        let synthetic_generations = values
            .get("--synthetic-generations")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "bad generations".to_string())
            })
            .transpose()?;
        if song_id.is_some() == synthetic_generations.is_some() {
            return Err("select exactly one of --song-id or --synthetic-generations".into());
        }
        if synthetic_generations.is_some_and(|count| ![100, 1_000, 10_000].contains(&count)) {
            return Err("synthetic generations must be 100, 1000 or 10000".into());
        }
        let width = values.get("--width").map_or(Ok(1usize), |value| {
            value.parse::<usize>().map_err(|_| "bad width".to_string())
        })?;
        if !(1..=15).contains(&width) || (song_id.is_some() && values.contains_key("--width")) {
            return Err("width 1..15 is supported for synthetic workloads only".into());
        }
        let warmup = match values.get("--warmup").map(String::as_str).unwrap_or("0") {
            "0" => false,
            "1" => true,
            _ => return Err("--warmup must be 0 or 1".into()),
        };
        let output = values
            .get("--output")
            .ok_or_else(|| "missing --output".to_string())
            .map(PathBuf::from)?;
        if !output.is_absolute() || output.extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            return Err("--output must be an absolute .json path in TEMP".into());
        }
        let temp = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| format!("TEMP canonical path: {error}"))?;
        let parent = output
            .parent()
            .ok_or_else(|| "missing output parent".to_string())?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|error| format!("output parent must already exist: {error}"))?;
        if !canonical_parent.starts_with(&temp) {
            return Err("--output must remain under canonical TEMP directory".into());
        }
        Ok(Options {
            variant,
            song_id,
            synthetic_generations,
            width,
            warmup,
            output,
        })
    }

    fn root_from_manifest_dir() -> Result<PathBuf, String> {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../")
            .canonicalize()
            .map_err(|error| format!("repository root: {error}"))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn git_head(root: &Path) -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|error| format!("git rev-parse: {error}"))?;
        if !output.status.success() {
            return Err("git rev-parse HEAD failed".into());
        }
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_string())
            .map_err(|error| format!("invalid git SHA: {error}"))
    }

    fn map_actions(actions: &[KeyAction]) -> Result<Vec<KeyActionInput>, String> {
        actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                Ok(KeyActionInput {
                    source_action_index: u32::try_from(index)
                        .map_err(|_| "source action index exceeds u32".to_string())?,
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

    fn song_schedule(
        root: &Path,
        manifest: &BuiltinCatalogManifest,
        id: &str,
        policy: &MaterializedTimingPolicy,
    ) -> Result<(RuntimeSchedule, String, Value), String> {
        let song = manifest
            .songs
            .iter()
            .find(|song| song.id == id)
            .ok_or_else(|| "song ID not present in verified manifest".to_string())?;
        let filename = song
            .path
            .strip_prefix("sheets/")
            .ok_or_else(|| "song manifest path is not in sheets/".to_string())?;
        if filename.is_empty()
            || filename == "."
            || filename == ".."
            || filename.contains('/')
            || filename.contains('\\')
        {
            return Err("song manifest path is not a top-level file".into());
        }
        let path = root.join("songs").join(filename);
        let bytes = fs::read(&path).map_err(|error| format!("song read: {error}"))?;
        let hash = sha256_hex(&bytes);
        if hash != song.sha256 {
            return Err("song SHA-256 does not match verified manifest".into());
        }
        let fallback = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(&song.title);
        let plan = build_song_plan(&bytes, fallback, &PianoPlannerOptions::default(), policy)
            .map_err(|error| format!("song planner: {error}"))?;
        let schedule = match plan {
            SongPlan::LegacyCompatibility(legacy) => legacy.schedule,
            SongPlan::PianoConversion(piano) if piano.status == PlanStatus::Ready => piano
                .schedule
                .ok_or_else(|| "ready song plan lacks a schedule".to_string())?,
            SongPlan::PianoConversion(_) => {
                return Err("unsupported/unconfirmed song cannot be benchmarked".into());
            }
        };
        let actions = map_actions(&schedule.actions)?;
        let compiled = compile_runtime_intents(&actions, &SKY_SCAN_CODES)
            .map_err(|error| format!("song compiler: {error}"))?;
        Ok((
            compiled,
            format!("song:{id}"),
            json!({"song_sha256":hash,"path":song.path}),
        ))
    }

    // Each Down generation must admit its Up before the next Down.
    // The 60 FPS frozen policy requires >=16,667 us physical hold;
    // the original 4,000/1,000 us synthetic times were not preparable.
    // Leave a second full frame of slack after the authored Up.
    const SYNTHETIC_GENERATION_STEP_US: u64 = 60_000;
    const SYNTHETIC_UP_OFFSET_US: u64 = 25_000;

    fn synthetic_schedule(
        generations: usize,
        width: usize,
    ) -> Result<(RuntimeSchedule, String, Value), String> {
        let mut actions = Vec::new();
        actions
            .try_reserve_exact(generations * 2)
            .map_err(|error| format!("reserve synthetic actions: {error}"))?;
        for index in 0..generations {
            let start = (index as u64)
                .checked_mul(SYNTHETIC_GENERATION_STEP_US)
                .ok_or_else(|| "synthetic timestamp overflow".to_string())?;
            let up_at = start
                .checked_add(SYNTHETIC_UP_OFFSET_US)
                .ok_or_else(|| "synthetic release timestamp overflow".to_string())?;
            for (offset, kind, at) in [
                (0usize, ActionKind::Down, start),
                (1usize, ActionKind::Up, up_at),
            ] {
                let action_index = index
                    .checked_mul(2)
                    .and_then(|number| number.checked_add(offset))
                    .ok_or_else(|| "synthetic action index overflow".to_string())?;
                actions.push(KeyActionInput {
                    source_action_index: u32::try_from(action_index)
                        .map_err(|_| "synthetic action index >u32".to_string())?,
                    kind,
                    scheduled_us: at,
                    scan_codes: SKY_SCAN_CODES[..width].iter().copied().collect(),
                    reason: "offline-synthetic".into(),
                });
            }
        }
        let compiled = compile_runtime_intents(&actions, &SKY_SCAN_CODES)
            .map_err(|error| format!("synthetic compiler: {error}"))?;
        Ok((
            compiled,
            format!("synthetic:{generations}:width:{width}"),
            json!({
                "generations":generations,
                "width":width,
                "generation_step_us":SYNTHETIC_GENERATION_STEP_US,
                "up_offset_us":SYNTHETIC_UP_OFFSET_US
            }),
        ))
    }

    pub fn run() -> Result<(), String> {
        let opts = parse_options()?;
        let root = root_from_manifest_dir()?;
        let revision = git_head(&root)?;
        let manifest_bytes = fs::read(root.join("builtin-songs/manifest.json"))
            .map_err(|error| format!("manifest read: {error}"))?;
        let manifest: BuiltinCatalogManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("manifest decode: {error}"))?;
        manifest
            .validate()
            .map_err(|error| format!("manifest validate: {error}"))?;
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
            .map_err(|error| format!("timing policy: {error}"))?;
        let (compiled, workload, identity) = match opts.song_id.as_deref() {
            Some(id) => song_schedule(&root, &manifest, id, &policy)?,
            None => synthetic_schedule(
                opts.synthetic_generations
                    .expect("validated generation count"),
                opts.width,
            )?,
        };
        let builder = ProductionDispatchTestHarness::new_prepared_startup_builder_for_test(
            compiled,
            1_000_000,
            policy.min_hold_us,
        )?;
        if opts.warmup {
            let warm_builder = builder.clone_coordinator_for_test();
            match opts.variant.as_str() {
                "compact" => {
                    warm_builder.prepare_compact_direct_measurement()?;
                }
                "compact-masks" => {
                    warm_builder.with_held_prepared_owner_for_study_with_input_hint(
                        true,
                        true,
                        |_| Ok(()),
                    )?;
                }
                _ => {
                    warm_builder.prepare()?;
                }
            }
        }
        let before_memory = memory_snapshot()?;
        let before_allocator = allocator_snapshot();

        // The callback runs with the chosen stream and reset coordinator still
        // live. Its allocation counters are sampled BEFORE JSON serialization.
        let (held, measured_memory, after_allocator) = builder
            .with_held_prepared_owner_for_study_with_input_hint(
                opts.variant != "inline",
                opts.variant == "compact-masks",
                |held| {
                    let after_allocator = allocator_snapshot();
                    let measured_memory = memory_snapshot()?;
                    Ok((held, measured_memory, after_allocator))
                },
            )?;
        // with_held_prepared_owner_for_study_with_input_hint returns only
        // AFTER the prepared stream and reset coordinator have been dropped.
        // Do not build any JSON or format a report before this snapshot.
        let after_drop_memory = memory_snapshot()?;
        let after_drop_allocator = allocator_snapshot();

        let counts = [
            after_allocator
                .alloc_or_realloc_calls
                .saturating_sub(before_allocator.alloc_or_realloc_calls),
            after_allocator
                .requested_bytes
                .saturating_sub(before_allocator.requested_bytes),
            after_allocator
                .dealloc_calls
                .saturating_sub(before_allocator.dealloc_calls),
        ];
        let report = trial_report(
            &revision,
            &manifest_sha256,
            &opts,
            &workload,
            &identity,
            held,
            before_memory,
            measured_memory,
            after_drop_memory,
            before_allocator,
            after_allocator,
            after_drop_allocator,
            counts,
        );
        let mut output = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("serialize trial: {error}"))?;
        output.push('\n');
        // The script creates the TEMP evidence directory. Reject accidental
        // tracked-file outputs and do not create any new repository files.
        fs::write(&opts.output, output)
            .map_err(|error| format!("write trial evidence: {error}"))?;
        println!(
            "OFFLINE_PREPARED_AB variant={} workload={} warmup={} output={}",
            opts.variant,
            workload,
            opts.warmup,
            opts.output.display()
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn trial_report(
        revision: &str,
        manifest_hash: &str,
        opts: &Options,
        workload: &str,
        identity: &Value,
        held: HeldPreparedStudyEvidence,
        before: ProcessMemorySnapshot,
        while_held: ProcessMemorySnapshot,
        after_drop: ProcessMemorySnapshot,
        allocator_before: AllocatorSnapshot,
        allocator_held: AllocatorSnapshot,
        allocator_after_drop: AllocatorSnapshot,
        counts: [u64; 3],
    ) -> Value {
        json!({
            "schema":1,
            "scope":"test-support-only_single_process_one_variant_NO_SENDINPUT",
            "source_revision":revision,
            "manifest_sha256":manifest_hash,
            "variant":opts.variant,
            "workload":workload,
            "identity":identity,
            "warmup_in_same_process":opts.warmup,
            "prepared":{
                "entry_count":held.entry_count,
                "physical_packet_count":held.physical_count,
                "metadata_entry_count":held.metadata_count,
                "input_events":held.input_events,
                "packet_histogram":held.histogram,
                "reserved_stream_buffer_bytes":held.reserved_stream_bytes,
                "entry_reserved_bytes":held.entry_reserved_bytes,
                "descriptor_reserved_bytes":held.descriptor_reserved_bytes,
                "input_arena_reserved_bytes":held.input_arena_reserved_bytes,
                "input_arena_used_bytes":held.input_arena_used_bytes,
                "input_arena_capacity_events":held.input_arena_capacity_events,
                "reset_coordinator_heap_payload_bytes":held.reset_coordinator_heap_bytes
            },
            "measurements":{
                "prepare_reset_ns":held.prepare_reset_ns,
                "before_prepare_process":before,
                "while_owner_held_process":while_held,
                "after_owner_drop_process":after_drop,
                "before_prepare_allocator":allocator_before,
                "while_owner_held_allocator":allocator_held,
                "after_owner_drop_allocator":allocator_after_drop,
                "rust_net_requested_layout_bytes_before_to_held":
                    rust_net_requested_layout_delta(allocator_before, allocator_held),
                "rust_net_requested_layout_bytes_before_to_after_drop":
                    rust_net_requested_layout_delta(allocator_before, allocator_after_drop),
                "rust_global_allocator_alloc_or_realloc_calls":counts[0],
                "rust_global_allocator_requested_bytes":counts[1],
                "rust_global_allocator_dealloc_calls":counts[2],
                "scope":"Build_and_reset_only_excludes_compile_and_io; Rust allocator counters exclude OS/CRT allocations; process peak counters are lifetime peaks since process start and warmup contaminates warm peaks. Working set != private commit; while_owner_held is sampled before owner drop; after_owner_drop sampled immediately on return. Net requested Rust layout bytes are not OS committed bytes, malloc overhead or RSS."
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn synthetic_workloads_compile_and_prepare_inline_and_direct_compact() {
            let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0).unwrap();
            assert!(SYNTHETIC_UP_OFFSET_US > policy.min_hold_us);
            assert!(SYNTHETIC_GENERATION_STEP_US - SYNTHETIC_UP_OFFSET_US > policy.min_hold_us);

            // All runner generation counts, including the full 10k stress
            // case, are exercised through the actual prepared builders.
            // Compilation alone cannot detect an infeasible release floor.
            for generations in [100, 1_000, 10_000] {
                for width in [1, 15] {
                    let (compiled, _, identity) = synthetic_schedule(generations, width).unwrap();
                    assert_eq!(compiled.packets.len(), generations * 2);
                    assert_eq!(
                        identity["generation_step_us"].as_u64(),
                        Some(SYNTHETIC_GENERATION_STEP_US)
                    );
                    assert!(compiled.packets.iter().all(|packet| {
                        packet.up_mask.count_ones() == width as u32
                            || packet.down_mask.count_ones() == width as u32
                    }));
                    let builder =
                        ProductionDispatchTestHarness::new_prepared_startup_builder_for_test(
                            compiled,
                            1_000_000,
                            policy.min_hold_us,
                        )
                        .unwrap();

                    let inline = builder.clone_coordinator_for_test().prepare().unwrap();
                    let direct = builder.prepare_compact_direct_measurement().unwrap();

                    assert_eq!(inline.physical_entry_count, generations * 2);
                    assert_eq!(direct.physical_entry_count, inline.physical_entry_count);
                    assert_eq!(direct.metadata_entry_count, inline.metadata_entry_count);
                    assert_eq!(
                        direct.physical_input_event_histogram,
                        inline.physical_input_event_histogram
                    );
                    assert_eq!(direct.input_event_count, generations * width * 2);
                    assert_eq!(
                        direct.rebuilt_coordinator_heap_payload_bytes,
                        inline.rebuilt_coordinator_heap_payload_bytes
                    );
                }
            }
        }

        #[test]
        fn mask_reserved_synthetic_owners_preserve_legacy_compact_accounting() {
            let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0).unwrap();
            for generations in [100, 1_000, 10_000] {
                for width in [1, 7, 14, 15] {
                    let (compiled, _, _) = synthetic_schedule(generations, width).unwrap();
                    let builder =
                        ProductionDispatchTestHarness::new_prepared_startup_builder_for_test(
                            compiled,
                            1_000_000,
                            policy.min_hold_us,
                        )
                        .unwrap();

                    let legacy = builder
                        .clone_coordinator_for_test()
                        .with_held_prepared_owner_for_study(true, |held| Ok(held))
                        .unwrap();
                    let hinted = builder
                        .with_held_prepared_owner_for_study_with_input_hint(true, true, |held| {
                            Ok(held)
                        })
                        .unwrap();
                    assert_eq!(legacy.entry_count, hinted.entry_count);
                    assert_eq!(legacy.physical_count, hinted.physical_count);
                    assert_eq!(legacy.metadata_count, hinted.metadata_count);
                    assert_eq!(legacy.input_events, hinted.input_events);
                    assert_eq!(legacy.histogram, hinted.histogram);
                    assert_eq!(
                        legacy.reset_coordinator_heap_bytes,
                        hinted.reset_coordinator_heap_bytes
                    );
                    assert_eq!(hinted.input_arena_used_bytes, generations * width * 2 * 40);
                    assert!(hinted.input_arena_reserved_bytes >= hinted.input_arena_used_bytes);
                    assert_eq!(
                        hinted.reserved_stream_bytes,
                        hinted.entry_reserved_bytes
                            + hinted.descriptor_reserved_bytes
                            + hinted.input_arena_reserved_bytes
                    );
                }
            }
        }

        #[test]
        fn requested_layout_balance_includes_realloc_old_layouts() {
            let a = AllocatorSnapshot {
                alloc_or_realloc_calls: 10,
                requested_bytes: 1_000,
                dealloc_calls: 3,
                released_layout_bytes: 500,
            };
            let b = AllocatorSnapshot {
                alloc_or_realloc_calls: 12,
                requested_bytes: 1_400,
                dealloc_calls: 4,
                released_layout_bytes: 700,
            };
            assert_eq!(rust_net_requested_layout_delta(a, b), 200);
            assert_eq!(rust_net_requested_layout_delta(b, a), -200);
        }

        #[test]
        fn extended_win32_memory_counter_layout_covers_base() {
            assert!(size_of::<PROCESS_MEMORY_COUNTERS_EX>() > size_of::<PROCESS_MEMORY_COUNTERS>());
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(message) = windows_trial::run() {
        eprintln!("rt_prepared_process_probe: {message}");
        std::process::exit(1);
    }
}

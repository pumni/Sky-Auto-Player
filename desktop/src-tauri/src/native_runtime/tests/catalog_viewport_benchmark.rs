use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn viewport_benchmark_fixture(
    size: usize,
) -> (PathBuf, Arc<NativeDesktopRuntime>, u64, Vec<String>) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-viewport-projection-bench-{size}-{suffix}"));
    let songs_root = root.join("songs");
    fs::create_dir_all(&songs_root).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    for index in 0..size {
        fs::write(
            songs_root.join(format!("song-{index:05}.json")),
            format!(
                "{{\"name\":\"Bench Song {index:05}\",\"songNotes\":[{{\"time\":0,\"key\":\"Key0\"}}]}}"
            ),
        )
        .expect("benchmark song");
    }
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let catalog = runtime.catalog.lock().expect("catalog lock");
    let ids = catalog
        .index
        .snapshot()
        .items
        .into_iter()
        .map(|row| row.song_id)
        .collect();
    drop(catalog);
    (root, runtime, generation, ids)
}

fn benchmark_request(
    generation: u64,
    first: usize,
    width: usize,
    selected_song_id: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "generation": generation,
        "firstIndex": first,
        "lastIndex": first + width - 1,
        "selectedSongId": selected_song_id,
        "songIds": []
    })
}

fn run_viewport(runtime: &NativeDesktopRuntime, request: &serde_json::Value) {
    std::hint::black_box(
        runtime
            .dispatch("catalog.set_viewport", request.clone())
            .expect("viewport benchmark"),
    );
}

fn measure_once(runtime: &NativeDesktopRuntime, request: &serde_json::Value) -> f64 {
    let started = Instant::now();
    run_viewport(runtime, request);
    started.elapsed().as_secs_f64() * 1_000_000.0
}

fn measure_detail(runtime: &NativeDesktopRuntime, song_id: &str, generation: u64) -> f64 {
    let started = Instant::now();
    std::hint::black_box(
        runtime
            .dispatch(
                "catalog.detail",
                serde_json::json!({"songId":song_id,"generation":generation}),
            )
            .expect("detail benchmark"),
    );
    started.elapsed().as_secs_f64() * 1_000_000.0
}

fn warm_distribution(mut operation: impl FnMut()) -> Vec<f64> {
    const SAMPLES: usize = 9;
    const SAMPLE_WINDOW: Duration = Duration::from_millis(25);
    operation();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let mut iterations = 0_u64;
        while started.elapsed() < SAMPLE_WINDOW {
            operation();
            iterations += 1;
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
    }
    samples
}

fn print_distribution(size: usize, scenario: &str, samples: usize, values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
    println!(
        "package_b_projection size={size} scenario={scenario} samples={samples} p50_us={:.3} p90_us={:.3} min_us={:.3} max_us={:.3}",
        values[values.len() / 2],
        values[((values.len() - 1) as f64 * 0.9).ceil() as usize],
        values[0],
        values[values.len() - 1]
    );
}

/// Paired cold, exact-repeat, shift/back, and selected/detail viewport workload.
/// Run the same test against the approved B SHA and the correction SHA with
/// `cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked
/// --release --lib --features tauri-test
/// native_runtime::tests::catalog_viewport_benchmark::opt_in_package_b_viewport_projection_benchmark
/// -- --ignored --nocapture`. Timing values are diagnostic only.
#[test]
#[ignore = "opt-in Package B compact viewport projection benchmark"]
fn opt_in_package_b_viewport_projection_benchmark() {
    const COLD_SAMPLES: usize = 7;
    const SHIFT_SAMPLES: usize = 7;
    println!(
        "package_b_bench layer=native-viewport-projection fixture=package-b-native-files-v1 seed=0x560 size_set=96,1000,10000 profile=release cold_samples={COLD_SAMPLES} shift_back_samples={SHIFT_SAMPLES} warm_samples=9 warm_sample_ms=25 selected_detail_samples={COLD_SAMPLES}"
    );
    for size in [96_usize, 1_000, 10_000] {
        let (root, runtime, generation, ids) = viewport_benchmark_fixture(size);
        let width = size.min(200);
        let request = benchmark_request(generation, 0, width, None);

        let mut cold = Vec::with_capacity(COLD_SAMPLES);
        for _ in 0..COLD_SAMPLES {
            runtime.invalidate_analysis_cache();
            cold.push(measure_once(&runtime, &request));
        }
        print_distribution(size, "plain-cold", COLD_SAMPLES, &mut cold);

        runtime.invalidate_analysis_cache();
        run_viewport(&runtime, &request);
        let mut exact_warm = warm_distribution(|| run_viewport(&runtime, &request));
        print_distribution(size, "plain-exact-warm", exact_warm.len(), &mut exact_warm);

        if size > width {
            let shifted = benchmark_request(generation, 1, width, None);
            let mut shift_samples = Vec::with_capacity(SHIFT_SAMPLES);
            let mut back_samples = Vec::with_capacity(SHIFT_SAMPLES);
            for _ in 0..SHIFT_SAMPLES {
                run_viewport(&runtime, &request);
                shift_samples.push(measure_once(&runtime, &shifted));
                back_samples.push(measure_once(&runtime, &request));
            }
            print_distribution(size, "shift-plus-one", SHIFT_SAMPLES, &mut shift_samples);
            print_distribution(size, "shift-back", SHIFT_SAMPLES, &mut back_samples);
        } else {
            println!(
                "package_b_projection size={size} scenario=shift-plus-one-and-back samples=0 unavailable=viewport-covers-catalog"
            );
        }

        let mut selected_cases = vec![("first", 0_usize), ("last", width - 1)];
        if size > width {
            selected_cases.push(("outside", width));
        }
        for (placement, selected_index) in selected_cases {
            let selected = ids[selected_index].clone();
            let selected_request = benchmark_request(generation, 0, width, Some(selected.clone()));
            let mut selected_cold = Vec::with_capacity(COLD_SAMPLES);
            let mut detail_after_hydration = Vec::with_capacity(COLD_SAMPLES);
            for _ in 0..COLD_SAMPLES {
                runtime.invalidate_analysis_cache();
                selected_cold.push(measure_once(&runtime, &selected_request));
                detail_after_hydration.push(measure_detail(&runtime, &selected, generation));
            }
            print_distribution(
                size,
                &format!("selected-{placement}-cold"),
                COLD_SAMPLES,
                &mut selected_cold,
            );
            print_distribution(
                size,
                &format!("selected-{placement}-detail-after-hydration"),
                COLD_SAMPLES,
                &mut detail_after_hydration,
            );
            let mut selected_warm = warm_distribution(|| run_viewport(&runtime, &selected_request));
            print_distribution(
                size,
                &format!("selected-{placement}-exact-warm"),
                selected_warm.len(),
                &mut selected_warm,
            );
        }
        runtime.shutdown();
        fs::remove_dir_all(&root).expect("remove benchmark fixture");
    }
}

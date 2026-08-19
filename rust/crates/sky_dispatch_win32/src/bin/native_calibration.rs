//! Process-isolated Raw Input delivery-proxy calibration command.
//!
//! The player never invokes the calibration library in its own process. This
//! command owns the Raw Input registration and exits after cleanup, so Windows
//! restores the host process's registration state even if the calibration
//! process terminates unexpectedly.

use sky_dispatch_win32::calibration::{
    CALIBRATION_EVIDENCE_KIND, CALIBRATION_MIN_TOTAL_BUDGET_SECONDS, CALIBRATION_SCHEMA_VERSION,
    CalibrationConfig, CalibrationFailureReport, HOST_FINGERPRINT_VERSION,
    MEASUREMENT_PROTOCOL_VERSION, SampleClass, run_calibration_json,
    run_calibration_pair_bucket_json,
};

fn parse_u8(value: Option<String>, name: &str) -> Result<u8, String> {
    value
        .ok_or_else(|| format!("{name} requires a value"))?
        .parse::<u8>()
        .map_err(|_| format!("{name} must be an integer"))
}

fn parse_u64(value: Option<String>, name: &str) -> Result<u64, String> {
    value
        .ok_or_else(|| format!("{name} requires a value"))?
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a non-negative integer"))
}

fn parse_class(value: &str) -> Result<SampleClass, String> {
    match value {
        "hot" => Ok(SampleClass::Hot),
        "cold" => Ok(SampleClass::Cold),
        _ => Err("--class must be hot or cold".to_string()),
    }
}

fn main() -> Result<(), String> {
    let mut mode = String::from("quick");
    let mut class = None;
    let mut polyphony = None;
    let mut samples = None;
    let mut warmup_samples = 50u32;
    let mut budget_seconds = 120u64;
    let mut hot_gap_target_us = None;
    let mut cold_threshold_us = None;
    let mut cold_idle_gap_us = None;
    let mut metadata = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                mode = args
                    .next()
                    .ok_or_else(|| "--mode requires quick or full".to_string())?;
                if !matches!(mode.as_str(), "quick" | "full" | "bucket") {
                    return Err("--mode must be quick, full, or bucket".to_string());
                }
            }
            // Accepted only for a clear migration error. Protocol vNext has no
            // independent directional bucket.
            "--kind" => {
                return Err("--kind is retired; protocol v4 measures Down/Up pairs".to_string());
            }
            "--class" => {
                class =
                    Some(parse_class(&args.next().ok_or_else(|| {
                        "--class requires hot or cold".to_string()
                    })?)?)
            }
            "--polyphony" => polyphony = Some(parse_u8(args.next(), "--polyphony")?),
            "--samples" => {
                samples = Some(
                    args.next()
                        .ok_or_else(|| "--samples requires a value".to_string())?
                        .parse::<u32>()
                        .map_err(|_| "--samples must be an integer".to_string())?,
                )
            }
            "--warmup-samples" => {
                warmup_samples = args
                    .next()
                    .ok_or_else(|| "--warmup-samples requires a value".to_string())?
                    .parse::<u32>()
                    .map_err(|_| "--warmup-samples must be an integer".to_string())?;
            }
            "--budget-seconds" => {
                budget_seconds = args
                    .next()
                    .ok_or_else(|| "--budget-seconds requires a value".to_string())?
                    .parse::<u64>()
                    .map_err(|_| "--budget-seconds must be an integer".to_string())?;
                if !(CALIBRATION_MIN_TOTAL_BUDGET_SECONDS..=120).contains(&budget_seconds) {
                    return Err("--budget-seconds must be between 6 and 120".to_string());
                }
            }
            "--hot-gap-target-us" => {
                hot_gap_target_us = Some(parse_u64(args.next(), "--hot-gap-target-us")?);
            }
            "--cold-threshold-us" => {
                cold_threshold_us = Some(parse_u64(args.next(), "--cold-threshold-us")?);
            }
            "--cold-idle-gap-us" => {
                cold_idle_gap_us = Some(parse_u64(args.next(), "--cold-idle-gap-us")?);
            }
            "--metadata" => metadata = true,
            "--help" => {
                println!(
                    "usage: native_calibration --mode bucket --class hot|cold --polyphony N --samples N [--warmup-samples N] [--hot-gap-target-us N] [--cold-threshold-us N] [--cold-idle-gap-us N] [--budget-seconds 6..120]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if metadata {
        let host = sky_dispatch_win32::calibration::build_host_fingerprint()
            .map_err(|error| error.to_string())?;
        println!(
            "{}",
            serde_json::json!({
                "version": CALIBRATION_SCHEMA_VERSION,
                "calibration_schema_version": CALIBRATION_SCHEMA_VERSION,
                "measurement_protocol_version": MEASUREMENT_PROTOCOL_VERSION,
                "evidence_kind": CALIBRATION_EVIDENCE_KIND,
                "host_fingerprint_version": HOST_FINGERPRINT_VERSION,
                "source_git_sha": env!("SKY_NATIVE_BUILD_COMMIT"),
                "native_build_id": env!("SKY_NATIVE_BUILD_COMMIT"),
                "native_source_fingerprint": env!("SKY_NATIVE_SOURCE_FINGERPRINT"),
                "dirty_worktree": env!("SKY_NATIVE_DIRTY_WORKTREE") == "true",
                "rustc_version": env!("SKY_RUSTC_VERSION"),
                "host_fingerprint": host,
                "configuration": {
                    "hot_gap_target_us": CalibrationConfig::default().hot_gap_target_us,
                    "cold_threshold_us": CalibrationConfig::default().cold_threshold_us,
                    "cold_idle_gap_us": CalibrationConfig::default().cold_idle_gap_us,
                },
            })
        );
        return Ok(());
    }

    if mode == "bucket" {
        let class = class.ok_or_else(|| "--class is required in bucket mode".to_string())?;
        let polyphony =
            polyphony.ok_or_else(|| "--polyphony is required in bucket mode".to_string())?;
        let sample_count =
            samples.ok_or_else(|| "--samples is required in bucket mode".to_string())?;
        if sample_count == 0 || sample_count > 5_000 {
            return Err("--samples must be between 1 and 5000".to_string());
        }
        let mut config = CalibrationConfig::full();
        config.polyphonies = vec![polyphony];
        config.samples_per_hot_bucket = sample_count;
        config.samples_per_cold_bucket = sample_count;
        config.warmup_samples = warmup_samples;
        config.budget_seconds = budget_seconds;
        if let Some(value) = hot_gap_target_us {
            config.hot_gap_target_us = value;
        }
        if let Some(value) = cold_threshold_us {
            config.cold_threshold_us = value;
        }
        if let Some(value) = cold_idle_gap_us {
            config.cold_idle_gap_us = value;
        }
        let result = run_calibration_pair_bucket_json(&config, class);
        match result {
            Ok(output) => {
                println!("{output}");
                Ok(())
            }
            Err(error) => {
                let report = match error {
                    sky_dispatch_win32::calibration::CalibrationError::BucketFailed { report } => {
                        *report
                    }
                    other => CalibrationFailureReport {
                        kind: "pair".to_string(),
                        class: format!("{class:?}").to_lowercase(),
                        polyphony,
                        sample_index: 0,
                        phase: "setup down".to_string(),
                        exact_error: other.to_string(),
                        win32_error: None,
                        cleanup_success: false,
                        cleanup_stuck_keys: Vec::new(),
                        cleanup_verification_inconclusive: true,
                        raw_input_restore_failed: true,
                        pump_thread_failed: false,
                    },
                };
                eprintln!(
                    "CALIBRATION_FAILURE_JSON:{}",
                    serde_json::to_string(&report)
                        .map_err(|serialize_error| serialize_error.to_string())?
                );
                Err(report.to_string())
            }
        }
    } else if mode == "quick" {
        let output = {
            let mut config = CalibrationConfig::quick();
            config.budget_seconds = budget_seconds;
            if let Some(value) = hot_gap_target_us {
                config.hot_gap_target_us = value;
            }
            if let Some(value) = cold_threshold_us {
                config.cold_threshold_us = value;
            }
            if let Some(value) = cold_idle_gap_us {
                config.cold_idle_gap_us = value;
            }
            run_calibration_json(&config).map_err(|error| error.to_string())?
        };
        println!("{output}");
        Ok(())
    } else {
        Err("full mode is orchestrated by scripts/run_native_calibration.py; use --mode bucket for one bucket".to_string())
    }
}

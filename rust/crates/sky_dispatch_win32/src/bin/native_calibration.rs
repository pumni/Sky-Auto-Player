//! Process-isolated Raw Input delivery-proxy calibration command.
//!
//! The player never invokes the calibration library in its own process. This
//! command owns the Raw Input registration and exits after cleanup, so Windows
//! restores the host process's registration state even if the calibration
//! process terminates unexpectedly.

use sky_dispatch_win32::calibration::{CalibrationConfig, run_calibration_json};

fn main() -> Result<(), String> {
    let mut mode = String::from("quick");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                mode = args
                    .next()
                    .ok_or_else(|| "--mode requires quick or full".to_string())?;
                if !matches!(mode.as_str(), "quick" | "full") {
                    return Err("--mode must be quick or full".to_string());
                }
            }
            "--help" => {
                println!("usage: native_calibration [--mode quick|full]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let config = match mode.as_str() {
        "full" => CalibrationConfig::full(),
        "quick" => CalibrationConfig::quick(),
        _ => return Err("invalid calibration mode".to_string()),
    };
    let output = run_calibration_json(&config).map_err(|error| error.to_string())?;
    println!("{output}");
    Ok(())
}

use super::{RunArgs, acceptance_min_hold_us, acceptance_min_release_gap_us};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Pass,
    NonQualifying,
    Fail,
    Inconclusive,
}

impl Verdict {
    const fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::NonQualifying => "NON_QUALIFYING",
            Self::Fail => "FAIL",
            Self::Inconclusive => "INCONCLUSIVE",
        }
    }

    pub(super) const fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::NonQualifying => 3,
            Self::Fail => 1,
            Self::Inconclusive => 2,
        }
    }
}

pub(super) fn write_report(args: &RunArgs, verdict: Verdict, reason: &str, details: Value) -> i32 {
    let report = json!({"status": verdict.label(), "scenario": args.scenario.label(), "run_id": args.run_id, "timing_margin_us": args.timing_margin_us, "min_hold_us": acceptance_min_hold_us(args.timing_margin_us), "min_release_gap_us": acceptance_min_release_gap_us(args.timing_margin_us), "reason": reason, "details": details});
    let serialized = serde_json::to_string(&report).unwrap_or_else(|_| {
        format!(
            r#"{{"status":"{}","reason":"report serialization failed"}}"#,
            verdict.label()
        )
    });
    println!("{serialized}");
    if let Err(error) = append_json_line(&args.evidence, &serialized) {
        eprintln!("failed to write evidence: {error}");
        return Verdict::Inconclusive.exit_code();
    }
    verdict.exit_code()
}

fn append_json_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

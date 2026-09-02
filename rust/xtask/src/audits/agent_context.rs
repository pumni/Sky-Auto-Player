use crate::Result;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const CONTEXT_BUDGETS: &[(&str, usize)] = &[
    ("AGENTS.md", 7_000),
    ("CLAUDE.md", 1_500),
    ("CONTRIBUTING.md", 6_000),
    ("docs/INDEX.md", 5_000),
    ("docs/README.md", 3_000),
    (".github/PULL_REQUEST_TEMPLATE.md", 3_000),
    (".github/ISSUE_TEMPLATE/bug_report.md", 3_000),
    (".github/ISSUE_TEMPLATE/feature_request.md", 2_000),
    (".github/ISSUE_TEMPLATE/config.yml", 1_500),
];

const RETIRED_PATHS: &[&str] = &[
    ".agent",
    ".claude",
    ".codex",
    ".cursor",
    ".windsurf",
    ".cursorrules",
    ".windsurfrules",
    "GEMINI.md",
    "COPILOT.md",
    "site/AGENTS.md",
    "site/CLAUDE.md",
    "docs/archive",
    "docs/plan",
    "docs/rust-dispatch-migration",
    "docs/PORTING_GUIDE.md",
    "docs/rust-migration-plan.md",
    "docs/2026-08-01-rust-overhaul-plan.md",
    "docs/2026-08-rust-core-consolidation-plan.md",
    "docs/dispatch-chord-timing-residual-review-2026-07-23.md",
    ".github/ISSUE_TEMPLATE/security_p0.md",
    ".github/copilot-instructions.md",
];

const SECURITY_OWNED_SURFACES: &[&str] = &[
    "scripts/audit_security_mandates.py",
    ".config/security_audit_baseline.json",
    ".github/workflows/release.yml",
];
const SCAN_ROOTS: &[&str] = &["src", "rust", "tests", "scripts", "docs", ".github", "site"];
const GENERATED_DIRS: &[&str] = &[
    ".git",
    ".venv",
    ".astro",
    ".cache",
    ".pytest_cache",
    ".ruff_cache",
    "__pycache__",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
];
const SHADOW_GUIDES: &[&str] = &["AGENTS.md", "CLAUDE.md", "GEMINI.md", "COPILOT.md"];
const FORBIDDEN_CHOREOGRAPHY: &[&str] = &[
    "priority stack",
    "altitude table",
    "coding_agent_handoff",
    "coding agent handoff",
    "agents.md p0",
    "<security_mandates>",
    "porting_guide.md",
    "read every plan",
    "preload all",
];
const FORBIDDEN_DOC_MARKERS: &[&str] = &["handoff", "coding-agent", "ai-coding"];

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_plan_name(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace('_', "-");
    stem == "plan"
        || stem.starts_with("plan-")
        || stem.ends_with("-plan")
        || stem.contains("-plan-")
}

fn walk_surface(root: &Path, base: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !entry
                .file_name()
                .to_str()
                .is_some_and(|name| GENERATED_DIRS.contains(&name))
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(move |entry| {
            entry
                .into_path()
                .strip_prefix(root)
                .unwrap_or_else(|_| Path::new(""))
                .to_path_buf()
        })
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let mut failures = Vec::new();
    for (path, limit) in CONTEXT_BUDGETS {
        let target = root.join(path);
        if !target.is_file() {
            failures.push(format!("missing context surface: {path}"));
        } else if fs::metadata(&target)?.len() as usize > *limit {
            failures.push(format!("{path} exceeds context budget of {limit} bytes"));
        }
    }
    for path in RETIRED_PATHS {
        if root.join(path).exists() {
            failures.push(format!("retired context surface returned: {path}"));
        }
    }
    for root_name in SCAN_ROOTS {
        let base = root.join(root_name);
        if !base.is_dir() {
            continue;
        }
        for path in walk_surface(root, &base) {
            if SHADOW_GUIDES.contains(
                &path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(""),
            ) {
                failures.push(format!(
                    "nested agent authority is forbidden: {}",
                    relative(root, &path)
                ));
            }
        }
    }
    let docs = root.join("docs");
    if docs.is_dir() {
        for entry in WalkDir::new(&docs)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path();
            if entry.file_type().is_dir()
                && ["plan", "plans", "archive", "archives"].contains(
                    &entry
                        .file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .as_str(),
                )
            {
                failures.push(format!(
                    "plan/archive directory must live in Git history: {}",
                    relative(root, path)
                ));
            }
            if !entry.file_type().is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("md")
            {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .replace('_', "-");
            if is_plan_name(path) {
                failures.push(format!(
                    "implementation plan must live in Git history: {}",
                    relative(root, path)
                ));
            }
            if FORBIDDEN_DOC_MARKERS
                .iter()
                .any(|marker| name.contains(marker))
            {
                failures.push(format!(
                    "agent handoff/runbook document is forbidden: {}",
                    relative(root, path)
                ));
            }
            if path.parent() == Some(docs.as_path()) {
                let text = fs::read_to_string(path)?.to_ascii_lowercase();
                for phrase in FORBIDDEN_CHOREOGRAPHY {
                    if text.contains(phrase) {
                        failures.push(format!(
                            "{} contains retired instruction choreography: {phrase:?}",
                            relative(root, path)
                        ));
                    }
                }
            }
        }
    }
    for path in CONTEXT_BUDGETS.iter().map(|(path, _)| *path) {
        let target = root.join(path);
        if !target.is_file() {
            continue;
        }
        let text = fs::read_to_string(&target)?.to_ascii_lowercase();
        for phrase in FORBIDDEN_CHOREOGRAPHY {
            if text.contains(phrase) {
                failures.push(format!(
                    "{path} contains retired instruction choreography: {phrase:?}"
                ));
            }
        }
    }
    let agents = root.join("AGENTS.md");
    if agents.is_file() {
        let text = fs::read_to_string(&agents)?;
        if !text.contains("SECURITY.md") {
            failures.push("AGENTS.md must route security authority to SECURITY.md".into());
        }
        if !text.contains("vendor-neutral repository contract") {
            failures.push(
                "AGENTS.md must identify itself as the vendor-neutral repository contract".into(),
            );
        }
    }
    let claude = root.join("CLAUDE.md");
    if claude.is_file() && !fs::read_to_string(&claude)?.contains("AGENTS.md") {
        failures.push("CLAUDE.md must remain a thin adapter to AGENTS.md".into());
    }
    for path in SECURITY_OWNED_SURFACES {
        let target = root.join(path);
        if target.is_file()
            && fs::read_to_string(target)?
                .to_ascii_lowercase()
                .contains("agents.md p0")
        {
            failures.push(format!(
                "{path} still derives security authority from AGENTS.md P0 wording"
            ));
        }
    }
    if let Some(first) = failures.first() {
        return Err(format!(
            "agent-context audit failed: {first} ({} failure(s))",
            failures.len()
        )
        .into());
    }
    println!("[xtask] agent-context checks: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_name_detection_matches_repository_policy() {
        assert!(is_plan_name(Path::new("migration-plan.md")));
        assert!(is_plan_name(Path::new("plan.md")));
        assert!(!is_plan_name(Path::new("architecture.md")));
    }
}

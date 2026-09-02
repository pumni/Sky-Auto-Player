use crate::{Result, process};
use std::path::Path;

const FORBIDDEN_FILENAMES: &[&str] = &[
    "pyproject.toml",
    ".python-version",
    "uv.lock",
    "Pipfile",
    "Pipfile.lock",
    "poetry.lock",
    "setup.py",
    "setup.cfg",
    "tox.ini",
];
const AUTOMATION_ROOTS: &[&str] = &[
    ".github/workflows/",
    ".github/actions/",
    "scripts/",
    "rust/xtask/",
    "branding/scripts/",
    "site/scripts/",
];

fn forbidden_filename(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".py")
        || lower == "pyproject.toml"
        || lower == ".python-version"
        || lower == "uv.lock"
        || lower.ends_with("requirements.txt")
        || (lower.contains("requirements-") && lower.ends_with(".txt"))
        || FORBIDDEN_FILENAMES
            .iter()
            .any(|name| lower.ends_with(&name.to_ascii_lowercase()))
}

fn strip_line_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with("//") || trimmed.starts_with("<!--") {
        return "";
    }
    line.split_once(" # ").map_or(line, |(code, _)| code)
}

fn executable_invocation(line: &str) -> Option<&'static str> {
    let lower = strip_line_comment(line).to_ascii_lowercase();
    let value = lower.trim();
    let markers = [
        ("uv run", "uv run"),
        ("py -m", "py -m"),
        ("python3 -m", "python3"),
        ("python -m", "python"),
        ("pytest", "pytest"),
        ("pyright", "pyright"),
        ("pyrefly", "pyrefly"),
        ("ruff", "ruff"),
        ("pip3", "pip3"),
        ("pip ", "pip"),
        ("python3 ", "python3"),
        ("python ", "python"),
    ];
    markers.iter().find_map(|(marker, name)| {
        let position = value.find(marker)?;
        let before = value[..position].chars().next_back();
        let boundary = before.is_none()
            || before
                .is_some_and(|character| !character.is_ascii_alphanumeric() && character != '_');
        let prefix = value[..position].trim_end();
        let command_position = position == 0
            || prefix.ends_with(':')
            || prefix.ends_with("run")
            || prefix.ends_with("&&")
            || prefix.ends_with("||")
            || prefix.ends_with(';')
            || prefix.ends_with('|')
            || prefix.contains("command::new")
            || prefix.contains("process::run");
        (boundary && command_position).then_some(*name)
    })
}

pub(crate) fn find_violations(root: &Path) -> Result<Vec<String>> {
    let output = process::capture_text("git", &["ls-files"], root, &[])?;
    let paths = output
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let mut violations = Vec::new();
    let mut automation_files = Vec::new();
    for path in paths {
        if forbidden_filename(path) && root.join(path).is_file() {
            violations.push(format!("tracked Python/toolchain file: {path}"));
        }
        if AUTOMATION_ROOTS
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            automation_files.push(path.to_owned());
        }
    }
    for path in automation_files {
        if path == "rust/xtask/src/audits/zero_python.rs" {
            continue;
        }
        let absolute = root.join(&path);
        if !absolute.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&absolute)?;
        for (line_number, line) in text.lines().enumerate() {
            if let Some(tool) = executable_invocation(line) {
                violations.push(format!(
                    "Python tool invocation `{tool}` in {path}:{}",
                    line_number + 1
                ));
            }
        }
    }
    Ok(violations)
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let violations = find_violations(root)?;
    if let Some(first) = violations.first() {
        return Err(format!(
            "zero-Python audit failed: {first} ({} violation(s))",
            violations.len()
        )
        .into());
    }
    println!("[xtask] zero-Python checks: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_surface_detection_allows_historical_words_and_rejects_commands() {
        assert_eq!(
            executable_invocation("uv run python scripts/tool.py"),
            Some("uv run")
        );
        assert_eq!(executable_invocation("cargo test"), None);
        assert_eq!(
            executable_invocation("# pytest is historical evidence"),
            None
        );
        assert!(forbidden_filename("tools/requirements-dev.txt"));
        assert!(forbidden_filename("scripts/tool.py"));
        assert!(!forbidden_filename("tests/fixtures/old-release.json"));
    }
}

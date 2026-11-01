use crate::Result;
use semver::Version;
use serde_json::Value;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const BUN_ACTION: &str = "oven-sh/setup-bun";
const BUN_VERSION_FILE: &str = "site/package.json";
const BUN_VERIFIER_STEP: &str = "bun scripts/verify-bun-pin.mjs";

fn canonical_bun_version(source: &str) -> Result<Version> {
    if source.matches("\"packageManager\"").count() != 1 {
        return Err("site/package.json must contain exactly one packageManager field".into());
    }

    let manifest: Value = serde_json::from_str(source)?;
    let package_manager = manifest
        .get("packageManager")
        .and_then(Value::as_str)
        .ok_or("site/package.json packageManager must be a string")?;
    let raw_version = package_manager
        .strip_prefix("bun@")
        .ok_or("site/package.json packageManager must use bun@<exact-version>")?;
    let version = Version::parse(raw_version)
        .map_err(|error| format!("site/package.json has an invalid Bun version: {error}"))?;
    if !version.pre.is_empty() || !version.build.is_empty() || version.to_string() != raw_version {
        return Err(
            "site/package.json packageManager must contain an exact numeric Bun version".into(),
        );
    }
    Ok(version)
}

fn code_without_comment(line: &str) -> &str {
    line.char_indices()
        .find(|(index, character)| {
            *character == '#' && (*index == 0 || line.as_bytes()[index - 1].is_ascii_whitespace())
        })
        .map_or(line, |(index, _)| &line[..index])
}

fn indentation(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn active_line(line: &str) -> &str {
    code_without_comment(line).trim()
}

fn list_item_indent(line: &str) -> Option<usize> {
    let code = code_without_comment(line);
    code.trim_start()
        .starts_with("- ")
        .then(|| indentation(line))
}

fn block_end(lines: &[&str], start: usize, list_indent: usize, limit: usize) -> usize {
    for (index, line) in lines.iter().enumerate().take(limit).skip(start + 1) {
        let code = active_line(line);
        if code.is_empty() {
            continue;
        }
        let current_indent = indentation(line);
        if current_indent < list_indent
            || (current_indent == list_indent && list_item_indent(line).is_some())
        {
            return index;
        }
    }
    limit
}

fn yaml_scalar(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn contract_error<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into().into())
}

fn containing_steps_end(
    lines: &[&str],
    step_start: usize,
    list_indent: usize,
    path: &Path,
) -> Result<usize> {
    for (index, line) in lines.iter().enumerate().take(step_start).rev() {
        let steps_indent = indentation(line);
        if active_line(line) != "steps:" || list_indent != steps_indent + 2 {
            continue;
        }
        let steps_end = (index + 1..lines.len())
            .find(|candidate| {
                !active_line(lines[*candidate]).is_empty()
                    && indentation(lines[*candidate]) <= steps_indent
            })
            .unwrap_or(lines.len());
        if step_start < steps_end {
            return Ok(steps_end);
        }
    }
    contract_error(format!(
        "{} setup-bun use is outside a steps: list",
        path.display()
    ))
}

fn validate_step_controls(
    lines: &[&str],
    step_start: usize,
    step_end: usize,
    list_indent: usize,
    path: &Path,
    label: &str,
) -> Result<()> {
    let direct_indent = list_indent + 2;
    let mut saw_continue = false;
    for line in &lines[step_start + 1..step_end] {
        if indentation(line) != direct_indent {
            continue;
        }
        let Some((key, value)) = active_line(line).split_once(':') else {
            continue;
        };
        match key.trim() {
            "if" => {
                return contract_error(format!("{} {label} step is conditional", path.display()));
            }
            "continue-on-error"
                if saw_continue || !yaml_scalar(value).eq_ignore_ascii_case("false") =>
            {
                return contract_error(format!(
                    "{} {label} step can ignore failures",
                    path.display()
                ));
            }
            "continue-on-error" => saw_continue = true,
            _ => {}
        }
    }
    Ok(())
}

fn validate_verifier_step(
    lines: &[&str],
    step_start: usize,
    step_end: usize,
    list_indent: usize,
    path: &Path,
) -> Result<()> {
    let direct_indent = list_indent + 2;
    let mut seen_fields = Vec::new();
    let mut run_value = None;

    for line in &lines[step_start + 1..step_end] {
        if active_line(line).is_empty() || indentation(line) != direct_indent {
            continue;
        }
        let Some((key, value)) = active_line(line).split_once(':') else {
            return contract_error("invalid Bun verifier field".to_owned());
        };
        let key = key.trim();
        if seen_fields.contains(&key) {
            return contract_error(format!("{} duplicate verifier field {key}", path.display()));
        }
        seen_fields.push(key);

        match key {
            "run" => run_value = Some(yaml_scalar(value)),
            "name" | "id" | "timeout-minutes" => (),
            "shell" if matches!(yaml_scalar(value), "bash" | "pwsh" | "powershell") => (),
            _ => {
                return contract_error(format!(
                    "{} unsupported verifier field {key}",
                    path.display()
                ));
            }
        }
    }

    if run_value != Some(BUN_VERIFIER_STEP) {
        return contract_error(format!(
            "{} verifier requires one supported run command",
            path.display()
        ));
    }
    Ok(())
}

fn validate_setup_step(lines: &[&str], uses_index: usize, path: &Path) -> Result<()> {
    let uses_line = lines[uses_index];
    let uses_indent = indentation(uses_line);
    let source_line = active_line(uses_line);
    let value = source_line
        .strip_prefix("uses:")
        .ok_or_else(|| format!("{} has an unsupported setup-bun YAML form", path.display()))?
        .trim();
    let reference = value.split_whitespace().next().unwrap_or_default();
    let (action, sha) = reference.split_once('@').ok_or_else(|| {
        format!(
            "{} setup-bun reference must stay SHA-pinned",
            path.display()
        )
    })?;
    if action != BUN_ACTION || sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "{} setup-bun reference must use its full 40-character Action SHA",
            path.display()
        )
        .into());
    }

    let step_indent = (0..uses_index)
        .rev()
        .find_map(|index| list_item_indent(lines[index]).filter(|indent| *indent < uses_indent))
        .ok_or_else(|| {
            format!(
                "{} setup-bun use is not inside a supported step",
                path.display()
            )
        })?;
    if uses_indent != step_indent + 2 {
        return Err(format!(
            "{} setup-bun use must be a direct step field",
            path.display()
        )
        .into());
    }
    let step_start = (0..uses_index)
        .rev()
        .find(|index| list_item_indent(lines[*index]) == Some(step_indent))
        .ok_or_else(|| format!("{} setup-bun use has no containing step", path.display()))?;
    let steps_end = containing_steps_end(lines, step_start, step_indent, path)?;
    let step_end = block_end(lines, step_start, step_indent, steps_end);
    if uses_index >= step_end {
        return Err(format!(
            "{} setup-bun use is outside its containing step",
            path.display()
        )
        .into());
    }
    validate_step_controls(lines, step_start, step_end, step_indent, path, "setup-bun")?;
    let with_indices = (uses_index + 1..step_end)
        .filter(|index| {
            indentation(lines[*index]) == uses_indent && active_line(lines[*index]) == "with:"
        })
        .collect::<Vec<_>>();
    if with_indices.len() != 1 {
        return Err(format!(
            "{} setup-bun step must have exactly one supported with: block",
            path.display()
        )
        .into());
    }

    let with_index = with_indices[0];
    let with_indent = indentation(lines[with_index]);
    let with_end = (with_index + 1..step_end)
        .find(|index| {
            !active_line(lines[*index]).is_empty() && indentation(lines[*index]) <= with_indent
        })
        .unwrap_or(step_end);
    let mut version_file_values = Vec::new();
    for line in &lines[with_index + 1..with_end] {
        let code = active_line(line);
        let Some((key, value)) = code.split_once(':') else {
            continue;
        };
        match key.trim() {
            "bun-version" => {
                return Err(format!(
                    "{} setup-bun must not select a literal or implicit Bun version",
                    path.display()
                )
                .into());
            }
            "bun-version-file" => {
                if indentation(line) != with_indent + 2 {
                    return Err(format!(
                        "{} setup-bun bun-version-file must be a direct with: input",
                        path.display()
                    )
                    .into());
                }
                version_file_values.push(yaml_scalar(value));
            }
            _ => {}
        }
    }
    if version_file_values.len() != 1 || version_file_values[0] != BUN_VERSION_FILE {
        return Err(format!(
            "{} setup-bun must set bun-version-file to {BUN_VERSION_FILE}",
            path.display()
        )
        .into());
    }

    let next_step_start =
        (step_end..steps_end).find(|index| list_item_indent(lines[*index]) == Some(step_indent));
    let Some(next_step_start) = next_step_start else {
        return Err(format!(
            "{} setup-bun step must be followed immediately by the Bun pin verifier",
            path.display()
        )
        .into());
    };
    let next_step_end = block_end(lines, next_step_start, step_indent, steps_end);
    validate_verifier_step(lines, next_step_start, next_step_end, step_indent, path)
}

fn validate_yaml_source(source: &str, path: &Path) -> Result<usize> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut consumers = 0;
    let mut verifier_definitions = 0;
    for (index, line) in lines.iter().enumerate() {
        let code = active_line(line);
        if code.split_once(':').is_some_and(|(key, value)| {
            key.trim() == "run" && yaml_scalar(value) == BUN_VERIFIER_STEP
        }) {
            verifier_definitions += 1;
        }
        if !code.contains(BUN_ACTION) {
            continue;
        }
        if !code.starts_with("uses:") {
            return Err(format!(
                "{} contains an unsupported or ambiguous setup-bun consumer",
                path.display()
            )
            .into());
        }
        validate_setup_step(&lines, index, path)?;
        consumers += 1;
    }
    if verifier_definitions != consumers {
        return Err(format!(
            "{} must define exactly one Bun verifier command per setup-bun consumer (found {verifier_definitions} for {consumers})",
            path.display()
        )
        .into());
    }
    Ok(consumers)
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yml" | "yaml")
    )
}

fn validate_repository(root: &Path) -> Result<usize> {
    let manifest_path = root.join(BUN_VERSION_FILE);
    let manifest = fs::read_to_string(&manifest_path)?;
    let version = canonical_bun_version(&manifest)?;
    let mut consumers = 0usize;

    for directory in [".github/workflows", ".github/actions"] {
        let scan_root = root.join(directory);
        for entry in WalkDir::new(&scan_root).follow_links(false) {
            let entry = entry?;
            let path = entry.path();
            if !is_yaml(path) {
                continue;
            }
            if entry.file_type().is_symlink() {
                return Err(format!(
                    "setup-bun audit does not follow YAML symlinks: {}",
                    path.display()
                )
                .into());
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let source = fs::read_to_string(path)?;
            consumers += validate_yaml_source(&source, path)?;
        }
    }
    if consumers == 0 {
        return Err("no SHA-pinned setup-bun consumers were found under .github".into());
    }
    println!("[xtask] Bun pin contract: PASS (Bun {version}; {consumers} consumers verified)");
    Ok(consumers)
}

pub(crate) fn bun_pin_contract(root: &Path) -> Result<()> {
    validate_repository(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci_artifacts;
    use std::fs;
    use std::path::PathBuf;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const PACKAGE: &str = r#"{"name":"fixture","packageManager":"bun@9.8.7"}"#;
    const VALID_STEP: &str = "jobs:\n  verify:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Set up Bun\n        uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n        with:\n          bun-version-file: site/package.json\n      - name: Verify Bun pin\n        run: bun scripts/verify-bun-pin.mjs\n";
    const VALID_COMPOSITE: &str = "runs:\n  using: composite\n  steps:\n    - name: Set up Bun\n      uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n      with:\n        bun-version-file: site/package.json\n    - name: Verify Bun pin\n      shell: bash\n      run: bun scripts/verify-bun-pin.mjs\n";

    fn fixture_root() -> PathBuf {
        let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join("sky-bun-pin"), "test");
        fs::create_dir_all(path.join(".github/workflows")).unwrap();
        fs::create_dir_all(path.join(".github/actions/site-validate")).unwrap();
        fs::create_dir_all(path.join("site")).unwrap();
        fs::write(path.join(BUN_VERSION_FILE), PACKAGE).unwrap();
        path
    }

    fn write_workflow(root: &Path, path: &str, source: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn assert_rejected(source: &str, name: &str) {
        assert!(
            validate_yaml_source(source, Path::new(name)).is_err(),
            "{name}"
        );
    }

    #[test]
    fn accepts_exact_numeric_manifest_and_sha_pinned_file_input() {
        let version = canonical_bun_version(PACKAGE).unwrap();
        assert_eq!(version.to_string(), "9.8.7");
        assert_eq!(
            validate_yaml_source(VALID_STEP, Path::new("fixture.yml")).unwrap(),
            1
        );
    }

    #[test]
    fn rejects_missing_floating_duplicate_and_ambiguous_package_manager() {
        for source in [
            r#"{"name":"fixture"}"#,
            r#"{"name":"fixture","packageManager":"bun@latest"}"#,
            r#"{"name":"fixture","packageManager":"bun@9.8"}"#,
            r#"{"packageManager":"bun@9.8.7","packageManager":"bun@9.8.8"}"#,
            r#"{"nested":{"packageManager":"bun@9.8.7"},"packageManager":"bun@9.8.7"}"#,
        ] {
            assert!(canonical_bun_version(source).is_err(), "{source}");
        }
    }

    #[test]
    fn rejects_missing_wrong_literal_and_implicit_latest_inputs() {
        for (name, source) in [
            (
                "missing.yml",
                VALID_STEP.replace("          bun-version-file: site/package.json\n", ""),
            ),
            (
                "wrong.yml",
                VALID_STEP.replace("site/package.json", ".bun-version"),
            ),
            (
                "literal.yml",
                VALID_STEP.replace(
                    "          bun-version-file: site/package.json",
                    "          bun-version: 9.8.7\n          bun-version-file: site/package.json",
                ),
            ),
            (
                "latest.yml",
                VALID_STEP.replace(
                    "          bun-version-file: site/package.json",
                    "          bun-version: latest",
                ),
            ),
        ] {
            assert_rejected(&source, name);
        }
    }

    #[test]
    fn rejects_unpinned_and_unrecognized_new_consumers() {
        let pinned_reference = format!("@{SHA}");
        let unpinned = VALID_STEP.replace(&pinned_reference, "@v2");
        assert_rejected(&unpinned, "tag.yml");

        let inline = VALID_STEP.replace(
            "        uses: oven-sh/setup-bun@",
            "      - uses: oven-sh/setup-bun@",
        );
        assert_rejected(&inline, "inline.yml");
    }

    #[test]
    fn rejects_conditionally_skipped_or_failure_ignored_verifier() {
        for control in ["if: false", "continue-on-error: true"] {
            let source = VALID_STEP.replace(
                "        run: bun scripts/verify-bun-pin.mjs",
                &format!("        {control}\n        run: bun scripts/verify-bun-pin.mjs"),
            );
            assert_rejected(&source, "verification-bypass.yml");
        }
    }

    #[test]
    fn rejects_duplicate_ambiguous_and_unsupported_verifier_definitions() {
        let run = "        run: bun scripts/verify-bun-pin.mjs";
        for source in [
            VALID_STEP.replace(run, &format!("{run}\n{run}")),
            VALID_STEP.replace(
                run,
                &format!("{run}\n      - name: Duplicate verifier\n{run}"),
            ),
            VALID_STEP.replace(run, "        uses: local/verify-bun-pin@v1"),
            VALID_STEP.replace(
                run,
                "        run: |\n          bun scripts/verify-bun-pin.mjs",
            ),
        ] {
            assert_rejected(&source, "ambiguous-verifier.yml");
        }
    }

    #[test]
    fn rejects_verifier_in_a_different_job() {
        let source = "jobs:\n  setup:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Set up Bun\n        uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n        with:\n          bun-version-file: site/package.json\n  verify:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Verify Bun pin\n        run: bun scripts/verify-bun-pin.mjs\n";
        let error = validate_yaml_source(source, Path::new("two-jobs.yml")).unwrap_err();
        assert!(error.to_string().contains("must be followed immediately"));
    }

    #[test]
    fn repository_scan_discovers_new_workflows_and_composite_actions() {
        let root = fixture_root();
        write_workflow(&root, ".github/workflows/first.yml", VALID_STEP);
        write_workflow(
            &root,
            ".github/actions/site-validate/action.yml",
            VALID_COMPOSITE,
        );
        assert_eq!(validate_repository(&root).unwrap(), 2);

        write_workflow(
            &root,
            ".github/workflows/new-consumer.yml",
            &VALID_STEP.replace("site/package.json", ".bun-version"),
        );
        assert!(validate_repository(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

use std::collections::BTreeSet;

pub(super) fn workflow_job_blocks(source: &str) -> Vec<(String, String)> {
    let mut jobs = Vec::new();
    let mut in_jobs = false;
    let mut current_id = None;
    let mut current_block = String::new();

    for line in source.lines() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        if !in_jobs {
            if indent == 0 && trimmed == "jobs:" {
                in_jobs = true;
            }
            continue;
        }
        if indent == 0 && !trimmed.is_empty() {
            if let Some(job_id) = current_id.take() {
                jobs.push((job_id, std::mem::take(&mut current_block)));
            }
            break;
        }
        if indent == 2 && !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed.ends_with(':')
        {
            if let Some(job_id) = current_id.take() {
                jobs.push((job_id, std::mem::take(&mut current_block)));
            }
            current_id = Some(trimmed.trim_end_matches(':').to_owned());
            continue;
        }
        if current_id.is_some() {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }
    if let Some(job_id) = current_id {
        jobs.push((job_id, current_block));
    }

    jobs
}

pub(super) fn workflow_step_blocks(source: &str) -> Vec<(String, String)> {
    let mut steps = Vec::new();
    let mut current_name = None;
    let mut current_block = String::new();

    for line in source.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - line.trim_start().len();
        if indent == 6 && trimmed.starts_with("- name: ") {
            if let Some(name) = current_name.take() {
                steps.push((name, std::mem::take(&mut current_block)));
            }
            current_name = Some(trimmed["- name: ".len()..].to_owned());
            current_block.push_str(line);
            current_block.push('\n');
            continue;
        }
        if current_name.is_some() {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }
    if let Some(name) = current_name {
        steps.push((name, current_block));
    }

    steps
}

const V4_RELEASE_STATE_INVOCATION_REQUIREMENTS: &[(&str, &[&str])] = &[
    (
        "Preflight release request and repository readiness",
        &["-State Preflight", "-StateRoot $env:V4_RELEASE_STATE_ROOT"],
    ),
    (
        "Build and qualify the single production candidate",
        &[
            "-State BuildCandidate",
            "-StateRoot $env:V4_RELEASE_STATE_ROOT",
            "-UpdaterPrivateKeyPath $env:V4_UPDATER_PRIVATE_KEY_PATH",
        ],
    ),
    (
        "Publish the qualified candidate immutably",
        &[
            "-State PublishRelease",
            "-StateRoot $env:V4_RELEASE_STATE_ROOT",
        ],
    ),
    (
        "Promote release metadata only after immutable publication",
        &[
            "-State PromoteMetadata",
            "-StateRoot $env:V4_RELEASE_STATE_ROOT",
        ],
    ),
    (
        "Re-fetch and verify final public release and metadata",
        &[
            "-State FinalVerify",
            "-StateRoot $env:V4_RELEASE_STATE_ROOT",
        ],
    ),
];

fn pipeline_validate_set_states(pipeline: &str) -> std::result::Result<BTreeSet<String>, String> {
    let start = pipeline
        .find("[ValidateSet(")
        .ok_or_else(|| "release pipeline is missing its State ValidateSet".to_owned())?;
    let end = pipeline[start..]
        .find(")]")
        .map(|offset| start + offset)
        .ok_or_else(|| "release pipeline State ValidateSet is unterminated".to_owned())?;
    let states = pipeline[start..end]
        .split('"')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value.to_owned()))
        .collect::<BTreeSet<_>>();
    if states.is_empty() {
        return Err("release pipeline State ValidateSet is empty".to_owned());
    }
    Ok(states)
}

pub(super) fn workflow_state_invocations(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("-State "))
        .map(|state| state.trim().trim_end_matches('`').trim().to_owned())
        .collect()
}

pub(super) fn validate_v4_release_state_invocations(
    workflow: &str,
    pipeline: &str,
) -> std::result::Result<(), String> {
    let accepted_states = pipeline_validate_set_states(pipeline)?;
    for state in workflow_state_invocations(workflow) {
        if !accepted_states.contains(&state) {
            return Err(format!(
                "approved release workflow invokes state `{state}` not accepted by v4_release_pipeline.ps1"
            ));
        }
    }
    let jobs = workflow_job_blocks(workflow);
    let qualification_requirements: &[(&str, &[&str])] = &[
        (
            "Run canonical production Preflight without publication",
            &["-State Preflight", "-StateRoot $env:V4_RELEASE_STATE_ROOT"],
        ),
        (
            "Run canonical production BuildCandidate and stop",
            &[
                "-State BuildCandidate",
                "-StateRoot $env:V4_RELEASE_STATE_ROOT",
                "-UpdaterPrivateKeyPath $env:V4_UPDATER_PRIVATE_KEY_PATH",
            ],
        ),
    ];
    let (job_id, job_block, requirements) =
        if let Some((_, block)) = jobs.iter().find(|(job_id, _)| job_id == "release") {
            (
                "release",
                block.as_str(),
                V4_RELEASE_STATE_INVOCATION_REQUIREMENTS,
            )
        } else if let Some((_, block)) = jobs.iter().find(|(job_id, _)| job_id == "qualification") {
            ("qualification", block.as_str(), qualification_requirements)
        } else {
            return Err(
                "approved release workflow is missing its release or qualification job block"
                    .to_owned(),
            );
        };
    let steps = workflow_step_blocks(job_block);
    for &(step_name, required_arguments) in requirements {
        let step = steps
            .iter()
            .find(|(name, _)| name == step_name)
            .map(|(_, block)| block.as_str())
            .ok_or_else(|| format!("v4 {job_id} workflow is missing state step `{step_name}`"))?;
        for &required_argument in required_arguments {
            if !step.contains(required_argument) {
                return Err(format!(
                    "state step `{step_name}` is missing required CLI argument `{required_argument}`"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn workflow_declares_only_dispatch(source: &str) -> bool {
    let mut in_on = false;
    let mut found_dispatch = false;
    for line in source.lines() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        if !in_on {
            if indent == 0 && trimmed == "on:" {
                in_on = true;
            }
            continue;
        }
        if indent == 0 && !trimmed.is_empty() {
            break;
        }
        if indent == 2 && trimmed.ends_with(':') {
            let event = trimmed.trim_end_matches(':');
            if event == "workflow_dispatch" {
                found_dispatch = true;
            } else {
                return false;
            }
        }
    }
    found_dispatch
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn release_workflow_state_invocation_contract_rejects_missing_required_arguments() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(".github/workflows/release-v4.yml"))
            .expect("production release workflow fixture must exist");
        let pipeline = fs::read_to_string(root.join("scripts/v4_release_pipeline.ps1"))
            .expect("release pipeline fixture must exist");
        validate_v4_release_state_invocations(&source, &pipeline)
            .expect("production release workflow must satisfy every state CLI requirement");

        let remove_from_step = |source: &str, step_name: &str, argument: &str| {
            let step_marker = format!("      - name: {step_name}");
            let step_start = source
                .find(&step_marker)
                .expect("state step fixture must exist");
            let step_end = source[step_start..]
                .find("\n      - name: ")
                .map_or(source.len(), |offset| step_start + offset);
            let block = &source[step_start..step_end];
            let without_argument = block.replacen(argument, "", 1);
            assert_ne!(block, without_argument, "argument fixture must exist");
            format!(
                "{}{}{}",
                &source[..step_start],
                without_argument,
                &source[step_end..]
            )
        };

        for &(step_name, required_arguments) in V4_RELEASE_STATE_INVOCATION_REQUIREMENTS {
            for &required_argument in required_arguments {
                let missing = remove_from_step(&source, step_name, required_argument);
                let error = validate_v4_release_state_invocations(&missing, &pipeline)
                    .expect_err("missing state CLI arguments must fail the static contract");
                assert!(
                    error.contains(required_argument),
                    "diagnostic must identify the missing argument: {error}"
                );
            }
        }
    }
}

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

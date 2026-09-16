use std::process::Command;

pub(crate) fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn emit_git_metadata_rerun_if_changed(build_support_path: &str) {
    println!("cargo:rerun-if-changed={build_support_path}");

    for metadata in ["HEAD", "packed-refs"] {
        if let Some(path) = command_output("git", &["rev-parse", "--git-path", metadata]) {
            emit_watch_path(&path);
        }
    }

    if let Some(current_ref) = command_output("git", &["symbolic-ref", "--quiet", "HEAD"])
        && let Some(path) = command_output("git", &["rev-parse", "--git-path", &current_ref])
    {
        emit_watch_path(&path);
    }
}

fn emit_watch_path(path: &str) {
    let normalized = path
        .chars()
        .map(|character| match character {
            '\\' => '/',
            '\r' | '\n' => ' ',
            _ => character,
        })
        .collect::<String>();
    println!("cargo:rerun-if-changed={normalized}");
}

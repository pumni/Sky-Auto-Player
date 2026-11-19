use super::source::{strip_rust_comments, walk_source};
use crate::Result;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

const FORBIDDEN_SECURITY_APIS: &[&str] = &[
    "SetWindowsHookEx",
    "SetWindowsHookExA",
    "SetWindowsHookExW",
    "SetWinEventHook",
    "ReadProcessMemory",
    "WriteProcessMemory",
    "NtReadVirtualMemory",
    "NtWriteVirtualMemory",
    "VirtualAllocEx",
    "VirtualFreeEx",
    "VirtualProtectEx",
    "VirtualQueryEx",
    "CreateRemoteThread",
    "CreateRemoteThreadEx",
    "NtCreateThreadEx",
    "RtlCreateUserThread",
    "QueueUserAPC",
    "GetThreadContext",
    "SetThreadContext",
    "SuspendThread",
    "DebugActiveProcess",
    "DebugActiveProcessStop",
    "ContinueDebugEvent",
    "WaitForDebugEvent",
    "NtQueryInformationProcess",
    "keybd_event",
    "mouse_event",
];
const ALLOWED_WINDOWS_SYS_MODULES: &[&str] = &[
    "Win32::Foundation",
    "Win32::Media",
    "Win32::UI::Input",
    "Win32::System::Performance",
    // Playback lifecycle requests and explicit suspend/resume notification boundaries.
    "Win32::System::Power",
    "Win32::System::LibraryLoader",
    "Win32::System::SystemInformation",
    "Win32::System::Threading",
    "Win32::System::JobObjects",
    "Win32::System::Registry",
    "Win32::Security",
    "Win32::UI::Input::KeyboardAndMouse",
    "Win32::UI::Controls",
    "Win32::UI::Shell",
    "Win32::UI::HiDpi",
    "Win32::UI::WindowsAndMessaging",
    "Win32::Networking::WinHttp",
    "Win32::Storage::FileSystem",
];
const FORBIDDEN_DLLS: &[&str] = &["ntdll.dll"];
const APPROVED_FOREGROUND_WINEVENT_ADAPTER: &str =
    "rust/crates/sky_dispatch_win32/src/focus/foreground_watcher.rs";

#[derive(Clone, Debug, Eq, PartialEq)]
struct Finding {
    path: String,
    line: usize,
    rule: String,
    detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "{}:{} {}: {}",
            self.path, self.line, self.rule, self.detail
        )
    }
}

fn windows_sys_paths(line: &str) -> Vec<String> {
    let marker = "windows_sys::";
    let mut paths = Vec::new();
    let mut start = 0;
    while let Some(relative) = line[start..].find(marker) {
        let begin = start + relative + marker.len();
        let end = line[begin..]
            .find(|character: char| {
                !(character.is_ascii_alphanumeric() || character == '_' || character == ':')
            })
            .map_or(line.len(), |offset| begin + offset);
        paths.push(line[begin..end].trim_end_matches(':').to_owned());
        start = end.max(begin + 1);
    }
    paths
}

fn approved_windows_sys(source_path: &str, path: &str) -> bool {
    (source_path == APPROVED_FOREGROUND_WINEVENT_ADAPTER
        && (path == "Win32::UI::Accessibility" || path.starts_with("Win32::UI::Accessibility::")))
        || ALLOWED_WINDOWS_SYS_MODULES
            .iter()
            .any(|allowed| path == *allowed || path.starts_with(&format!("{allowed}::")))
}

fn call_arguments<'a>(source: &'a str, function: &str) -> Option<&'a str> {
    let call = source.find(&format!("{function}("))?;
    let arguments_start = call + function.len() + 1;
    let mut depth = 1usize;
    for (offset, character) in source[arguments_start..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[arguments_start..arguments_start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn scan_rust_text(path: &Path, source: &str) -> Vec<Finding> {
    let clean = strip_rust_comments(source);
    let relative = path.to_string_lossy().replace('\\', "/");
    let approved_foreground_adapter = relative == APPROVED_FOREGROUND_WINEVENT_ADAPTER;
    let mut findings = Vec::new();
    for (line_number, line) in clean.lines().enumerate() {
        for token in FORBIDDEN_SECURITY_APIS {
            if line
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|word| word == *token)
            {
                if *token == "SetWinEventHook" && approved_foreground_adapter {
                    continue;
                }
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: format!("forbidden-call:{token}"),
                    detail: format!("`{token}` violates SECURITY.md"),
                });
            }
        }
        if approved_foreground_adapter && line.contains("WINEVENT_INCONTEXT") {
            findings.push(Finding {
                path: relative.clone(),
                line: line_number + 1,
                rule: "forbidden-foreground-hook-flag".into(),
                detail: "foreground watcher must use WINEVENT_OUTOFCONTEXT only".into(),
            });
        }
        let lower = line.to_ascii_lowercase();
        for dll in FORBIDDEN_DLLS {
            if lower.contains(dll) {
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: "forbidden-dll-load".into(),
                    detail: format!("Rust reference to `{dll}` is forbidden"),
                });
            }
        }
        for module in windows_sys_paths(line) {
            if !approved_windows_sys(&relative, &module) {
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: "disallowed-windows-sys-module".into(),
                    detail: format!("`windows_sys::{module}` is outside the approved allowlist"),
                });
            }
        }
    }
    if approved_foreground_adapter {
        let valid_call = call_arguments(&clean, "SetWinEventHook").is_some_and(|arguments| {
            let actual_arguments = arguments
                .split(',')
                .filter(|argument| !argument.trim().is_empty())
                .map(|argument| argument.split_whitespace().collect::<String>())
                .collect::<Vec<_>>();
            actual_arguments.iter().map(String::as_str).eq([
                "EVENT_SYSTEM_FOREGROUND",
                "EVENT_SYSTEM_FOREGROUND",
                "std::ptr::null_mut()",
                "Some(foreground_event_callback)",
                "0",
                "0",
                "WINEVENT_OUTOFCONTEXT",
            ])
        });
        let valid_adapter = valid_call
            && clean.matches("SetWinEventHook(").count() == 1
            && clean.matches("UnhookWinEvent(").count() == 1
            && clean.contains("EVENT_SYSTEM_FOREGROUND")
            && clean.contains("WINEVENT_OUTOFCONTEXT")
            && !clean.contains("SetWindowsHookEx")
            && !clean.contains("WH_KEYBOARD")
            && !clean.contains("WINEVENT_INCONTEXT");
        if !valid_adapter {
            findings.push(Finding {
                path: relative,
                line: 1,
                rule: "invalid-approved-foreground-hook-adapter".into(),
                detail: "approved adapter must register and unhook exactly one foreground out-of-context event source".into(),
            });
        }
    }
    findings
}

fn security_findings(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for prefix in ["rust/crates", "apps/desktop/src-tauri"] {
        for path in walk_source(root, prefix)? {
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            findings.extend(scan_rust_text(relative, &fs::read_to_string(&path)?));
        }
    }
    Ok(findings)
}

fn security_baseline(root: &Path) -> Result<BTreeSet<(String, usize, String)>> {
    let path = root.join(".config/security_audit_baseline.json");
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    let mut entries = BTreeSet::new();
    for entry in payload
        .get("exceptions")
        .and_then(Value::as_array)
        .ok_or("security baseline exceptions must be an array")?
    {
        let object = entry
            .as_object()
            .ok_or("security baseline entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("security baseline path missing")?;
        let line = object
            .get("line")
            .and_then(Value::as_u64)
            .ok_or("security baseline line missing")? as usize;
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .ok_or("security baseline rule missing")?;
        entries.insert((path.replace('\\', "/"), line, rule.to_owned()));
    }
    Ok(entries)
}

pub(crate) fn security(root: &Path) -> Result<()> {
    let baseline = security_baseline(root)?;
    let findings = security_findings(root)?;
    let mut fresh = Vec::new();
    for finding in findings {
        let key = (finding.path.clone(), finding.line, finding.rule.clone());
        if !baseline.contains(&key) {
            fresh.push(finding);
        }
    }
    if let Some(finding) = fresh.first() {
        return Err(format!("security audit failed: {finding}").into());
    }
    println!(
        "[xtask] security checks: PASS ({} baseline-covered finding(s))",
        baseline.len()
    );
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    security(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_REPO_ID: AtomicU64 = AtomicU64::new(0);

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEMP_REPO_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "sky-xtask-security-audit-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => {
                        fs::create_dir_all(path.join("rust/crates"))
                            .expect("create Rust workspace source root");
                        fs::create_dir_all(path.join("apps/desktop/src-tauri"))
                            .expect("create native desktop source root");
                        return Self(path);
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create isolated audit fixture: {error}"),
                }
            }
            panic!("could not allocate isolated audit fixture directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, source: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("fixture file has a parent"))
                .expect("create fixture parent");
            fs::write(path, source).expect("write isolated audit fixture");
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn security_ignores_comments_but_flags_the_complete_forbidden_set() {
        let source = "// NtReadVirtualMemory and ntdll.dll are documentation only\n/* SetWindowsHookExW */\nunsafe { NtReadVirtualMemory(); DebugActiveProcessStop(); ContinueDebugEvent(); WaitForDebugEvent(); NtQueryInformationProcess(); keybd_event(); mouse_event(); }\n";
        let findings = scan_rust_text(Path::new("fixture.rs"), source);
        let rules = findings
            .iter()
            .map(|finding| finding.rule.as_str())
            .collect::<BTreeSet<_>>();
        assert!(rules.contains("forbidden-call:NtReadVirtualMemory"));
        assert!(rules.contains("forbidden-call:DebugActiveProcessStop"));
        assert!(rules.contains("forbidden-call:ContinueDebugEvent"));
        assert!(rules.contains("forbidden-call:WaitForDebugEvent"));
        assert!(rules.contains("forbidden-call:NtQueryInformationProcess"));
        assert!(rules.contains("forbidden-call:keybd_event"));
        assert!(rules.contains("forbidden-call:mouse_event"));
        assert!(
            !findings
                .iter()
                .any(|finding| finding.rule == "forbidden-call:SetWindowsHookExW")
        );
        assert!(
            !findings
                .iter()
                .any(|finding| finding.rule == "forbidden-dll-load")
        );
    }

    #[test]
    fn security_allows_sendinput_and_approved_windows_modules() {
        let source = "unsafe { SendInput(1, inputs, size); }\nuse windows_sys::Win32::Foundation::HANDLE;\nuse windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;\nuse windows_sys::Win32::System::Threading::CloseHandle;\nuse windows_sys::Win32::System::Registry::RegOpenKeyExW;\n";
        assert!(scan_rust_text(Path::new("fixture.rs"), source).is_empty());
    }

    fn approved_foreground_watcher_source(flag: &str) -> String {
        format!(
            "use windows_sys::Win32::UI::Accessibility::{{SetWinEventHook, UnhookWinEvent}};\nunsafe {{ SetWinEventHook(\n EVENT_SYSTEM_FOREGROUND,\n EVENT_SYSTEM_FOREGROUND,\n std::ptr::null_mut(),\n Some(foreground_event_callback),\n 0,\n 0,\n {flag},\n); }}\nunsafe {{ UnhookWinEvent(hook); }}\n"
        )
    }

    #[test]
    fn security_allows_only_the_exact_foreground_winevent_adapter() {
        let approved = approved_foreground_watcher_source("WINEVENT_OUTOFCONTEXT");
        assert!(
            scan_rust_text(Path::new(APPROVED_FOREGROUND_WINEVENT_ADAPTER), &approved).is_empty(),
            "approved fixture findings: {:?}",
            scan_rust_text(Path::new(APPROVED_FOREGROUND_WINEVENT_ADAPTER), &approved)
        );

        let unapproved_path = Path::new("rust/crates/sky_player/src/worker.rs");
        assert!(
            scan_rust_text(unapproved_path, &approved)
                .iter()
                .any(|finding| finding.rule == "forbidden-call:SetWinEventHook")
        );
    }

    #[test]
    fn security_rejects_in_context_and_keyboard_hooks_in_the_approved_adapter() {
        let in_context = approved_foreground_watcher_source("WINEVENT_INCONTEXT");
        let findings = scan_rust_text(Path::new(APPROVED_FOREGROUND_WINEVENT_ADAPTER), &in_context);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "forbidden-foreground-hook-flag")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "invalid-approved-foreground-hook-adapter")
        );

        let keyboard = approved_foreground_watcher_source("WINEVENT_OUTOFCONTEXT").replace(
            "UnhookWinEvent(hook)",
            "SetWindowsHookExW(WH_KEYBOARD_LL, callback, module, 0)",
        );
        let findings = scan_rust_text(Path::new(APPROVED_FOREGROUND_WINEVENT_ADAPTER), &keyboard);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "forbidden-call:SetWindowsHookExW")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "invalid-approved-foreground-hook-adapter")
        );
    }

    #[test]
    fn security_rejects_process_status_for_all_sources() {
        let usage = "use windows_sys::Win32::System::ProcessStatus::GetProcessMemoryInfo;";
        for path in [
            "rust/crates/sky_player/examples/rt_prepared_process_probe.rs",
            "rust/crates/sky_player/src/lib.rs",
            "rust/crates/sky_dispatch_win32/src/input.rs",
        ] {
            assert!(
                scan_rust_text(Path::new(path), usage)
                    .iter()
                    .any(|finding| finding.rule == "disallowed-windows-sys-module"),
                "{path} must not bypass the security module allowlist"
            );
        }
    }

    #[test]
    fn security_rejects_unapproved_windows_diagnostics_module_and_ntdll() {
        let source = "use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;\nlet name = \"ntdll.dll\";\n";
        let findings = scan_rust_text(Path::new("fixture.rs"), source);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "disallowed-windows-sys-module")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "forbidden-dll-load")
        );
    }

    #[test]
    fn security_finding_scan_covers_rust_workspace_and_native_desktop_sources() {
        let repo = TempRepo::new();
        for path in [
            "rust/crates/fixture/src/lib.rs",
            "apps/desktop/src-tauri/src/native.rs",
        ] {
            repo.write(path, "unsafe { NtReadVirtualMemory(); }\n");
        }

        let findings = security_findings(repo.path()).expect("active Rust roots are traversable");
        for path in [
            "rust/crates/fixture/src/lib.rs",
            "apps/desktop/src-tauri/src/native.rs",
        ] {
            assert!(
                findings.iter().any(|finding| finding.path == path),
                "security scanner omitted {path}: {findings:?}"
            );
        }
    }

    #[test]
    fn security_finding_scan_fails_closed_on_unreadable_rust_sources() {
        let repo = TempRepo::new();
        let source = repo.path().join("rust/crates/fixture/src/lib.rs");
        fs::create_dir_all(source.parent().expect("fixture source has a parent"))
            .expect("create fixture source directory");
        fs::write(source, [0xff]).expect("write invalid UTF-8 source");

        let error = security_findings(repo.path())
            .expect_err("unreadable required Rust source must fail closed")
            .to_string()
            .to_ascii_lowercase();
        assert!(error.contains("utf-8"), "unexpected read error: {error}");
    }

    #[test]
    fn security_finding_scan_ignores_comments_and_keeps_sendinput_available() {
        let repo = TempRepo::new();
        repo.write(
            "rust/crates/fixture/src/lib.rs",
            "// NtReadVirtualMemory is documentation only\nunsafe { SendInput(1, inputs, size); }\n",
        );
        repo.write(
            "apps/desktop/src-tauri/src/native.rs",
            "use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;\n",
        );

        assert!(
            security_findings(repo.path())
                .expect("active Rust roots are traversable")
                .is_empty()
        );
    }
}

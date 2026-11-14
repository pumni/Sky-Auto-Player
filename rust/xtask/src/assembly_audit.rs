#![forbid(unsafe_code)]

use crate::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn assembly_file_name_matches(path: &Path, stem: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let folded = name.to_ascii_lowercase();
    let Some(suffix) = folded.strip_prefix(stem) else {
        return false;
    };
    let suffix = suffix.strip_suffix(".s").unwrap_or("");
    (suffix.is_empty() && folded == format!("{stem}.s"))
        || (suffix.starts_with('-')
            && suffix.len() > 1
            && suffix[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn locate_assembly(root: &Path, explicit: Option<&str>, stem: &str) -> Result<PathBuf> {
    let path = if let Some(explicit) = explicit {
        PathBuf::from(explicit)
    } else {
        let directory = root.join("rust/target/x86_64-pc-windows-msvc/dist/deps");
        let mut candidates = fs::read_dir(&directory)
            .map_err(|error| {
                format!(
                    "could not inspect Windows x64 dist assembly directory {}: {error}",
                    directory.display()
                )
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file() && assembly_file_name_matches(path, stem))
            .collect::<Vec<_>>();
        candidates.sort();
        if candidates.len() != 1 {
            return Err(format!(
                "expected one authoritative {stem} Windows x64 dist assembly in {}, found {}",
                directory.display(),
                candidates.len()
            )
            .into());
        }
        candidates.remove(0)
    };
    let resolved = fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve assembly {}: {error}", path.display()))?;
    if !assembly_file_name_matches(&resolved, stem) {
        return Err(format!(
            "expected authoritative {stem} assembly output, got {}",
            resolved.display()
        )
        .into());
    }
    Ok(resolved)
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    Ok(fs::read_to_string(path)
        .map_err(|error| format!("could not read assembly {}: {error}", path.display()))?
        .lines()
        .map(str::to_owned)
        .collect())
}

fn find_unique_function_body(
    lines: &[String],
    label_matches: impl Fn(&str) -> bool,
    description: &str,
) -> Result<(usize, usize)> {
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| label_matches(line).then_some(index))
        .collect::<Vec<_>>();
    if starts.len() != 1 {
        return Err(format!(
            "expected one emitted assembly function for {description}, found {}",
            starts.len()
        )
        .into());
    }
    let start = starts[0];
    let Some(end_offset) = lines[start + 1..]
        .iter()
        .position(|line| line.trim() == ".seh_endproc")
    else {
        return Err(format!("assembly function for {description} has no .seh_endproc").into());
    };
    Ok((start, start + 1 + end_offset))
}

fn body(lines: &[String], range: (usize, usize)) -> &[String] {
    &lines[range.0..=range.1]
}

fn is_call_to(line: &str, fragment: &str) -> bool {
    let folded = line.to_ascii_lowercase();
    folded.contains("callq") && folded.contains(&fragment.to_ascii_lowercase())
}

fn call_indices(lines: &[String], fragment: &str) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_call_to(line, fragment).then_some(index))
        .collect()
}

fn is_call_instruction(line: &str) -> bool {
    let instruction = line.trim_start().to_ascii_lowercase();
    instruction.starts_with("call ")
        || instruction.starts_with("callq ")
        || instruction.starts_with("call\t")
        || instruction.starts_with("callq\t")
}

fn is_physical_send_reference(line: &str) -> bool {
    let folded = line.trim().to_ascii_lowercase();
    if folded.contains("__imp_sendinput") {
        return true;
    }
    let Some(target) = folded
        .strip_prefix("callq")
        .or_else(|| folded.strip_prefix("call"))
    else {
        return false;
    };
    let target = target
        .trim_start()
        .trim_start_matches('*')
        .trim_start_matches('_');
    let direct_sendinput = target.starts_with("sendinput")
        && target["sendinput".len()..]
            .chars()
            .next()
            .is_none_or(|suffix| !suffix.is_ascii_alphanumeric());
    direct_sendinput
        || [
            "send_prepared_physical_packet",
            "send_physical_packet",
            "send_packet",
            "send_keyboard_packet",
        ]
        .iter()
        .any(|needle| target.contains(needle))
}

fn is_forbidden_suffix_effect(line: &str) -> bool {
    let folded = line.trim_start().to_ascii_lowercase();
    let forbidden_symbols = [
        "__rust_alloc",
        "__rust_realloc",
        "__rust_dealloc",
        "malloc",
        "calloc",
        "realloc",
        "heapalloc",
        "heapfree",
        "memcpy",
        "memmove",
        "memset",
        "memcpy_inline",
        "memmove_inline",
        "criticalsection",
        "srwlock",
        "mutex",
        "prepare_physical_packet",
        "materialize_physical_packet",
        "compile_physical_packet",
    ];
    forbidden_symbols
        .iter()
        .any(|needle| folded.contains(needle))
        || folded.starts_with("lock ")
        || folded.starts_with("lock\t")
        || folded.starts_with("rep movs")
        || folded.starts_with("rep stos")
        || [
            "movsb", "movsw", "movsl", "movsq", "stosb", "stosw", "stosl", "stosq",
        ]
        .iter()
        .any(|mnemonic| {
            folded
                .split_whitespace()
                .next()
                .is_some_and(|actual| actual == *mnemonic)
        })
        || (folded.starts_with("subq ") && folded.contains("%rsp"))
        || (folded.starts_with("andq ") && folded.contains("%rsp"))
}

fn validate_scoped_send_suffix(
    function: &[String],
    final_owner: usize,
    deferred_sender: usize,
) -> Result<()> {
    for (index, line) in function.iter().enumerate() {
        if index != deferred_sender && is_physical_send_reference(line) {
            return Err(format!(
                "production dispatch boundary contains an additional physical-send reference at line {}: {}",
                index + 1,
                line.trim()
            )
            .into());
        }
    }

    for (index, line) in function[final_owner..deferred_sender].iter().enumerate() {
        let function_index = final_owner + index;
        if is_forbidden_suffix_effect(line) {
            return Err(format!(
                "scoped final-admission→sender suffix contains allocation/copy/lock/packet-build work at line {}: {}",
                function_index + 1,
                line.trim()
            )
            .into());
        }
        if is_call_instruction(line)
            && !(function_index == final_owner
                && line
                    .to_ascii_lowercase()
                    .contains("foreground_window_owner_matches"))
        {
            return Err(format!(
                "scoped final-admission→sender suffix contains an unexpected call at line {}: {}",
                function_index + 1,
                line.trim()
            )
            .into());
        }
    }
    Ok(())
}

fn production_dispatch_range(lines: &[String]) -> Result<(usize, usize)> {
    find_unique_function_body(
        lines,
        |line| {
            line.starts_with(".seh_proc _")
                && line.contains("dispatch_staged_candidate")
                && !line.contains("?dtor$")
        },
        "DispatchExecutor production dispatch_staged_candidate boundary",
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DispatchAssemblyShape {
    dispatch_lines: (usize, usize),
    final_owner_rechecks: usize,
    modifier_queries: usize,
    deferred_sender_calls: usize,
    executor_completion_commits: usize,
    journal_commits: usize,
    send_suffix_lines: (usize, usize),
}

fn validate_production_dispatch_assembly(lines: &[String]) -> Result<DispatchAssemblyShape> {
    let range = production_dispatch_range(lines)?;
    let function = body(lines, range);
    let senders = call_indices(
        function,
        "send_prepared_physical_packet_at_final_boundary_deferred_commit",
    );
    if senders.len() != 1 {
        return Err(format!(
            "production dispatch boundary must have one deferred-commit sender call, found {}",
            senders.len()
        )
        .into());
    }
    let sender = senders[0];
    let owner_checks = call_indices(function, "foreground_window_owner_matches")
        .into_iter()
        .filter(|index| *index < sender)
        .count();
    let modifier_queries = call_indices(function, "query_async_key_state")
        .into_iter()
        .filter(|index| *index < sender)
        .count();
    if owner_checks == 0 || modifier_queries != 5 {
        return Err(format!(
            "final Down gate must retain a pre-send owner check and five fixed modifier queries, found owner={owner_checks} modifiers={modifier_queries}"
        )
        .into());
    }

    let executor_commits = call_indices(function, "DispatchExecutor20on_send_completed_at");
    let journal_commits = call_indices(function, "commit_physical_packet");
    if executor_commits.len() != 1
        || journal_commits.len() != 1
        || !(sender < executor_commits[0] && executor_commits[0] < journal_commits[0])
    {
        return Err(format!(
            "deferred sender must precede one live scheduler completion commit and one journal commit; sender={}, scheduler={}, journal={}",
            sender,
            executor_commits.len(),
            journal_commits.len()
        )
        .into());
    }

    let final_owner = call_indices(function, "foreground_window_owner_matches")
        .into_iter()
        .filter(|index| *index < sender)
        .max()
        .expect("checked owner recheck");
    validate_scoped_send_suffix(function, final_owner, sender)?;
    Ok(DispatchAssemblyShape {
        dispatch_lines: (range.0 + 1, range.1 + 1),
        final_owner_rechecks: owner_checks,
        modifier_queries,
        deferred_sender_calls: senders.len(),
        executor_completion_commits: executor_commits.len(),
        journal_commits: journal_commits.len(),
        send_suffix_lines: (range.0 + final_owner + 1, range.0 + sender + 1),
    })
}

fn require_source_contract(root: &Path) -> Result<()> {
    let dispatch_path = root.join("rust/crates/sky_player/src/engine/worker/dispatch_loop.rs");
    let dispatch = fs::read_to_string(&dispatch_path)
        .map_err(|error| format!("could not read production dispatch source: {error}"))?;
    let ordered = [
        "verify_final_down_admission(",
        ".send_prepared_physical_packet_at_final_boundary_deferred_commit(",
        ".on_send_completed_at(",
        ".commit_physical_packet(",
    ];
    let mut previous = None;
    for required in ordered {
        let Some(index) = dispatch.find(required) else {
            return Err(format!("production dispatch source lost '{required}'").into());
        };
        if previous.is_some_and(|previous| index <= previous) {
            return Err("production source order must be final admission → deferred send → scheduler commit → journal commit".into());
        }
        previous = Some(index);
    }

    let admission_path = root.join("rust/crates/sky_player/src/engine/worker/admission.rs");
    let admission = fs::read_to_string(&admission_path)
        .map_err(|error| format!("could not read final admission source: {error}"))?;
    for required in [
        "target_stamp_still_current",
        "foreground_window_owner_matches",
        "owner_identity_status",
        "final_down_control_verdict",
        "final_down_modifier_verdict",
        "final_down_fresh_target_admission",
        "final_down_post_modifier_verdict",
        "lease_timeout_ticks",
        "system_power",
    ] {
        if !admission.contains(required) {
            return Err(format!("final Down admission source lost '{required}'").into());
        }
    }
    Ok(())
}

fn report_dispatch_audit(root: &Path, assembly_argument: Option<&str>, alias: &str) -> Result<()> {
    let path = locate_assembly(root, assembly_argument, "sky_player")?;
    let lines = read_lines(&path)?;
    let shape = validate_production_dispatch_assembly(&lines)?;
    require_source_contract(root)?;
    println!("assembly={}", path.display());
    println!("audit={alias}");
    println!("boundary=DispatchExecutor::dispatch_staged_candidate");
    println!(
        "dispatch_lines={}-{}",
        shape.dispatch_lines.0, shape.dispatch_lines.1
    );
    println!(
        "pre_send_target_owner_rechecks={}",
        shape.final_owner_rechecks
    );
    println!("pre_send_fixed_modifier_queries={}", shape.modifier_queries);
    println!("deferred_send_calls={}", shape.deferred_sender_calls);
    println!(
        "post_send_live_scheduler_completion_commits={}",
        shape.executor_completion_commits
    );
    println!(
        "post_scheduler_send_journal_commits={}",
        shape.journal_commits
    );
    println!(
        "scoped_send_suffix_lines={}-{}; non_owner_calls=0; forbidden_effects=0; worker-wide copy/division thresholds=not applied",
        shape.send_suffix_lines.0, shape.send_suffix_lines.1
    );
    println!("production dispatch assembly audit: PASS");
    Ok(())
}

fn parse_modifier_vk_names(source: &str) -> Result<Vec<String>> {
    let start = source
        .find("const MODIFIER_VKS")
        .ok_or("fixed modifier VK array was not found in source")?;
    let array = &source[start..];
    let end = array
        .find("];")
        .ok_or("fixed modifier VK array was not closed in source")?;
    let array = &array[..end];
    let mut names = Vec::new();
    for (start, _) in array.match_indices("(VK_") {
        let suffix = &array[start + 1..];
        let count = suffix
            .bytes()
            .take_while(|byte| byte.is_ascii_uppercase() || *byte == b'_')
            .count();
        if count > 3 && suffix.as_bytes().get(count) == Some(&b',') {
            names.push(suffix[..count].to_owned());
        }
    }
    Ok(names)
}

fn validate_win32_sender_assembly(lines: &[String]) -> Result<(usize, usize, usize)> {
    let deferred = find_unique_function_body(
        lines,
        |line| {
            line.starts_with(".seh_proc _")
                && line.contains("send_prepared_physical_packet_at_final_boundary_deferred_commit")
        },
        "TrackedKeyState deferred-commit sender",
    )?;
    let policy = find_unique_function_body(
        lines,
        |line| {
            line.starts_with(".seh_proc _")
                && line.contains("send_prepared_physical_packet_view_with_commit_policy")
        },
        "tracked packet send commit-policy boundary",
    )?;
    let deferred_body = body(lines, deferred);
    let policy_body = body(lines, policy);
    let deferred_calls = call_indices(
        deferred_body,
        "send_prepared_physical_packet_view_with_commit_policy",
    )
    .len();
    let sendinput_indices = policy_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (line.to_ascii_lowercase().contains("callq") && line.contains("__imp_SendInput"))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let qpc_indices = policy_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (line.to_ascii_lowercase().contains("callq")
                && line.contains("__imp_QueryPerformanceCounter"))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if deferred_calls != 1
        || sendinput_indices.len() != 1
        || qpc_indices.len() != 2
        || !(qpc_indices[0] < sendinput_indices[0] && sendinput_indices[0] < qpc_indices[1])
    {
        return Err(format!(
            "Win32 deferred sender path must reach one commit-policy boundary with one SendInput between pre-call and completion QPC samples; deferred_calls={deferred_calls}, SendInput={}, QPC={} (positions: {qpc_indices:?}, {sendinput_indices:?})",
            sendinput_indices.len(),
            qpc_indices.len()
        )
        .into());
    }
    Ok((deferred_calls, sendinput_indices.len(), qpc_indices.len()))
}

fn modifier_guard_audit(root: &Path, sky_argument: &str, win32_argument: &str) -> Result<()> {
    let sky_path = locate_assembly(root, Some(sky_argument), "sky_player")?;
    let win32_path = locate_assembly(root, Some(win32_argument), "sky_dispatch_win32")?;
    let source_path = root.join("rust/crates/sky_dispatch_win32/src/input/modifier_guard.rs");
    let source = fs::read_to_string(&source_path)
        .map_err(|error| format!("could not read fixed modifier source: {error}"))?;
    let modifier_names = parse_modifier_vk_names(&source)?;
    let expected = ["VK_LWIN", "VK_RWIN", "VK_CONTROL", "VK_SHIFT", "VK_MENU"];
    if modifier_names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != expected
    {
        return Err(format!("fixed modifier order changed: {modifier_names:?}").into());
    }

    let sky_lines = read_lines(&sky_path)?;
    let dispatch = validate_production_dispatch_assembly(&sky_lines)?;
    let win32_lines = read_lines(&win32_path)?;
    let query = find_unique_function_body(
        &win32_lines,
        |line| line.starts_with(".seh_proc _") && line.contains("query_async_key_state"),
        "fixed modifier query wrapper",
    )?;
    let query_body = body(&win32_lines, query);
    let win32_query_calls = query_body
        .iter()
        .filter(|line| line.to_ascii_lowercase().contains("callq"))
        .count();
    let get_async_calls = query_body
        .iter()
        .filter(|line| {
            line.to_ascii_lowercase().contains("callq") && line.contains("__imp_GetAsyncKeyState")
        })
        .count();
    if win32_query_calls != 1 || get_async_calls != 1 {
        return Err(format!(
            "fixed modifier wrapper must make exactly one GetAsyncKeyState call, found calls={win32_query_calls} import={get_async_calls}"
        )
        .into());
    }

    let (deferred_calls, sendinput_calls, qpc_calls) =
        validate_win32_sender_assembly(&win32_lines)?;
    println!("sky_player_assembly={}", sky_path.display());
    println!("sky_dispatch_win32_assembly={}", win32_path.display());
    println!("fixed_modifier_order={}", modifier_names.join(","));
    println!(
        "live_dispatch_pre_send_modifier_queries={}",
        dispatch.modifier_queries
    );
    println!("modifier_query_GetAsyncKeyState_calls={get_async_calls}");
    println!("deferred_sender_commit_policy_calls={deferred_calls}");
    println!("deferred_sender_SendInput_calls={sendinput_calls}");
    println!("deferred_sender_QPC_calls={qpc_calls}");
    println!("production modifier and Win32 sender assembly audit: PASS");
    Ok(())
}

fn parse_options(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>> {
    let mut options = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("assembly audit option must start with --: {}", args[index]))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown assembly audit option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!("assembly audit option was repeated: --{name}").into());
        }
        index += 1;
    }
    Ok(options)
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("assembly audit requires dispatch, prepared-normal alias, or modifier-guard")?;
    match operation {
        "dispatch" | "prepared-normal" => {
            let options = parse_options(&args[1..], &["assembly-path"])?;
            report_dispatch_audit(
                root,
                options.get("assembly-path").map(String::as_str),
                operation,
            )
        }
        "modifier-guard" => {
            let options = parse_options(
                &args[1..],
                &["sky-player-assembly-path", "win32-assembly-path"],
            )?;
            let sky = options
                .get("sky-player-assembly-path")
                .ok_or("modifier-guard requires --sky-player-assembly-path")?;
            let win32 = options
                .get("win32-assembly-path")
                .ok_or("modifier-guard requires --win32-assembly-path")?;
            modifier_guard_audit(root, sky, win32)
        }
        other => Err(format!("unknown assembly audit operation: {other}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn valid_dispatch() -> Vec<String> {
        fixture(
            ".seh_proc _fixture_dispatch_staged_candidate\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq record_sendinput_pre_call_lateness\n\
             callq foreground_window_owner_matches\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             .seh_endproc",
        )
    }

    #[test]
    fn assembly_path_accepts_windows_x64_dist_crate_hashes_only() {
        assert!(assembly_file_name_matches(
            Path::new("sky_player-d67d4fe1c153db33.s"),
            "sky_player"
        ));
        assert!(assembly_file_name_matches(
            Path::new("sky_dispatch_win32-a679f6aba20cc301.s"),
            "sky_dispatch_win32"
        ));
        assert!(!assembly_file_name_matches(
            Path::new("sky_player-extra.s"),
            "sky_player"
        ));
    }

    #[test]
    fn production_dispatch_assembly_requires_admission_sender_scheduler_and_journal_order() {
        let shape = validate_production_dispatch_assembly(&valid_dispatch()).unwrap();
        assert_eq!(shape.deferred_sender_calls, 1);
        assert_eq!(shape.executor_completion_commits, 1);
        assert_eq!(shape.journal_commits, 1);
        assert_eq!(shape.modifier_queries, 5);
        assert_eq!(shape.final_owner_rechecks, 1);

        let missing_boundary = fixture(".seh_proc _dispatch_old_path\n.seh_endproc");
        assert!(validate_production_dispatch_assembly(&missing_boundary).is_err());

        let missing_owner = valid_dispatch()
            .into_iter()
            .filter(|line| !line.contains("foreground_window_owner_matches"))
            .collect::<Vec<_>>();
        assert!(validate_production_dispatch_assembly(&missing_owner).is_err());

        let double_sender = fixture(
            ".seh_proc _fixture_dispatch_staged_candidate\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\n\
             callq foreground_window_owner_matches\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n.seh_endproc",
        );
        assert!(validate_production_dispatch_assembly(&double_sender).is_err());

        let wrong_commit_order = fixture(
            ".seh_proc _fixture_dispatch_staged_candidate\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\n\
             callq foreground_window_owner_matches\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq commit_physical_packet\n.seh_endproc",
        );
        assert!(validate_production_dispatch_assembly(&wrong_commit_order).is_err());

        for unexpected_effect in [
            "callq *__imp_SendInput",
            "callq __rust_alloc",
            "callq memcpy",
        ] {
            let mut injected = valid_dispatch();
            let sender = injected
                .iter()
                .position(|line| {
                    line.contains("send_prepared_physical_packet_at_final_boundary_deferred_commit")
                })
                .expect("fixture deferred sender");
            injected.insert(sender, unexpected_effect.to_owned());
            assert!(
                validate_production_dispatch_assembly(&injected).is_err(),
                "production dispatch assembly accepted injected effect: {unexpected_effect}"
            );
        }

        let diagnostic_only = fixture(
            ".seh_proc _fixture_dispatch_staged_candidate\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\ncallq query_async_key_state\n\
             callq query_async_key_state\n\
             callq record_sendinput_pre_call_lateness\n\
             callq foreground_window_owner_matches\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n.seh_endproc",
        );
        assert!(validate_production_dispatch_assembly(&diagnostic_only).is_ok());
    }

    #[test]
    fn dispatch_audit_does_not_require_retired_coordinator_or_prepared_symbols() {
        let assembly = valid_dispatch();
        let text = assembly.join("\n");
        assert!(!text.contains("dispatch_due_from_plan"));
        assert!(!text.contains("dispatch_prepared_normal_frame"));
        assert!(!text.contains("recover_missed_down_boundary"));
        assert!(validate_production_dispatch_assembly(&assembly).is_ok());
    }

    #[test]
    fn modifier_source_contract_keeps_fixed_key_order() {
        let source = "const MODIFIER_VKS: [VirtualKey; 5] = [(VK_LWIN, true), (VK_RWIN, true), (VK_CONTROL, false), (VK_SHIFT, false), (VK_MENU, false)];";
        assert_eq!(
            parse_modifier_vk_names(source).unwrap(),
            ["VK_LWIN", "VK_RWIN", "VK_CONTROL", "VK_SHIFT", "VK_MENU"]
        );
        assert!(
            parse_modifier_vk_names("const MODIFIER_VKS: [VirtualKey; 5] = [];")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn win32_sender_requires_deferred_commit_and_one_sendinput_boundary() {
        let assembly = fixture(
            ".seh_proc _TrackedKeyState_send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_view_with_commit_policy\n.seh_endproc\n\
             .seh_proc _TrackedKeyState_send_prepared_physical_packet_view_with_commit_policy\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_SendInput\n\
             callq *__imp_QueryPerformanceCounter\n.seh_endproc",
        );
        assert_eq!(
            validate_win32_sender_assembly(&assembly).unwrap(),
            (1, 1, 2)
        );

        let double_send = fixture(
            ".seh_proc _TrackedKeyState_send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_view_with_commit_policy\n.seh_endproc\n\
             .seh_proc _TrackedKeyState_send_prepared_physical_packet_view_with_commit_policy\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_SendInput\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_SendInput\n.seh_endproc",
        );
        assert!(validate_win32_sender_assembly(&double_send).is_err());

        let missing_completion_sample = fixture(
            ".seh_proc _TrackedKeyState_send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_view_with_commit_policy\n.seh_endproc\n\
             .seh_proc _TrackedKeyState_send_prepared_physical_packet_view_with_commit_policy\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_SendInput\n.seh_endproc",
        );
        assert!(validate_win32_sender_assembly(&missing_completion_sample).is_err());

        let samples_before_send = fixture(
            ".seh_proc _TrackedKeyState_send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_view_with_commit_policy\n.seh_endproc\n\
             .seh_proc _TrackedKeyState_send_prepared_physical_packet_view_with_commit_policy\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_QueryPerformanceCounter\n\
             callq *__imp_SendInput\n.seh_endproc",
        );
        assert!(validate_win32_sender_assembly(&samples_before_send).is_err());
    }
}

#![forbid(unsafe_code)]

use crate::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_word(line: &str, token: &str) -> bool {
    let folded_line = line.to_ascii_lowercase();
    let folded_token = token.to_ascii_lowercase();
    folded_line
        .match_indices(&folded_token)
        .any(|(start, matched)| {
            let end = start + matched.len();
            let before = start
                .checked_sub(1)
                .and_then(|index| folded_line.as_bytes().get(index))
                .copied();
            let after = folded_line.as_bytes().get(end).copied();
            !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
        })
}

fn contains_word_with_suffix(line: &str, token: &str) -> bool {
    let folded_line = line.to_ascii_lowercase();
    let folded_token = token.to_ascii_lowercase();
    folded_line
        .match_indices(&folded_token)
        .any(|(start, matched)| {
            let token_end = start + matched.len();
            let before = start
                .checked_sub(1)
                .and_then(|index| folded_line.as_bytes().get(index))
                .copied();
            let suffix_len = folded_line.as_bytes()[token_end..]
                .iter()
                .take_while(|byte| is_word_byte(**byte))
                .count();
            let end = token_end + suffix_len;
            let after = folded_line.as_bytes().get(end).copied();
            !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
        })
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn ends_with_ascii_case_insensitive(value: &str, suffix: &str) -> bool {
    value
        .as_bytes()
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix.as_bytes()))
}

fn is_call_to(line: &str, fragment: &str) -> bool {
    let folded = line.to_ascii_lowercase();
    folded
        .find("callq")
        .is_some_and(|index| folded[index..].contains(&fragment.to_ascii_lowercase()))
}

fn is_call_sequence(line: &str, fragments: &[&str]) -> bool {
    let folded = line.to_ascii_lowercase();
    let mut offset = 0;
    for fragment in fragments {
        let fragment = fragment.to_ascii_lowercase();
        let Some(index) = folded[offset..].find(&fragment) else {
            return false;
        };
        offset += index + fragment.len();
    }
    true
}

fn assembly_file_name_matches(path: &Path, stem: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let folded_name = name.to_ascii_lowercase();
    let Some(suffix) = folded_name.strip_prefix(stem) else {
        return false;
    };
    let suffix = suffix.strip_suffix(".s").unwrap_or("");
    if folded_name != format!("{stem}.s") && suffix.is_empty() {
        return false;
    }
    suffix.is_empty()
        || (suffix.starts_with('-')
            && !suffix[1..].is_empty()
            && suffix[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn locate_assembly(root: &Path, explicit: Option<&str>, stem: &str) -> Result<PathBuf> {
    let path = if let Some(path) = explicit {
        PathBuf::from(path)
    } else {
        let directory = root.join("rust/target/dist/deps");
        let mut candidates = fs::read_dir(&directory)
            .map_err(|error| {
                format!(
                    "could not inspect authoritative {stem} dist assembly directory {}: {error}",
                    directory.display()
                )
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file() && assembly_file_name_matches(path, stem))
            .collect::<Vec<_>>();
        candidates.sort();
        if candidates.len() != 1 {
            return Err(format!(
                "expected exactly one authoritative {stem} dist assembly in {}, found {}",
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
            "assembly audit must inspect authoritative {stem} output, got {}",
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

fn display_assembly_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc_path) = value.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{unc_path}");
    }
    value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned()
}

fn find_function_body(
    lines: &[String],
    label_matches: impl Fn(&str) -> bool,
    description: &str,
) -> Result<Option<(usize, usize)>> {
    let Some(start) = lines.iter().position(|line| label_matches(line)) else {
        return Ok(None);
    };
    let Some(offset) = lines[start + 1..]
        .iter()
        .position(|line| line.trim() == ".seh_endproc")
    else {
        return Err(format!("No .seh_endproc found for {description}").into());
    };
    Ok(Some((start, start + 1 + offset)))
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
            "expected exactly one assembly function matching '{description}', found {}",
            starts.len()
        )
        .into());
    }
    let start = starts[0];
    let Some(offset) = lines[start + 1..]
        .iter()
        .position(|line| line.trim() == ".seh_endproc")
    else {
        return Err(format!("no .seh_endproc found for '{description}'").into());
    };
    Ok((start, start + 1 + offset))
}

fn named_body(lines: &[String], fragment: &str) -> Result<Option<(usize, usize)>> {
    find_function_body(
        lines,
        |line| line.len() < 400 && line.trim_end().ends_with(fragment),
        fragment,
    )
}

fn prepared_named_body(lines: &[String], fragment: &str) -> Result<Option<(usize, usize)>> {
    find_function_body(lines, |line| line.trim_end().ends_with(fragment), fragment)
}

fn body(lines: &[String], range: (usize, usize)) -> &[String] {
    &lines[range.0..=range.1]
}

fn count_word(lines: &[String], token: &str) -> usize {
    lines
        .iter()
        .filter(|line| contains_word(line, token))
        .count()
}

fn is_instruction(line: &str) -> bool {
    let instruction = line.trim_start();
    let Some(first) = instruction.as_bytes().first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    let length = instruction
        .bytes()
        .take_while(u8::is_ascii_alphanumeric)
        .count();
    length == instruction.len() || instruction.as_bytes()[length].is_ascii_whitespace()
}

fn stack_allocations(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            let folded = line.to_ascii_lowercase();
            let (_, suffix) = folded.split_once(".seh_stackalloc")?;
            let digits = suffix
                .trim_start()
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            (digits > 0).then(|| suffix.trim_start()[..digits].to_owned())
        })
        .collect()
}

fn is_division_instruction(line: &str) -> bool {
    let folded = line.to_ascii_lowercase();
    let trimmed = folded.trim_start();
    let opcode = trimmed.split_ascii_whitespace().next().unwrap_or_default();
    [
        "divb", "divw", "divl", "divq", "udivb", "udivw", "udivl", "udivq", "idivb", "idivw",
        "idivl", "idivq",
    ]
    .contains(&opcode)
        && trimmed
            .as_bytes()
            .get(opcode.len())
            .is_some_and(u8::is_ascii_whitespace)
}

fn copy_size_immediates(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            let folded = line.to_ascii_lowercase();
            let (_, suffix) = folded.split_once("movl")?;
            let suffix = suffix.trim_start().strip_prefix('$')?;
            let (number, destination) = suffix.split_once(',')?;
            (number.bytes().all(|byte| byte.is_ascii_digit())
                && destination.chars().next().is_some_and(char::is_whitespace)
                && destination.trim() == "%r8d")
                .then(|| number.to_owned())
        })
        .collect()
}

fn dispatch_audit(root: &Path, assembly_argument: Option<&str>) -> Result<()> {
    let assembly_path = locate_assembly(root, assembly_argument, "sky_player")?;
    let lines = read_lines(&assembly_path)?;
    let targets = [
        (
            "plan_next_dispatch_projected",
            "plan_next_dispatch_projected:",
            "report",
        ),
        (
            "physical_plan_from_view",
            "physical_plan_from_view:",
            "report",
        ),
        ("dispatch_due_from_plan", "dispatch_due_from_plan:", "clean"),
        (
            "recover_missed_down_boundary",
            "recover_missed_down_boundary:",
            "inlining-report",
        ),
        (
            "normal shipping caller",
            "dispatch_loop8dispatch0B9_:",
            "report",
        ),
    ];
    let mut failed = false;
    println!("assembly={}", display_assembly_path(&assembly_path));
    for (name, fragment, policy) in targets {
        let Some(range) = named_body(&lines, fragment)? else {
            match policy {
                "clean" => {
                    println!("{name}: NOT_FOUND (required clean target)");
                    failed = true;
                }
                "inlining-report" => println!(
                    "{name}: INLINED_OR_NOT_FOUND (ThinLTO may omit the standalone symbol; shipping caller is reported below)"
                ),
                _ => println!("{name}: INLINED_OR_NOT_FOUND (report-only target)"),
            }
            continue;
        };
        let function = body(&lines, range);
        let stackalloc = stack_allocations(function).join(",");
        let memcpy = count_word(function, "memcpy");
        let memmove = count_word(function, "memmove");
        let chkstk = count_word(function, "__chkstk");
        let udivti3 = count_word(function, "__udivti3");
        let divti3 = count_word(function, "__divti3");
        let division = function
            .iter()
            .filter(|line| is_division_instruction(line))
            .count();
        let copy_sizes = copy_size_immediates(function).join(",");
        println!(
            "{name}: lines={}-{} instructions={} seh_stackalloc=[{}] memcpy={} memmove={} __chkstk={} __udivti3={} __divti3={} div_instructions={} copy_size_immediates=[{}]",
            range.0 + 1,
            range.1 + 1,
            function.iter().filter(|line| is_instruction(line)).count(),
            stackalloc,
            memcpy,
            memmove,
            chkstk,
            udivti3,
            divti3,
            division,
            copy_sizes
        );
        if policy == "clean" && (memcpy + memmove + chkstk + udivti3 + divti3 + division > 0) {
            failed = true;
        }
    }

    if let Some(range) = named_body(&lines, "dispatch_loop8dispatch0B9_:")? {
        let caller = body(&lines, range);
        let helper_calls = caller
            .iter()
            .filter(|line| is_call_to(line, "send_prepared_normal_precision_frame"))
            .count();
        let sender_calls = caller
            .iter()
            .filter(|line| is_call_to(line, "send_prepared_physical_packet_at_final_boundary"))
            .count();
        let atomic_rmw = caller
            .iter()
            .filter(|line| {
                contains_word_with_suffix(line, "xchg")
                    || contains_word_with_suffix(line, "cmpxchg")
            })
            .count();
        let power_consume_rmw = caller
            .iter()
            .filter(|line| contains_word_with_suffix(line, "cmpxchg"))
            .count();
        println!(
            "normal precision caller: retained_helper_call_refs={helper_calls} prepared_sender_calls={sender_calls} atomic_rmw_instructions={atomic_rmw} power_consume_rmw_candidates={power_consume_rmw}"
        );
        if helper_calls > 0 {
            println!(
                "normal precision caller: compiler retained an out-of-line helper call; inspect this caller region as the shipping suffix"
            );
        } else {
            println!(
                "normal precision caller: no retained send_prepared_normal_precision_frame call (inlined or eliminated)"
            );
        }
    }
    if failed {
        return Err("dispatch assembly audit failed".into());
    }
    Ok(())
}

fn prepared_normal_audit(root: &Path, assembly_argument: Option<&str>) -> Result<()> {
    let assembly_path = locate_assembly(root, assembly_argument, "sky_player")?;
    let lines = read_lines(&assembly_path)?;
    let range = prepared_named_body(&lines, "dispatch_prepared_normal_frame:")?
        .ok_or("prepared normal dispatch boundary was not emitted")?;
    let function = body(&lines, range);
    let focus_range = prepared_named_body(&lines, "handle_final_focus_loss:")?
        .ok_or("failure-only focus-loss handler was not emitted as an auditable function")?;
    let focus_qpc_calls = body(&lines, focus_range)
        .iter()
        .filter(|line| is_call_sequence(line, &["callq", "QpcClock", "3now"]))
        .count();
    let sender_indices = function
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            is_call_to(line, "send_prepared_physical_packet_at_final_boundary").then_some(index)
        })
        .collect::<Vec<_>>();
    if sender_indices.len() != 1 {
        return Err(format!(
            "expected exactly one full prepared sender transaction in the normal prepared boundary, found {}",
            sender_indices.len()
        )
        .into());
    }
    let sender_index = sender_indices[0];
    let admission_calls = function[..sender_index]
        .iter()
        .filter(|line| is_call_to(line, "final_down_target_admission"))
        .count();
    if admission_calls > 1 {
        return Err(format!(
            "expected at most one out-of-line final Down authority call before the prepared sender, found {admission_calls}"
        )
        .into());
    }
    let suffix_start = sender_index.saturating_sub(96);
    let suffix = &function[suffix_start..=sender_index];
    let forbidden_tokens = [
        "plan_next_dispatch",
        "dispatch_due_from_plan",
        "recover_missed_down_boundary",
        "commit_prepared",
        "commit_pending",
        "prepare_current_authored_packet",
        "pending_release",
        "coordinator",
        "planner",
        "schedule",
        "prepare_",
        "packet_",
        "PreparedPhysicalPacket",
        "alloc",
        "free",
        "lock",
        "Mutex",
        "OpenProcess",
        "GetProcessTimes",
        "QueryFullProcessImageNameW",
        "GetWindowThreadProcessId",
        "GetForegroundWindow",
    ];
    let forbidden_calls = suffix
        .iter()
        .filter(|line| {
            contains_ascii_case_insensitive(line, "callq")
                && !is_call_to(line, "send_prepared_physical_packet_at_final_boundary")
                && forbidden_tokens
                    .iter()
                    .any(|token| contains_ascii_case_insensitive(line, token))
        })
        .count();
    let copy_matches = suffix
        .iter()
        .filter(|line| {
            ["memcpy", "memmove", "__chkstk", "__udivti3", "__divti3"]
                .iter()
                .any(|token| contains_word(line, token))
        })
        .count();
    let helper_calls = function
        .iter()
        .filter(|line| is_call_to(line, "send_prepared_normal_precision_frame"))
        .count();
    let modifier_indices = function
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (is_call_to(line, "modifier_guard")
                || is_call_to(line, "observe_current_modifier_keys"))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let qpc_indices = function
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            is_call_sequence(line, &["callq", "QpcClock", "3now"]).then_some(index)
        })
        .collect::<Vec<_>>();
    let control_indices = function
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_call_to(line, "final_control_precheck").then_some(index))
        .collect::<Vec<_>>();
    if modifier_indices.len() != 1 || control_indices.is_empty() {
        return Err(format!(
            "could not identify the modifier probe and final control gate in the prepared boundary: modifier={}, control={}",
            modifier_indices.len(),
            control_indices.len()
        )
        .into());
    }
    let last_control = *control_indices.last().expect("checked nonempty controls");
    let post_modifier_qpc = qpc_indices
        .iter()
        .filter(|index| **index > modifier_indices[0] && **index < last_control)
        .count();

    println!("assembly={}", display_assembly_path(&assembly_path));
    println!(
        "scope=dispatch_prepared_normal_frame suffix_lines={}-{}",
        suffix_start + range.0 + 1,
        sender_index + range.0 + 1
    );
    println!("normal_prepared_sender_calls={}", sender_indices.len());
    println!("pre_sender_final_down_authority_calls={admission_calls}");
    println!("retained_precision_helper_calls={helper_calls}");
    println!("prepared_boundary_qpc_clock_calls={}", qpc_indices.len());
    println!("post_modifier_pre_control_lease_age_qpc_calls={post_modifier_qpc}");
    println!("focus_loss_handler_qpc_clock_calls={focus_qpc_calls}");
    println!("scoped_forbidden_calls={forbidden_calls}");
    println!("scoped_copy_or_division_symbols={copy_matches}");
    if helper_calls != 0
        || forbidden_calls != 0
        || copy_matches != 0
        || qpc_indices.len() != 1
        || post_modifier_qpc != 1
        || focus_qpc_calls != 1
    {
        return Err("scoped optimized normal-prepared audit failed".into());
    }
    println!("scoped optimized normal-prepared audit: PASS");
    Ok(())
}

fn parse_modifier_vk_names(source: &str) -> Result<Vec<String>> {
    let start = source
        .find("const MODIFIER_VKS")
        .ok_or("fixed modifier VK array was not found in source")?;
    let array = &source[start..];
    let end = array
        .find("];")
        .ok_or("fixed modifier VK array was not found in source")?;
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

fn is_zero_window_test(line: &str) -> bool {
    let folded = line.to_ascii_lowercase();
    let instruction = folded.trim_start();
    let Some(rest) = instruction.strip_prefix("xorl") else {
        return false;
    };
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return false;
    }
    let Some(rest) = rest.trim_start().strip_prefix("%ecx,") else {
        return false;
    };
    rest.trim_start().starts_with("%ecx")
}

fn moved_virtual_key(line: &str) -> Option<u32> {
    let folded = line.to_ascii_lowercase();
    let instruction = folded.trim_start();
    let rest = instruction.strip_prefix("movl")?;
    if !rest.chars().next()?.is_whitespace() {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('$')?;
    let digit_count = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let immediate = &rest[..digit_count];
    let rest = rest[digit_count..].strip_prefix(',')?;
    rest.trim_start().strip_prefix("%edx")?;
    immediate.parse().ok()
}

fn assert_five_modifier_queries(
    function: &[String],
    scope: &str,
    allow_other_calls: bool,
) -> Result<()> {
    let expected = [0x5b, 0x5c, 0x11, 0x10, 0x12];
    let mut actual = Vec::new();
    let mut zero_windows = Vec::new();
    let mut other_calls = Vec::new();
    let mut next_virtual_key = None;
    let mut zero_window = false;
    for line in function {
        if is_zero_window_test(line) {
            zero_window = true;
        }
        if let Some(virtual_key) = moved_virtual_key(line) {
            next_virtual_key = Some(virtual_key);
        }
        if is_call_to(line, "query_async_key_state") {
            let Some(virtual_key) = next_virtual_key else {
                return Err(format!("{scope} query call had no constant VK immediate").into());
            };
            actual.push(virtual_key);
            zero_windows.push(zero_window);
            next_virtual_key = None;
            zero_window = false;
        } else if contains_ascii_case_insensitive(line, "callq") {
            other_calls.push(line.trim().to_owned());
        }
    }
    if actual != expected {
        return Err(format!(
            "{scope} modifier VK calls were '{actual:?}', expected '{expected:?}'"
        )
        .into());
    }
    if zero_windows.len() != 5 || zero_windows.iter().any(|zero| !zero) {
        return Err(format!(
            "{scope} did not pass a zero window handle for all five modifier queries"
        )
        .into());
    }
    if !other_calls.is_empty() && !allow_other_calls {
        return Err(format!(
            "{scope} made a non-query call inside the fixed modifier probe: {}",
            other_calls.join("; ")
        )
        .into());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn modifier_guard_audit(root: &Path, sky_argument: &str, win32_argument: &str) -> Result<()> {
    let sky_path = locate_assembly(root, Some(sky_argument), "sky_player")?;
    let win32_path = locate_assembly(root, Some(win32_argument), "sky_dispatch_win32")?;
    let source_path = root.join("rust/crates/sky_dispatch_win32/src/input/modifier_guard.rs");
    let source = fs::read_to_string(&source_path).map_err(|error| {
        format!(
            "could not read modifier guard source {}: {error}",
            source_path.display()
        )
    })?;
    let source_vks = parse_modifier_vk_names(&source)?;
    let expected_names = ["VK_LWIN", "VK_RWIN", "VK_CONTROL", "VK_SHIFT", "VK_MENU"];
    if source_vks.iter().map(String::as_str).collect::<Vec<_>>() != expected_names {
        return Err(format!(
            "source modifier VK order was '{}', expected '{}'",
            source_vks.join(","),
            expected_names.join(",")
        )
        .into());
    }
    if source.contains("(VK_") && source_vks.len() != 5 {
        return Err("source modifier VK array must contain exactly five entries".into());
    }
    if !source
        .to_ascii_lowercase()
        .contains("observe_modifier_keys_with(|virtual_key| query_async_key_state(0, virtual_key))")
    {
        return Err("production modifier source no longer queries GetAsyncKeyState with a null window handle".into());
    }

    let sky_lines = read_lines(&sky_path)?;
    let win32_lines = read_lines(&win32_path)?;
    let query_range = find_unique_function_body(
        &sky_lines,
        |line| {
            ends_with_ascii_case_insensitive(line, "modifier_guard26observe_modifier_keys_with:")
        },
        "modifier_guard26observe_modifier_keys_with.*:$",
    )?;
    let admission_range = find_unique_function_body(
        &sky_lines,
        |line| {
            ends_with_ascii_case_insensitive(
                line,
                "admission37modifier_guard_and_final_revalidation:",
            )
        },
        "admission37modifier_guard_and_final_revalidation:$",
    )?;
    let prepared_range = find_unique_function_body(
        &sky_lines,
        |line| ends_with_ascii_case_insensitive(line, "prepared30dispatch_prepared_normal_frame:"),
        "prepared30dispatch_prepared_normal_frame:$",
    )?;
    let authored_range = find_unique_function_body(
        &sky_lines,
        |line| {
            line.starts_with(".seh_proc _")
                && line
                    .trim_end()
                    .ends_with("authored24dispatch_authored_packet")
        },
        "\\.seh_proc _.*authored24dispatch_authored_packet$",
    )?;
    let win32_query_range = find_unique_function_body(
        &win32_lines,
        |line| ends_with_ascii_case_insensitive(line, "physical21query_async_key_state:"),
        "physical21query_async_key_state:$",
    )?;
    let query_body = body(&sky_lines, query_range);
    let admission_body = body(&sky_lines, admission_range);
    let prepared_body = body(&sky_lines, prepared_range);
    let authored_body = body(&sky_lines, authored_range);
    let win32_query_body = body(&win32_lines, win32_query_range);
    assert_five_modifier_queries(query_body, "compiled fixed modifier observer", false)?;
    assert_five_modifier_queries(admission_body, "compiled authored Down admission", true)?;

    let query_calls = query_body
        .iter()
        .filter(|line| contains_ascii_case_insensitive(line, "callq"))
        .count();
    let forbidden_probe_calls = query_body
        .iter()
        .filter(|line| {
            [
                "memcpy",
                "memmove",
                "alloc",
                "free",
                "lock",
                "GetForegroundWindow",
                "OpenProcess",
                "GetProcessTimes",
                "QueryPerformanceCounter",
                "MapVirtualKey",
                "GetKeyboardLayout",
            ]
            .iter()
            .any(|token| contains_word(line, token))
        })
        .count();
    let prepared_probe_calls = prepared_body
        .iter()
        .filter(|line| is_call_to(line, "modifier_guard26observe_modifier_keys_with"))
        .count();
    let prepared_sender_calls = prepared_body
        .iter()
        .filter(|line| is_call_to(line, "send_prepared_physical_packet_at_final_boundary"))
        .count();
    let prepared_modifier_indices = prepared_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (is_call_to(line, "modifier_guard")
                || is_call_to(line, "observe_current_modifier_keys"))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let prepared_qpc_indices = prepared_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            is_call_sequence(line, &["callq", "QpcClock", "3now"]).then_some(index)
        })
        .collect::<Vec<_>>();
    let prepared_control_indices = prepared_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_call_to(line, "final_control_precheck").then_some(index))
        .collect::<Vec<_>>();
    let prepared_modifier_index = prepared_modifier_indices
        .last()
        .copied()
        .unwrap_or(usize::MAX);
    let prepared_control_index = prepared_control_indices
        .last()
        .copied()
        .unwrap_or(usize::MAX);
    let prepared_lease_age_qpc = prepared_qpc_indices
        .iter()
        .filter(|index| **index > prepared_modifier_index && **index < prepared_control_index)
        .count();
    if query_calls != 5 || forbidden_probe_calls != 0 {
        return Err(format!(
            "compiled modifier observer call count/forbidden-call audit failed: calls={query_calls}, forbidden={forbidden_probe_calls}"
        )
        .into());
    }
    if prepared_probe_calls != 1 || prepared_sender_calls != 1 {
        return Err(format!(
            "prepared Down assembly expected one modifier probe and one sender transaction; found probe={prepared_probe_calls}, sender={prepared_sender_calls}"
        )
        .into());
    }
    if prepared_modifier_indices.is_empty()
        || prepared_control_index <= prepared_modifier_index
        || prepared_qpc_indices.len() != 1
        || prepared_lease_age_qpc != 1
    {
        return Err(format!(
            "prepared Down assembly must contain exactly one post-modifier, pre-control lease-age QPC call; total={}, lease_age={prepared_lease_age_qpc}",
            prepared_qpc_indices.len()
        )
        .into());
    }
    let authored_modifier_indices = admission_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_call_to(line, "query_async_key_state").then_some(index))
        .collect::<Vec<_>>();
    let authored_age_qpc = admission_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            is_call_sequence(line, &["callq", "QpcClock", "3now"]).then_some(index)
        })
        .collect::<Vec<_>>();
    let authored_control_indices = admission_body
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_call_to(line, "final_control_precheck").then_some(index))
        .collect::<Vec<_>>();
    let authored_dispatch_qpc = authored_body
        .iter()
        .filter(|line| is_call_sequence(line, &["callq", "QpcClock", "3now"]))
        .count();
    if authored_modifier_indices.len() != 5
        || authored_age_qpc.len() != 1
        || authored_control_indices.is_empty()
        || authored_age_qpc[0]
            <= *authored_modifier_indices
                .last()
                .expect("five calls checked")
        || authored_age_qpc[0]
            >= *authored_control_indices
                .last()
                .expect("checked nonempty controls")
        || authored_dispatch_qpc != 1
    {
        return Err(format!(
            "authored Down assembly must retain one post-modifier, pre-control lease-age QPC sample and one existing final-policy sample; modifiers={}, lease_age={}, final_policy={authored_dispatch_qpc}",
            authored_modifier_indices.len(),
            authored_age_qpc.len()
        )
        .into());
    }
    let get_async_calls = win32_query_body
        .iter()
        .filter(|line| is_call_to(line, "*__imp_GetAsyncKeyState"))
        .count();
    let win32_calls = win32_query_body
        .iter()
        .filter(|line| contains_ascii_case_insensitive(line, "callq"))
        .count();
    if get_async_calls != 1 || win32_calls != 1 {
        return Err(format!(
            "Win32 query wrapper must make exactly one GetAsyncKeyState call; imports={get_async_calls}, calls={win32_calls}"
        )
        .into());
    }
    println!("sky_player_assembly={}", display_assembly_path(&sky_path));
    println!("win32_assembly={}", display_assembly_path(&win32_path));
    println!("source_modifier_vk_order={}", source_vks.join(","));
    println!("compiled_observer_GetAsyncKeyState_calls={query_calls}");
    println!(
        "compiled_authored_admission_modifier_calls={}",
        admission_body
            .iter()
            .filter(|line| is_call_to(line, "query_async_key_state"))
            .count()
    );
    println!("prepared_dispatch_modifier_probe_calls={prepared_probe_calls}");
    println!("prepared_sender_transactions={prepared_sender_calls}");
    println!(
        "prepared_boundary_qpc_clock_calls={}",
        prepared_qpc_indices.len()
    );
    println!("post_modifier_pre_control_lease_age_qpc_calls={prepared_lease_age_qpc}");
    println!(
        "authored_admission_lease_age_qpc_calls={}",
        authored_age_qpc.len()
    );
    println!("authored_dispatch_final_policy_qpc_calls={authored_dispatch_qpc}");
    println!("GetAsyncKeyState_import_calls_per_query_async_wrapper={get_async_calls}");
    println!("fixed modifier probe allocation/mapping/foreground/process/QPC/lock/log calls=0");
    println!("optimized modifier guard assembly audit: PASS");
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
            return Err(
                format!("assembly audit option was provided more than once: --{name}").into(),
            );
        }
        index += 1;
    }
    Ok(options)
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("assembly audit requires dispatch, prepared-normal, or modifier-guard")?;
    match operation {
        "dispatch" | "prepared-normal" => {
            let options = parse_options(&args[1..], &["assembly-path"])?;
            let path = options.get("assembly-path").map(String::as_str);
            if operation == "dispatch" {
                dispatch_audit(root, path)
            } else {
                prepared_normal_audit(root, path)
            }
        }
        "modifier-guard" => {
            let options = parse_options(
                &args[1..],
                &["sky-player-assembly-path", "win32-assembly-path"],
            )?;
            let sky_path = options
                .get("sky-player-assembly-path")
                .ok_or("modifier-guard requires --sky-player-assembly-path")?;
            let win32_path = options
                .get("win32-assembly-path")
                .ok_or("modifier-guard requires --win32-assembly-path")?;
            modifier_guard_audit(root, sky_path, win32_path)
        }
        other => Err(format!("unknown assembly audit operation: {other}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempAssemblyRoot(PathBuf);

    impl TempAssemblyRoot {
        fn new(name: &str) -> Self {
            let path =
                crate::ci_artifacts::unique_sibling(&std::env::temp_dir().join(name), "audit-test");
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempAssemblyRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn append_modifier_queries(assembly: &mut Vec<String>) {
        for virtual_key in [91, 92, 17, 16, 18] {
            assembly.extend([
                "  xorl %ecx, %ecx".to_owned(),
                format!("  movl ${virtual_key}, %edx"),
                "  callq *query_async_key_state".to_owned(),
            ]);
        }
    }

    fn modifier_guard_fixture() -> Vec<String> {
        let mut sky = vec!["modifier_guard26observe_modifier_keys_with:".to_owned()];
        append_modifier_queries(&mut sky);
        sky.push(".seh_endproc".to_owned());
        sky.push("admission37modifier_guard_and_final_revalidation:".to_owned());
        append_modifier_queries(&mut sky);
        sky.extend([
            "  callq QpcClock3now".to_owned(),
            "  callq final_control_precheck".to_owned(),
            ".seh_endproc".to_owned(),
            "prepared30dispatch_prepared_normal_frame:".to_owned(),
            "  callq modifier_guard26observe_modifier_keys_with".to_owned(),
            "  callq QpcClock3now".to_owned(),
            "  callq final_control_precheck".to_owned(),
            "  callq send_prepared_physical_packet_at_final_boundary".to_owned(),
            ".seh_endproc".to_owned(),
            ".seh_proc _fixture_authored24dispatch_authored_packet".to_owned(),
        ]);
        append_modifier_queries(&mut sky);
        sky.extend([
            "  callq QpcClock3now".to_owned(),
            "  callq final_control_precheck".to_owned(),
            ".seh_endproc".to_owned(),
        ]);
        sky
    }

    fn modifier_guard_paths(directory: &TempAssemblyRoot, sky: &[String]) -> (PathBuf, PathBuf) {
        let sky_path = directory.write("sky_player.s", &sky.join("\n"));
        let win32_path = directory.write(
            "sky_dispatch_win32.s",
            "physical21query_async_key_state:\n  callq *__imp_GetAsyncKeyState\n.seh_endproc\n",
        );
        (sky_path, win32_path)
    }

    fn modifier_guard_result(sky_path: &Path, win32_path: &Path) -> Result<()> {
        modifier_guard_audit(
            crate::repo::root().as_path(),
            &sky_path.to_string_lossy(),
            &win32_path.to_string_lossy(),
        )
    }

    fn prepared_fixture(sender_count: usize) -> String {
        let mut assembly = vec![
            "dispatch_prepared_normal_frame:".to_owned(),
            "  callq modifier_guard".to_owned(),
            "  callq QpcClock3now".to_owned(),
        ];
        assembly.extend(std::iter::repeat_n("  nop".to_owned(), 100));
        assembly.push("  callq final_control_precheck".to_owned());
        assembly.extend(std::iter::repeat_n(
            "  callq send_prepared_physical_packet_at_final_boundary".to_owned(),
            sender_count,
        ));
        assembly.extend([
            ".seh_endproc".to_owned(),
            "handle_final_focus_loss:".to_owned(),
            "  callq QpcClock3now".to_owned(),
            ".seh_endproc".to_owned(),
        ]);
        assembly.join("\n")
    }

    #[test]
    fn assembly_names_require_authoritative_crate_and_optional_hex_hash() {
        assert!(assembly_file_name_matches(
            Path::new("sky_player.s"),
            "sky_player"
        ));
        assert!(assembly_file_name_matches(
            Path::new("sky_player-6da0da9f.s"),
            "sky_player"
        ));
        assert!(!assembly_file_name_matches(
            Path::new("sky_player-extra.s"),
            "sky_player"
        ));
        assert!(!assembly_file_name_matches(
            Path::new("sky_dispatch_win32-a.s"),
            "sky_player"
        ));
    }

    #[test]
    fn function_body_ends_at_the_first_seh_endproc_and_requires_a_closed_body() {
        let assembly = lines("label:\n  callq foo\n.seh_endproc\nnext:");
        assert_eq!(
            find_function_body(&assembly, |line| line == "label:", "label").unwrap(),
            Some((0, 2))
        );
        let incomplete = lines("label:\n  callq foo");
        assert!(find_function_body(&incomplete, |line| line == "label:", "label").is_err());
    }

    #[test]
    fn assembly_word_prefix_counts_match_the_original_regex_for_lock_prefixed_mnemonics() {
        assert!(contains_word_with_suffix(
            "lock cmpxchgq %rax, (%rbx)",
            "cmpxchg"
        ));
        assert!(contains_word_with_suffix("xchgb %al, (%rcx)", "xchg"));
        assert!(!contains_word_with_suffix(
            "callq helper_cmpxchgq",
            "cmpxchg"
        ));
        assert!(!contains_word_with_suffix("callq helperxchg", "xchg"));
    }

    #[test]
    fn modifier_symbol_suffix_matching_preserves_the_powershell_regex_contract() {
        assert!(ends_with_ascii_case_insensitive(
            "_ZN12rust_mangled_ADMISSION37MODIFIER_GUARD_AND_FINAL_REVALIDATION:",
            "admission37modifier_guard_and_final_revalidation:"
        ));
        assert!(!ends_with_ascii_case_insensitive(
            "admission37modifier_guard_and_final_revalidation_extra:",
            "admission37modifier_guard_and_final_revalidation:"
        ));
        assert!(!ends_with_ascii_case_insensitive(
            "admission37modifier_guard_and_final_revalidation: ",
            "admission37modifier_guard_and_final_revalidation:"
        ));
    }

    #[test]
    fn fixed_modifier_query_decoder_requires_order_and_zero_window_arguments() {
        let valid = lines(
            "xorl %ecx, %ecx\nmovl $91, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $92, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $17, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $16, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $18, %edx\ncallq *query_async_key_state",
        );
        assert!(assert_five_modifier_queries(&valid, "fixture", false).is_ok());
        let reordered = lines(
            "xorl %ecx, %ecx\nmovl $92, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $91, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $17, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $16, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $18, %edx\ncallq *query_async_key_state",
        );
        assert!(assert_five_modifier_queries(&reordered, "fixture", false).is_err());
        let nonzero_window = lines(
            "movl $0, %ecx\nmovl $91, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $92, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $17, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $16, %edx\ncallq *query_async_key_state\nxorl %ecx, %ecx\nmovl $18, %edx\ncallq *query_async_key_state",
        );
        assert!(assert_five_modifier_queries(&nonzero_window, "fixture", false).is_err());
    }

    #[test]
    fn source_modifier_list_parser_preserves_order_and_rejects_extra_entries() {
        let source = "const MODIFIER_VKS: [VirtualKey; 5] = [(VK_LWIN, true), (VK_RWIN, true), (VK_CONTROL, false), (VK_SHIFT, false), (VK_MENU, false)];";
        assert_eq!(
            parse_modifier_vk_names(source).unwrap(),
            ["VK_LWIN", "VK_RWIN", "VK_CONTROL", "VK_SHIFT", "VK_MENU"]
        );
        assert_eq!(
            parse_modifier_vk_names(&source.replace("];", ", (VK_EXTRA, false)];"))
                .unwrap()
                .len(),
            6
        );
    }

    #[test]
    fn dispatch_metrics_keep_instructions_symbols_and_copy_immediates_separate() {
        let assembly = lines(
            "dispatch_due_from_plan:\n  movl $12, %r8d\n  callq memcpy@PLT\n  idivq %rcx\n  .seh_stackalloc 64\n.seh_endproc",
        );
        let range = named_body(&assembly, "dispatch_due_from_plan:")
            .unwrap()
            .unwrap();
        let function = body(&assembly, range);
        assert_eq!(
            function.iter().filter(|line| is_instruction(line)).count(),
            3
        );
        assert_eq!(stack_allocations(function), ["64"]);
        assert_eq!(count_word(function, "memcpy"), 1);
        assert_eq!(
            function
                .iter()
                .filter(|line| is_division_instruction(line))
                .count(),
            1
        );
        assert_eq!(copy_size_immediates(function), ["12"]);
    }

    #[test]
    fn dispatch_and_prepared_audits_accept_the_qualified_shape_and_reject_regressions() {
        let directory = TempAssemblyRoot::new("sky-xtask-assembly-audit-shape");
        let mut assembly = vec![
            "dispatch_due_from_plan:".to_owned(),
            ".seh_endproc".to_owned(),
            "dispatch_loop8dispatch0B9_:".to_owned(),
            "  callq send_prepared_physical_packet_at_final_boundary".to_owned(),
            ".seh_endproc".to_owned(),
        ];
        assembly.extend(lines(&prepared_fixture(1)));
        let path = directory.write("sky_player.s", &assembly.join("\n"));
        let path = path.to_string_lossy().into_owned();
        assert!(dispatch_audit(&directory.0, Some(&path)).is_ok());
        assert!(prepared_normal_audit(&directory.0, Some(&path)).is_ok());

        fs::write(
            Path::new(&path),
            "dispatch_due_from_plan:\n  callq memcpy\n.seh_endproc\n",
        )
        .unwrap();
        assert!(dispatch_audit(&directory.0, Some(&path)).is_err());

        fs::write(Path::new(&path), prepared_fixture(2)).unwrap();
        assert!(prepared_normal_audit(&directory.0, Some(&path)).is_err());
    }

    #[test]
    fn modifier_guard_audit_accepts_the_expected_source_and_optimized_call_shape() {
        let directory = TempAssemblyRoot::new("sky-xtask-modifier-audit-shape");
        let sky = modifier_guard_fixture();
        let (sky_path, win32_path) = modifier_guard_paths(&directory, &sky);
        assert!(modifier_guard_result(&sky_path, &win32_path).is_ok());

        let reordered = sky.join("\n").replace("$91", "$92");
        fs::write(&sky_path, reordered).unwrap();
        assert!(modifier_guard_result(&sky_path, &win32_path).is_err());
    }

    #[test]
    fn modifier_guard_audit_rejects_fake_and_duplicate_admission_or_prepared_labels() {
        let directory = TempAssemblyRoot::new("sky-xtask-modifier-audit-label-identity");
        let canonical = modifier_guard_fixture();
        let (sky_path, win32_path) = modifier_guard_paths(&directory, &canonical);

        let mut fake_admission = canonical.clone();
        *fake_admission
            .iter_mut()
            .find(|line| line.as_str() == "admission37modifier_guard_and_final_revalidation:")
            .unwrap() = "admission37modifier_guard_and_final_revalidation_extra:".to_owned();
        fs::write(&sky_path, fake_admission.join("\n")).unwrap();
        assert!(modifier_guard_result(&sky_path, &win32_path).is_err());

        let mut fake_prepared = canonical.clone();
        *fake_prepared
            .iter_mut()
            .find(|line| line.as_str() == "prepared30dispatch_prepared_normal_frame:")
            .unwrap() = "prepared30dispatch_prepared_normal_frame_fake:".to_owned();
        fs::write(&sky_path, fake_prepared.join("\n")).unwrap();
        assert!(modifier_guard_result(&sky_path, &win32_path).is_err());

        let mut duplicate_admission = canonical.clone();
        duplicate_admission.extend([
            "admission37modifier_guard_and_final_revalidation:".to_owned(),
            "  nop".to_owned(),
            ".seh_endproc".to_owned(),
        ]);
        fs::write(&sky_path, duplicate_admission.join("\n")).unwrap();
        assert!(modifier_guard_result(&sky_path, &win32_path).is_err());

        let mut duplicate_prepared = canonical;
        duplicate_prepared.extend([
            "prepared30dispatch_prepared_normal_frame:".to_owned(),
            "  nop".to_owned(),
            ".seh_endproc".to_owned(),
        ]);
        fs::write(&sky_path, duplicate_prepared.join("\n")).unwrap();
        assert!(modifier_guard_result(&sky_path, &win32_path).is_err());
    }
}

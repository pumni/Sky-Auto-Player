#![forbid(unsafe_code)]

use crate::Result;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

struct BasicBlock {
    start: usize,
    end: usize,
    succs: Vec<usize>,
}

fn parse_jump_tables(lines: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if let Some(label) = trimmed.strip_suffix(':') {
            if label.starts_with(".LJTI") {
                current = Some(label.to_string());
                tables.entry(label.to_string()).or_default();
                continue;
            }
            if !label.starts_with(".L") || label.starts_with(".LBB") {
                current = None;
            }
            continue;
        }
        if let Some(table_name) = current.as_ref() {
            if let Some(rest) = trimmed.strip_prefix(".long") {
                let expr = rest.trim();
                if let Some(target) = expr.split('-').next().map(str::trim)
                    && target.starts_with(".LBB")
                {
                    tables
                        .entry(table_name.clone())
                        .or_default()
                        .push(target.to_string());
                }
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                current = None;
            }
        }
    }
    tables
}

fn is_branch_or_terminator(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(op) = trimmed.split_whitespace().next() else {
        return false;
    };
    let op_lower = op.to_ascii_lowercase();
    op_lower == "jmp"
        || op_lower == "jmpq"
        || op_lower == "ret"
        || op_lower == "retq"
        || op_lower == "ud2"
        || (op_lower.starts_with('j')
            && trimmed
                .split_whitespace()
                .nth(1)
                .is_some_and(|arg| arg.starts_with(".LBB")))
}

fn build_function_cfg(
    function: &[String],
    jump_tables: &BTreeMap<String, Vec<String>>,
) -> Result<(Vec<BasicBlock>, Vec<usize>, bool)> {
    let mut block_starts = BTreeSet::new();
    block_starts.insert(0usize);
    let mut label_lines: Vec<(String, usize)> = Vec::new();
    for (idx, line) in function.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with(".LBB") && trimmed.ends_with(':') {
            block_starts.insert(idx);
            label_lines.push((trimmed.trim_end_matches(':').to_string(), idx));
        } else if is_branch_or_terminator(trimmed) && idx + 1 < function.len() {
            block_starts.insert(idx + 1);
        }
    }
    let block_starts: Vec<usize> = block_starts.into_iter().collect();
    let mut start_to_block: BTreeMap<usize, usize> = BTreeMap::new();
    for (block_id, &start) in block_starts.iter().enumerate() {
        start_to_block.insert(start, block_id);
    }
    let mut label_to_block: BTreeMap<String, usize> = BTreeMap::new();
    for (label, idx) in label_lines {
        if let Some(&block_id) = start_to_block.get(&idx) {
            label_to_block.insert(label, block_id);
        }
    }

    let mut line_to_block = vec![0usize; function.len()];
    let mut blocks = Vec::with_capacity(block_starts.len());
    for (block_id, &start) in block_starts.iter().enumerate() {
        let end = block_starts
            .get(block_id + 1)
            .copied()
            .unwrap_or(function.len());
        for slot in &mut line_to_block[start..end] {
            *slot = block_id;
        }
        blocks.push(BasicBlock {
            start,
            end,
            succs: Vec::new(),
        });
    }

    let block_count = blocks.len();
    let mut has_jump_table_dispatch = false;
    for (block_id, block) in blocks.iter_mut().enumerate() {
        let start = block.start;
        let end = block.end;
        let mut last_jump_table: Option<String> = None;
        let mut terminates = false;
        let mut succs = Vec::new();
        for (offset, line) in function[start..end].iter().enumerate() {
            let line_no = start + offset + 1;
            let trimmed = line.trim();
            if let Some(pos) = trimmed.find(".LJTI") {
                let tail = &trimmed[pos..];
                let len = tail
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                    .unwrap_or(tail.len());
                last_jump_table = Some(tail[..len].to_string());
            }
            let mut parts = trimmed.split_whitespace();
            let Some(op) = parts.next() else {
                continue;
            };
            let op_lower = op.to_ascii_lowercase();
            let arg = parts.next().unwrap_or("");
            if op_lower == "jmp" || op_lower == "jmpq" {
                terminates = true;
                if arg.starts_with(".LBB") {
                    let Some(&target) = label_to_block.get(arg) else {
                        return Err(format!(
                            "unresolved local jump target '{arg}' at function line {line_no}"
                        )
                        .into());
                    };
                    succs.push(target);
                } else if arg.starts_with('*') {
                    let Some(table_name) = last_jump_table.as_ref() else {
                        return Err(format!(
                            "unsupported/ambiguous indirect branch without jump table at function line {line_no}: {trimmed}"
                        )
                        .into());
                    };
                    let Some(targets) = jump_tables.get(table_name) else {
                        return Err(format!(
                            "unresolved jump table '{table_name}' at function line {line_no}"
                        )
                        .into());
                    };
                    if targets.is_empty() {
                        return Err(format!(
                            "empty jump table '{table_name}' at function line {line_no}"
                        )
                        .into());
                    }
                    has_jump_table_dispatch = true;
                    for target_label in targets {
                        let Some(&target) = label_to_block.get(target_label) else {
                            return Err(format!(
                                "jump table '{table_name}' references unresolved label '{target_label}'"
                            )
                            .into());
                        };
                        succs.push(target);
                    }
                } else if arg.starts_with(".L") {
                    return Err(format!(
                        "unresolved local jump target '{arg}' at function line {line_no}"
                    )
                    .into());
                }
            } else if op_lower.starts_with('j') && arg.starts_with(".LBB") {
                let Some(&target) = label_to_block.get(arg) else {
                    return Err(format!(
                        "unresolved local branch target '{arg}' at function line {line_no}"
                    )
                    .into());
                };
                succs.push(target);
            } else if op_lower == "ret" || op_lower == "retq" || op_lower == "ud2" {
                terminates = true;
            }
        }
        if !terminates && block_id + 1 < block_count {
            succs.push(block_id + 1);
        }
        succs.sort_unstable();
        succs.dedup();
        block.succs = succs;
    }

    Ok((blocks, line_to_block, has_jump_table_dispatch))
}

fn cfg_reachable(blocks: &[BasicBlock], start: usize, stop_at: Option<usize>) -> BTreeSet<usize> {
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);
    while let Some(node) = queue.pop_front() {
        if stop_at == Some(node) {
            continue;
        }
        for &next in &blocks[node].succs {
            if visited.insert(next) {
                queue.push_back(next);
            }
        }
    }
    visited
}

/// Returns `true` if there is a CFG path from `entry_block` to `target_line`
/// that avoids every instruction in `required_lines`. Respects intra-block
/// instruction ordering: a required site inside `target_block` satisfies the
/// requirement iff it precedes `target_line`.
fn has_bypass_path(
    blocks: &[BasicBlock],
    line_to_block: &[usize],
    entry_block: usize,
    target_line: usize,
    required_lines: &[usize],
) -> bool {
    let target_block = line_to_block[target_line];
    let blocking_blocks: BTreeSet<usize> = required_lines
        .iter()
        .filter_map(|&req_line| {
            let req_block = line_to_block[req_line];
            if req_block == target_block {
                (req_line < target_line).then_some(req_block)
            } else {
                Some(req_block)
            }
        })
        .collect();
    if blocking_blocks.contains(&entry_block) {
        return false;
    }
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(entry_block);
    queue.push_back(entry_block);
    while let Some(node) = queue.pop_front() {
        if node == target_block {
            return true;
        }
        for &next in &blocks[node].succs {
            if !blocking_blocks.contains(&next) && visited.insert(next) {
                queue.push_back(next);
            }
        }
    }
    false
}

fn precedes_in_cfg(
    blocks: &[BasicBlock],
    line_to_block: &[usize],
    from_line: usize,
    to_line: usize,
) -> bool {
    let from_block = line_to_block[from_line];
    let to_block = line_to_block[to_line];
    if from_block == to_block {
        from_line < to_line
    } else {
        cfg_reachable(blocks, from_block, None).contains(&to_block)
    }
}

fn find_unique_function_or_leaf_body(
    lines: &[String],
    label_suffix: &str,
    description: &str,
) -> Result<(usize, usize)> {
    let seh_starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            (trimmed.starts_with(".seh_proc _")
                && !trimmed.contains("?dtor$")
                && trimmed.ends_with(label_suffix))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if seh_starts.len() == 1 {
        let start = seh_starts[0];
        let Some(end_offset) = lines[start + 1..]
            .iter()
            .position(|line| line.trim() == ".seh_endproc")
        else {
            return Err(format!("assembly function for {description} has no .seh_endproc").into());
        };
        return Ok((start, start + 1 + end_offset));
    }
    if seh_starts.len() > 1 {
        return Err(format!(
            "expected one emitted assembly function for {description}, found {}",
            seh_starts.len()
        )
        .into());
    }

    let leaf_starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            trimmed
                .strip_suffix(':')
                .is_some_and(|label| {
                    label.starts_with('_')
                        && !label.contains("?dtor$")
                        && label.ends_with(label_suffix)
                })
                .then_some(index)
        })
        .collect::<Vec<_>>();
    if leaf_starts.len() != 1 {
        return Err(format!(
            "expected one emitted assembly function for {description}, found {}",
            leaf_starts.len()
        )
        .into());
    }
    let start = leaf_starts[0];
    let end = lines[start + 1..]
        .iter()
        .position(|line| {
            let trimmed = line.trim();
            trimmed.starts_with(".section")
                || trimmed.starts_with(".def")
                || trimmed.starts_with(".seh_proc")
                || trimmed == ".seh_endproc"
        })
        .map_or(lines.len() - 1, |offset| start + offset);
    Ok((start, end))
}

fn audit_helper_function_body(
    all_lines: &[String],
    label_suffix: &str,
    description: &str,
    allow_calls: bool,
) -> Result<()> {
    let range = find_unique_function_or_leaf_body(all_lines, label_suffix, description)?;
    let helper = body(all_lines, range);
    for (offset, line) in helper.iter().enumerate() {
        if is_physical_send_reference(line) {
            return Err(format!(
                "{description} contains a physical-send reference at line {}: {}",
                range.0 + offset + 1,
                line.trim()
            )
            .into());
        }
        if is_forbidden_suffix_effect(line) {
            return Err(format!(
                "{description} contains allocation/copy/lock/packet-build work at line {}: {}",
                range.0 + offset + 1,
                line.trim()
            )
            .into());
        }
        if !allow_calls && is_call_instruction(line) {
            return Err(format!(
                "{description} contains an unexpected call at line {}: {}",
                range.0 + offset + 1,
                line.trim()
            )
            .into());
        }
    }
    Ok(())
}

struct CfgAuditContext<'a> {
    function: &'a [String],
    blocks: &'a [BasicBlock],
    line_to_block: &'a [usize],
    admission_sites: &'a [usize],
}

fn validate_cfg_segment(
    ctx: &CfgAuditContext<'_>,
    start_line: usize,
    end_line: usize,
) -> Result<usize> {
    let start_block = ctx.line_to_block[start_line];
    let end_block = ctx.line_to_block[end_line];
    // For multi-gate functions (e.g., Mixed→Up fallback calling
    // `verify_final_packet_admission` a second time), stop traversal at any
    // *other* admission gate site so only the true post-final-gate suffix
    // reaching `deferred_sender` is audited as the zero-effect suffix.
    let other_gate_blocks: BTreeSet<usize> = ctx
        .admission_sites
        .iter()
        .copied()
        .filter(|&site| site != start_line)
        .map(|site| ctx.line_to_block[site])
        .filter(|&blk| blk != start_block && blk != end_block)
        .collect();
    let scoped_blocks: BTreeSet<usize> = if start_block == end_block {
        BTreeSet::from([start_block])
    } else {
        let mut forward = BTreeSet::new();
        let mut queue = VecDeque::new();
        forward.insert(start_block);
        queue.push_back(start_block);
        while let Some(node) = queue.pop_front() {
            if node == end_block {
                continue;
            }
            for &next in &ctx.blocks[node].succs {
                if !other_gate_blocks.contains(&next) && forward.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        if !forward.contains(&end_block) {
            return Ok(0);
        }
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); ctx.blocks.len()];
        for (id, block) in ctx.blocks.iter().enumerate() {
            for &succ in &block.succs {
                preds[succ].push(id);
            }
        }
        let mut backward = BTreeSet::new();
        let mut bqueue = VecDeque::new();
        backward.insert(end_block);
        bqueue.push_back(end_block);
        while let Some(node) = bqueue.pop_front() {
            if node == start_block {
                continue;
            }
            for &prev in &preds[node] {
                if !other_gate_blocks.contains(&prev) && backward.insert(prev) {
                    bqueue.push_back(prev);
                }
            }
        }
        forward.intersection(&backward).copied().collect()
    };

    for &block_id in &scoped_blocks {
        let start = if block_id == start_block {
            start_line + 1
        } else {
            ctx.blocks[block_id].start
        };
        let end = if block_id == end_block {
            end_line
        } else {
            ctx.blocks[block_id].end
        };
        if start >= end {
            continue;
        }
        for (offset, line) in ctx.function[start..end].iter().enumerate() {
            let function_index = start + offset;
            if is_forbidden_suffix_effect(line) {
                return Err(format!(
                    "scoped final-admission→sender suffix contains allocation/copy/lock/packet-build work at line {}: {}",
                    function_index + 1,
                    line.trim()
                )
                .into());
            }
            if is_call_instruction(line) {
                let folded = line.to_ascii_lowercase();
                let allowed_same_owner = folded.contains("releaseownerproof10same_owner");
                let allowed_observer_qpc = folded.contains("qpcclock3now");
                if !(allowed_same_owner || allowed_observer_qpc) {
                    return Err(format!(
                        "scoped final-admission→sender suffix contains an unexpected call at line {}: {}",
                        function_index + 1,
                        line.trim()
                    )
                    .into());
                }
            }
        }
    }
    Ok(scoped_blocks.len())
}

fn validate_scoped_send_suffix(ctx: &CfgAuditContext<'_>, deferred_sender: usize) -> Result<usize> {
    for (index, line) in ctx.function.iter().enumerate() {
        if index != deferred_sender && is_physical_send_reference(line) {
            return Err(format!(
                "production dispatch boundary contains an additional physical-send reference at line {}: {}",
                index + 1,
                line.trim()
            )
            .into());
        }
    }

    let mut total_blocks = 0usize;
    for &admission_site in ctx.admission_sites {
        total_blocks += validate_cfg_segment(ctx, admission_site, deferred_sender)?;
    }
    if total_blocks == 0 {
        return Err(
            "no direct verify_final_packet_admission→deferred_sender suffix path found in dispatch CFG"
                .into(),
        );
    }
    Ok(total_blocks)
}

fn production_dispatch_range(lines: &[String]) -> Result<(usize, usize)> {
    find_unique_function_body(
        lines,
        |line| {
            let trimmed = line.trim();
            trimmed.starts_with(".seh_proc _")
                && !trimmed.contains("?dtor$")
                && (trimmed.ends_with("13dispatch_loop18dispatch_candidate")
                    || trimmed == ".seh_proc _fixture_dispatch_candidate")
        },
        "sky_player production dispatch_candidate boundary",
    )
}

fn final_admission_gate_range(lines: &[String]) -> Result<(usize, usize)> {
    find_unique_function_body(
        lines,
        |line| {
            let trimmed = line.trim();
            trimmed.starts_with(".seh_proc _")
                && !trimmed.contains("?dtor$")
                && (trimmed.ends_with("9admission29verify_final_packet_admission")
                    || trimmed == ".seh_proc _fixture_verify_final_packet_admission")
        },
        "sky_player final admission boundary verify_final_packet_admission",
    )
}

fn audit_final_admission_gate_assembly(lines: &[String]) -> Result<(usize, usize)> {
    let gate_range = final_admission_gate_range(lines)?;
    let gate_body = body(lines, gate_range);
    for (offset, line) in gate_body.iter().enumerate() {
        if is_physical_send_reference(line) {
            return Err(format!(
                "verify_final_packet_admission contains a physical-send reference at line {}: {}",
                gate_range.0 + offset + 1,
                line.trim()
            )
            .into());
        }
    }

    let modifier_queries = call_indices(gate_body, "query_async_key_state").len();
    if modifier_queries != 5 {
        return Err(format!(
            "verify_final_packet_admission must retain five fixed modifier queries, found {modifier_queries}"
        )
        .into());
    }

    let direct_owner_checks = call_indices(gate_body, "foreground_window_owner_matches");
    let outlined_owner_checks = if direct_owner_checks.is_empty() {
        let calls = call_indices(gate_body, "fresh_final_release_owner_proof");
        if !calls.is_empty() {
            let proof_range = find_unique_function_body(
                lines,
                |line| {
                    let trimmed = line.trim();
                    trimmed.starts_with(".seh_proc _")
                        && !trimmed.contains("?dtor$")
                        && (trimmed.ends_with("31fresh_final_release_owner_proof")
                            || trimmed == ".seh_proc _fixture_fresh_final_release_owner_proof")
                },
                "admission::fresh_final_release_owner_proof",
            )?;
            let proof_body = body(lines, proof_range);
            if call_indices(proof_body, "foreground_window_owner_matches").is_empty() {
                return Err(
                    "outlined fresh_final_release_owner_proof is missing foreground_window_owner_matches"
                        .into(),
                );
            }
            for (offset, line) in proof_body.iter().enumerate() {
                if is_physical_send_reference(line) {
                    return Err(format!(
                        "outlined fresh_final_release_owner_proof contains a physical-send reference at line {}: {}",
                        proof_range.0 + offset + 1,
                        line.trim()
                    )
                    .into());
                }
            }
        }
        calls
    } else {
        Vec::new()
    };
    let owner_checks = if !direct_owner_checks.is_empty() {
        direct_owner_checks.len()
    } else {
        outlined_owner_checks.len()
    };
    if owner_checks == 0 {
        return Err("verify_final_packet_admission must retain a fresh owner proof check".into());
    }

    if !call_indices(gate_body, "22final_control_precheck").is_empty() {
        audit_helper_function_body(
            lines,
            "22final_control_precheck",
            "admission::final_control_precheck",
            false,
        )?;
    }

    Ok((owner_checks, modifier_queries))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DispatchAssemblyShape {
    dispatch_lines: (usize, usize),
    admission_gate_calls: usize,
    final_owner_rechecks: usize,
    modifier_queries: usize,
    deferred_sender_calls: usize,
    executor_completion_commits: usize,
    journal_commits: usize,
    send_suffix_lines: (usize, usize),
    scoped_suffix_blocks: usize,
}

fn validate_production_dispatch_assembly(lines: &[String]) -> Result<DispatchAssemblyShape> {
    let range = production_dispatch_range(lines)?;
    let function = body(lines, range);
    let jump_tables = parse_jump_tables(lines);
    let (blocks, line_to_block, _has_jump_table_dispatch) =
        build_function_cfg(function, &jump_tables)?;
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

    let admission_calls = call_indices(function, "verify_final_packet_admission")
        .into_iter()
        .filter(|&index| precedes_in_cfg(&blocks, &line_to_block, index, sender))
        .collect::<Vec<_>>();
    if admission_calls.is_empty() {
        return Err(
            "production dispatch boundary must call verify_final_packet_admission before deferred sender"
                .into(),
        );
    }

    // Unconditional pre-send dominance (no jump-table waiver): every
    // entry-reachable CFG path to deferred sender must pass through
    // `verify_final_packet_admission`.
    if has_bypass_path(&blocks, &line_to_block, 0, sender, &admission_calls) {
        return Err(
            "entry-reachable deferred sender bypasses verify_final_packet_admission".into(),
        );
    }

    let (owner_checks, modifier_queries) = audit_final_admission_gate_assembly(lines)?;

    let executor_commits = call_indices(function, "DispatchExecutor20on_send_completed_at");
    let journal_commits = call_indices(function, "commit_physical_packet");
    if executor_commits.len() != 1
        || journal_commits.len() != 1
        || !(precedes_in_cfg(&blocks, &line_to_block, sender, executor_commits[0])
            && precedes_in_cfg(
                &blocks,
                &line_to_block,
                executor_commits[0],
                journal_commits[0],
            ))
    {
        return Err(format!(
            "deferred sender must precede one live scheduler completion commit and one journal commit; sender={}, scheduler={}, journal={}",
            sender,
            executor_commits.len(),
            journal_commits.len()
        )
        .into());
    }

    // Unconditional post-send dominance: no entry-reachable path may reach
    // scheduler completion commit without executing deferred sender, nor reach
    // journal commit without executing both deferred sender and scheduler
    // completion commit.
    if has_bypass_path(&blocks, &line_to_block, 0, executor_commits[0], &[sender]) {
        return Err("entry-reachable scheduler completion commit bypasses deferred sender".into());
    }
    if has_bypass_path(
        &blocks,
        &line_to_block,
        0,
        journal_commits[0],
        &[executor_commits[0]],
    ) || has_bypass_path(&blocks, &line_to_block, 0, journal_commits[0], &[sender])
    {
        return Err(
            "entry-reachable journal commit bypasses deferred sender or scheduler completion commit"
                .into(),
        );
    }

    let final_gate = *admission_calls.last().expect("checked admission gate call");
    let cfg_ctx = CfgAuditContext {
        function,
        blocks: &blocks,
        line_to_block: &line_to_block,
        admission_sites: &admission_calls,
    };
    let scoped_suffix_blocks = validate_scoped_send_suffix(&cfg_ctx, sender)?;
    Ok(DispatchAssemblyShape {
        dispatch_lines: (range.0 + 1, range.1 + 1),
        admission_gate_calls: admission_calls.len(),
        final_owner_rechecks: owner_checks,
        modifier_queries,
        deferred_sender_calls: senders.len(),
        executor_completion_commits: executor_commits.len(),
        journal_commits: journal_commits.len(),
        send_suffix_lines: (range.0 + final_gate + 1, range.0 + sender + 1),
        scoped_suffix_blocks,
    })
}

fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_brace_balanced_fn_body(source: &str, signature: &str) -> Result<String> {
    let start = source
        .find(signature)
        .ok_or_else(|| format!("production source missing '{signature}'"))?;
    let tail = &source[start..];
    let Some(open_rel) = tail.find('{') else {
        return Err(format!("function '{signature}' has no opening brace").into());
    };
    let mut depth = 0usize;
    let mut end_rel = None;
    for (idx, ch) in tail[open_rel..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end_rel = Some(open_rel + idx + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end_rel) = end_rel else {
        return Err(format!("function '{signature}' has unbalanced braces").into());
    };
    Ok(strip_line_comments(&tail[..end_rel]))
}

fn validate_source_contract(dispatch: &str, admission: &str) -> Result<()> {
    let dispatch_body =
        extract_brace_balanced_fn_body(dispatch, "pub(crate) fn dispatch_candidate(")?;
    if dispatch_body.contains("verify_final_down_admission(") {
        return Err(
            "production dispatch_candidate must not reference test-only verify_final_down_admission"
                .into(),
        );
    }

    let ordered = [
        "verify_final_packet_admission(",
        ".send_prepared_physical_packet_at_final_boundary_deferred_commit_for_target(",
        ".on_send_completed_at(",
        ".commit_physical_packet(",
    ];
    let mut previous = None;
    for required in ordered {
        let Some(index) = dispatch_body.find(required) else {
            return Err(format!("production dispatch_candidate lost '{required}'").into());
        };
        if previous.is_some_and(|previous| index <= previous) {
            return Err("production source order must be final admission → deferred send → scheduler commit → journal commit".into());
        }
        previous = Some(index);
    }

    // Exclude test-only module code and cfg(test)-gated function definitions so
    // cfg(test) wrappers cannot satisfy the live admission contract.
    let admission_prod = admission
        .split("#[cfg(test)]\nmod ")
        .next()
        .and_then(|head| head.split("#[cfg(test)]\r\nmod ").next())
        .unwrap_or(admission);
    let has_cfg_test_attribute_before = |source: &str, item_start: usize| {
        source[..item_start]
            .lines()
            .rev()
            .take_while(|line| {
                let t = line.trim();
                t.is_empty() || t.starts_with("#[") || t.starts_with("//")
            })
            .any(|line| line.trim() == "#[cfg(test)]")
    };
    let gate_start = admission_prod
        .find("pub(crate) fn verify_final_packet_admission(")
        .ok_or(
            "production admission source missing 'pub(crate) fn verify_final_packet_admission('",
        )?;
    if has_cfg_test_attribute_before(admission_prod, gate_start) {
        return Err(
            "production verify_final_packet_admission must not be gated by #[cfg(test)]".into(),
        );
    }
    let gate_body = extract_brace_balanced_fn_body(
        admission_prod,
        "pub(crate) fn verify_final_packet_admission(",
    )?;
    for required in [
        "final_down_control_verdict(",
        "final_down_target_hint(",
        "final_down_modifier_verdict(",
        "final_packet_lease_age_proof(",
        "fresh_final_release_owner_proof(",
        "final_down_atomic_revalidation(",
        "lease_timeout_ticks",
        "system_power",
    ] {
        if !gate_body.contains(required) {
            return Err(
                format!("production verify_final_packet_admission lost '{required}'").into(),
            );
        }
    }

    let owner_start = admission_prod
        .find("fn fresh_final_release_owner_proof(")
        .ok_or("production admission source missing 'fn fresh_final_release_owner_proof('")?;
    if has_cfg_test_attribute_before(admission_prod, owner_start) {
        return Err(
            "production fresh_final_release_owner_proof must not be gated by #[cfg(test)]".into(),
        );
    }
    let owner_body =
        extract_brace_balanced_fn_body(admission_prod, "fn fresh_final_release_owner_proof(")?;
    for required in [
        "verify_final_process_identity(",
        "foreground_window_owner_matches(",
        "release_owner_proof_is_current(",
    ] {
        if !owner_body.contains(required) {
            return Err(
                format!("production fresh_final_release_owner_proof lost '{required}'").into(),
            );
        }
    }
    Ok(())
}

fn require_source_contract(root: &Path) -> Result<()> {
    let dispatch_path = root.join("rust/crates/sky_player/src/engine/worker/dispatch_loop.rs");
    let dispatch = fs::read_to_string(&dispatch_path)
        .map_err(|error| format!("could not read production dispatch source: {error}"))?;
    let admission_path = root.join("rust/crates/sky_player/src/engine/worker/admission.rs");
    let admission = fs::read_to_string(&admission_path)
        .map_err(|error| format!("could not read final admission source: {error}"))?;
    validate_source_contract(&dispatch, &admission)
}

fn report_dispatch_audit(root: &Path, assembly_argument: Option<&str>, alias: &str) -> Result<()> {
    let path = locate_assembly(root, assembly_argument, "sky_player")?;
    let lines = read_lines(&path)?;
    let shape = validate_production_dispatch_assembly(&lines)?;
    require_source_contract(root)?;
    println!("assembly={}", path.display());
    println!("audit={alias}");
    println!("boundary=sky_player::engine::worker::dispatch_loop::dispatch_candidate");
    println!(
        "dispatch_lines={}-{}",
        shape.dispatch_lines.0, shape.dispatch_lines.1
    );
    println!(
        "pre_send_admission_gate_calls={}",
        shape.admission_gate_calls
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
        "scoped_send_suffix_lines={}-{}; scoped_suffix_blocks={}; forbidden_effects=0; worker-wide copy/division thresholds=not applied",
        shape.send_suffix_lines.0, shape.send_suffix_lines.1, shape.scoped_suffix_blocks
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
    let deferred_body = body(lines, deferred);
    let (policy, callee_fragment) = if lines.iter().any(|line| {
        line.starts_with(".seh_proc _")
            && line.contains("send_prepared_physical_packet_view_with_commit_policy")
    }) {
        (
            find_unique_function_body(
                lines,
                |line| {
                    line.starts_with(".seh_proc _")
                        && line.contains("send_prepared_physical_packet_view_with_commit_policy")
                },
                "tracked packet send commit-policy boundary",
            )?,
            "send_prepared_physical_packet_view_with_commit_policy",
        )
    } else {
        (
            find_unique_function_body(
                lines,
                |line| {
                    line.starts_with(".seh_proc _")
                        && line.contains("send_prepared_physical_packet_once_impl")
                },
                "tracked packet single-send implementation boundary",
            )?,
            "send_prepared_physical_packet_once_impl",
        )
    };
    let policy_body = body(lines, policy);
    let deferred_calls = call_indices(deferred_body, callee_fragment).len();
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

    fn valid_gate_fixture() -> &'static str {
        ".seh_proc _fixture_verify_final_packet_admission\n\
         callq query_async_key_state\n\
         callq query_async_key_state\n\
         callq query_async_key_state\n\
         callq query_async_key_state\n\
         callq query_async_key_state\n\
         callq foreground_window_owner_matches\n\
         retq\n\
         .seh_endproc\n"
    }

    fn valid_dispatch() -> Vec<String> {
        fixture(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq record_sendinput_pre_call_lateness\n\
             callq verify_final_packet_admission\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             .seh_endproc\n{}",
            valid_gate_fixture()
        ))
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

        let double_sender = fixture(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n.seh_endproc\n{}",
            valid_gate_fixture()
        ));
        assert!(validate_production_dispatch_assembly(&double_sender).is_err());

        let wrong_commit_order = fixture(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq commit_physical_packet\n.seh_endproc\n{}",
            valid_gate_fixture()
        ));
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

        let diagnostic_only = valid_dispatch();
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

    #[test]
    fn source_contract_requires_live_production_admission_and_rejects_test_only_or_missing_proof() {
        let valid_dispatch = "\
            pub(crate) fn dispatch_candidate() {\n\
                verify_final_packet_admission();\n\
                backend.send_prepared_physical_packet_at_final_boundary_deferred_commit_for_target();\n\
                scheduler.on_send_completed_at();\n\
                backend.commit_physical_packet();\n\
            }\n\
            pub(super) fn dispatch() {}\n";
        let valid_admission = "\
            fn fresh_final_release_owner_proof() {\n\
                authority.verify_final_process_identity();\n\
                foreground_window_owner_matches();\n\
                target.release_owner_proof_is_current();\n\
            }\n\
            pub(crate) fn enter_focus_pause() {}\n\
            pub(crate) fn verify_final_packet_admission(lease_timeout_ticks: u64, system_power: u64) {\n\
                final_down_control_verdict();\n\
                final_down_target_hint();\n\
                final_down_modifier_verdict();\n\
                final_packet_lease_age_proof();\n\
                fresh_final_release_owner_proof();\n\
                final_down_atomic_revalidation();\n\
            }\n";
        assert!(validate_source_contract(valid_dispatch, valid_admission).is_ok());

        // Reject outdated test-only verify_final_down_admission in production dispatch_candidate.
        let outdated_dispatch = valid_dispatch.replace(
            "verify_final_packet_admission(",
            "verify_final_down_admission(",
        );
        assert!(validate_source_contract(&outdated_dispatch, valid_admission).is_err());

        // Reject reversed commit ordering.
        let wrong_order = "\
            pub(crate) fn dispatch_candidate() {\n\
                verify_final_packet_admission();\n\
                scheduler.on_send_completed_at();\n\
                backend.send_prepared_physical_packet_at_final_boundary_deferred_commit_for_target();\n\
                backend.commit_physical_packet();\n\
            }\n\
            pub(super) fn dispatch() {}\n";
        assert!(validate_source_contract(wrong_order, valid_admission).is_err());

        // Reject missing live proof (or proof moved behind #[cfg(test)]).
        let missing_owner_proof = valid_admission.replace("foreground_window_owner_matches();", "");
        assert!(validate_source_contract(valid_dispatch, &missing_owner_proof).is_err());

        let test_only_admission = format!("#[cfg(test)]\n{valid_admission}");
        assert!(validate_source_contract(valid_dispatch, &test_only_admission).is_err());

        // Reject comment-only tokens masquerading as live calls.
        let commented_owner = valid_admission.replace(
            "foreground_window_owner_matches();",
            "// foreground_window_owner_matches();",
        );
        assert!(validate_source_contract(valid_dispatch, &commented_owner).is_err());
    }

    #[test]
    fn cfg_oracle_rejects_bypass_jump_table_bypass_skipped_commit_forbidden_suffix_and_unresolved_jumps()
     {
        let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
        let gate = valid_gate_fixture();

        // 1. Negative: direct branch bypassing verify_final_packet_admission to reach deferred sender.
        let gate_bypass = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             testb $1, %al\n\
             jne .LBB0_send\n\
             callq verify_final_packet_admission\n\
             .LBB0_send:\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             retq\n\
             .seh_endproc\n{gate}"
        ));
        assert!(
            validate_production_dispatch_assembly(&gate_bypass).is_err(),
            "entry-reachable sender bypassing verify_final_packet_admission must fail"
        );

        // 2. Negative: jump-table CFG where one jump-table arm bypasses verify_final_packet_admission.
        // Proves there is NO blanket jump-table waiver!
        let jump_table_bypass = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             leaq .LJTI0_0(%rip), %rax\n\
             jmpq *%rax\n\
             .LBB0_gated:\n\
             callq verify_final_packet_admission\n\
             .LBB0_send:\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             retq\n\
             .seh_endproc\n\
             .LJTI0_0:\n\
             .long .LBB0_gated-.LJTI0_0\n\
             .long .LBB0_send-.LJTI0_0\n{gate}"
        ));
        assert!(
            validate_production_dispatch_assembly(&jump_table_bypass).is_err(),
            "jump-table arm bypassing verify_final_packet_admission must fail unconditionally"
        );

        // 3. Negative: entry branch skipping scheduler completion commit to reach journal commit.
        let skipped_commit = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             testb $1, %al\n\
             jne .LBB0_journal\n\
             callq DispatchExecutor20on_send_completed_at\n\
             .LBB0_journal:\n\
             callq commit_physical_packet\n\
             retq\n\
             .seh_endproc\n{gate}"
        ));
        assert!(
            validate_production_dispatch_assembly(&skipped_commit).is_err(),
            "journal commit bypassing scheduler completion commit must fail"
        );

        // 4. Negative: forbidden memcpy on post-gate branch reaching sender (even with jump table).
        let suffix_forbidden = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             leaq .LJTI0_0(%rip), %rax\n\
             jmpq *%rax\n\
             .LBB0_dirty:\n\
             callq memcpy\n\
             .LBB0_send:\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             retq\n\
             .LBB0_reject:\n\
             retq\n\
             .seh_endproc\n\
             .LJTI0_0:\n\
             .long .LBB0_dirty-.LJTI0_0\n\
             .long .LBB0_reject-.LJTI0_0\n{gate}"
        ));
        assert!(
            validate_production_dispatch_assembly(&suffix_forbidden).is_err(),
            "post-gate branch with forbidden memcpy reaching sender must fail even with jump table"
        );

        // 5. Negative: unresolved indirect branch must fail closed.
        let unresolved_indirect = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             jmpq *%rax\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             .seh_endproc\n{gate}"
        ));
        assert!(
            validate_production_dispatch_assembly(&unresolved_indirect).is_err(),
            "unresolved indirect branch must fail closed"
        );

        // 6. Negative: physical send inside verify_final_packet_admission must fail.
        let send_in_gate = lines(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             retq\n\
             .seh_endproc\n\
             .seh_proc _fixture_verify_final_packet_admission\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq query_async_key_state\n\
             callq foreground_window_owner_matches\n\
             callq *__imp_SendInput\n\
             retq\n\
             .seh_endproc",
        );
        assert!(
            validate_production_dispatch_assembly(&send_in_gate).is_err(),
            "physical send inside verify_final_packet_admission must fail"
        );

        // 7. Positive: reordered blocks + jump table on admission verdict + legitimate Up-only bypass inside gate.
        let valid_reordered = lines(&format!(
            ".seh_proc _fixture_dispatch_candidate\n\
             callq verify_final_packet_admission\n\
             leaq .LJTI0_0(%rip), %rax\n\
             jmpq *%rax\n\
             .LBB0_send:\n\
             callq send_prepared_physical_packet_at_final_boundary_deferred_commit\n\
             callq DispatchExecutor20on_send_completed_at\n\
             callq commit_physical_packet\n\
             retq\n\
             .LBB0_allowed:\n\
             callq ReleaseOwnerProof10same_owner\n\
             jmp .LBB0_send\n\
             .LBB0_reject:\n\
             callq __rust_alloc\n\
             retq\n\
             .seh_endproc\n\
             .LJTI0_0:\n\
             .long .LBB0_allowed-.LJTI0_0\n\
             .long .LBB0_reject-.LJTI0_0\n{gate}"
        ));
        let shape = validate_production_dispatch_assembly(&valid_reordered)
            .expect("reordered blocks with jump table on gate verdict must pass");
        assert_eq!(shape.admission_gate_calls, 1);
        assert_eq!(shape.final_owner_rechecks, 1);
        assert_eq!(shape.modifier_queries, 5);
    }
}

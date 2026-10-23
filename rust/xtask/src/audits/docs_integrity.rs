use crate::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const REQUIRED_CORE_FILES: &[&str] = &["docs/INDEX.md", "SECURITY.md", "AGENTS.md"];

pub(crate) fn check_core_files(root: &Path) -> Result<()> {
    for rel_path in REQUIRED_CORE_FILES {
        let path = root.join(rel_path);
        if !path.is_file() {
            return Err(format!(
                "Required core file is missing: `{}` (expected at {})",
                rel_path,
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn check_adr_uniqueness(root: &Path) -> Result<()> {
    let adr_dir = root.join("docs/adr");
    if !adr_dir.is_dir() {
        return Err(format!("ADR directory missing: {}", adr_dir.display()).into());
    }

    let mut id_map: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let entries = fs::read_dir(&adr_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        if let Some(rest) = file_name.strip_prefix("ADR-") {
            let id = match rest.split('-').next() {
                Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => id,
                _ => {
                    return Err(format!(
                        "ADR filename does not follow `ADR-####-<name>.md` convention: {}",
                        file_name
                    )
                    .into());
                }
            };
            id_map.entry(id.to_string()).or_default().push(path);
        }
    }

    let mut duplicates = Vec::new();
    for (id, files) in &id_map {
        if files.len() > 1 {
            let names: Vec<_> = files
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            duplicates.push(format!("ADR ID {id}: {}", names.join(", ")));
        }
    }

    if !duplicates.is_empty() {
        return Err(format!(
            "Duplicate ADR IDs detected in docs/adr:\n  - {}",
            duplicates.join("\n  - ")
        )
        .into());
    }

    Ok(())
}

fn fence_marker(line: &str) -> Option<(u8, usize, usize)> {
    let bytes = line.as_bytes();
    let marker_start = bytes.iter().take_while(|byte| **byte == b' ').count();
    if marker_start > 3 {
        return None;
    }

    let marker = *bytes.get(marker_start)?;
    if marker != b'`' && marker != b'~' {
        return None;
    }

    let marker_len = bytes[marker_start..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    (marker_len >= 3).then_some((marker, marker_len, marker_start + marker_len))
}

fn parse_inline_link_destination(line: &str, open_paren: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut cursor = open_paren + 1;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }

    if bytes.get(cursor) == Some(&b'<') {
        let start = cursor + 1;
        let end = line[start..].find('>')? + start;
        let link_end = line[end + 1..].find(')')? + end + 2;
        return Some((line[start..end].to_string(), link_end));
    }

    let start = cursor;
    let mut nested_parens = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            b'(' => {
                nested_parens += 1;
                cursor += 1;
            }
            b')' if nested_parens == 0 => {
                return Some((line[start..cursor].to_string(), cursor + 1));
            }
            b')' => {
                nested_parens -= 1;
                cursor += 1;
            }
            byte if byte.is_ascii_whitespace() && nested_parens == 0 => {
                let link_end = line[cursor..].find(')')? + cursor + 1;
                return Some((line[start..cursor].to_string(), link_end));
            }
            _ => cursor += 1,
        }
    }
    None
}

fn extract_markdown_links_from_line(
    line: &str,
    line_number: usize,
    links: &mut Vec<(usize, String)>,
) {
    let bytes = line.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'`' => {
                let run_len = bytes[cursor..]
                    .iter()
                    .take_while(|byte| **byte == b'`')
                    .count();
                let end = cursor + run_len;
                let delimiter = "`".repeat(run_len);
                cursor = line[end..]
                    .find(&delimiter)
                    .map(|offset| end + offset + run_len)
                    .unwrap_or(end);
            }
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            b'[' => {
                let Some(label_end_offset) =
                    bytes[cursor + 1..].iter().position(|byte| *byte == b']')
                else {
                    break;
                };
                let label_end = cursor + 1 + label_end_offset;
                let open_paren = label_end + 1;
                if bytes.get(open_paren) == Some(&b'(') {
                    if let Some((destination, link_end)) =
                        parse_inline_link_destination(line, open_paren)
                    {
                        if !destination.is_empty() {
                            links.push((line_number, destination));
                        }
                        cursor = link_end;
                        continue;
                    }
                }
                cursor = label_end + 1;
            }
            _ => cursor += 1,
        }
    }
}

pub(crate) fn extract_markdown_links(content: &str) -> Vec<(usize, String)> {
    // This intentionally handles one-line inline links/images, not all CommonMark forms.
    let mut links = Vec::new();
    let mut active_fence: Option<(u8, usize)> = None;

    for (line_index, line) in content.lines().enumerate() {
        if let Some((active_marker, minimum_len)) = active_fence {
            if let Some((marker, marker_len, tail_start)) = fence_marker(line)
                && marker == active_marker
                && marker_len >= minimum_len
                && line[tail_start..].trim().is_empty()
            {
                active_fence = None;
            }
            continue;
        }

        if let Some((marker, marker_len, _)) = fence_marker(line) {
            active_fence = Some((marker, marker_len));
            continue;
        }

        extract_markdown_links_from_line(line, line_index + 1, &mut links);
    }

    links
}

fn is_external_or_fragment(link: &str) -> bool {
    if link.starts_with('#') || link.starts_with("//") {
        return true;
    }

    let Some((scheme, _)) = link.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

fn decode_percent_escapes(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%'
            && cursor + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                (bytes[cursor + 1] as char).to_digit(16),
                (bytes[cursor + 2] as char).to_digit(16),
            )
        {
            decoded.push(((high << 4) | low) as u8);
            cursor += 3;
        } else {
            decoded.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| path.to_string())
}

fn relative_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn resolve_local_link(
    root: &Path,
    source: &Path,
    link: &str,
) -> std::result::Result<Option<PathBuf>, String> {
    if is_external_or_fragment(link) {
        return Ok(None);
    }

    let path_end = link.find(['#', '?']).unwrap_or(link.len());
    let decoded_path = decode_percent_escapes(&link[..path_end]);
    if decoded_path.is_empty() {
        return Ok(None);
    }

    let root_relative = decoded_path.starts_with('/');
    let path_text = decoded_path.trim_start_matches('/');
    let target = Path::new(path_text);
    let mut resolved = if root_relative {
        root.to_path_buf()
    } else {
        source
            .parent()
            .ok_or_else(|| "source Markdown file has no parent directory".to_string())?
            .to_path_buf()
    };

    for component in target.components() {
        match component {
            Component::Normal(value) => resolved.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() || !resolved.starts_with(root) {
                    return Err("target escapes repository root".to_string());
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute target is outside repository-relative link scope".to_string());
            }
        }
    }

    Ok(Some(resolved))
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.to_str() == Some("md"))
        {
            files.push(entry.path());
        }
    }

    let docs_dir = root.join("docs");
    for entry in WalkDir::new(&docs_dir).sort_by_file_name() {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.to_str() == Some("md"))
        {
            files.push(entry.into_path());
        }
    }

    files.sort_by_key(|path| relative_display_path(path.strip_prefix(root).unwrap_or(path)));
    Ok(files)
}

fn display_source_path(root: &Path, source: &Path) -> String {
    relative_display_path(source.strip_prefix(root).unwrap_or(source))
}

/// Checks local, one-line inline links in root and docs Markdown files.
/// Fenced/inline code, reference links, HTML links, external URLs, and fragment targets are out of scope.
pub(crate) fn check_markdown_links(root: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    let mut broken_links = Vec::new();

    for source in markdown_files(&root)? {
        let content = fs::read_to_string(&source)?;
        for (line_number, link) in extract_markdown_links(&content) {
            match resolve_local_link(&root, &source, &link) {
                Ok(Some(resolved)) if !resolved.exists() => {
                    broken_links.push(format!(
                        "{}:{line_number}: `{link}` resolves to missing `{}`",
                        display_source_path(&root, &source),
                        display_source_path(&root, &resolved)
                    ));
                }
                Ok(_) => {}
                Err(reason) => {
                    broken_links.push(format!(
                        "{}:{line_number}: `{link}` {reason}",
                        display_source_path(&root, &source)
                    ));
                }
            }
        }
    }

    if !broken_links.is_empty() {
        broken_links.sort();
        return Err(format!(
            "Broken local Markdown links detected:\n  - {}",
            broken_links.join("\n  - ")
        )
        .into());
    }

    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    check_core_files(root)?;
    check_adr_uniqueness(root)?;
    check_markdown_links(root)?;
    println!("[xtask] documentation integrity checks: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_repository_docs_integrity_passes() {
        let root = crate::repo::root();
        run(&root).expect("repository docs integrity check must pass");
    }

    #[test]
    fn extract_markdown_links_finds_relative_and_external() {
        let md = "Here is a [link](file.md), an [anchor](#section), and [external](https://example.com).";
        let links = extract_markdown_links(md);
        assert_eq!(
            links,
            vec![
                (1, "file.md".to_string()),
                (1, "#section".to_string()),
                (1, "https://example.com".to_string()),
            ]
        );
    }

    #[test]
    fn extract_markdown_links_skips_code_and_handles_common_destinations() {
        let md = concat!(
            "[angled](<docs/space%20name.md#section>) ",
            "[nested](docs/design_(old).md) ` [inline](ignored.md) `\n",
            "```markdown\n[example](ignored.md)\n```\n",
            "~~~text\n[example](also-ignored.md)\n~~~\n",
        );
        let links = extract_markdown_links(md);
        assert_eq!(
            links,
            vec![
                (1, "docs/space%20name.md#section".to_string()),
                (1, "docs/design_(old).md".to_string()),
            ]
        );
    }

    #[test]
    fn core_files_detects_missing() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_missing_core");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        let result = check_core_files(&temp);
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn adr_uniqueness_detects_duplicates() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_adr_dup");
        let _ = fs::remove_dir_all(&temp);
        let adr_dir = temp.join("docs/adr");
        fs::create_dir_all(&adr_dir).unwrap();
        fs::write(adr_dir.join("ADR-0001-first.md"), "# ADR 1").unwrap();
        fs::write(adr_dir.join("ADR-0001-duplicate.md"), "# ADR 1 duplicate").unwrap();
        let result = check_adr_uniqueness(&temp);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Duplicate ADR IDs detected"));
        assert!(err_msg.contains("0001"));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn markdown_links_detects_broken_target_with_file_and_line() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_broken_markdown_link");
        let _ = fs::remove_dir_all(&temp);
        let docs_dir = temp.join("docs");
        let guide_dir = docs_dir.join("guide");
        fs::create_dir_all(&guide_dir).unwrap();
        fs::write(
            guide_dir.join("guide.md"),
            "# Guide\n\n[broken](does_not_exist.md)\n",
        )
        .unwrap();
        let result = check_markdown_links(&temp);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Broken local Markdown links detected"));
        assert!(err_msg.contains("docs/guide/guide.md:3"));
        assert!(err_msg.contains("does_not_exist.md"));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn markdown_links_accept_local_paths_directories_and_non_file_targets() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_valid_markdown_links");
        let _ = fs::remove_dir_all(&temp);
        let guide_dir = temp.join("docs/guide");
        fs::create_dir_all(guide_dir.join("adr")).unwrap();
        fs::write(guide_dir.join("target.md"), "# Historical target\n").unwrap();
        fs::write(guide_dir.join("space name.md"), "# Encoded target\n").unwrap();
        fs::write(guide_dir.join("design_(old).md"), "# Parentheses\n").unwrap();
        fs::write(
            guide_dir.join("guide.md"),
            concat!(
                "[relative](target.md#old-anchor)\n",
                "[directory](adr/?view=old#section)\n",
                "[encoded](<space%20name.md?download=1#part>)\n",
                "[nested](design_(old).md)\n",
                "[https](https://example.com/missing.md) [mail](mailto:docs@example.com) [same](#top)\n",
                "`[inline code](missing-inline.md)`\n",
                "```markdown\n[code example](missing-fenced.md)\n```\n",
            ),
        )
        .unwrap();

        let result = check_markdown_links(&temp);
        let _ = fs::remove_dir_all(&temp);
        assert!(result.is_ok(), "unexpected link error: {result:?}");
    }

    #[test]
    fn markdown_links_reject_targets_outside_repository_root() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_markdown_link_escape");
        let _ = fs::remove_dir_all(&temp);
        let guide_dir = temp.join("docs/nested");
        fs::create_dir_all(&guide_dir).unwrap();
        fs::write(
            guide_dir.join("guide.md"),
            "[escape](../../../outside.md)\n",
        )
        .unwrap();

        let result = check_markdown_links(&temp);
        let _ = fs::remove_dir_all(&temp);
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("docs/nested/guide.md:1"));
        assert!(err_msg.contains("escapes repository root"));
    }
}

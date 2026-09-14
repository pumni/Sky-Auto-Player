use crate::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

pub(crate) fn extract_markdown_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = content;
    while let Some(start_bracket) = rest.find('[') {
        let after_start = &rest[start_bracket + 1..];
        if let Some(end_bracket) = after_start.find(']') {
            let after_end = &after_start[end_bracket + 1..];
            if let Some(inside_paren) = after_end.strip_prefix('(')
                && let Some(close_paren) = inside_paren.find(')')
            {
                let link = inside_paren[..close_paren].trim();
                let target = link.split_whitespace().next().unwrap_or(link);
                if !target.is_empty() {
                    links.push(target.to_string());
                }
                rest = &inside_paren[close_paren + 1..];
                continue;
            }
        }
        rest = after_start;
    }
    links
}

pub(crate) fn check_index_links(root: &Path) -> Result<()> {
    let index_path = root.join("docs/INDEX.md");
    if !index_path.is_file() {
        return Err(format!("Documentation router missing: {}", index_path.display()).into());
    }

    let content = fs::read_to_string(&index_path)?;
    let docs_dir = root.join("docs");
    let links = extract_markdown_links(&content);

    let mut broken_links = Vec::new();
    for link in links {
        // Strip intra-page anchors (#heading)
        let path_part = match link.split('#').next() {
            Some(part) => part.trim(),
            None => continue,
        };

        if path_part.is_empty() {
            continue;
        }

        // Ignore external or non-file URLs
        if path_part.starts_with("http://")
            || path_part.starts_with("https://")
            || path_part.starts_with("mailto:")
        {
            continue;
        }

        // Resolve relative to docs/
        let resolved = docs_dir.join(path_part);
        if !resolved.exists() {
            broken_links.push(format!(
                "`{link}` -> resolved to non-existent path: {}",
                resolved.display()
            ));
        }
    }

    if !broken_links.is_empty() {
        return Err(format!(
            "Broken relative links detected in docs/INDEX.md:\n  - {}",
            broken_links.join("\n  - ")
        )
        .into());
    }

    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    check_core_files(root)?;
    check_adr_uniqueness(root)?;
    check_index_links(root)?;
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
        assert_eq!(links, vec!["file.md", "#section", "https://example.com"]);
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
    fn index_links_detects_broken_relative_target() {
        let temp = std::env::temp_dir().join("sap_docs_integrity_test_broken_link");
        let _ = fs::remove_dir_all(&temp);
        let docs_dir = temp.join("docs");
        fs::create_dir_all(&docs_dir).unwrap();
        fs::write(
            docs_dir.join("INDEX.md"),
            "# Router\n- [valid](valid.md)\n- [broken](does_not_exist.md)\n",
        )
        .unwrap();
        fs::write(docs_dir.join("valid.md"), "# Valid").unwrap();
        let result = check_index_links(&temp);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Broken relative links detected"));
        assert!(err_msg.contains("does_not_exist.md"));
        let _ = fs::remove_dir_all(&temp);
    }
}

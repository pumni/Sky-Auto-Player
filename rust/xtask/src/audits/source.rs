use crate::Result;
use std::path::Path;
use walkdir::WalkDir;

pub(super) fn strip_rust_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut block_depth = 0usize;
    while index < bytes.len() {
        if block_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_depth += 1;
                result.push(' ');
                result.push(' ');
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_depth -= 1;
                result.push(' ');
                result.push(' ');
                index += 2;
            } else {
                if bytes[index] == b'\n' {
                    result.push('\n');
                } else {
                    result.push(' ');
                }
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                result.push(' ');
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            block_depth = 1;
            result.push(' ');
            result.push(' ');
            index += 2;
        } else {
            result.push(bytes[index] as char);
            index += 1;
        }
    }
    result
}

pub(crate) fn walk_source(root: &Path, prefix: &str) -> Result<Vec<std::path::PathBuf>> {
    let directory = root.join(prefix);
    let mut files = Vec::new();
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file()
            && matches!(
                entry.path().extension().and_then(|e| e.to_str()),
                Some("rs" | "toml" | "yml" | "yaml" | "json" | "ts" | "tsx")
            )
        {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

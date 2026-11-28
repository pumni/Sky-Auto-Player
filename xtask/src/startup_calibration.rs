#![forbid(unsafe_code)]

use crate::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn canonical_projection_bytes(serialized_json: &str) -> Vec<u8> {
    let normalized = serialized_json.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim_end_matches(['\r', '\n']);
    let mut bytes = normalized.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(".sha256");
    PathBuf::from(sidecar)
}

fn write_projection(serialized_json: &str, output: &Path) -> Result<String> {
    if serialized_json.is_empty() {
        return Err("startup calibration projection input must not be empty".into());
    }
    let file_name = output
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or("startup calibration output path must name a file")?;
    let bytes = canonical_projection_bytes(serialized_json);
    let digest = crate::repo::hex_digest(Sha256::digest(&bytes));
    let mut sidecar = digest.clone().into_bytes();
    sidecar.extend_from_slice(b"  ");
    sidecar.extend_from_slice(file_name.to_string_lossy().as_bytes());
    sidecar.push(b'\n');
    fs::write(output, &bytes)?;
    fs::write(sidecar_path(output), sidecar)?;
    Ok(digest)
}

fn project_evidence(input: &Path, output: &Path) -> Result<()> {
    let serialized = fs::read_to_string(input)
        .map_err(|error| format!("could not read startup calibration JSON input: {error}"))?;
    let digest = write_projection(&serialized, output)?;
    println!("SHA256 {digest}");
    Ok(())
}

fn parse_options(args: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut options = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].strip_prefix("--").ok_or_else(|| {
            format!(
                "startup calibration option must start with --: {}",
                args[index]
            )
        })?;
        if !["input", "output"].contains(&name) {
            return Err(format!("unknown startup calibration option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!(
                "startup calibration option was provided more than once: --{name}"
            )
            .into());
        }
        index += 1;
    }
    Ok(options)
}

pub(crate) fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("project-evidence") {
        return Err("startup calibration tooling only supports project-evidence".into());
    }
    let options = parse_options(&args[1..])?;
    let input = options
        .get("input")
        .ok_or("startup calibration project-evidence requires --input")?;
    let output = options
        .get("output")
        .ok_or("startup calibration project-evidence requires --output")?;
    project_evidence(Path::new(input), Path::new(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nonce:x}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn startup_projection_normalizes_line_endings_and_hashes_exact_utf8_bytes() {
        let root = temp_dir("sky-xtask-startup-projection");
        let lf_path = root.join("wake-calibration-lf.json");
        let crlf_path = root.join("wake-calibration-crlf.json");
        let lf = "{\n  \"label\": \"Māori 🎵\"\n}\n\n";
        let crlf = lf.replace('\n', "\r\n");
        let lf_digest = write_projection(lf, &lf_path).unwrap();
        let crlf_digest = write_projection(&crlf, &crlf_path).unwrap();
        assert_eq!(lf_digest, crlf_digest);
        assert_eq!(fs::read(&lf_path).unwrap(), fs::read(&crlf_path).unwrap());
        assert_eq!(
            fs::read(sidecar_path(&lf_path)).unwrap(),
            format!("{lf_digest}  wake-calibration-lf.json\n").as_bytes()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_projection_rejects_empty_input_and_paths_without_file_names() {
        let root = temp_dir("sky-xtask-startup-projection-invalid");
        assert!(write_projection("", &root.join("out.json")).is_err());
        assert!(write_projection("{}", &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

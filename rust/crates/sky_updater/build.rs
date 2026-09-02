use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() {
    if let Err(error) = generate() {
        panic!("failed to generate update trust metadata: {error}");
    }
}

fn generate() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());

    let value: toml::Value = toml::from_str(&fs::read_to_string(&manifest)?)?;
    let signing = value
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("metadata"))
        .and_then(toml::Value::as_table)
        .and_then(|metadata| metadata.get("sky-update-signing"))
        .and_then(toml::Value::as_table)
        .ok_or("workspace.metadata.sky-update-signing is missing")?;
    let key_id = signing
        .get("key-id")
        .and_then(toml::Value::as_str)
        .ok_or("update signing key-id is missing")?;
    if key_id.is_empty()
        || key_id.len() > 64
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("update signing key-id is unsafe".into());
    }
    let public_key_hex = signing
        .get("public-key-hex")
        .and_then(toml::Value::as_str)
        .ok_or("update signing public-key-hex is missing")?;
    let public_key = decode_hex(public_key_hex)
        .ok_or("update signing public-key-hex must contain exactly 32 bytes of hexadecimal")?;

    let generated = format!(
        "pub const RELEASE_KEY_ID: &str = {key_id:?};\npub const RELEASE_PUBLIC_KEY: [u8; 32] = {public_key:?};\n"
    );
    let output = PathBuf::from(env::var("OUT_DIR")?).join("update_trust.rs");
    fs::write(output, generated)?;
    Ok(())
}

fn decode_hex(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte =
            (hex_digit(value.as_bytes()[offset])? << 4) | hex_digit(value.as_bytes()[offset + 1])?;
    }
    Some(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

use crate::Result;
use std::path::Path;
use toml::Value;

#[derive(Debug, Clone)]
pub struct UpdateTrustConfig {
    pub key_id: String,
    pub public_key: [u8; 32],
    pub public_key_hex: String,
}

pub fn load(root: &Path) -> Result<UpdateTrustConfig> {
    let path = root.join("rust/Cargo.toml");
    let value: Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    let signing = value
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("metadata"))
        .and_then(Value::as_table)
        .and_then(|metadata| metadata.get("sky-update-signing"))
        .and_then(Value::as_table)
        .ok_or("workspace.metadata.sky-update-signing is missing")?;
    let key_id = signing
        .get("key-id")
        .and_then(Value::as_str)
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
        .and_then(Value::as_str)
        .ok_or("update signing public-key-hex is missing")?
        .to_ascii_lowercase();
    let public_key = decode_hex::<32>(&public_key_hex)
        .ok_or("update signing public-key-hex must contain exactly 32 bytes of hexadecimal")?;
    Ok(UpdateTrustConfig {
        key_id: key_id.to_owned(),
        public_key,
        public_key_hex,
    })
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0u8; N];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_single_update_trust_source() {
        let trust = load(&crate::repo::root()).expect("update trust metadata");
        assert_eq!(trust.key_id, "release-2026");
        assert_eq!(trust.public_key_hex.len(), 64);
        assert_eq!(
            trust.public_key_hex,
            crate::repo::hex_digest(trust.public_key)
        );
    }

    #[test]
    fn rejects_invalid_public_key_hex() {
        assert!(decode_hex::<32>("not-a-key").is_none());
        assert!(decode_hex::<32>(&"0".repeat(64)).is_some());
    }
}

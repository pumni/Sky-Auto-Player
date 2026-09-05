use crate::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    Ok(crate::repo::hex_digest(digest.finalize()))
}

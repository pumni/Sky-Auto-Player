use crate::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub(crate) fn sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(crate::repo::hex_digest(digest.finalize()))
}

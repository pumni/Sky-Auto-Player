use super::*;

#[derive(Debug, Default)]
pub(super) struct DownloadProgressAccumulator {
    pub(super) downloaded: u64,
}

impl DownloadProgressAccumulator {
    pub(super) fn record_chunk(
        &mut self,
        chunk_length: usize,
        content_length: Option<u64>,
    ) -> Result<UpdateProgressDto, String> {
        let chunk_length = u64::try_from(chunk_length)
            .map_err(|_| "update download chunk length is not representable".to_string())?;
        let downloaded = self
            .downloaded
            .checked_add(chunk_length)
            .ok_or_else(|| "update download progress overflowed".to_string())?;
        if downloaded > MAX_ARTIFACT_BYTES {
            return Err("update download exceeds the bounded artifact size".to_string());
        }

        let total = content_length.filter(|value| (1..=MAX_ARTIFACT_BYTES).contains(value));
        if total.is_some_and(|total| downloaded > total) {
            return Err("update download progress exceeds its content length".to_string());
        }

        self.downloaded = downloaded;
        Ok(UpdateProgressDto {
            completed: downloaded,
            total,
            message: "Downloading update".to_string(),
        })
    }
}

pub(super) fn verified_artifact_progress(bytes: &[u8], message: &str) -> UpdateProgressDto {
    let bytes_len = bytes.len() as u64;
    debug_assert!(bytes_len <= MAX_ARTIFACT_BYTES);
    UpdateProgressDto {
        completed: bytes_len,
        total: Some(bytes_len),
        message: message.to_string(),
    }
}

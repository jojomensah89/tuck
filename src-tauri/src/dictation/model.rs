use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

const MODEL_FILE: &str = "ggml-tiny.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
const MODEL_MAX_BYTES: u64 = 100 * 1024 * 1024;
const MODEL_SHA256: &str = "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21";

pub(super) fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve model directory: {e}"))?;
    Ok(root.join("dictation").join(MODEL_FILE))
}

pub(super) fn download(
    app: &AppHandle,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let path = model_path(app)?;
    if path.is_file() {
        return Err("The offline dictation model is already installed".into());
    }
    let parent = path.parent().ok_or("Invalid model directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create model directory: {e}"))?;
    let temporary = parent.join(format!("{MODEL_FILE}.part"));
    let response = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Could not initialize model download: {e}"))?
        .get(MODEL_URL)
        .send()
        .map_err(|e| format!("Model download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Model download failed: {e}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > MODEL_MAX_BYTES)
    {
        return Err("The model server reported a file larger than the 100 MiB limit".into());
    }
    let mut reader = response;
    let cleanup = TemporaryModelFile(temporary.clone());
    let mut output = std::fs::File::create(&temporary)
        .map_err(|e| format!("Could not create temporary model file: {e}"))?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            drop(output);
            let _ = std::fs::remove_file(&temporary);
            return Err("Model download cancelled".into());
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("Could not read model response: {e}"))?;
        if count == 0 {
            break;
        }
        received = received
            .checked_add(count as u64)
            .ok_or("Model size overflow")?;
        if received > MODEL_MAX_BYTES {
            drop(output);
            let _ = std::fs::remove_file(&temporary);
            return Err("Model download exceeded the 100 MiB limit".into());
        }
        hash.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|e| format!("Could not write model file: {e}"))?;
        progress(received);
    }
    output
        .flush()
        .map_err(|e| format!("Could not flush model file: {e}"))?;
    output
        .sync_all()
        .map_err(|e| format!("Could not sync model file: {e}"))?;
    drop(output);
    if cancel.load(Ordering::Acquire) {
        let _ = std::fs::remove_file(&temporary);
        return Err("Model download cancelled".into());
    }
    let actual = hex_digest(hash.finalize().as_slice());
    if actual != MODEL_SHA256 {
        let _ = std::fs::remove_file(&temporary);
        return Err("Downloaded model failed its SHA-256 integrity check".into());
    }
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not install verified model: {e}"))?;
    drop(cleanup);
    Ok(())
}

struct TemporaryModelFile(PathBuf);

impl Drop for TemporaryModelFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_model_digest_is_sha256_width_and_hex() {
        assert_eq!(MODEL_SHA256.len(), 64);
        assert!(MODEL_SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn digest_encoding_is_lowercase_hex() {
        assert_eq!(hex_digest(&[0x00, 0xab, 0xff]), "00abff");
    }
}

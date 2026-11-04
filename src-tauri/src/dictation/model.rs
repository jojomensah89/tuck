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

pub(super) fn verify_installed(path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Could not open the offline dictation model: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("Could not inspect the offline dictation model: {e}"))?
        .len();
    if size == 0 || size > MODEL_MAX_BYTES {
        return Err(
            "The installed offline model has an invalid size. Remove it and download it again."
                .into(),
        );
    }
    let mut reader = file;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("Could not verify the offline dictation model: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if !matches_expected_digest(&hex_digest(&hash.finalize())) {
        return Err("The installed offline model failed its SHA-256 integrity check. Remove it and download it again.".into());
    }
    Ok(())
}

pub(super) async fn download(
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
    let _cleanup = TemporaryModelFile(temporary.clone());
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Could not initialize model download: {e}"))?;
    let response = tokio::select! {
        _ = wait_until_cancelled(cancel) => return Err("Model download cancelled".into()),
        result = client.get(MODEL_URL).send() => result
            .map_err(|e| format!("Model download failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Model download failed: {e}"))?,
    };
    if response.url().scheme() != "https" {
        return Err("Model download redirected to a non-HTTPS URL".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size > MODEL_MAX_BYTES)
    {
        return Err("The model server reported a file larger than the 100 MiB limit".into());
    }
    let mut reader = response;
    let mut output = std::fs::File::create(&temporary)
        .map_err(|e| format!("Could not create temporary model file: {e}"))?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            drop(output);
            let _ = std::fs::remove_file(&temporary);
            return Err("Model download cancelled".into());
        }
        let chunk = tokio::select! {
            _ = wait_until_cancelled(cancel) => return Err("Model download cancelled".into()),
            result = reader.chunk() => result
                .map_err(|e| format!("Could not read model response: {e}"))?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or("Model size overflow")?;
        if received > MODEL_MAX_BYTES {
            return Err("Model download exceeded the 100 MiB limit".into());
        }
        hash.update(&chunk);
        output
            .write_all(&chunk)
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
        return Err("Model download cancelled".into());
    }
    let digest = hash.finalize();
    let actual = hex_digest(&digest);
    if !matches_expected_digest(&actual) {
        return Err("Downloaded model failed its SHA-256 integrity check".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Model download cancelled".into());
    }
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not install verified model: {e}"))?;
    Ok(())
}

async fn wait_until_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
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

fn matches_expected_digest(actual: &str) -> bool {
    actual == MODEL_SHA256
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

    #[test]
    fn expected_model_digest_is_required_for_inference() {
        assert!(matches_expected_digest(MODEL_SHA256));
        assert!(!matches_expected_digest(&"0".repeat(64)));
    }

    #[test]
    fn temporary_model_file_guard_removes_partial_downloads() {
        let path = std::env::temp_dir().join(format!("tuck-model-{}.part", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"partial").unwrap();
        {
            let _cleanup = TemporaryModelFile(path.clone());
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn cancelled_download_waiter_resolves_without_network_or_model_data() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let cancel = AtomicBool::new(true);
        runtime.block_on(wait_until_cancelled(&cancel));
    }
}

//! Identity receipts for files created by Tuck's downloader. Catalog names are not ownership.
use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct Receipt {
    format_version: u32,
    name: String,
    temporary: bool,
    identity: FileIdentity,
}

pub(super) fn record(path: &Path, temporary: bool) -> Result<PathBuf, String> {
    let receipt = Receipt {
        format_version: 2,
        name: path
            .file_name()
            .ok_or("Missing filename")?
            .to_string_lossy()
            .into_owned(),
        temporary,
        identity: file_identity(path)?,
    };
    if receipt.identity.file_id.is_empty() {
        return Err("Filesystem cannot establish download ownership".into());
    }
    let record_path = path
        .parent()
        .ok_or("Missing model folder")?
        .join(format!(".tuck-download-{}.json", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&record_path)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(record_path)
}

fn protected_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options.open(path).map_err(|e| e.to_string())
}

pub(super) fn record_completed(
    path: &Path,
    source: &FileIdentity,
    digest: &str,
) -> Result<(), String> {
    let file = protected_file(path)?;
    let current = identity_from_file(&file)?;
    // Removing the temporary hard link changes change time, but not this identity/content stamp.
    if current.file_id != source.file_id || current.size != source.size {
        return Err("Installed file changed; ownership was not claimed".into());
    }
    if hash_file(path, true)? != digest {
        return Err("Installed model digest changed; ownership was not claimed".into());
    }
    record(path, false)?;
    Ok(())
}

fn receipts(dir: &Path) -> Vec<(PathBuf, Receipt)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            if !name.starts_with(".tuck-download-") || !name.ends_with(".json") {
                return None;
            }
            if item.metadata().ok()?.len() > 4096 {
                return None;
            }
            let bytes = std::fs::read(item.path()).ok()?;
            if bytes.len() > 4096 {
                return None;
            }
            let receipt: Receipt = serde_json::from_slice(&bytes).ok()?;
            if receipt.format_version != 2 {
                return None;
            }
            if Path::new(&receipt.name).components().count() != 1
                || receipt.name.contains(['/', '\\', ':'])
            {
                return None;
            }
            Some((item.path(), receipt))
        })
        .collect()
}

fn matches(receipt: &Receipt, identity: &FileIdentity) -> bool {
    !identity.file_id.is_empty()
        && if receipt.temporary {
            receipt.identity.file_id == identity.file_id
        } else {
            receipt.identity == *identity
        }
}

pub(in crate::dictation) fn is_owned(path: &Path, entry: &CatalogEntry) -> bool {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    let Ok(identity) = file_identity(path) else {
        return false;
    };
    let Some(dir) = path.parent() else {
        return false;
    };
    path.file_name().is_some_and(|name| name == entry.id)
        && receipts(dir).iter().any(|(_, r)| {
            !r.temporary
                && path.file_name().is_some_and(|n| n == r.name.as_str())
                && matches(r, &identity)
        })
}

// Hold a non-delete-sharing Windows handle from identity validation through deletion.
// This prevents replacement between checking a receipt and deleting the named file.
fn delete_matching(path: &Path, receipt: &Receipt) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Linked files are preserved".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
            },
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80000000 | 0x00010000)
            .share_mode(1 | 2)
            .open(path)
            .map_err(|e| e.to_string())?;
        if !matches(receipt, &identity_from_file(&file)?) {
            return Err("Model identity changed; file was preserved".into());
        }
        let info = FILE_DISPOSITION_INFO {
            DeleteFile: true.into(),
        };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        }
        .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, receipt);
        Err("Safe owned-file removal requires Windows".into())
    }
}

pub(in crate::dictation) fn remove_owned(path: &Path, entry: &CatalogEntry) -> Result<(), String> {
    if !is_owned(path, entry) {
        return Ok(());
    } // Stop using, preserve unowned file.
    let dir = path.parent().ok_or("Missing model directory")?;
    let (record_path, receipt) = receipts(dir)
        .into_iter()
        .find(|(_, r)| {
            !r.temporary
                && path.file_name().is_some_and(|n| n == r.name.as_str())
                && file_identity(path).is_ok_and(|i| matches(r, &i))
        })
        .ok_or("Ownership changed; model preserved")?;
    delete_matching(path, &receipt)?;
    let _ = std::fs::remove_file(record_path);
    Ok(())
}

pub(super) struct TrackedTemporary {
    path: PathBuf,
}
impl Drop for TrackedTemporary {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            sweep_one(dir, &self.path);
        }
    }
}
fn sweep_one(dir: &Path, path: &Path) {
    for (record_path, receipt) in receipts(dir) {
        if receipt.temporary
            && path.file_name().is_some_and(|n| n == receipt.name.as_str())
            && delete_matching(path, &receipt).is_ok()
        {
            let _ = std::fs::remove_file(record_path);
        }
    }
}
pub(super) fn sweep(dir: &Path) {
    for (_, receipt) in receipts(dir) {
        if receipt.temporary {
            sweep_one(dir, &dir.join(receipt.name));
        }
    }
}
pub(super) fn new_temporary(
    dir: &Path,
) -> Result<(PathBuf, std::fs::File, TrackedTemporary), String> {
    let path = dir.join(format!(".tuck-{}.part", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(&path).map_err(|e| e.to_string())?;
    record(&path, true)?;
    Ok((path.clone(), file, TrackedTemporary { path }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuck-ownership-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn exclusive_download_install_preserves_existing_destinations() {
        let dir = temp_dir();
        let (part, mut file, guard) = new_temporary(&dir).unwrap();
        file.write_all(b"download").unwrap();
        file.sync_all().unwrap();
        let destination = dir.join("ggml-base.bin");
        std::fs::write(&destination, b"owner file").unwrap();
        assert!(std::fs::hard_link(&part, &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"owner file");
        let installed = dir.join("ggml-small.bin");
        std::fs::hard_link(&part, &installed).unwrap();
        let source_identity = identity_from_file(&file).unwrap();
        drop(file);
        drop(guard);
        record_completed(
            &installed,
            &source_identity,
            &hash_file(&installed, false).unwrap(),
        )
        .unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"download");
        assert!(is_owned(&installed, entry("ggml-small.bin").unwrap()));
        let (part, file, guard) = new_temporary(&dir).unwrap();
        drop(file);
        std::fs::remove_file(&part).unwrap();
        std::fs::write(&part, b"replacement").unwrap();
        drop(guard);
        assert!(part.exists(), "replaced partial preserved");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unowned_replaced_and_tracked_files_are_distinguished() {
        let dir = temp_dir();
        let path = dir.join("ggml-base.bin");
        std::fs::write(&path, b"user model").unwrap();
        let entry = entry("ggml-base.bin").unwrap();
        remove_owned(&path, entry).unwrap();
        assert!(path.exists());
        record(&path, false).unwrap();
        assert!(is_owned(&path, entry));
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert!(!is_owned(&path, entry));
        remove_owned(&path, entry).unwrap();
        assert!(path.exists());
        let (part, file, guard) = new_temporary(&dir).unwrap();
        drop(file);
        drop(guard);
        assert!(!part.exists());
        let arbitrary = dir.join("ggml-base.bin.part");
        std::fs::write(&arbitrary, b"keep").unwrap();
        let (part, file, guard) = new_temporary(&dir).unwrap();
        drop(file);
        std::mem::forget(guard);
        sweep(&dir);
        assert!(!part.exists());
        assert!(arbitrary.exists());
        record(&path, false).unwrap();
        remove_owned(&path, entry).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    fn tiny_fixture() -> (PathBuf, PathBuf, CatalogEntry) {
        let dir = temp_dir();
        let path = dir.join("ggml-tiny.bin");
        std::fs::write(&path, b"matching tiny fixture").unwrap();
        let tiny = CatalogEntry {
            bytes: 21,
            sha256: Box::leak(hash_file(&path, false).unwrap().into_boxed_str()),
            ..*entry("ggml-tiny.bin").unwrap()
        };
        (dir, path, tiny)
    }
    fn assert_preserved(path: &Path, tiny: &CatalogEntry) {
        assert_eq!(hash_file(path, false).unwrap(), tiny.sha256);
        assert!(!is_owned(path, tiny));
        remove_owned(path, tiny).unwrap();
        assert!(path.exists());
    }
    #[test]
    fn untracked_matching_tiny_is_not_adopted() {
        let (dir, path, tiny) = tiny_fixture();
        assert_preserved(&path, &tiny);
        assert!(receipts(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn old_unversioned_adoption_receipts_do_not_establish_provenance() {
        let (dir, path, tiny) = tiny_fixture();
        let receipt = serde_json::json!({"name":"ggml-tiny.bin", "temporary":false, "identity":file_identity(&path).unwrap()});
        std::fs::write(
            dir.join(".tuck-download-old.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert_preserved(&path, &tiny);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn matching_tiny_with_a_lost_receipt_is_not_removable() {
        let (dir, path, tiny) = tiny_fixture();
        let receipt = record(&path, false).unwrap();
        assert!(is_owned(&path, &tiny));
        std::fs::remove_file(receipt).unwrap();
        assert_preserved(&path, &tiny);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn matching_tiny_with_a_corrupt_receipt_is_not_removable() {
        let (dir, path, tiny) = tiny_fixture();
        let receipt = record(&path, false).unwrap();
        std::fs::write(receipt, b"not a receipt").unwrap();
        assert_preserved(&path, &tiny);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn replacement_tiny_with_identical_content_is_not_removable() {
        let (dir, path, tiny) = tiny_fixture();
        record(&path, false).unwrap();
        let content = std::fs::read(&path).unwrap();
        std::fs::rename(&path, dir.join("original.bin")).unwrap();
        std::fs::write(&path, content).unwrap();
        assert_preserved(&path, &tiny);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

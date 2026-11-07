//! Compile-time Whisper catalog, integrity checks, downloads and on-demand detection of
//! models other apps already downloaded.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

/// Commit of `ggerganov/whisper.cpp` on Hugging Face, so a re-upload cannot change a file.
/// Sizes and digests below come from that repository's tree API at this commit.
const SOURCE_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
/// The file Tuck installed before the picker existed. Old installs keep using it.
pub(super) const LEGACY_MODEL: &str = "ggml-tiny.bin";
/// A download fails when no data arrives for this long, however large the file is.
const STALL_LIMIT: Duration = Duration::from_secs(60);
const FREE_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
const VERIFY_CACHE_FILE: &str = "verified-models.json";
const VERIFY_CACHE_MAX_BYTES: u64 = 256 * 1024;
const VERIFY_CACHE_MAX_ENTRIES: usize = 128;
const GIB: u64 = 1024 * 1024 * 1024;
const SCAN_MAX_ENTRIES: usize = 20_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CatalogEntry {
    /// The file name, also the stable id stored in `dictation.json`.
    pub id: &'static str,
    pub label: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub speed: &'static str,
    pub accuracy: &'static str,
    pub note: Option<&'static str>,
    /// Offered for download. The others are only shown when found on this computer.
    pub offered: bool,
    /// Peak memory while transcribing, measured or scaled as noted at `CATALOG`.
    pub memory_mb: u64,
}

const SLOW_NOTE: &str =
    "Takes longer to write the text than you took to say it, and live transcript turns itself off.";
const ENGLISH_NOTE: &str = "English only.";
const COMPRESSED_NOTE: &str = "Smaller than Tiny because this file is compressed.";

/// `memory_mb` is the peak working set of a whole 36 s session with live transcript on, from
/// `real_session_with_live_transcript`, for Tiny, Base (both), Small, Medium and Large v3
/// Turbo as offered. The other files are estimates from the research document.
/// Speed was measured on an 8-core i7-11800H under load: the final pass on that clip took
/// 3 to 7 s on Tiny and Base, 18 s on Small, 58 s on Medium and 89 s on Large v3 Turbo.
#[allow(clippy::too_many_arguments)]
const fn offered(
    id: &'static str,
    label: &'static str,
    bytes: u64,
    sha256: &'static str,
    speed: &'static str,
    accuracy: &'static str,
    note: Option<&'static str>,
    memory_mb: u64,
) -> CatalogEntry {
    CatalogEntry {
        id,
        label,
        bytes,
        sha256,
        speed,
        accuracy,
        note,
        offered: true,
        memory_mb,
    }
}

#[allow(clippy::too_many_arguments)]
const fn detected(
    id: &'static str,
    label: &'static str,
    bytes: u64,
    sha256: &'static str,
    speed: &'static str,
    accuracy: &'static str,
    note: Option<&'static str>,
    memory_mb: u64,
) -> CatalogEntry {
    CatalogEntry {
        id,
        label,
        bytes,
        sha256,
        speed,
        accuracy,
        note,
        offered: false,
        memory_mb,
    }
}

/// Offered models are multilingual, including Base: the owner may dictate in more than
/// English, and `ggml-base-q5_1.bin` costs about the same as the `.en` build. The accuracy
/// order follows the WER table in `docs/research/resources-lyrics-models.md`; the speed
/// labels are estimates from that document, not measurements on this computer.
pub(super) const PARAKEET: &str = "parakeet-unified-en-0.6b-Q8_0.gguf";
const PARAKEET_REVISION: &str = "7e948f21b7bdbac698d3318db9d350f1096f3b6c";
pub(super) const CATALOG: &[CatalogEntry] = &[
    offered(
        PARAKEET,
        "Parakeet Unified English (recommended)",
        731357568,
        "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
        "Not measured",
        "Not measured",
        Some("English only. CPU performance and peak memory not measured on this computer."),
        2048,
    ),
    offered(
        "ggml-tiny.bin",
        "Tiny",
        77_691_713,
        "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        "Fastest",
        "Lower",
        None,
        280,
    ),
    offered(
        "ggml-base-q5_1.bin",
        "Base",
        59_707_625,
        "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        "Very fast",
        "Good",
        Some(COMPRESSED_NOTE),
        310,
    ),
    offered(
        "ggml-small-q5_1.bin",
        "Small",
        190_085_487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        "Fast",
        "Better",
        None,
        550,
    ),
    offered(
        "ggml-medium-q5_0.bin",
        "Medium",
        539_212_467,
        "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        "Slow",
        "High",
        Some(SLOW_NOTE),
        1070,
    ),
    offered(
        "ggml-large-v3-turbo-q5_0.bin",
        "Large v3 Turbo",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        "Slowest",
        "Highest",
        Some(SLOW_NOTE),
        1010,
    ),
    detected(
        "ggml-tiny.en.bin",
        "Tiny English",
        77_704_715,
        "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        "Fastest",
        "Good",
        Some(ENGLISH_NOTE),
        280,
    ),
    detected(
        "ggml-base.bin",
        "Base",
        147_951_465,
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        "Very fast",
        "Good",
        None,
        390,
    ),
    detected(
        "ggml-base.en.bin",
        "Base English",
        147_964_211,
        "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        "Very fast",
        "Good",
        Some(ENGLISH_NOTE),
        390,
    ),
    detected(
        "ggml-base.en-q5_1.bin",
        "Base English",
        59_721_011,
        "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        "Very fast",
        "Good",
        Some(ENGLISH_NOTE),
        310,
    ),
    detected(
        "ggml-small.bin",
        "Small",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "Fast",
        "Better",
        None,
        900,
    ),
    detected(
        "ggml-small.en.bin",
        "Small English",
        487_614_201,
        "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        "Fast",
        "Better",
        Some(ENGLISH_NOTE),
        900,
    ),
    detected(
        "ggml-small.en-q5_1.bin",
        "Small English",
        190_098_681,
        "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        "Fast",
        "Better",
        Some(ENGLISH_NOTE),
        550,
    ),
    detected(
        "ggml-medium.bin",
        "Medium",
        1_533_763_059,
        "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        "Slow",
        "High",
        Some(SLOW_NOTE),
        2100,
    ),
    detected(
        "ggml-medium.en.bin",
        "Medium English",
        1_533_774_781,
        "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        "Slow",
        "High",
        Some(SLOW_NOTE),
        2100,
    ),
    detected(
        "ggml-large-v3-turbo.bin",
        "Large v3 Turbo",
        1_624_555_275,
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        "Slowest",
        "Highest",
        Some(SLOW_NOTE),
        2200,
    ),
];

pub(super) fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.id == id)
}

fn download_url(entry: &CatalogEntry) -> String {
    if entry.id == PARAKEET {
        return format!("https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/{PARAKEET_REVISION}/{}", entry.id);
    }
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/{SOURCE_REVISION}/{}",
        entry.id
    )
}

/* ---------- Integrity: hash once, then remember ---------- */

/// `path`, `size`, `modified_ns`, `file_id` and `change_ns` identify the file. `file_id` and
/// `change_ns` come from NTFS and catch a same-size replacement with its modified time set
/// back, which the first three cannot. `sha256` is what the file hashed to, kept for
/// mismatches too, so a same-size stranger is not hashed again on every scan.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct VerifiedFile {
    path: String,
    size: u64,
    modified_ns: u64,
    #[serde(default)]
    file_id: String,
    #[serde(default)]
    change_ns: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileIdentity {
    size: u64,
    modified_ns: u64,
    file_id: String,
    change_ns: u64,
}

pub(super) struct VerifyCache {
    file: Option<PathBuf>,
    entries: Vec<VerifiedFile>,
    /// Other apps' files hashed since launch. Their results are never trusted from disk:
    /// those apps can replace them at any time.
    hashed_this_launch: Option<HashSet<String>>,
}

impl VerifyCache {
    pub(super) const fn empty() -> Self {
        Self {
            file: None,
            entries: Vec::new(),
            hashed_this_launch: None,
        }
    }

    /// A missing or damaged cache only means files are hashed once more.
    pub(super) fn load(models_dir: &Path) -> Self {
        let file = models_dir.join(VERIFY_CACHE_FILE);
        let entries = std::fs::metadata(&file)
            .ok()
            .filter(|meta| meta.len() <= VERIFY_CACHE_MAX_BYTES)
            .and_then(|_| std::fs::read(&file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            file: Some(file),
            entries,
            hashed_this_launch: None,
        }
    }

    fn is_external(&self, path: &Path) -> bool {
        self.file
            .as_deref()
            .and_then(Path::parent)
            .is_none_or(|dir| !path.starts_with(dir))
    }

    /// `None` means "hash it". A file with no NTFS identity is never trusted from the cache.
    fn lookup(&self, path: &Path, key: &str, identity: &FileIdentity) -> Option<&str> {
        if identity.file_id.is_empty() {
            return None;
        }
        if self.is_external(path)
            && !self
                .hashed_this_launch
                .as_ref()
                .is_some_and(|seen| seen.contains(key))
        {
            return None;
        }
        self.entries
            .iter()
            .find(|e| {
                e.path == key
                    && e.size == identity.size
                    && e.modified_ns == identity.modified_ns
                    && e.file_id == identity.file_id
                    && e.change_ns == identity.change_ns
            })
            .map(|e| e.sha256.as_str())
    }

    fn record(&mut self, verified: VerifiedFile) {
        self.hashed_this_launch
            .get_or_insert_with(HashSet::new)
            .insert(verified.path.clone());
        self.entries.retain(|e| e.path != verified.path);
        self.entries.push(verified);
        if self.entries.len() > VERIFY_CACHE_MAX_ENTRIES {
            let excess = self.entries.len() - VERIFY_CACHE_MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.persist();
    }

    fn forget(&mut self, path: &Path) {
        let path = path.to_string_lossy();
        self.entries.retain(|e| e.path != path);
        self.persist();
    }

    fn persist(&self) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let Some(parent) = file.parent() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&self.entries) else {
            return;
        };
        let temporary = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let _cleanup = TemporaryModelFile(temporary.clone());
        let written = std::fs::create_dir_all(parent)
            .and_then(|_| std::fs::write(&temporary, bytes))
            .and_then(|_| super::replace_file(&temporary, file));
        if let Err(error) = written {
            eprintln!("Could not save the verified model list: {error}");
        }
    }
}

fn file_identity(path: &Path) -> Result<FileIdentity, String> {
    let resolved = resolved_local(path)?;
    let file =
        std::fs::File::open(resolved).map_err(|e| format!("Could not open the model: {e}"))?;
    identity_from_file(&file)
}

fn identity_from_file(file: &std::fs::File) -> Result<FileIdentity, String> {
    let meta = file
        .metadata()
        .map_err(|e| format!("Could not open the model: {e}"))?;
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
        });
    let (file_id, change_ns) = native_identity(file);
    Ok(FileIdentity {
        size: meta.len(),
        modified_ns,
        file_id,
        change_ns,
    })
}

/// The volume and file IDs and the change time, which user code cannot set back through
/// the usual APIs. Empty on a volume that has neither, which turns off caching for that file.
#[cfg(windows)]
fn native_identity(file: &std::fs::File) -> (String, u64) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO, FILE_ID_INFO,
        },
    };
    let handle = HANDLE(file.as_raw_handle());
    let mut id = FILE_ID_INFO::default();
    let mut basic = FILE_BASIC_INFO::default();
    let read_id = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    let read_basic = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if read_id.is_err() || read_basic.is_err() {
        return (String::new(), 0);
    }
    (
        format!(
            "{:016x}-{}",
            id.VolumeSerialNumber,
            hex_digest(&id.FileId.Identifier)
        ),
        u64::try_from(basic.ChangeTime).unwrap_or(0),
    )
}

#[cfg(not(windows))]
fn native_identity(_file: &std::fs::File) -> (String, u64) {
    (String::new(), 0)
}

/// Confirms `path` is exactly `entry`, hashing only when this size and modified time have
/// not been seen. Large models are slow to hash, so a recording start must not do it again.
pub(super) fn verify(
    path: &Path,
    entry: &CatalogEntry,
    cache: &Mutex<VerifyCache>,
    low_priority: bool,
) -> Result<(), String> {
    verify_current(path, entry, cache, low_priority, false)
}

fn verify_current(
    path: &Path,
    entry: &CatalogEntry,
    cache: &Mutex<VerifyCache>,
    low_priority: bool,
    fresh: bool,
) -> Result<(), String> {
    let identity = file_identity(path)?;
    if identity.size != entry.bytes {
        return Err(format!(
            "{} has the wrong size. Remove it and download it again.",
            entry.label
        ));
    }
    let key = path.to_string_lossy().into_owned();
    let cached = cache
        .lock()
        .ok()
        .and_then(|c| c.lookup(path, &key, &identity).map(str::to_owned));
    // Acquisition must confirm current bytes even when Windows coalesces metadata
    // timestamps for an immediate same-size rewrite. Recording retains its cache.
    let actual = match cached.filter(|_| !fresh) {
        Some(sha256) => sha256,
        None => {
            let sha256 = hash_file(path, low_priority)?;
            if let Ok(mut cache) = cache.lock() {
                cache.record(VerifiedFile {
                    path: key,
                    size: identity.size,
                    modified_ns: identity.modified_ns,
                    file_id: identity.file_id,
                    change_ns: identity.change_ns,
                    sha256: sha256.clone(),
                });
            }
            sha256
        }
    };
    if actual != entry.sha256 {
        return Err(format!(
            "{} failed its SHA-256 integrity check. Remove it and download it again.",
            entry.label
        ));
    }
    Ok(())
}

/// Drops a removed file from the cache, so a later file at that path is hashed.
pub(super) fn forget(path: &Path, cache: &Mutex<VerifyCache>) {
    if let Ok(mut cache) = cache.lock() {
        cache.forget(path);
    }
}

fn hash_file(path: &Path, low_priority: bool) -> Result<String, String> {
    let resolved = resolved_local(path)?;
    let path = resolved.as_path();
    let _priority = low_priority.then(BackgroundPriority::enter);
    let mut reader =
        std::fs::File::open(path).map_err(|e| format!("Could not open the model: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("Could not verify the model: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex_digest(&hash.finalize()))
}

/// Background mode lowers this thread's CPU and disk priority until the guard drops.
struct BackgroundPriority;

impl BackgroundPriority {
    fn enter() -> Self {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
        Self
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_END,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
}

#[path = "ownership.rs"]
mod ownership;
pub(super) use ownership::{is_owned, remove_owned};

fn resolved_local(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_string_lossy().replace('/', "\\");
    if text.to_ascii_lowercase().starts_with(r"\\?\unc\")
        || (text.starts_with(r"\\") && !text.starts_with(r"\\?\"))
    {
        return Err("Network models are not supported".into());
    }
    if let Ok(link) = std::fs::read_link(path) {
        let name = link
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if name.starts_with(r"\\?\unc\") || (name.starts_with(r"\\") && !name.starts_with(r"\\?\"))
        {
            return Err("Network models are not supported".into());
        }
    }
    let resolved =
        std::fs::canonicalize(path).map_err(|e| format!("Could not resolve model: {e}"))?;
    let name = resolved
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if name.starts_with(r"\\?\unc\") || (name.starts_with(r"\\") && !name.starts_with(r"\\?\")) {
        return Err("Network models are not supported".into());
    }
    Ok(resolved)
}

/// Resolve links and reject network destinations before asking for target metadata.
pub(super) fn local_model_exists(path: &Path) -> bool {
    resolved_local(path).is_ok_and(|resolved| resolved.is_file())
}

/* ---------- Download ---------- */

/// Fresh bounded discovery, followed by catalog integrity verification. Never copies
/// or modifies a borrowed model. Called only on the model worker thread.
pub(super) fn verified_existing(
    dir: &Path,
    entry: &CatalogEntry,
    locations: &[Location],
    cache: &Mutex<VerifyCache>,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>, String> {
    verified_existing_with_catalog(dir, entry, locations, cache, cancel, CATALOG)
}

pub(super) fn verified_existing_with_catalog(
    dir: &Path,
    entry: &CatalogEntry,
    locations: &[Location],
    cache: &Mutex<VerifyCache>,
    cancel: &AtomicBool,
    catalog: &'static [CatalogEntry],
) -> Result<Option<PathBuf>, String> {
    let own = dir.join(entry.id);
    let candidates = std::iter::once((own, false)).chain(
        find_candidates_with_catalog(locations, catalog)
            .into_iter()
            .filter(|c| c.entry.id == entry.id)
            .map(|c| (c.path, true)),
    );
    verify_candidates(candidates, entry, cache, cancel)
}

fn verify_candidates(
    candidates: impl IntoIterator<Item = (PathBuf, bool)>,
    entry: &CatalogEntry,
    cache: &Mutex<VerifyCache>,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>, String> {
    for (path, external) in candidates {
        if cancel.load(Ordering::Acquire) {
            return Err("Model operation cancelled".into());
        }
        if local_model_exists(&path) && verify_current(&path, entry, cache, external, true).is_ok()
        {
            if cancel.load(Ordering::Acquire) {
                return Err("Model operation cancelled".into());
            }
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// The network callback is lazy: a verified local copy never starts HTTP.
pub(super) async fn acquire<F, Fut>(
    existing: Result<Option<PathBuf>, String>,
    network: F,
) -> Result<Option<PathBuf>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    match existing? {
        Some(path) => Ok(Some(path)),
        None => {
            network().await?;
            Ok(None)
        }
    }
}

pub(super) async fn download(
    models_dir: &Path,
    entry: &CatalogEntry,
    cache: &Mutex<VerifyCache>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let path = models_dir.join(entry.id);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(format!(
            "{} already has a file at its install path",
            entry.label
        ));
    }
    std::fs::create_dir_all(models_dir)
        .map_err(|e| format!("Could not create model directory: {e}"))?;
    check_free_space(models_dir, entry.bytes)?;
    let (temporary, mut output, _cleanup) = ownership::new_temporary(models_dir)?;
    // No total timeout: 600 MB on a slow link is fine as long as data keeps arriving.
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Could not initialize model download: {e}"))?;
    let response = tokio::select! {
        _ = wait_until_cancelled(cancel) => return Err("Model download cancelled".into()),
        result = within_stall_limit(client.get(download_url(entry)).send(), STALL_LIMIT) => result?
            .map_err(|e| format!("Model download failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Model download failed: {e}"))?,
    };
    if response.url().scheme() != "https" {
        return Err("Model download redirected to a non-HTTPS URL".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size != entry.bytes)
    {
        return Err(format!(
            "The model server reported a different size for {}",
            entry.label
        ));
    }
    let mut reader = response;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = wait_until_cancelled(cancel) => return Err("Model download cancelled".into()),
            result = within_stall_limit(reader.chunk(), STALL_LIMIT) => result?
                .map_err(|e| format!("Could not read model response: {e}"))?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or("Model size overflow")?;
        if received > entry.bytes {
            return Err(format!("{} is larger than expected", entry.label));
        }
        hash.update(&chunk);
        output
            .write_all(&chunk)
            .map_err(|e| format!("Could not write model file: {e}"))?;
        progress(received);
    }
    if received != entry.bytes {
        return Err(format!(
            "The download of {} ended early. Try again.",
            entry.label
        ));
    }
    output
        .flush()
        .map_err(|e| format!("Could not flush model file: {e}"))?;
    output
        .sync_all()
        .map_err(|e| format!("Could not sync model file: {e}"))?;
    if hex_digest(&hash.finalize()) != entry.sha256 {
        return Err(format!(
            "The download of {} failed its SHA-256 integrity check",
            entry.label
        ));
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Model download cancelled".into());
    }
    let source_identity = identity_from_file(&output)?;
    // hard_link installs without replacing an existing destination.
    std::fs::hard_link(&temporary, &path)
        .map_err(|e| format!("Could not install verified model without replacing a file: {e}"))?;
    drop(output);
    drop(_cleanup);
    ownership::record_completed(&path, &source_identity, entry.sha256)?;
    // Already hashed while downloading, so the first recording does not repeat it.
    let identity = file_identity(&path)?;
    if let Ok(mut cache) = cache.lock() {
        cache.record(VerifiedFile {
            path: path.to_string_lossy().into_owned(),
            size: identity.size,
            modified_ns: identity.modified_ns,
            file_id: identity.file_id,
            change_ns: identity.change_ns,
            sha256: entry.sha256.to_owned(),
        });
    }
    Ok(())
}

/// Only identity-tracked temporary files from this downloader are eligible.
pub(super) fn sweep_partial_downloads(models_dir: &Path) {
    ownership::sweep(models_dir);
}

async fn within_stall_limit<T>(
    work: impl std::future::Future<Output = T>,
    limit: Duration,
) -> Result<T, String> {
    tokio::time::timeout(limit, work).await.map_err(|_| {
        format!(
            "The model download stalled: no data for {} seconds",
            limit.as_secs()
        )
    })
}

async fn wait_until_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(windows)]
fn check_free_space(dir: &Path, needed: u64) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    // If Windows cannot say, let the download run and fail on the write instead.
    if unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.is_ok()
        && free < needed.saturating_add(FREE_SPACE_MARGIN)
    {
        return Err(format!(
            "Not enough disk space: this model needs {} MB free",
            needed.saturating_add(FREE_SPACE_MARGIN) / 1_000_000
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn check_free_space(_dir: &Path, _needed: u64) -> Result<(), String> {
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

/* ---------- Detection of other apps' models ---------- */

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Location {
    pub app: &'static str,
    pub root: PathBuf,
    pub depth: usize,
    /// In the Hugging Face cache, only `ggml*.bin` under `models--*/snapshots/*`.
    pub huggingface: bool,
}

/// Where other Windows apps keep ggml models. See section 3.6 of the research document.
/// Nothing but these model files is opened: never another app's settings or credentials.
pub(super) fn known_locations(env: impl Fn(&str) -> Option<OsString>) -> Vec<Location> {
    let mut locations = Vec::new();
    let mut add = |app, base: Option<OsString>, parts: &[&str], depth, huggingface| {
        if let Some(base) = base {
            let mut root = PathBuf::from(base);
            root.extend(parts);
            locations.push(Location {
                app,
                root,
                depth,
                huggingface,
            });
        }
    };
    add(
        "Handy",
        env("APPDATA"),
        &["com.pais.handy", "models"],
        1,
        false,
    );
    add("Hex", env("APPDATA"), &["hex", "models"], 1, false);
    add(
        "Buzz",
        env("LOCALAPPDATA"),
        &["Buzz", "Buzz", "Cache"],
        4,
        false,
    );
    add(
        "OpenWhispr",
        env("USERPROFILE"),
        &[".cache", "openwhispr", "whisper-models"],
        1,
        false,
    );
    add(
        "OpenWhispr",
        env("ProgramData"),
        &["OpenWhispr", "cache"],
        2,
        false,
    );
    add(
        "podcli",
        env("LOCALAPPDATA"),
        &["podcli", "models"],
        1,
        false,
    );
    add(
        "Hugging Face cache",
        env("USERPROFILE"),
        &[".cache", "huggingface", "hub"],
        4,
        true,
    );
    locations
}

/// A saved path is only ever a file in a folder this code scans. A network path is refused
/// without being touched, so a slow share cannot stall every state update.
pub(super) fn path_is_known(path: &Path, locations: &[Location]) -> bool {
    use std::path::Component;
    let is_network = path.to_string_lossy().starts_with(r"\\");
    !is_network
        && (std::fs::symlink_metadata(path).is_err() || resolved_local(path).is_ok())
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        && locations
            .iter()
            .any(|location| path.starts_with(&location.root))
}

pub(super) fn system_locations() -> Vec<Location> {
    known_locations(|name| std::env::var_os(name))
}

/// Names the app a model file belongs to, for "Found in <app>".
pub(super) fn app_for(path: &Path, locations: &[Location]) -> Option<&'static str> {
    locations
        .iter()
        .find(|location| path.starts_with(&location.root))
        .map(|location| location.app)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub entry: &'static CatalogEntry,
    pub path: PathBuf,
    pub app: &'static str,
}

fn name_is_candidate(name: &str, huggingface: bool) -> bool {
    let name = name.to_ascii_lowercase();
    if name.eq_ignore_ascii_case(PARAKEET) {
        return true;
    }
    if !name.ends_with(".bin") {
        return false; // Only the pinned Parakeet artifact is supported as GGUF.
    }
    name.starts_with("ggml") || (!huggingface && name.contains("whisper"))
}

fn huggingface_path_allowed(root: &Path, dir: &Path) -> bool {
    let Ok(relative) = dir.strip_prefix(root) else {
        return false;
    };
    let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
    match parts.as_slice() {
        [] => true,
        [repo] => repo.starts_with("models--"),
        [repo, snapshots] => repo.starts_with("models--") && snapshots == "snapshots",
        [repo, snapshots, _] => repo.starts_with("models--") && snapshots == "snapshots",
        _ => false,
    }
}

/// Lists files by name and size only, without opening any of them. Directory links are not
/// followed. Hugging Face snapshots are file links, so the size of a file link is its target's.
pub(super) fn find_candidates(locations: &[Location]) -> Vec<Candidate> {
    find_candidates_with_catalog(locations, CATALOG)
}

pub(super) fn find_candidates_with_catalog(
    locations: &[Location],
    catalog: &'static [CatalogEntry],
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let mut visited = 0usize;
    for location in locations {
        let mut pending = vec![(location.root.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for item in read.flatten() {
                visited += 1;
                if visited > SCAN_MAX_ENTRIES {
                    return candidates;
                }
                let Ok(file_type) = item.file_type() else {
                    continue;
                };
                let path = item.path();
                if file_type.is_dir() && !file_type.is_symlink() {
                    if depth + 1 < location.depth
                        && (!location.huggingface
                            || huggingface_path_allowed(&location.root, &path))
                    {
                        pending.push((path, depth + 1));
                    }
                    continue;
                }
                if location.huggingface && !huggingface_path_allowed(&location.root, &dir) {
                    continue;
                }
                let name = item.file_name();
                if !name_is_candidate(&name.to_string_lossy(), location.huggingface) {
                    continue;
                }
                let Ok(resolved) = resolved_local(&path) else {
                    continue;
                };
                let Ok(size) = std::fs::metadata(&resolved).map(|meta| meta.len()) else {
                    continue;
                };
                if let Some(entry) = catalog.iter().find(|entry| entry.bytes == size) {
                    candidates.push(Candidate {
                        entry,
                        path,
                        app: location.app,
                    });
                }
            }
        }
    }
    candidates
}

/* ---------- Which model is active ---------- */

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Active {
    pub entry: &'static CatalogEntry,
    pub path: PathBuf,
    /// Set when the file belongs to another app and is used in place.
    pub external: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Resolution {
    pub active: Option<Active>,
    pub notice: Option<String>,
}

/// Decides the model a recording uses. A selection whose file is gone falls back to a
/// model Tuck installed, with a message, and returns to it if the file comes back. Without
/// a selection (settings from before the picker) Tiny is used first, so an old install
/// keeps working untouched.
pub(super) fn resolve_active(
    selected: Option<&str>,
    selected_path: Option<&Path>,
    models_dir: &Path,
    path_allowed: impl Fn(&Path) -> bool,
    exists: impl Fn(&Path) -> bool,
) -> Resolution {
    let fallback = || {
        std::iter::once(LEGACY_MODEL)
            .chain(CATALOG.iter().map(|entry| entry.id))
            .filter_map(entry)
            .find(|entry| exists(&models_dir.join(entry.id)))
            .map(|entry| Active {
                entry,
                path: models_dir.join(entry.id),
                external: false,
            })
    };
    let Some(selected) = selected else {
        return Resolution {
            active: fallback(),
            notice: None,
        };
    };
    let Some(chosen) = entry(selected) else {
        let active = fallback();
        let notice = Some(match &active {
            Some(active) => format!(
                "The saved model is not one Tuck knows. Using {} instead.",
                active.entry.label
            ),
            None => "The saved model is not one Tuck knows. Choose a model.".to_owned(),
        });
        return Resolution { active, notice };
    };
    let path = selected_path.map_or_else(|| models_dir.join(chosen.id), Path::to_path_buf);
    if selected_path.is_none_or(path_allowed) && exists(&path) {
        return Resolution {
            active: Some(Active {
                entry: chosen,
                path,
                external: selected_path.is_some(),
            }),
            notice: None,
        };
    }
    let active = fallback();
    let notice = Some(match &active {
        Some(active) => format!(
            "{} is no longer on this computer. Using {} instead.",
            chosen.label, active.entry.label
        ),
        None => format!(
            "{} is no longer on this computer. Choose another model.",
            chosen.label
        ),
    });
    Resolution { active, notice }
}

/* ---------- Picker rows ---------- */

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub speed: String,
    pub accuracy: String,
    pub note: Option<String>,
    /// `available`, `downloading`, `checking`, `installed` or `found`.
    pub state: String,
    pub found_in: Option<String>,
    pub active: bool,
    pub can_remove: bool,
    pub memory_note: String,
    /// Set when this computer has less free memory right now than the model uses.
    pub memory_warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Found {
    pub id: &'static str,
    pub path: PathBuf,
    pub app: &'static str,
}

pub(super) struct RowInputs<'a> {
    pub models_dir: Option<&'a Path>,
    pub active: Option<&'a Active>,
    pub found: &'a [Found],
    pub downloading: Option<&'a str>,
    pub checking: &'a [String],
    pub locations: &'a [Location],
    pub available_memory: Option<u64>,
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIB as f64)
}

fn memory_note(entry: &CatalogEntry) -> String {
    if entry.id == PARAKEET {
        return "Peak memory not measured; 2 GB is a planning estimate.".into();
    }
    if entry.memory_mb < 1000 {
        format!("Uses about {} MB of memory.", entry.memory_mb)
    } else {
        format!(
            "Uses about {} of memory.",
            format_gb(entry.memory_mb * 1024 * 1024)
        )
    }
}

fn memory_warning(entry: &CatalogEntry, available: Option<u64>) -> Option<String> {
    let needed = entry.memory_mb * 1024 * 1024;
    available.filter(|free| *free < needed).map(|free| {
        format!(
            "Only {} of memory is free right now, so this may be very slow.",
            format_gb(free)
        )
    })
}

/// Free physical memory right now, for the warning on a model row.
pub(super) fn available_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut status) }
            .ok()
            .map(|()| status.ullAvailPhys)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub(super) fn build_rows(inputs: &RowInputs<'_>, exists: impl Fn(&Path) -> bool) -> Vec<ModelRow> {
    let mut rows: Vec<(u8, ModelRow)> = Vec::new();
    for entry in CATALOG {
        let active = inputs.active.filter(|active| active.entry.id == entry.id);
        let found = inputs.found.iter().find(|found| found.id == entry.id);
        let external_active = active.filter(|active| active.external);
        let owned = external_active.is_none()
            && inputs
                .models_dir
                .is_some_and(|dir| exists(&dir.join(entry.id)));
        let deletable = owned
            && inputs
                .models_dir
                .is_some_and(|dir| is_owned(&dir.join(entry.id), entry));
        let state = if inputs.downloading == Some(entry.id) {
            "downloading"
        } else if inputs.checking.iter().any(|id| id == entry.id) {
            "checking"
        } else if owned {
            "installed"
        } else if owned || external_active.is_some() || found.is_some() {
            "found"
        } else {
            "available"
        };
        if !entry.offered && state == "available" {
            continue;
        }
        let found_in = if owned {
            None
        } else if let Some(active) = external_active {
            Some(app_for(&active.path, inputs.locations).unwrap_or("another app"))
        } else {
            found.map(|found| found.app)
        };
        // One row per model name. The file most worth showing wins: a compressed Base to
        // download and a full-size Base found in another app are the same choice.
        let rank = if active.is_some() {
            0
        } else {
            match state {
                "downloading" => 1,
                "checking" => 2,
                "installed" => 3,
                "found" => 4,
                _ => 5,
            }
        };
        let row = ModelRow {
            id: entry.id.to_owned(),
            label: entry.label.to_owned(),
            bytes: entry.bytes,
            speed: entry.speed.to_owned(),
            accuracy: entry.accuracy.to_owned(),
            note: entry.note.map(str::to_owned),
            state: state.to_owned(),
            found_in: found_in.map(str::to_owned),
            active: active.is_some(),
            can_remove: deletable,
            memory_note: memory_note(entry),
            memory_warning: memory_warning(entry, inputs.available_memory),
        };
        match rows.iter_mut().find(|(_, kept)| kept.label == entry.label) {
            Some(slot) if rank < slot.0 => *slot = (rank, row),
            Some(_) => {}
            None => rows.push((rank, row)),
        }
    }
    rows.into_iter().map(|(_, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuck-model-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cache() -> Mutex<VerifyCache> {
        Mutex::new(VerifyCache::empty())
    }

    /// A catalog entry shaped like a small test file, so nothing large is written.
    fn small_entry(content: &[u8]) -> CatalogEntry {
        let digest = hex_digest(&Sha256::digest(content));
        CatalogEntry {
            id: "ggml-test.bin",
            label: "Test",
            bytes: content.len() as u64,
            sha256: Box::leak(digest.into_boxed_str()),
            speed: "Fast",
            accuracy: "Good",
            note: None,
            offered: true,
            memory_mb: 100,
        }
    }

    #[test]
    fn acquisition_suppresses_http_for_verified_copy_and_cancel_but_calls_once_when_missing() {
        let dir = temp_dir();
        let entry = small_entry(b"model bytes");
        let path = dir.join(entry.id);
        let cache = cache();
        let cancel = AtomicBool::new(false);
        let calls = std::cell::Cell::new(0);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        std::fs::write(&path, b"model bytes").unwrap();
        let acquire_now = || {
            rt.block_on(acquire(
                verified_existing(&dir, &entry, &[], &cache, &cancel),
                || async {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
            ))
        };
        assert_eq!(acquire_now().unwrap(), Some(path.clone()));
        assert_eq!(calls.get(), 0);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(acquire_now().unwrap(), None);
        assert_eq!(calls.get(), 1);
        cancel.store(true, Ordering::Release);
        assert!(acquire_now().is_err());
        assert_eq!(calls.get(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moved_external_copy_is_rediscovered_without_network_or_removal() {
        let dir = temp_dir();
        let first = dir.join("first.bin");
        let moved = dir.join("moved.bin");
        let entry = small_entry(b"model bytes");
        let cache = cache();
        let cancel = AtomicBool::new(false);
        std::fs::write(&first, b"model bytes").unwrap();
        verify(&first, &entry, &cache, true).unwrap();
        std::fs::rename(&first, &moved).unwrap();
        let existing = verify_candidates(
            [(first, true), (moved.clone(), true)],
            &entry,
            &cache,
            &cancel,
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(
            rt.block_on(acquire(existing, || async {
                panic!("HTTP started for moved copy")
            }))
            .unwrap(),
            Some(moved.clone())
        );
        assert!(moved.exists());
        assert!(!is_owned(&moved, &entry));
        remove_owned(&moved, &entry).unwrap();
        assert_eq!(std::fs::read(moved).unwrap(), b"model bytes");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fresh_preflight_reuses_verified_bytes_and_rejects_invalid_or_cancelled_copy() {
        let dir = temp_dir();
        let entry = small_entry(b"model bytes");
        let path = dir.join(entry.id);
        let cache = cache();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            verified_existing(&dir, &entry, &[], &cache, &cancel).unwrap(),
            None
        );
        std::fs::write(&path, b"model bytes").unwrap();
        assert_eq!(
            verified_existing(&dir, &entry, &[], &cache, &cancel).unwrap(),
            Some(path.clone())
        );
        std::fs::write(&path, b"wrong bytes").unwrap();
        assert_eq!(
            verified_existing(&dir, &entry, &[], &cache, &cancel).unwrap(),
            None
        );
        cancel.store(true, Ordering::Release);
        assert!(verified_existing(&dir, &entry, &[], &cache, &cancel).is_err());
        assert!(path.exists(), "preflight never deletes even invalid files");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "NOT RUN by default locally: requires Windows symlink privilege; mandatory explicit CI step"]
    fn local_huggingface_links_work_and_network_links_are_rejected() {
        use std::os::windows::fs::symlink_file;
        let dir = temp_dir();
        let blob = dir.join("blob");
        std::fs::write(&blob, b"model").unwrap();
        let local = dir.join("ggml-local.bin");
        // Windows Developer Mode or symlink privilege is required for this filesystem test.
        symlink_file(&blob, &local).expect("NOT RUN: Windows symlink privilege is required; this mandatory CI test must exercise its assertions");
        assert!(resolved_local(&local).is_ok());
        assert_eq!(
            hash_file(&local, false).unwrap(),
            hash_file(&blob, false).unwrap()
        );
        let remote = dir.join("ggml-remote.bin");
        symlink_file(r"\\example.invalid\models\model.bin", &remote).unwrap();
        assert!(resolved_local(&remote).unwrap_err().contains("Network"));
        assert!(file_identity(&remote).is_err());
        assert!(hash_file(&remote, false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
        println!("SYMLINK_ASSERTIONS_EXERCISED: local link verified; UNC link rejected before metadata and hash reads");
    }

    #[test]
    fn catalog_entries_are_valid_and_unambiguous() {
        let mut ids = HashSet::new();
        let mut sizes = HashSet::new();
        let mut digests = HashSet::new();
        for entry in CATALOG {
            assert!(
                entry.id == PARAKEET
                    || (entry.id.starts_with("ggml-") && entry.id.ends_with(".bin"))
            );
            assert!(!entry.label.is_empty());
            assert_eq!(entry.sha256.len(), 64, "{}", entry.id);
            assert!(entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
            assert!(entry.bytes > 0);
            assert!(ids.insert(entry.id), "duplicate id {}", entry.id);
            // Detection maps a size to one entry, so sizes must be unique.
            assert!(sizes.insert(entry.bytes), "duplicate size {}", entry.id);
            assert!(digests.insert(entry.sha256), "duplicate hash {}", entry.id);
            let url = download_url(entry);
            assert!(url.starts_with("https://huggingface.co/"));
            assert!(url.contains(if entry.id == PARAKEET {
                PARAKEET_REVISION
            } else {
                SOURCE_REVISION
            }));
        }
    }

    #[test]
    fn picker_offers_the_five_multilingual_tiers_in_order() {
        let offered: Vec<_> = CATALOG
            .iter()
            .filter(|e| e.offered)
            .map(|e| e.label)
            .collect();
        assert_eq!(
            offered,
            [
                "Parakeet Unified English (recommended)",
                "Tiny",
                "Base",
                "Small",
                "Medium",
                "Large v3 Turbo"
            ]
        );
        assert!(CATALOG
            .iter()
            .filter(|e| e.offered)
            .all(|e| !e.id.contains(".en")));
        assert_eq!(CATALOG.iter().filter(|e| e.id.ends_with("gguf")).count(), 1);
    }

    #[test]
    fn legacy_tiny_keeps_its_original_digest_and_size() {
        let tiny = entry(LEGACY_MODEL).unwrap();
        assert_eq!(
            tiny.sha256,
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"
        );
        assert_eq!(tiny.bytes, 77_691_713);
    }

    #[test]
    fn digest_encoding_is_lowercase_hex() {
        assert_eq!(hex_digest(&[0x00, 0xab, 0xff]), "00abff");
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

    #[test]
    fn stall_limit_fails_silence_but_not_slow_progress() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let stalled = runtime.block_on(within_stall_limit(
            std::future::pending::<()>(),
            Duration::from_millis(20),
        ));
        assert!(stalled.unwrap_err().contains("stalled"));
        let arrived = runtime.block_on(within_stall_limit(async { 7 }, Duration::from_millis(20)));
        assert_eq!(arrived, Ok(7));
    }

    #[test]
    fn verify_hashes_once_then_trusts_the_cache_until_the_file_changes() {
        let dir = temp_dir();
        let path = dir.join("ggml-test.bin");
        std::fs::write(&path, b"model bytes").unwrap();
        let entry = small_entry(b"model bytes");
        let cache = Mutex::new(VerifyCache::load(&dir));
        verify(&path, &entry, &cache, false).unwrap();
        assert_eq!(cache.lock().unwrap().entries.len(), 1);

        // The persisted entry answers for a fresh process. Poisoning its digest proves the
        // second check reads the cache instead of hashing the file.
        let reloaded = VerifyCache::load(&dir);
        assert_eq!(reloaded.entries.len(), 1, "the result survives a restart");
        let reloaded = Mutex::new(reloaded);
        verify(&path, &entry, &reloaded, false).unwrap();
        reloaded.lock().unwrap().entries[0].sha256 = "0".repeat(64);
        assert!(verify(&path, &entry, &reloaded, false)
            .unwrap_err()
            .contains("integrity"));

        // Same size, new content and modified time: hashed again.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, b"MODEL BYTES").unwrap();
        assert!(verify(&path, &entry, &reloaded, false)
            .unwrap_err()
            .contains("integrity"));
        std::fs::write(&path, b"model bytes").unwrap();
        verify(&path, &entry, &reloaded, false).unwrap();

        std::fs::write(&path, b"short").unwrap();
        assert!(verify(&path, &entry, &reloaded, false)
            .unwrap_err()
            .contains("wrong size"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A same-size replacement with its modified time set back used to pass unhashed.
    #[cfg(windows)]
    #[test]
    fn a_replacement_with_the_modified_time_set_back_is_hashed_again() {
        let dir = temp_dir();
        let path = dir.join("ggml-test.bin");
        std::fs::write(&path, b"model bytes").unwrap();
        let entry = small_entry(b"model bytes");
        let cache = Mutex::new(VerifyCache::load(&dir));
        verify(&path, &entry, &cache, false).unwrap();
        let original = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, b"EVIL  BYTES").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(original)
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            original
        );
        assert!(verify(&path, &entry, &cache, false)
            .unwrap_err()
            .contains("integrity"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Another app can replace its files at any time, so a saved result does not count
    /// across launches. Tuck's own folder does, or large models would hash on every start.
    #[test]
    fn other_apps_files_are_hashed_once_per_launch() {
        let models = temp_dir();
        let other = temp_dir();
        let path = other.join("ggml-test.bin");
        std::fs::write(&path, b"model bytes").unwrap();
        let entry = small_entry(b"model bytes");
        let cache = Mutex::new(VerifyCache::load(&models));
        verify(&path, &entry, &cache, true).unwrap();
        // Poison the saved digest: the same launch trusts it, a new launch must not.
        cache.lock().unwrap().entries[0].sha256 = "0".repeat(64);
        assert!(verify(&path, &entry, &cache, true).is_err(), "same launch");
        cache.lock().unwrap().persist();
        let next_launch = Mutex::new(VerifyCache::load(&models));
        verify(&path, &entry, &next_launch, true).unwrap();
        std::fs::remove_dir_all(&models).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
    }

    #[test]
    fn an_interrupted_download_is_swept_at_start_and_nothing_else_is() {
        let dir = temp_dir();
        let part = dir.join("ggml-large-v3-turbo-q5_0.bin.part");
        let model = dir.join("ggml-tiny.bin");
        let other = dir.join("notes.part");
        for path in [&part, &model, &other] {
            std::fs::write(path, b"x").unwrap();
        }
        sweep_partial_downloads(&dir);
        assert!(part.exists(), "untracked legacy partials are never deleted");
        assert!(model.exists());
        assert!(other.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_saved_path_must_be_in_a_known_folder() {
        let locations = known_locations(|name| fake_env(name, Path::new("C:/x")));
        let good = Path::new("C:/x/Local/podcli/models/ggml-base.bin");
        assert!(path_is_known(good, &locations));
        assert!(!path_is_known(
            Path::new("C:/Windows/ggml-base.bin"),
            &locations
        ));
        assert!(!path_is_known(
            Path::new("C:/x/Local/podcli/models/../../../secret.bin"),
            &locations
        ));
        assert!(!path_is_known(
            Path::new(r"\\server\share\ggml-base.bin"),
            &locations
        ));
    }

    #[test]
    fn memory_warning_follows_what_is_free_now() {
        let turbo = entry("ggml-large-v3-turbo-q5_0.bin").unwrap();
        assert!(memory_warning(turbo, Some(16 * GIB)).is_none());
        assert!(memory_warning(turbo, Some(1024 * 1024)).is_some());
        assert!(
            memory_warning(turbo, None).is_none(),
            "no guess when unknown"
        );
        assert!(memory_note(turbo).starts_with("Uses about"));
    }

    #[test]
    fn a_mismatch_is_remembered_so_it_is_not_hashed_on_every_scan() {
        let dir = temp_dir();
        let path = dir.join("ggml-test.bin");
        std::fs::write(&path, b"other bytes").unwrap();
        let entry = small_entry(b"model bytes");
        let cache = cache();
        assert!(verify(&path, &entry, &cache, true).is_err());
        let stored = cache.lock().unwrap().entries[0].sha256.clone();
        assert_eq!(stored, hex_digest(&Sha256::digest(b"other bytes")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn damaged_cache_file_is_ignored() {
        let dir = temp_dir();
        std::fs::write(dir.join(VERIFY_CACHE_FILE), b"not json").unwrap();
        assert!(VerifyCache::load(&dir).entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn fake_env(name: &str, root: &Path) -> Option<OsString> {
        match name {
            "APPDATA" => Some(root.join("Roaming").into()),
            "LOCALAPPDATA" => Some(root.join("Local").into()),
            "USERPROFILE" => Some(root.join("Home").into()),
            "ProgramData" => Some(root.join("ProgramData").into()),
            _ => None,
        }
    }

    /// A sparse file of the exact size, so detection is exercised without writing 60 MB.
    fn write_sized(path: &Path, size: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    #[test]
    fn locations_cover_the_known_apps_and_tolerate_missing_variables() {
        let locations = known_locations(|name| fake_env(name, Path::new("C:/x")));
        let apps: HashSet<_> = locations.iter().map(|l| l.app).collect();
        for app in [
            "Handy",
            "Hex",
            "Buzz",
            "OpenWhispr",
            "podcli",
            "Hugging Face cache",
        ] {
            assert!(apps.contains(app), "{app}");
        }
        assert!(known_locations(|_| None).is_empty());
    }

    #[test]
    fn only_the_pinned_parakeet_gguf_is_a_scan_candidate() {
        assert!(name_is_candidate(PARAKEET, false));
        assert!(name_is_candidate(&PARAKEET.to_ascii_lowercase(), true));
        assert!(!name_is_candidate("unrelated-model.gguf", false));
        let locations = known_locations(|name| fake_env(name, Path::new("C:/x")));
        assert!(path_is_known(
            Path::new("C:/x/Roaming/hex/models")
                .join(PARAKEET)
                .as_path(),
            &locations
        ));
    }

    #[test]
    fn detection_matches_by_name_and_exact_size_only() {
        let root = temp_dir();
        let locations = known_locations(|name| fake_env(name, &root));
        let base = entry("ggml-base.bin").unwrap();
        let podcli = root.join("Local/podcli/models");
        write_sized(&podcli.join("ggml-base.bin"), base.bytes);
        write_sized(&podcli.join("ggml-wrong-size.bin"), base.bytes - 1);
        write_sized(&podcli.join("ggml-base.gguf"), base.bytes);
        write_sized(&podcli.join("settings.bin"), base.bytes);
        write_sized(&podcli.join("credentials.json"), base.bytes);
        let handy = root.join("Roaming/com.pais.handy/models");
        write_sized(
            &handy.join("ggml-small-q5_1.bin"),
            entry("ggml-small-q5_1.bin").unwrap().bytes,
        );
        let buzz = root.join("Local/Buzz/Buzz/Cache/models/whisper_cpp");
        write_sized(
            &buzz.join("ggml-tiny.en.bin"),
            entry("ggml-tiny.en.bin").unwrap().bytes,
        );
        let found = find_candidates(&locations);
        let mut got: Vec<_> = found.iter().map(|c| (c.app, c.entry.id)).collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("Buzz", "ggml-tiny.en.bin"),
                ("Handy", "ggml-small-q5_1.bin"),
                ("podcli", "ggml-base.bin"),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn huggingface_cache_only_yields_ggml_files_in_snapshots() {
        let root = temp_dir();
        let locations = known_locations(|name| fake_env(name, &root));
        let bytes = entry("ggml-base.bin").unwrap().bytes;
        let hub = root.join("Home/.cache/huggingface/hub");
        let repo = hub.join("models--ggerganov--whisper.cpp");
        write_sized(&repo.join("snapshots/abc/ggml-base.bin"), bytes);
        write_sized(&repo.join("snapshots/abc/whisper-base.bin"), bytes);
        write_sized(&repo.join("blobs/ggml-base.bin"), bytes);
        write_sized(&hub.join("datasets--x/snapshots/abc/ggml-base.bin"), bytes);
        let found = find_candidates(&locations);
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("snapshots/abc/ggml-base.bin"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_same_size_file_with_the_wrong_hash_is_not_accepted() {
        // Sizes match the candidate filter; only the digest can tell these apart.
        let root = temp_dir();
        let path = root.join("ggml-test.bin");
        std::fs::write(&path, b"12345").unwrap();
        assert!(verify(&path, &small_entry(b"abcde"), &cache(), true).is_err());
        assert!(verify(&path, &small_entry(b"12345"), &cache(), true).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn resolve(selected: Option<&str>, external: Option<&str>, present: &[&str]) -> Resolution {
        let present: Vec<PathBuf> = present.iter().map(PathBuf::from).collect();
        resolve_active(
            selected,
            external.map(Path::new),
            Path::new("models"),
            |p| p.starts_with("elsewhere"),
            |p| present.iter().any(|item| item == p),
        )
    }

    #[test]
    fn no_selection_uses_tiny_if_installed_and_otherwise_nothing() {
        let tiny = resolve(None, None, &["models/ggml-tiny.bin"]);
        assert_eq!(tiny.active.unwrap().entry.id, "ggml-tiny.bin");
        assert_eq!(tiny.notice, None);
        let none = resolve(None, None, &[]);
        assert_eq!(none, Resolution::default());
    }

    #[test]
    fn selection_wins_over_tiny_and_external_files_are_marked() {
        let chosen = resolve(
            Some("ggml-small-q5_1.bin"),
            None,
            &["models/ggml-tiny.bin", "models/ggml-small-q5_1.bin"],
        );
        assert_eq!(chosen.active.unwrap().entry.label, "Small");
        let external = resolve(
            Some("ggml-base.bin"),
            Some("elsewhere/ggml-base.bin"),
            &["elsewhere/ggml-base.bin"],
        )
        .active
        .unwrap();
        assert!(external.external);
        assert_eq!(external.path, Path::new("elsewhere/ggml-base.bin"));
    }

    #[test]
    fn a_vanished_selection_falls_back_with_a_clear_message() {
        let with_tiny = resolve(
            Some("ggml-base.bin"),
            Some("elsewhere/ggml-base.bin"),
            &["models/ggml-tiny.bin"],
        );
        assert_eq!(with_tiny.active.unwrap().entry.id, "ggml-tiny.bin");
        assert!(with_tiny.notice.unwrap().contains("Using Tiny instead"));

        let with_small = resolve(
            Some("ggml-medium-q5_0.bin"),
            None,
            &["models/ggml-small-q5_1.bin"],
        );
        assert_eq!(with_small.active.unwrap().entry.label, "Small");

        let unknown_folder = resolve(
            Some("ggml-base.bin"),
            Some("unknown/ggml-base.bin"),
            &["unknown/ggml-base.bin", "models/ggml-tiny.bin"],
        );
        assert_eq!(unknown_folder.active.unwrap().entry.id, "ggml-tiny.bin");

        let nothing = resolve(Some("ggml-base.bin"), Some("gone.bin"), &[]);
        assert_eq!(nothing.active, None);
        assert!(nothing.notice.unwrap().contains("Choose another model"));
    }

    #[test]
    fn a_catalog_id_from_elsewhere_is_never_loaded() {
        let unknown = resolve(Some("evil.bin"), Some("evil.bin"), &["evil.bin"]);
        assert_eq!(unknown.active, None);
        assert!(unknown.notice.unwrap().contains("not one Tuck knows"));
    }

    fn rows(active: Option<&Active>, found: &[Found], present: &[&str]) -> Vec<ModelRow> {
        let present: Vec<PathBuf> = present.iter().map(PathBuf::from).collect();
        build_rows(
            &RowInputs {
                models_dir: Some(Path::new("models")),
                active,
                found,
                downloading: Some("ggml-medium-q5_0.bin"),
                checking: &["ggml-small-q5_1.bin".to_owned()],
                locations: &known_locations(|name| fake_env(name, Path::new("C:/x"))),
                available_memory: None,
            },
            |p| present.iter().any(|item| item == p),
        )
    }

    #[test]
    fn rows_show_each_state_and_hide_undetected_extras() {
        let tiny = Active {
            entry: entry("ggml-tiny.bin").unwrap(),
            path: PathBuf::from("models/ggml-tiny.bin"),
            external: false,
        };
        let found = [Found {
            id: "ggml-base.bin",
            path: PathBuf::from("C:/x/Local/podcli/models/ggml-base.bin"),
            app: "podcli",
        }];
        let rows = rows(Some(&tiny), &found, &["models/ggml-tiny.bin"]);
        let state = |id: &str| rows.iter().find(|r| r.id == id).map(|r| r.state.as_str());
        assert_eq!(state("ggml-tiny.bin"), Some("installed"));
        assert_eq!(
            state("ggml-base-q5_1.bin"),
            None,
            "one Base row: the found file"
        );
        assert_eq!(state("ggml-small-q5_1.bin"), Some("checking"));
        assert_eq!(state("ggml-medium-q5_0.bin"), Some("downloading"));
        assert_eq!(state("ggml-base.bin"), Some("found"));
        assert_eq!(state("ggml-medium.bin"), None, "not found, so not listed");
        assert_eq!(rows.iter().filter(|r| r.active).count(), 1);
        assert!(
            rows.iter()
                .find(|r| r.id == "ggml-tiny.bin")
                .unwrap()
                .active
        );
        let base = rows.iter().find(|r| r.id == "ggml-base.bin").unwrap();
        assert_eq!(base.found_in.as_deref(), Some("podcli"));
    }

    #[test]
    fn each_model_name_has_one_row_in_order_of_speed() {
        let names: Vec<_> = rows(None, &[], &[]).into_iter().map(|r| r.label).collect();
        assert_eq!(
            names,
            [
                "Parakeet Unified English (recommended)",
                "Tiny",
                "Base",
                "Small",
                "Medium",
                "Large v3 Turbo"
            ]
        );
    }

    #[test]
    fn an_active_external_model_stays_listed_after_a_restart_without_a_scan() {
        let path = PathBuf::from("C:/x/Local/podcli/models/ggml-base.bin");
        let active = Active {
            entry: entry("ggml-base.bin").unwrap(),
            path,
            external: true,
        };
        let rows = rows(Some(&active), &[], &[]);
        let base = rows.iter().find(|r| r.id == "ggml-base.bin").unwrap();
        assert_eq!(base.state, "found");
        assert!(base.active);
        assert_eq!(base.found_in.as_deref(), Some("podcli"));
    }

    /// Reads the real folders on this computer without writing to them.
    #[test]
    #[ignore = "reads this computer's app folders; run with --ignored --nocapture"]
    fn real_detection_on_this_computer() {
        let locations = system_locations();
        let cache = cache();
        for candidate in find_candidates(&locations) {
            let result = verify(&candidate.path, candidate.entry, &cache, true);
            println!(
                "{} {} {:?} -> {:?}",
                candidate.app, candidate.entry.id, candidate.path, result
            );
        }
    }

    /// Downloads one real model into a temp folder. `TUCK_TEST_DOWNLOAD_DIR` keeps it.
    #[test]
    #[ignore = "downloads a real model over the network"]
    fn real_download_is_verified_and_installed() {
        let dir = std::env::var_os("TUCK_TEST_DOWNLOAD_DIR").map_or_else(temp_dir, PathBuf::from);
        let id = std::env::var("TUCK_TEST_MODEL_ID").unwrap_or("ggml-base-q5_1.bin".to_owned());
        let entry = entry(&id).unwrap();
        let _ = std::fs::remove_file(dir.join(entry.id));
        let cache = Mutex::new(VerifyCache::load(&dir));
        let cancel = AtomicBool::new(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let mut last = 0;
        runtime
            .block_on(download(&dir, entry, &cache, &cancel, |received| {
                last = received;
            }))
            .unwrap();
        println!("downloaded {last} bytes in {:?}", started.elapsed());
        assert_eq!(last, entry.bytes);
        assert!(!dir.join(format!("{}.part", entry.id)).exists());
        let hash_started = std::time::Instant::now();
        verify(&dir.join(entry.id), entry, &cache, false).unwrap();
        assert!(hash_started.elapsed() < Duration::from_millis(50), "cached");
    }
}

//! A small local log of what each dictation did, for diagnosing "nothing was inserted".
//! It records events, timings, audio levels and text lengths. It never records words.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 256 * 1024;

static FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();

/// Called once at startup with Tuck's log folder.
pub(crate) fn init(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = FILE.set(Mutex::new(dir.join("dictation.log")));
}

/// Appends one line. Failures are ignored: the log must never affect dictation.
pub(crate) fn log(message: impl AsRef<str>) {
    let Some(file) = FILE.get() else {
        return;
    };
    let Ok(path) = file.lock() else {
        return;
    };
    // Keep one previous file so the log stays small.
    if std::fs::metadata(&*path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = std::fs::rename(&*path, path.with_extension("log.old"));
    }
    if let Ok(mut out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*path)
    {
        let _ = writeln!(
            out,
            "{} {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            message.as_ref()
        );
    }
}

/// Peak and RMS of a clip, in dBFS, for the log.
pub(crate) fn levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (f32::NEG_INFINITY, f32::NEG_INFINITY);
    }
    let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
    let db = |v: f32| 20.0 * v.max(1e-9).log10();
    (db(peak), db(rms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_read_in_dbfs() {
        let (peak, rms) = levels(&[0.5, -0.5, 0.5, -0.5]);
        assert!((peak - -6.02).abs() < 0.05);
        assert!((rms - -6.02).abs() < 0.05);
        let (silent, _) = levels(&[0.0; 16]);
        assert!(silent < -150.0);
    }
}

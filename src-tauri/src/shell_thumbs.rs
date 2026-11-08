//! Explorer's own thumbnails for files Tuck shows: shelf tiles and clipboard file and
//! video cards. The Windows shell reads the file through its thumbnail handlers; Tuck
//! never opens file contents itself. One background thread (single-threaded COM
//! apartment, as shell handlers expect) makes each thumbnail once, the result lives in
//! memory under its item's key, and forgetting the item drops it. The thread sleeps on
//! its queue, so nothing runs until an item is added.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
};

/// Requested edge in pixels: about twice the 96px shelf tile
pub const THUMB_SIDE: u32 = 192;
/// Largest encoded thumbnail; bigger results are scaled down until they fit
pub const MAX_ENCODED: usize = 96 * 1024;
/// Smallest edge worth keeping when scaling down to fit
const MIN_SIDE: u32 = 48;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Thumb {
    /// A PNG data URL from the shell: a page, a picture or the app's icon
    Image { url: String },
    /// The file is gone
    Missing,
    /// The shell had nothing for it
    None,
}

/// Which keys are wanted and what has been made for them. Pure, so the lifecycle is testable.
#[derive(Default)]
struct Cache {
    live: HashSet<String>,
    made: HashMap<String, Thumb>,
}

impl Cache {
    fn register(&mut self, key: &str) {
        self.live.insert(key.to_owned());
    }
    fn forget(&mut self, key: &str) {
        self.live.remove(key);
        self.made.remove(key);
    }
    /// Keeps only the keys under `prefix` whose ids are still present
    fn retain(&mut self, prefix: &str, ids: &HashSet<String>) {
        let keep = |key: &String| key.strip_prefix(prefix).is_none_or(|id| ids.contains(id));
        self.live.retain(keep);
        self.made.retain(|key, _| keep(key));
    }
    fn get(&self, key: &str) -> Option<Thumb> {
        self.made.get(key).cloned()
    }
    fn is_live(&self, key: &str) -> bool {
        self.live.contains(key)
    }
    /// A result for an item removed while it was being made is dropped, not kept
    fn store(&mut self, key: &str, thumb: Thumb) -> bool {
        if !self.live.contains(key) {
            return false;
        }
        self.made.insert(key.to_owned(), thumb);
        true
    }
    #[cfg(test)]
    fn len(&self) -> usize {
        self.made.len()
    }
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

pub fn shelf_key(id: &str) -> String {
    format!("shelf:{id}")
}
pub fn clip_key(id: &str) -> String {
    format!("clip:{id}")
}
pub const CLIP_PREFIX: &str = "clip:";

pub fn register(key: &str) {
    cache().register(key);
}
pub fn forget(key: &str) {
    cache().forget(key);
}
pub fn retain<'a>(prefix: &str, ids: impl IntoIterator<Item = &'a str>) {
    let ids = ids.into_iter().map(str::to_owned).collect();
    cache().retain(prefix, &ids);
}

struct Job {
    key: String,
    path: PathBuf,
    reply: Option<mpsc::Sender<Thumb>>,
}

static WORKER: OnceLock<Mutex<Option<mpsc::Sender<Job>>>> = OnceLock::new();

fn send(job: Job) -> bool {
    let slot = WORKER.get_or_init(|| Mutex::new(None));
    let mut sender = slot.lock().unwrap_or_else(|p| p.into_inner());
    if sender.is_none() {
        let (tx, rx) = mpsc::channel::<Job>();
        let started = std::thread::Builder::new()
            .name("tuck-thumbnails".into())
            .spawn(move || work(rx))
            .is_ok();
        if !started {
            return false;
        }
        *sender = Some(tx);
    }
    sender.as_ref().is_some_and(|tx| tx.send(job).is_ok())
}

fn work(jobs: mpsc::Receiver<Job>) {
    #[cfg(windows)]
    let _apartment = unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
    };
    while let Ok(job) = jobs.recv() {
        let made = cache().get(&job.key);
        let thumb = match made {
            Some(thumb) => thumb,
            None if !cache().is_live(&job.key) => Thumb::None,
            None => {
                let thumb = render(&job.path);
                // Missing is checked again on every request, so it is never kept
                if thumb != Thumb::Missing {
                    cache().store(&job.key, thumb.clone());
                }
                thumb
            }
        };
        if let Some(reply) = job.reply {
            let _ = reply.send(thumb);
        }
    }
}

/// Starts making a thumbnail without waiting for it (an item was just added)
pub fn prefetch(key: &str, path: &Path) {
    register(key);
    send(Job {
        key: key.to_owned(),
        path: path.to_owned(),
        reply: None,
    });
}

/// The thumbnail for a registered key, made once. Blocks: call off the UI thread.
pub fn thumbnail(key: &str, path: &Path) -> Thumb {
    if !path.exists() {
        return Thumb::Missing;
    }
    if let Some(thumb) = cache().get(key) {
        return thumb;
    }
    if !cache().is_live(key) {
        return Thumb::None;
    }
    let (tx, rx) = mpsc::channel();
    if !send(Job {
        key: key.to_owned(),
        path: path.to_owned(),
        reply: Some(tx),
    }) {
        return Thumb::None;
    }
    rx.recv().unwrap_or(Thumb::None)
}

/// Shell parsing names do not take the `\\?\` form that `canonicalize` returns
pub fn shell_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_owned()
    }
}

fn render(path: &Path) -> Thumb {
    if !path.exists() {
        return Thumb::Missing;
    }
    match shell_pixels(&shell_path(path)) {
        Some((w, h, bgra)) => encode(w, h, &straight_rgba(&bgra))
            .map(|url| Thumb::Image { url })
            .unwrap_or(Thumb::None),
        None => Thumb::None,
    }
}

/// The shell's bitmap, top-down BGRA. Thumbnail first; the file type's icon when the
/// shell has no thumbnail handler for it (executables, archives and the like).
#[cfg(windows)]
fn shell_pixels(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::SIZE,
            Graphics::Gdi::{
                CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP,
                BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
            },
            System::Com::IBindCtx,
            UI::Shell::{
                IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
                SIIGBF_THUMBNAILONLY,
            },
        },
    };
    unsafe {
        let name = HSTRING::from(path.as_os_str());
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&name, None::<&IBindCtx>).ok()?;
        let side = THUMB_SIDE as i32;
        let size = SIZE { cx: side, cy: side };
        let bitmap = factory
            .GetImage(size, SIIGBF_THUMBNAILONLY)
            .or_else(|_| factory.GetImage(size, SIIGBF_ICONONLY))
            .ok()?;
        let mut info = BITMAP::default();
        let read = GetObjectW(
            bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut BITMAP as *mut _),
        );
        let (w, h) = (info.bmWidth, info.bmHeight.abs());
        let valid = read > 0 && w > 0 && h > 0 && w <= 1024 && h <= 1024;
        let mut pixels = vec![0u8; if valid { (w * h * 4) as usize } else { 0 }];
        let mut copied = 0;
        if valid {
            let mut header = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            copied = GetDIBits(
                dc,
                bitmap,
                0,
                h as u32,
                Some(pixels.as_mut_ptr() as *mut _),
                &mut header,
                DIB_RGB_COLORS,
            );
            let _ = DeleteDC(dc);
        }
        let _ = DeleteObject(bitmap);
        (copied == h).then_some((w as u32, h as u32, pixels))
    }
}

#[cfg(not(windows))]
fn shell_pixels(_path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    None
}

/// BGRA from the shell to straight RGBA. Shell bitmaps carry premultiplied alpha, or no
/// alpha at all (every byte zero), which means opaque.
fn straight_rgba(bgra: &[u8]) -> Vec<u8> {
    let pixels = bgra.as_chunks::<4>().0;
    let no_alpha = pixels.iter().all(|p| p[3] == 0);
    let premultiplied = !no_alpha && pixels.iter().all(|p| p[0].max(p[1]).max(p[2]) <= p[3]);
    let mut rgba = Vec::with_capacity(bgra.len());
    for &[b, g, r, a] in pixels {
        let a = if no_alpha { 255 } else { a };
        let un = |c: u8| {
            if premultiplied && a > 0 && a < 255 {
                ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
            } else {
                c
            }
        };
        rgba.extend_from_slice(&[un(r), un(g), un(b), a]);
    }
    rgba
}

/// Box-filters straight RGBA down to fit within `side`
fn shrink(w: u32, h: u32, rgba: &[u8], side: u32) -> (u32, u32, Vec<u8>) {
    let scale = (f64::from(side) / f64::from(w.max(h))).min(1.0);
    let tw = ((f64::from(w) * scale).round() as u32).max(1);
    let th = ((f64::from(h) * scale).round() as u32).max(1);
    if (tw, th) == (w, h) {
        return (w, h, rgba.to_vec());
    }
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        let (y0, y1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1));
        for x in 0..tw {
            let (x0, x1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = ((sy * w + sx) * 4) as usize;
                    for (total, value) in sum.iter_mut().zip(&rgba[at..at + 4]) {
                        *total += u32::from(*value);
                    }
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            out.extend(sum.iter().map(|total| (total / count) as u8));
        }
    }
    (tw, th, out)
}

fn png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    }
    Some(output)
}

/// A small PNG data URL: at most `THUMB_SIDE` per edge and `MAX_ENCODED` bytes
fn encode(w: u32, h: u32, rgba: &[u8]) -> Option<String> {
    if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
        return None;
    }
    let mut side = THUMB_SIDE;
    while side >= MIN_SIDE {
        let (tw, th, pixels) = shrink(w, h, rgba, side);
        let bytes = png(tw, th, &pixels)?;
        if bytes.len() <= MAX_ENCODED {
            return Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
        }
        side = side * 3 / 4;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keeps_one_thumbnail_per_live_item_and_drops_it_on_removal() {
        let mut cache = Cache::default();
        let image = Thumb::Image {
            url: "data:image/png;base64,AA==".into(),
        };
        // Nothing is kept for an item that was never added
        assert!(!cache.store("shelf:a", image.clone()));
        cache.register("shelf:a");
        cache.register("clip:b");
        assert!(cache.store("shelf:a", image.clone()));
        assert!(cache.store("clip:b", Thumb::None));
        assert_eq!(cache.get("shelf:a"), Some(image.clone()));
        assert_eq!(cache.len(), 2);
        // Removing the item drops its thumbnail, and a late result is not kept
        cache.forget("shelf:a");
        assert_eq!(cache.get("shelf:a"), None);
        assert!(!cache.store("shelf:a", image.clone()));
        // Clipboard pruning only touches its own keys
        cache.register("shelf:c");
        assert!(cache.store("shelf:c", image.clone()));
        cache.retain(CLIP_PREFIX, &HashSet::new());
        assert_eq!(cache.get("clip:b"), None);
        assert!(!cache.is_live("clip:b"));
        assert_eq!(cache.get("shelf:c"), Some(image));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn missing_files_are_reported_and_never_cached() {
        let key = clip_key(&uuid::Uuid::new_v4().to_string());
        register(&key);
        let path = std::env::temp_dir().join(format!("tuck-missing-{}.pdf", uuid::Uuid::new_v4()));
        assert_eq!(thumbnail(&key, &path), Thumb::Missing);
        assert_eq!(cache().get(&key), None);
        forget(&key);
        assert!(!cache().is_live(&key));
    }

    #[test]
    fn unregistered_keys_get_nothing() {
        let path = std::env::temp_dir().join(format!("tuck-thumb-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"fixture").unwrap();
        assert_eq!(thumbnail("shelf:not-on-the-shelf", &path), Thumb::None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verbatim_paths_become_shell_parsing_names() {
        assert_eq!(
            shell_path(Path::new(r"\\?\C:\Users\me\a.pdf")),
            PathBuf::from(r"C:\Users\me\a.pdf")
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\server\share\a.pdf")),
            PathBuf::from(r"\\server\share\a.pdf")
        );
        assert_eq!(
            shell_path(Path::new(r"D:\b.txt")),
            PathBuf::from(r"D:\b.txt")
        );
    }

    #[test]
    fn shell_pixels_become_straight_rgba() {
        // No alpha at all means opaque
        assert_eq!(straight_rgba(&[10, 20, 30, 0]), vec![30, 20, 10, 255]);
        // Premultiplied half-transparent white
        assert_eq!(
            straight_rgba(&[128, 128, 128, 128, 0, 0, 0, 255]),
            vec![255, 255, 255, 128, 0, 0, 0, 255]
        );
    }

    #[test]
    fn encoded_thumbnails_stay_small() {
        // Noise does not compress, so this forces the size fallback
        let (w, h) = (256u32, 256u32);
        let mut seed = 7u32;
        let noise: Vec<u8> = (0..w * h * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect();
        let url = encode(w, h, &noise).unwrap();
        let bytes = STANDARD
            .decode(url.trim_start_matches("data:image/png;base64,"))
            .unwrap();
        assert!(bytes.len() <= MAX_ENCODED);
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let info = decoder.read_info().unwrap();
        assert!(info.info().width <= THUMB_SIDE && info.info().height <= THUMB_SIDE);
        // A flat picture keeps its full size
        let flat = vec![200u8; (120 * 80 * 4) as usize];
        assert!(encode(120, 80, &flat).is_some());
        assert_eq!(encode(0, 0, &[]), None);
    }
}

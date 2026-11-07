//! Bounded clipboard codecs. No clipboard or filesystem access in the codecs.
use super::ClipboardHistoryItem;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{io::Cursor, sync::Arc};
pub const MAX_DECODED: usize = 32 * 1024 * 1024;
pub const MAX_SIDE: u32 = 8192;
pub const MAX_THUMBNAIL: usize = 96 * 1024;
// Reserve space for the bounded JSON metadata and settings within the 256 MiB store.
pub const MAX_IMAGES: usize = 207 * 1024 * 1024;
pub const MAX_CAPTURE: usize = MAX_DECODED + 1024 * 1024;
#[derive(Debug, PartialEq)]
pub enum Capture {
    Png(Vec<u8>),
    Dib(Vec<u8>),
    Files(Vec<String>),
}
fn u32_at(data: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4)
            .ok_or("Invalid image")?
            .try_into()
            .map_err(|_| "Invalid image")?,
    ))
}
pub fn dimensions(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err("Clipboard image skipped".into());
    }
    let bytes = u64::from(width) * u64::from(height) * 4;
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || bytes > MAX_DECODED as u64
    {
        return Err("Clipboard image skipped".into());
    }
    Ok(bytes as usize)
}
fn png_pixels(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if data.len() > MAX_CAPTURE {
        return Err("Clipboard image skipped".into());
    }
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits { bytes: MAX_DECODED });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "Invalid clipboard image")?;
    let (w, h) = (reader.info().width, reader.info().height);
    let size = dimensions(w, h)?;
    let output = reader
        .output_buffer_size()
        .ok_or("Invalid clipboard image")?;
    if output > MAX_DECODED {
        return Err("Clipboard image skipped".into());
    }
    let mut buffer = vec![0; output];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|_| "Invalid clipboard image")?;
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return Err("Invalid clipboard image".into()),
    };
    let mut rgba = Vec::with_capacity(size);
    for pixel in buffer[..frame.buffer_size()].chunks_exact(channels) {
        match channels {
            4 => rgba.extend_from_slice(pixel),
            3 => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            2 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            _ => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
        }
    }
    if rgba.len() != size {
        return Err("Invalid clipboard image".into());
    }
    Ok((w, h, rgba))
}
fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shifted = mask >> mask.trailing_zeros();
    ((((pixel & mask) >> mask.trailing_zeros()) as u64 * 255) / shifted as u64) as u8
}
fn dib_pixels(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let header = u32_at(data, 0)? as usize;
    if ![40, 52, 56, 108, 124].contains(&header) || data.len() < header {
        return Err("Invalid clipboard image".into());
    }
    let w = u32_at(data, 4)? as i32;
    let signed_h = u32_at(data, 8)? as i32;
    if w <= 0 || signed_h == i32::MIN {
        return Err("Invalid clipboard image".into());
    }
    let (w, h) = (w as u32, signed_h.unsigned_abs());
    let size = dimensions(w, h)?;
    let planes = u16::from_le_bytes(data[12..14].try_into().unwrap());
    let bits = u16::from_le_bytes(data[14..16].try_into().unwrap());
    let compression = u32_at(data, 16)?;
    if planes != 1
        || ![1, 4, 8, 16, 24, 32].contains(&bits)
        || ![0, 3, 6].contains(&compression)
        || (bits == 24 && compression != 0)
        || (bits <= 8 && compression != 0)
    {
        return Err("Unsupported clipboard bitmap".into());
    }
    let masks = if compression != 0 {
        let count = if compression == 6 || header >= 56 {
            4
        } else {
            3
        };
        let r = u32_at(data, 40)?;
        let g = u32_at(data, 44)?;
        let b = u32_at(data, 48)?;
        let a = if count == 4 { u32_at(data, 52)? } else { 0 };
        if r == 0
            || g == 0
            || b == 0
            || r & g != 0
            || r & b != 0
            || g & b != 0
            || a & (r | g | b) != 0
        {
            return Err("Invalid clipboard bitmap".into());
        }
        ([r, g, b, a], if header == 40 { count * 4 } else { 0 })
    } else {
        (
            if bits == 16 {
                [0x7c00, 0x3e0, 0x1f, 0]
            } else {
                [0xff0000, 0xff00, 0xff, 0]
            },
            0,
        )
    };
    let declared_palette = u32_at(data, 32)? as usize;
    let palette = if bits <= 8 && declared_palette == 0 {
        1usize << bits
    } else {
        declared_palette
    };
    if palette > 256 {
        return Err("Invalid clipboard bitmap".into());
    }
    let start = header + masks.1 + palette * 4;
    let stride = (w as usize * bits as usize).div_ceil(32) * 4;
    let end = start
        .checked_add(stride.checked_mul(h as usize).ok_or("Invalid image")?)
        .ok_or("Invalid image")?;
    if end > data.len() {
        return Err("Invalid clipboard bitmap".into());
    }
    let mut rgba = vec![0; size];
    for y in 0..h as usize {
        let sy = if signed_h < 0 { y } else { h as usize - 1 - y };
        for x in 0..w as usize {
            let at = start + sy * stride + x * (bits as usize / 8);
            let dst = (y * w as usize + x) * 4;
            if bits <= 8 {
                let row = start + sy * stride;
                let byte = data[row + x * bits as usize / 8];
                let shift = 8 - bits as usize - (x * bits as usize % 8);
                let index = ((byte >> shift) & ((1u16 << bits) - 1) as u8) as usize;
                if index >= palette {
                    return Err("Invalid clipboard palette".into());
                }
                let colour = header + masks.1 + index * 4;
                rgba[dst..dst + 4].copy_from_slice(&[
                    data[colour + 2],
                    data[colour + 1],
                    data[colour],
                    255,
                ]);
                continue;
            }
            let pixel = if bits == 16 {
                u16::from_le_bytes([data[at], data[at + 1]]) as u32
            } else if bits == 24 {
                u32::from_le_bytes([data[at], data[at + 1], data[at + 2], 0])
            } else {
                u32_at(data, at)?
            };
            rgba[dst..dst + 4].copy_from_slice(&[
                channel(pixel, masks.0[0]),
                channel(pixel, masks.0[1]),
                channel(pixel, masks.0[2]),
                channel(pixel, masks.0[3]),
            ]);
        }
    }
    Ok((w, h, rgba))
}
fn bitmap(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    // Top-down 32-bit CF_DIB, universally readable by bitmap-aware Windows apps.
    let mut dib = vec![0; 40];
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&w.to_le_bytes());
    dib[8..12].copy_from_slice(&(-(h as i32)).to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32u16.to_le_bytes());
    for p in rgba.as_chunks::<4>().0 {
        // CF_DIB BI_RGB has no alpha contract. Composite transparency on white.
        let blend =
            |c: u8| ((u16::from(c) * u16::from(p[3]) + 255 * (255 - u16::from(p[3]))) / 255) as u8;
        dib.extend_from_slice(&[blend(p[2]), blend(p[1]), blend(p[0]), 0]);
    }
    dib
}
fn full_bitmap(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut dib = vec![0; 124];
    dib[..4].copy_from_slice(&124u32.to_le_bytes());
    dib[4..8].copy_from_slice(&w.to_le_bytes());
    dib[8..12].copy_from_slice(&(-(h as i32)).to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32u16.to_le_bytes());
    dib[16..20].copy_from_slice(&3u32.to_le_bytes());
    for (at, mask) in [
        (40, 0xff0000u32),
        (44, 0xff00),
        (48, 0xff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        dib[at..at + 4].copy_from_slice(&mask.to_le_bytes());
    }
    for pixel in rgba.as_chunks::<4>().0 {
        dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    dib
}
#[cfg(windows)]
pub unsafe fn copy_legacy(data: &[u8]) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE},
        System::{
            DataExchange::SetClipboardData,
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };
    let (w, h, rgba) = dib_pixels(data)?;
    let dib = bitmap(w, h, &rgba);
    let memory =
        GlobalAlloc(GMEM_MOVEABLE, dib.len()).map_err(|_| "Clipboard allocation failed")?;
    let ptr = GlobalLock(memory) as *mut u8;
    if ptr.is_null() {
        let _ = GlobalFree(memory);
        return Err("Clipboard allocation failed".into());
    }
    std::ptr::copy_nonoverlapping(dib.as_ptr(), ptr, dib.len());
    let _ = GlobalUnlock(memory);
    if SetClipboardData(8, HANDLE(memory.0)).is_err() {
        let _ = GlobalFree(memory);
        return Err("Could not copy clipboard bitmap".into());
    }
    Ok(())
}
fn thumbnail(w: u32, h: u32, rgba: &[u8]) -> Result<String, String> {
    let scale = 128.0 / f64::from(w.max(h));
    let scale = scale.min(1.0);
    let tw = (f64::from(w) * scale).round().max(1.0) as u32;
    let th = (f64::from(h) * scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            let at = (((y as u64 * h as u64 / th as u64) * w as u64
                + (x as u64 * w as u64 / tw as u64))
                * 4) as usize;
            pixels.extend_from_slice(&rgba[at..at + 4]);
        }
    }
    let mut output = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut output, tw, th);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .map_err(|_| "Thumbnail failed")?
            .write_image_data(&pixels)
            .map_err(|_| "Thumbnail failed")?;
    }
    if output.len() > MAX_THUMBNAIL {
        return Err("Thumbnail too large".into());
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(output)))
}
pub fn file_kind(paths: &[String]) -> &'static str {
    const VIDEO: &[&str] = &[
        "mp4", "mkv", "mov", "avi", "webm", "wmv", "m4v", "mpeg", "mpg", "3gp", "mts", "m2ts",
    ];
    if !paths.is_empty()
        && paths.iter().all(|p| {
            p.rsplit('.')
                .next()
                .is_some_and(|ext| VIDEO.contains(&ext.to_ascii_lowercase().as_str()))
        })
    {
        "video"
    } else {
        "file"
    }
}
pub fn valid_paths(paths: &[String]) -> bool {
    !paths.is_empty()
        && paths.len() <= 256
        && paths
            .iter()
            .all(|p| !p.is_empty() && !p.contains('\0') && p.encode_utf16().count() <= 32767)
        && paths.iter().map(String::len).sum::<usize>() <= 32 * 1024
}
pub fn file_list(paths: &[String]) -> Result<Vec<u8>, String> {
    if !valid_paths(paths) {
        return Err("Invalid clipboard file list".into());
    }
    let mut data = vec![0; 20];
    data[0..4].copy_from_slice(&20u32.to_le_bytes());
    data[16..20].copy_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for unit in p.encode_utf16().chain(std::iter::once(0)) {
            data.extend_from_slice(&unit.to_le_bytes());
        }
    }
    data.extend_from_slice(&[0, 0]);
    Ok(data)
}
#[cfg(test)]
pub fn parse_file_list(data: &[u8]) -> Result<Vec<String>, String> {
    let start = u32_at(data, 0)? as usize;
    if u32_at(data, 16)? != 1
        || start < 20
        || !start.is_multiple_of(2)
        || start >= data.len()
        || data.len() > 128 * 1024
    {
        return Err("Invalid clipboard file list".into());
    }
    let units: Vec<u16> = data[start..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let mut paths = Vec::new();
    let mut at = 0;
    loop {
        let len = units
            .get(at..)
            .ok_or("Invalid clipboard file list")?
            .iter()
            .position(|u| *u == 0)
            .ok_or("Invalid clipboard file list")?;
        if len == 0 {
            break;
        }
        paths.push(
            String::from_utf16(&units[at..at + len]).map_err(|_| "Invalid clipboard file list")?,
        );
        at += len + 1;
    }
    if !valid_paths(&paths) {
        return Err("Invalid clipboard file list".into());
    }
    Ok(paths)
}
pub fn prepare(capture: Capture) -> Result<ClipboardHistoryItem, String> {
    let mut item = ClipboardHistoryItem::new(String::new());
    match capture {
        Capture::Files(paths) => {
            if !valid_paths(&paths) {
                return Err("Invalid clipboard file list".into());
            }
            item.kind = file_kind(&paths).into();
            item.text = paths
                .iter()
                .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p))
                .collect::<Vec<_>>()
                .join(", ");
            item.paths = paths;
        }
        capture => {
            let (w, h, rgba) = match capture {
                Capture::Png(data) => png_pixels(&data)?,
                Capture::Dib(data) => dib_pixels(&data)?,
                _ => unreachable!(),
            };
            item.kind = "image".into();
            item.text = "Image".into();
            item.thumbnail = Some(thumbnail(w, h, &rgba)?);
            item.image = Some(Arc::new(full_bitmap(w, h, &rgba)));
        }
    }
    Ok(item)
}
pub fn bytes(item: &ClipboardHistoryItem) -> usize {
    item.image.as_ref().map_or(0, |image| image.len())
        + item.thumbnail.as_ref().map_or(0, String::len)
}
pub fn missing(item: &ClipboardHistoryItem) -> bool {
    item.paths
        .iter()
        .any(|path| !std::path::Path::new(path).exists())
}
#[cfg(windows)]
pub unsafe fn read_native() -> Result<Option<Capture>, String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HGLOBAL,
            System::{
                DataExchange::{
                    GetClipboardData, IsClipboardFormatAvailable, RegisterClipboardFormatW,
                },
                Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            },
        },
    };
    unsafe fn bytes(format: u32, limit: usize) -> Result<Vec<u8>, String> {
        let memory = HGLOBAL(
            GetClipboardData(format)
                .map_err(|_| "Clipboard data unavailable")?
                .0,
        );
        let length = GlobalSize(memory);
        if length == 0 || length > limit {
            return Err("Clipboard item skipped".into());
        }
        let ptr = GlobalLock(memory) as *const u8;
        if ptr.is_null() {
            return Err("Clipboard data unavailable".into());
        }
        let data = std::slice::from_raw_parts(ptr, length).to_vec();
        let _ = GlobalUnlock(memory);
        Ok(data)
    }
    if IsClipboardFormatAvailable(15).is_ok() {
        // DragQueryFile handles both ANSI and Unicode CF_HDROP without opening files.
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
        let handle = GetClipboardData(15).map_err(|_| "Clipboard files unavailable")?;
        let drop = HDROP(handle.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        if count == 0 || count > 256 {
            return Err("Clipboard file list skipped".into());
        }
        let mut paths = Vec::new();
        for i in 0..count {
            let length = DragQueryFileW(drop, i, None);
            if length == 0 || length > 32767 {
                return Err("Clipboard file list skipped".into());
            }
            let mut name = vec![0u16; length as usize + 1];
            if DragQueryFileW(drop, i, Some(&mut name)) != length {
                return Err("Clipboard files unavailable".into());
            }
            paths.push(
                String::from_utf16(&name[..length as usize])
                    .map_err(|_| "Clipboard files unavailable")?,
            );
            if !valid_paths(&paths) {
                return Err("Clipboard file list skipped".into());
            }
        }
        return Ok(Some(Capture::Files(paths)));
    }
    let name: Vec<u16> = "PNG".encode_utf16().chain(std::iter::once(0)).collect();
    let png = RegisterClipboardFormatW(PCWSTR(name.as_ptr()));
    if png != 0 && IsClipboardFormatAvailable(png).is_ok() {
        return bytes(png, MAX_CAPTURE).map(|data| Some(Capture::Png(data)));
    }
    for format in [17, 8] {
        if IsClipboardFormatAvailable(format).is_ok() {
            return bytes(format, MAX_CAPTURE).map(|data| Some(Capture::Dib(data)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> Vec<u8> {
        bitmap(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255])
    }
    #[test]
    fn decoded_size_and_side_limits_are_checked_before_allocation() {
        assert_eq!(dimensions(8192, 1024).unwrap(), MAX_DECODED);
        for (w, h) in [
            (8193, 1),
            (1, 8193),
            (8192, 1025),
            (0, 1),
            (u32::MAX, u32::MAX),
        ] {
            assert!(dimensions(w, h).is_err());
        }
        let mut dib = fixture();
        dib[4..8].copy_from_slice(&8193i32.to_le_bytes());
        assert!(dib_pixels(&dib).is_err());
    }
    #[test]
    fn dib_and_dibv5_become_usable_bitmaps_and_bounded_thumbnails() {
        let old = fixture();
        let mut v5 = vec![0; 124];
        v5[..40].copy_from_slice(&old[..40]);
        v5[..4].copy_from_slice(&124u32.to_le_bytes());
        v5[16..20].copy_from_slice(&3u32.to_le_bytes());
        for (offset, mask) in [(40, 0xff0000u32), (44, 0xff00), (48, 0xff)] {
            v5[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        v5.extend_from_slice(&old[40..]);
        for input in [old, v5] {
            let item = prepare(Capture::Dib(input)).unwrap();
            assert_eq!(
                dib_pixels(item.image.as_ref().unwrap()).unwrap(),
                (2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255])
            );
            let thumb = item.thumbnail.as_ref().unwrap();
            assert!(thumb.len() < MAX_THUMBNAIL * 4 / 3 + 32);
            let serialized = serde_json::to_value(&item).unwrap();
            assert!(serialized.get("image").is_none());
        }
    }
    #[test]
    fn alpha_and_indexed_bitmaps_round_trip() {
        let pixels = vec![200, 100, 50, 64, 10, 20, 30, 0];
        assert_eq!(
            dib_pixels(&full_bitmap(2, 1, &pixels)).unwrap(),
            (2, 1, pixels)
        );
        let mut dib = vec![0; 52];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1u32.to_le_bytes());
        dib[8..12].copy_from_slice(&1u32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&1u16.to_le_bytes());
        dib[44..48].copy_from_slice(&[0, 0, 255, 0]);
        dib[48] = 128;
        assert_eq!(dib_pixels(&dib).unwrap(), (1, 1, vec![255, 0, 0, 255]));
    }
    #[test]
    fn noisy_png_thumbnail_is_small_and_created_only_once() {
        let (w, h) = (512, 256);
        let mut rgba = vec![0; (w * h * 4) as usize];
        let mut state = 1u32;
        for value in &mut rgba {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *value = (state >> 24) as u8;
        }
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
        }
        let item = prepare(Capture::Png(encoded)).unwrap();
        let encoded = item
            .thumbnail
            .as_ref()
            .unwrap()
            .strip_prefix("data:image/png;base64,")
            .unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert!(bytes.len() <= MAX_THUMBNAIL);
        let (tw, th, _) = png_pixels(&bytes).unwrap();
        assert_eq!((tw, th), (128, 64));
        let clone = item.clone();
        assert!(Arc::ptr_eq(
            item.image.as_ref().unwrap(),
            clone.image.as_ref().unwrap()
        ));
        assert_eq!(item.thumbnail, clone.thumbnail);
    }
    #[test]
    fn file_lists_round_trip_fake_unicode_paths_without_opening_files() {
        let paths = vec![
            r"Z:\tuck-fixture-no-real-files\clip.MP4".into(),
            r"Z:\tuck-fixture-no-real-files\??.webm".into(),
        ];
        assert_eq!(parse_file_list(&file_list(&paths).unwrap()).unwrap(), paths);
        assert_eq!(file_kind(&paths), "video");
        let item = prepare(Capture::Files(paths)).unwrap();
        assert!(missing(&item));
        assert!(item.image.is_none());
        assert!(item.thumbnail.is_none());
        assert_eq!(file_kind(&["movie.mp4".into(), "notes.txt".into()]), "file");
        assert!(file_list(&["bad\0path".into()]).is_err());
    }
    #[test]
    fn malformed_media_is_rejected_without_panics() {
        for length in 0..160 {
            assert!(dib_pixels(&vec![255; length]).is_err());
            assert!(png_pixels(&vec![0; length]).is_err());
        }
        assert!(parse_file_list(&[0; 20]).is_err());
    }
}

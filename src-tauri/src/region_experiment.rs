//! Opt-in isolated contour-region experiment; never enabled for the installed app.
use tauri::{Manager, WebviewWindow};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        CombineRgn, CreateEllipticRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject,
        SetWindowRgn, RGN_DIFF, RGN_OR,
    },
};
pub(crate) fn enabled(window: &WebviewWindow) -> bool {
    window.app_handle().config().identifier == "com.jojo.tuck.drop-probe"
        && std::env::var_os("TUCK_DROP_REGION").is_some()
}
pub(crate) fn apply(window: &WebviewWindow, w: u32, h: u32, top: u32) {
    if !enabled(window) {
        return;
    }
    let Ok(raw) = window.hwnd() else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let px = |n: f64| (n * scale).round() as i32;
    let left = ((size.width as f64 - w as f64 * scale) / 2.0).round() as i32;
    let right = left + px(w as f64);
    let y = px(top as f64);
    let bottom = px(h as f64);
    let radius = px(if h - top >= 200 {
        32.0
    } else if h - top > 40 {
        20.0
    } else {
        16.0
    });
    unsafe {
        let region = CreateRoundRectRgn(left, y, right + 1, bottom + 1, radius * 2, radius * 2);
        if top == 0 {
            let upper = CreateRectRgn(left, y, right + 1, bottom - radius);
            CombineRgn(region, region, upper, RGN_OR);
            let _ = DeleteObject(upper);
            let ear = px(14.0);
            for (l, r, c) in [
                (left - ear, left, left - ear),
                (right, right + ear, right + ear),
            ] {
                let e = CreateRectRgn(l, y, r + 1, y + ear);
                let hole = CreateEllipticRgn(c - ear, y, c + ear, y + 2 * ear);
                CombineRgn(e, e, hole, RGN_DIFF);
                CombineRgn(region, region, e, RGN_OR);
                let _ = DeleteObject(e);
                let _ = DeleteObject(hole);
            }
        }
        if SetWindowRgn(HWND(raw.0), region, true) == 0 {
            let _ = DeleteObject(region);
            eprintln!("region-experiment SetWindowRgn failed");
        }
    }
}

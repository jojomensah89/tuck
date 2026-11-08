//! Visible shell regions: OS hit testing follows the painted shape without cursor polling.
use serde::Deserialize;
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub top_radius: f64,
    pub bottom_radius: f64,
    pub ear: f64,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            x: 154.0,
            y: 0.0,
            width: 212.0,
            height: 32.0,
            top_radius: 0.0,
            bottom_radius: 16.0,
            ear: 14.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Physical {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    top_radius: i32,
    bottom_radius: i32,
    ear: i32,
}
impl Geometry {
    fn physical(self, scale: f64, origin: (i32, i32)) -> Option<Physical> {
        if ![
            self.x,
            self.y,
            self.width,
            self.height,
            self.top_radius,
            self.bottom_radius,
            self.ear,
            scale,
        ]
        .into_iter()
        .all(f64::is_finite)
            || scale <= 0.0
            || self.width < 0.0
            || self.height < 0.0
        {
            return None;
        }
        let width = self.width.min(1040.0);
        let height = self.height.min(576.0);
        let radius =
            |r: f64| (r.max(0.0).min(width / 2.0).min(height / 2.0) * scale).round() as i32;
        Some(Physical {
            left: origin.0 + (self.x * scale).floor() as i32,
            top: origin.1 + (self.y * scale).floor() as i32,
            right: origin.0 + ((self.x + width) * scale).ceil() as i32,
            bottom: origin.1 + ((self.y + height) * scale).ceil() as i32,
            top_radius: radius(self.top_radius),
            bottom_radius: radius(self.bottom_radius),
            ear: (self.ear.clamp(0.0, 32.0) * scale).round() as i32,
        })
    }
}
impl Physical {
    fn contains(self, x: i32, y: i32) -> bool {
        let body = x >= self.left && x < self.right && y >= self.top && y < self.bottom;
        if body {
            for (r, cx, cy, corner) in [
                (
                    self.top_radius,
                    self.left + self.top_radius,
                    self.top + self.top_radius,
                    x < self.left + self.top_radius && y < self.top + self.top_radius,
                ),
                (
                    self.top_radius,
                    self.right - self.top_radius,
                    self.top + self.top_radius,
                    x >= self.right - self.top_radius && y < self.top + self.top_radius,
                ),
                (
                    self.bottom_radius,
                    self.left + self.bottom_radius,
                    self.bottom - self.bottom_radius,
                    x < self.left + self.bottom_radius && y >= self.bottom - self.bottom_radius,
                ),
                (
                    self.bottom_radius,
                    self.right - self.bottom_radius,
                    self.bottom - self.bottom_radius,
                    x >= self.right - self.bottom_radius && y >= self.bottom - self.bottom_radius,
                ),
            ] {
                if corner && r > 0 {
                    return i64::from(x - cx).pow(2) + i64::from(y - cy).pow(2)
                        <= i64::from(r).pow(2);
                }
            }
            return true;
        }
        let e = self.ear;
        if e > 0 && y >= self.top && y < self.top + e {
            for (left, right, cx) in [
                (self.left - e, self.left, self.left - e),
                (self.right, self.right + e, self.right + e),
            ] {
                if x >= left && x < right {
                    return i64::from(x - cx).pow(2) + i64::from(y - self.top - e).pow(2)
                        >= i64::from(e).pow(2);
                }
            }
        }
        false
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        cell::RefCell,
        sync::{
            atomic::{AtomicU64, Ordering},
            OnceLock,
        },
    };
    use tauri::{AppHandle, Emitter, EventTarget, WebviewWindow};
    use windows::Win32::{
        Foundation::{HWND, POINT},
        Graphics::Gdi::{
            CombineRgn, CreateEllipticRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HRGN,
            RGN_DIFF, RGN_OR,
        },
        UI::WindowsAndMessaging::GetCursorPos,
    };
    static APP: OnceLock<AppHandle> = OnceLock::new();
    static DRAG_EPOCH: AtomicU64 = AtomicU64::new(0);
    struct State {
        window: WebviewWindow,
        geometry: Geometry,
        capture: bool,
        revision: u64,
        applied: Option<(Physical, bool, bool, u32, u32)>,
        hover: bool,
    }
    thread_local! {static STATE:RefCell<Option<State>>=const {RefCell::new(None)};}
    // All region handles created here are local until SetWindowRgn succeeds, which transfers ownership to Windows.
    unsafe fn contour(s: Physical) -> HRGN {
        let region = unsafe { CreateRectRgn(s.left, s.top, s.right, s.bottom) };
        for (r, l, t, right, bottom, cx, cy) in [
            (
                s.top_radius,
                s.left,
                s.top,
                s.left + s.top_radius,
                s.top + s.top_radius,
                s.left + s.top_radius,
                s.top + s.top_radius,
            ),
            (
                s.top_radius,
                s.right - s.top_radius,
                s.top,
                s.right,
                s.top + s.top_radius,
                s.right - s.top_radius,
                s.top + s.top_radius,
            ),
            (
                s.bottom_radius,
                s.left,
                s.bottom - s.bottom_radius,
                s.left + s.bottom_radius,
                s.bottom,
                s.left + s.bottom_radius,
                s.bottom - s.bottom_radius,
            ),
            (
                s.bottom_radius,
                s.right - s.bottom_radius,
                s.bottom - s.bottom_radius,
                s.right,
                s.bottom,
                s.right - s.bottom_radius,
                s.bottom - s.bottom_radius,
            ),
        ] {
            if r == 0 {
                continue;
            }
            unsafe {
                let cut = CreateRectRgn(l, t, right, bottom);
                let circle = CreateEllipticRgn(cx - r, cy - r, cx + r, cy + r);
                CombineRgn(cut, cut, circle, RGN_DIFF);
                CombineRgn(region, region, cut, RGN_DIFF);
                let _ = DeleteObject(cut);
                let _ = DeleteObject(circle);
            }
        }
        let e = s.ear;
        if e > 0 {
            for (l, r, cx) in [
                (s.left - e, s.left, s.left - e),
                (s.right, s.right + e, s.right + e),
            ] {
                unsafe {
                    let ear = CreateRectRgn(l, s.top, r, s.top + e);
                    let hole = CreateEllipticRgn(cx - e, s.top, cx + e, s.top + 2 * e);
                    CombineRgn(ear, ear, hole, RGN_DIFF);
                    CombineRgn(region, region, ear, RGN_OR);
                    let _ = DeleteObject(ear);
                    let _ = DeleteObject(hole);
                }
            }
        }
        region
    }
    fn redraw(force_hover: bool) {
        let snapshot = STATE.with(|state| {
            state
                .borrow()
                .as_ref()
                .map(|s| (s.window.clone(), s.geometry, s.capture, s.revision))
        });
        let Some((window, geometry, capture, revision)) = snapshot else {
            return;
        };
        let (Ok(raw), Ok(inner), Ok(outer), Ok(size), Ok(scale)) = (
            window.hwnd(),
            window.inner_position(),
            window.outer_position(),
            window.outer_size(),
            window.scale_factor(),
        ) else {
            return;
        };
        let Some(shape) = geometry.physical(scale, (inner.x - outer.x, inner.y - outer.y)) else {
            return;
        };
        let hidden = crate::system::cursor_paused();
        let key = (shape, capture, hidden, size.width, size.height);
        let mut point = POINT::default();
        let hovered = !hidden
            && unsafe { GetCursorPos(&mut point).is_ok() }
            && shape.contains(point.x - outer.x, point.y - outer.y);
        // Drop the RefCell borrow before Win32/COM calls, which can re-enter the event loop.
        let (apply, emit) = STATE.with(|state| {
            let mut slot = state.borrow_mut();
            let Some(s) = slot.as_mut() else {
                return (false, false);
            };
            if s.revision != revision {
                return (false, false);
            }
            let apply = s.applied != Some(key);
            s.applied = Some(key);
            let emit = force_hover || s.hover != hovered;
            s.hover = hovered;
            (apply, emit)
        });
        if apply {
            unsafe {
                let region = if hidden {
                    CreateRectRgn(0, 0, 0, 0)
                } else if capture {
                    CreateRectRgn(0, 0, size.width as i32, size.height as i32)
                } else {
                    contour(shape)
                };
                if SetWindowRgn(HWND(raw.0), region, true) == 0 {
                    let _ = DeleteObject(region);
                    eprintln!("Island region update failed");
                }
            }
        }
        if emit {
            let _ = window.emit_to(EventTarget::labeled(window.label()), "hover", hovered);
        }
    }
    pub(crate) fn start(window: &WebviewWindow) {
        use tauri::Manager;
        let _ = APP.set(window.app_handle().clone());
        let _ = window.set_ignore_cursor_events(false);
        STATE.with(|state| {
            *state.borrow_mut() = Some(State {
                window: window.clone(),
                geometry: Geometry::default(),
                capture: false,
                revision: 0,
                applied: None,
                hover: false,
            })
        });
        redraw(false);
    }
    pub(crate) fn update(window: &WebviewWindow, geometry: Geometry) {
        if window.label() != "main" {
            return;
        }
        STATE.with(|state| {
            if let Some(s) = state.borrow_mut().as_mut() {
                s.geometry = geometry;
                s.revision += 1;
            }
        });
        redraw(false);
    }
    pub(crate) fn refresh() {
        if let Some(app) = APP.get() {
            let _ = app.run_on_main_thread(|| {
                STATE.with(|state| {
                    if let Some(s) = state.borrow_mut().as_mut() {
                        s.applied = None;
                    }
                });
                redraw(false);
            });
        }
    }
    pub(crate) fn begin_drag() {
        DRAG_EPOCH.fetch_add(1, Ordering::AcqRel);
        STATE.with(|state| {
            if let Some(s) = state.borrow_mut().as_mut() {
                s.capture = true;
                s.revision += 1;
            }
        });
        redraw(true);
    }
    pub(crate) fn end_drag() {
        DRAG_EPOCH.fetch_add(1, Ordering::AcqRel);
        STATE.with(|state| {
            if let Some(s) = state.borrow_mut().as_mut() {
                s.capture = false;
                s.revision += 1;
            }
        });
        redraw(true);
    }
    pub(crate) fn leave_drag() {
        let epoch = DRAG_EPOCH.load(Ordering::Acquire);
        tauri::async_runtime::spawn(async move {
            // Child-to-child OLE transitions emit Leave then Enter; do not shrink between them.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Some(app) = APP.get() {
                let _ = app.run_on_main_thread(move || {
                    if DRAG_EPOCH.load(Ordering::Acquire) == epoch {
                        end_drag();
                    }
                });
            }
        });
    }
    pub(crate) fn stop() {
        STATE.with(|state| {
            state.borrow_mut().take();
        });
    }
}
#[cfg(windows)]
pub(crate) use native::{begin_drag, end_drag, leave_drag, refresh, start, stop, update};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notch_has_concave_ears_and_bottom_corners() {
        let s = Geometry::default().physical(1.0, (0, 0)).unwrap();
        assert!(s.contains(150, 1));
        assert!(!s.contains(141, 13));
        assert!(s.contains(260, 16));
        assert!(!s.contains(154, 31));
        assert!(!s.contains(260, 40));
    }
    #[test]
    fn float_gap_and_rounded_top_are_click_through() {
        let g = Geometry {
            x: 154.0,
            y: 10.0,
            top_radius: 16.0,
            ear: 0.0,
            ..Geometry::default()
        };
        let s = g.physical(1.0, (0, 0)).unwrap();
        assert!(!s.contains(260, 5));
        assert!(!s.contains(154, 10));
        assert!(s.contains(260, 20));
        assert!(!s.contains(150, 20));
    }
    #[test]
    fn physical_regions_scale_at_125_150_and_200_percent() {
        for scale in [1.25, 1.5, 2.0] {
            let s = Geometry::default().physical(scale, (3, 4)).unwrap();
            assert!(s.contains(3 + (260.0 * scale) as i32, 4 + (16.0 * scale) as i32));
            assert!(!s.contains(3 + (100.0 * scale) as i32, 4 + (16.0 * scale) as i32));
            assert_eq!(s.left, 3 + (154.0 * scale).floor() as i32);
        }
    }
    #[test]
    fn invalid_or_empty_geometry_never_captures_the_desktop() {
        assert!(Geometry {
            width: f64::NAN,
            ..Geometry::default()
        }
        .physical(1.0, (0, 0))
        .is_none());
        assert!(Geometry::default().physical(0.0, (0, 0)).is_none());
        let s = Geometry {
            width: 0.0,
            height: 0.0,
            ear: 0.0,
            ..Geometry::default()
        }
        .physical(1.0, (0, 0))
        .unwrap();
        assert!(!s.contains(154, 0));
    }
}

//! Pictures of a display, mouse pointer included.

use super::*;

pub(super) unsafe fn draw_cursor(target: HDC, area: Area) {
    let mut cursor: CURSORINFO = std::mem::zeroed();
    cursor.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
    if GetCursorInfo(&mut cursor) == 0 || cursor.flags & CURSOR_SHOWING == 0 {
        return;
    }
    let POINT { x, y } = cursor.ptScreenPos;
    if x < area.x || y < area.y || x >= area.x + area.width || y >= area.y + area.height {
        return;
    }
    let mut info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(cursor.hCursor, &mut info) == 0 {
        return;
    }
    DrawIconEx(
        target,
        x - area.x - info.xHotspot as i32,
        y - area.y - info.yHotspot as i32,
        cursor.hCursor,
        0,
        0,
        0,
        std::ptr::null_mut(),
        DI_NORMAL,
    );
    DeleteObject(info.hbmMask as HGDIOBJ);
    if !info.hbmColor.is_null() {
        DeleteObject(info.hbmColor as HGDIOBJ);
    }
}

/// The largest place a display fits in the shared picture with its shape
/// unchanged, centred: (left, top, width, height).
pub(super) fn fit(area: Area, width: i32, height: i32) -> (i32, i32, i32, i32) {
    let scale = (width as f32 / area.width as f32).min(height as f32 / area.height as f32);
    let fitted_width = ((area.width as f32 * scale).round() as i32).min(width);
    let fitted_height = ((area.height as f32 * scale).round() as i32).min(height);
    ((width - fitted_width) / 2, (height - fitted_height) / 2, fitted_width, fitted_height)
}

/// The drawing surfaces a capture needs, kept between pictures of the same
/// size: making them anew took a 1280 × 720 bitmap and more for each frame.
pub(super) struct Surfaces {
    size: (i32, i32, i32, i32),
    source: HDC,
    bitmap: HBITMAP,
    old_source: HGDIOBJ,
    target: HDC,
    canvas: HBITMAP,
    old_target: HGDIOBJ,
    bits: *mut std::ffi::c_void,
}

impl Surfaces {
    unsafe fn new(screen: HDC, area: Area, width: i32, height: i32) -> Option<Surfaces> {
        let source = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, area.width, area.height);
        let old_source = SelectObject(source, bitmap as HGDIOBJ);
        let target = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height; // top row first
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = std::ptr::null_mut();
        let canvas = CreateDIBSection(target, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old_target = SelectObject(target, canvas as HGDIOBJ);
        let made = Surfaces { size: (area.width, area.height, width, height), source, bitmap, old_source, target, canvas, old_target, bits };
        (!source.is_null() && !bitmap.is_null() && !target.is_null() && !canvas.is_null() && !bits.is_null()).then_some(made)
    }
}

impl Drop for Surfaces {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.target, self.old_target);
            DeleteObject(self.canvas as HGDIOBJ);
            DeleteDC(self.target);
            SelectObject(self.source, self.old_source);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.source);
        }
    }
}

// The handles are only used by one capture at a time, whichever thread it's on.
unsafe impl Send for Surfaces {}

/// Surfaces not in use, at most one per capture running at once: the pad's
/// and its picture on screen's. Each capture takes one of its size and puts
/// it back, so they move freely between threads.
static SPARE: Mutex<Vec<Surfaces>> = Mutex::new(Vec::new());
const MOST_SPARE: usize = 2;

/// Frees the kept surfaces, once Mini Monitor has stopped.
pub fn forget_surfaces() {
    lock(&SPARE).clear();
}

/// Takes a picture of a display, mouse pointer included, scaled to the shared
/// RGB picture without changing its shape.
pub fn grab(area: Area, width: i32, height: i32) -> Option<Vec<u8>> {
    if area.width <= 0 || area.height <= 0 || width <= 0 || height <= 0 {
        return None;
    }
    let size = (area.width, area.height, width, height);
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let found = {
            let mut spare = lock(&SPARE);
            spare.iter().position(|surfaces| surfaces.size == size).map(|at| spare.swap_remove(at))
        };
        let kept = found.or_else(|| Surfaces::new(screen, area, width, height));
        let result = kept.as_ref().and_then(|surfaces| {
            let copied = BitBlt(surfaces.source, 0, 0, area.width, area.height, screen, area.x, area.y, SRCCOPY);
            draw_cursor(surfaces.source, area);
            SetStretchBltMode(surfaces.target, HALFTONE);
            SetBrushOrgEx(surfaces.target, 0, 0, std::ptr::null_mut());
            let (left, top, fitted_width, fitted_height) = fit(area, width, height);
            // The bars round a display of another shape stay black.
            if left > 0 || top > 0 {
                PatBlt(surfaces.target, 0, 0, width, height, BLACKNESS);
            }
            let scaled = StretchBlt(
                surfaces.target, left, top, fitted_width, fitted_height, surfaces.source, 0, 0, area.width, area.height, SRCCOPY,
            );
            GdiFlush();
            let pixels = (width * height) as usize;
            (copied != 0 && scaled != 0).then(|| {
                let bgra = std::slice::from_raw_parts(surfaces.bits as *const u8, pixels * 4);
                let mut rgb = Vec::with_capacity(pixels * 3);
                for px in bgra.chunks_exact(4) {
                    rgb.extend_from_slice(&[px[2], px[1], px[0]]);
                }
                rgb
            })
        });
        ReleaseDC(std::ptr::null_mut(), screen);
        if let Some(surfaces) = kept {
            let mut spare = lock(&SPARE);
            if spare.len() >= MOST_SPARE {
                spare.remove(0);
            }
            spare.push(surfaces);
        }
        result
    }
}

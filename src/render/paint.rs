//! Drawing basics shared by keys and band tiles: colours, shapes, lines and pictures.

use super::*;
use image::RgbaImage;
use std::sync::Arc;

pub(super) fn blend(canvas: &mut RgbImage, x: i32, y: i32, color: [u8; 3], cover: f32) {
    if x < 0 || y < 0 || x >= canvas.width() as i32 || y >= canvas.height() as i32 {
        return;
    }
    let pixel = canvas.get_pixel_mut(x as u32, y as u32);
    for (old, new) in pixel.0.iter_mut().zip(color) {
        *old = (*old as f32 * (1.0 - cover) + new as f32 * cover).round() as u8;
    }
}

pub(super) fn brightness([r, g, b]: [u8; 3]) -> f32 {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

/// How far apart in brightness a big shape and its background need to be to show.
pub(super) const SHAPE_CONTRAST: f32 = 35.0;

/// The stroke around a title.
pub(super) const TITLE_OUTLINE: f32 = 2.0;

/// A title's stroke is the exact opposite of its colour.
pub(super) fn opposite([r, g, b]: [u8; 3]) -> [u8; 3] {
    [255 - r, 255 - g, 255 - b]
}

/// The wanted colour, unless it would vanish into the background; then black
/// or white, whichever shows up better.
pub(super) fn visible_on(background: [u8; 3], wanted: [u8; 3], contrast: f32) -> [u8; 3] {
    let light = brightness(background);
    if (brightness(wanted) - light).abs() >= contrast {
        wanted
    } else if light > 128.0 {
        [12, 13, 16]
    } else {
        [255, 255, 255]
    }
}

/// Draws a picture file into part of the canvas. `scale` is how much bigger
/// than usual the canvas is: a picture shown at its own size grows with it,
/// so the same part of it shows.
pub(super) fn draw_picture(canvas: &mut RgbImage, path: &str, area: (u32, u32), fit: PictureFit, scale: f32) -> bool {
    let (top, height) = (area.0, area.1.max(1));
    let width = canvas.width();
    let Some(picture) = fitted_file(path, width, height, fit, scale) else { return false };
    let left = (width.saturating_sub(picture.width())) as i32 / 2;
    let down = top as i32 + (height.saturating_sub(picture.height())) as i32 / 2;
    for (x, y, px) in picture.enumerate_pixels() {
        let [r, g, b, a] = px.0;
        blend(canvas, left + x as i32, down + y as i32, [r, g, b], a as f32 / 255.0);
    }
    true
}

/// A picture file fitted to an area, kept for when it's drawn again: titles,
/// states and graphs change far more often than pictures, and decoding a big
/// photo each time cost tens of milliseconds. A changed file is read again.
fn fitted_file(path: &str, width: u32, height: u32, fit: PictureFit, scale: f32) -> Option<Arc<RgbaImage>> {
    type Kept = HashMap<(String, Option<SystemTime>, u32, u32, u8, u32), Option<Arc<RgbaImage>>>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    /// Enough for every picture of a few full pages at each size drawn.
    const MOST: usize = 96;
    let when = super::keys::modified(Some(&path.to_string()));
    let key = (path.to_string(), when, width, height, fit as u8, scale.to_bits());
    if let Some(found) = lock(&KEPT).get_or_insert_with(HashMap::new).get(&key) {
        return found.clone();
    }
    let fitted = image::open(path).ok().map(|picture| {
        let picture = match fit {
            PictureFit::Original if scale != 1.0 => {
                let usual = |pixels: u32| (pixels as f32 / scale).ceil() as u32;
                let middle = fit_picture(&picture, usual(width), usual(height), fit);
                let grown = |pixels: u32| ((pixels as f32 * scale).round() as u32).max(1);
                middle.resize_exact(grown(middle.width()), grown(middle.height()), FilterType::Triangle)
            }
            _ => fit_picture(&picture, width, height, fit),
        };
        Arc::new(picture.to_rgba8())
    });
    let mut kept = lock(&KEPT);
    let kept = kept.get_or_insert_with(HashMap::new);
    if kept.len() >= MOST {
        kept.clear();
    }
    kept.insert(key, fitted.clone());
    fitted
}

/// How far in the shape's edge is at this height, on each side, on a key
/// `size` pixels across. The corner grows with the key, as everything does.
pub(super) fn edge_inset(style: &KeyStyle, y: f32, size: f32) -> f32 {
    match style.shape {
        KeyShape::Square => 0.0,
        KeyShape::Circle => {
            let radius = size / 2.0;
            let from_center = (y - radius).abs();
            if from_center >= radius {
                radius
            } else {
                radius - (radius * radius - from_center * from_center).sqrt()
            }
        }
        KeyShape::Rounded => {
            let radius = (style.corner as f32 * size / KEY_SIZE as f32).min(size / 2.0);
            let into_corner = (radius - y).max(y - (size - radius)).max(0.0);
            if into_corner <= 0.0 {
                0.0
            } else {
                radius - (radius * radius - into_corner * into_corner).max(0.0).sqrt()
            }
        }
    }
}

/// Blacks out everything outside the shape, with soft edges. `scale` is how
/// much bigger than usual the picture is drawn, which the corner follows.
pub(super) fn cut_shape(canvas: &mut RgbImage, style: &KeyStyle, scale: f32) {
    cut_area(canvas, style.shape, style.corner as f32 * scale, [0, 0, 0]);
}

/// Fills everything outside a shape with a colour, with soft edges. The picture
/// can be any size: on a wider one, a circle becomes a bar with round ends.
/// `corner` is in pixels of this picture.
pub fn cut_area(area: &mut RgbImage, shape: KeyShape, corner: f32, outside: [u8; 3]) {
    if shape == KeyShape::Square {
        return;
    }
    let (width, height) = (area.width() as f32, area.height() as f32);
    let radius = match shape {
        KeyShape::Circle => width.min(height) / 2.0,
        _ => corner.min(width.min(height) / 2.0),
    };
    let (inner_x, inner_y) = (width / 2.0 - radius, height / 2.0 - radius);
    for (x, y, pixel) in area.enumerate_pixels_mut() {
        let dx = ((x as f32 + 0.5 - width / 2.0).abs() - inner_x).max(0.0);
        let dy = ((y as f32 + 0.5 - height / 2.0).abs() - inner_y).max(0.0);
        let keep = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        for (value, colour) in pixel.0.iter_mut().zip(outside) {
            *value = (*value as f32 * keep + colour as f32 * (1.0 - keep)) as u8;
        }
    }
}

pub(super) fn rect(canvas: &mut RgbImage, origin: (f32, f32), unit: f32, box_: (f32, f32, f32, f32), color: [u8; 3]) {
    let (x, y, w, h) = box_;
    let left = (origin.0 + x * unit).round() as i32;
    let top = (origin.1 + y * unit).round() as i32;
    let right = (origin.0 + (x + w) * unit).round() as i32;
    let bottom = (origin.1 + (y + h) * unit).round() as i32;
    for py in top..bottom.max(top + 1) {
        for px in left..right.max(left + 1) {
            blend(canvas, px, py, color, 1.0);
        }
    }
}

/// Part of a circle, going clockwise from `start` degrees, with 0 straight up.
/// `paint` gives the colour at each share of the way along.
pub(super) fn arc(canvas: &mut RgbImage, center: (f32, f32), radius: f32, thickness: f32, (start, sweep): (f32, f32), paint: impl Fn(f32) -> [u8; 3]) {
    let reach = (radius + thickness) as i32 + 2;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (x, y) = (center.0 as i32 + dx, center.1 as i32 + dy);
            let (across, down) = (x as f32 + 0.5 - center.0, y as f32 + 0.5 - center.1);
            let edge = (thickness / 2.0 + 0.5 - (across.hypot(down) - radius).abs()).clamp(0.0, 1.0);
            let along = (across.atan2(-down).to_degrees() - start).rem_euclid(360.0) / sweep;
            if edge > 0.0 && along <= 1.0 {
                blend(canvas, x, y, paint(along), edge);
            }
        }
    }
}

/// A straight line with round ends.
pub(super) fn line(canvas: &mut RgbImage, from: (f32, f32), to: (f32, f32), thickness: f32, color: [u8; 3]) {
    let (run, rise) = (to.0 - from.0, to.1 - from.1);
    let length = (run * run + rise * rise).max(0.001);
    let reach = thickness / 2.0 + 2.0;
    let across = (from.0.min(to.0) - reach) as i32..=(from.0.max(to.0) + reach) as i32;
    for y in (from.1.min(to.1) - reach) as i32..=(from.1.max(to.1) + reach) as i32 {
        for x in across.clone() {
            let (px, py) = (x as f32 + 0.5 - from.0, y as f32 + 0.5 - from.1);
            let along = ((px * run + py * rise) / length).clamp(0.0, 1.0);
            let distance = (px - run * along).hypot(py - rise * along);
            let edge = (thickness / 2.0 + 0.5 - distance).clamp(0.0, 1.0);
            if edge > 0.0 {
                blend(canvas, x, y, color, edge);
            }
        }
    }
}

pub(super) fn ring(canvas: &mut RgbImage, center: (f32, f32), radius: f32, thickness: f32, color: [u8; 3]) {
    let reach = radius as i32 + 2;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (x, y) = (center.0 as i32 + dx, center.1 as i32 + dy);
            let distance = (x as f32 + 0.5 - center.0).hypot(y as f32 + 0.5 - center.1);
            let edge = (thickness / 2.0 + 0.5 - (distance - radius).abs()).clamp(0.0, 1.0);
            if edge > 0.0 {
                blend(canvas, x, y, color, edge);
            }
        }
    }
}

//! The side band's tiles and their meters.

use super::*;

/// The colour for a reading: that of the highest level it has reached.
pub(super) fn level_color(levels: &[(u8, [u8; 3])], percent: f32) -> [u8; 3] {
    let reached = levels.iter().rev().find(|(from, _)| *from as f32 <= percent);
    reached.or(levels.first()).map_or(METER_PLAIN, |(_, color)| *color)
}

/// A colour pushed towards the tile's background, for what isn't reached yet.
pub(super) fn faded(color: [u8; 3]) -> [u8; 3] {
    std::array::from_fn(|i| (color[i] as f32 * 0.3 + BAND_BACKGROUND[i] as f32 * 0.7) as u8)
}

/// Small drawings for the sensors, designed to differ at a glance: a chip with
/// pins all round, a card with two fans, and a long memory stick.
pub(super) fn draw_band_icon(canvas: &mut RgbImage, icon: BandIcon, center: (f32, f32), unit: f32, color: [u8; 3]) {
    let origin = (center.0 - 12.0 * unit, center.1 - 12.0 * unit);
    let mut r = |box_: (f32, f32, f32, f32)| rect(canvas, origin, unit, box_, color);
    match icon {
        BandIcon::Cpu => {
            for (x, y, w, h) in [(5.0, 5.0, 14.0, 2.0), (5.0, 17.0, 14.0, 2.0), (5.0, 5.0, 2.0, 14.0), (17.0, 5.0, 2.0, 14.0)] {
                r((x, y, w, h));
            }
            r((9.5, 9.5, 5.0, 5.0));
            for offset in [7.0, 11.0, 15.0] {
                r((offset, 1.5, 2.0, 3.5));
                r((offset, 19.0, 2.0, 3.5));
                r((1.5, offset, 3.5, 2.0));
                r((19.0, offset, 3.5, 2.0));
            }
        }
        BandIcon::Gpu => {
            for (x, y, w, h) in [(1.0, 5.0, 22.0, 2.0), (1.0, 16.0, 22.0, 2.0), (1.0, 5.0, 2.0, 13.0), (21.0, 5.0, 2.0, 13.0)] {
                r((x, y, w, h));
            }
            for x in [4.0, 7.0, 10.0, 13.0] {
                r((x, 19.0, 2.0, 2.5));
            }
            ring(canvas, (origin.0 + 8.0 * unit, origin.1 + 11.5 * unit), 3.2 * unit, 1.5 * unit, color);
            ring(canvas, (origin.0 + 16.0 * unit, origin.1 + 11.5 * unit), 3.2 * unit, 1.5 * unit, color);
        }
        BandIcon::Memory => {
            for (x, y, w, h) in [(0.5, 7.0, 23.0, 2.0), (0.5, 15.0, 23.0, 2.0), (0.5, 7.0, 2.0, 10.0), (21.5, 7.0, 2.0, 10.0)] {
                r((x, y, w, h));
            }
            for x in [4.5, 9.0, 13.5, 17.0] {
                r((x, 10.5, 3.0, 3.0));
            }
            for x in [3.0, 6.0, 9.0, 13.0, 16.0, 19.0] {
                r((x, 17.5, 1.6, 2.5));
            }
        }
    }
}

/// A tile's name or icon, drawn small to leave room for a bigger meter.
pub(super) fn small_heading(canvas: &mut RgbImage, face: &Face, style: &KeyStyle, center: (f32, f32), room: f32, scale: f32) {
    if let Some(icon) = face.icon {
        draw_band_icon(canvas, icon, center, 0.68 * scale, style.color);
    } else if let Some(symbol) = &face.symbol {
        draw_symbol(canvas, symbol, 17.0 * scale, center, style.color);
    } else if !face.label.is_empty() {
        let pen = fitted_pen(style, &face.label, (12.0 * scale, 8.0 * scale), room, scale);
        pen.draw(canvas, &face.label, center.0, center.1 + pen.size * 0.36);
    }
}

/// A place on a usual-size tile, in pixels of this one.
fn at(value: i32, scale: f32) -> i32 {
    (value as f32 * scale).round() as i32
}

/// A half circle marked with every level, and a needle at the reading.
pub(super) fn draw_gauge(canvas: &mut RgbImage, face: &Face, style: &KeyStyle, percent: u8, scale: f32) {
    let hub = (canvas.width() as f32 / 2.0, 51.0 * scale);
    let reached = percent as f32 / 100.0;
    let levels = &face.levels;
    arc(canvas, hub, 30.0 * scale, 6.0 * scale, (-90.0, 180.0), |along| {
        let color = level_color(levels, along * 100.0);
        if along <= reached { color } else { faded(color) }
    });
    let angle = (reached * 180.0 - 180.0).to_radians();
    let tip = (hub.0 + angle.cos() * 22.0 * scale, hub.1 + angle.sin() * 22.0 * scale);
    line(canvas, hub, tip, 2.5 * scale, style.color);
    line(canvas, hub, hub, 7.0 * scale, style.color);
    small_heading(canvas, face, style, (hub.0, 9.0 * scale), 56.0 * scale, scale);
    let mut pen = fitted_pen(style, &face.value, (20.0 * scale, 11.0 * scale), 64.0 * scale, scale);
    pen.color = face.value_color.unwrap_or(pen.color);
    pen.draw(canvas, &face.value, hub.0, 75.0 * scale);
}

/// A circle that fills up in the colour of the level reached.
pub(super) fn draw_ring(canvas: &mut RgbImage, face: &Face, style: &KeyStyle, percent: u8, scale: f32) {
    let center = canvas.width() as f32 / 2.0;
    let color = level_color(&face.levels, percent as f32);
    let reached = percent as f32 / 100.0;
    arc(canvas, (center, center), 33.0 * scale, 7.0 * scale, (0.0, 360.0), |along| if along <= reached { color } else { METER_TRACK });
    small_heading(canvas, face, style, (center, 29.0 * scale), 44.0 * scale, scale);
    let mut pen = fitted_pen(style, &face.value, (20.0 * scale, 10.0 * scale), 50.0 * scale, scale);
    pen.color = face.value_color.unwrap_or(pen.color);
    pen.draw(canvas, &face.value, center, 56.0 * scale);
}

/// The last few minutes, newest on the right, each reading in its own colour.
/// Each reading takes one column of a usual-size tile, and as many as that
/// grows into on a bigger one.
pub(super) fn draw_graph(canvas: &mut RgbImage, face: &Face, style: &KeyStyle, scale: f32) {
    let (left, right, top, bottom) = (at(8, scale), at(72, scale), at(25, scale), at(72, scale));
    let mut value = fitted_pen(style, &face.value, (17.0 * scale, 10.0 * scale), 40.0 * scale, scale);
    value.color = face.value_color.unwrap_or(value.color);
    let width = value.width(&face.value);
    value.draw(canvas, &face.value, right as f32 - width / 2.0, 20.0 * scale);
    let room = (right - left) as f32 - width - 3.0 * scale;
    if face.icon.is_some() || face.symbol.is_some() {
        small_heading(canvas, face, style, (left as f32 + 8.0 * scale, 13.0 * scale), room, scale);
    } else if !face.label.is_empty() {
        let pen = fitted_pen(style, &face.label, (11.0 * scale, 7.0 * scale), room, scale);
        let mut label = face.label.clone();
        while !label.is_empty() && pen.width(&label) > room {
            label.pop();
        }
        pen.draw(canvas, &label, left as f32 + pen.width(&label) / 2.0, 18.0 * scale);
    }
    for y in top..bottom {
        for x in left..right {
            blend(canvas, x, y, [30, 33, 41], 1.0);
        }
    }
    let height = (bottom - top) as f32;
    let (lowest, edge) = (scale.round().max(1.0), at(2, scale));
    let readings = face.history.iter().rev().take(((right - left) as f32 / scale).round() as usize);
    for (back, percent) in readings.enumerate() {
        let columns = right - at(back as i32 + 1, scale)..right - at(back as i32, scale);
        let color = level_color(&face.levels, *percent as f32);
        let peak = bottom - (height * *percent as f32 / 100.0).round().max(lowest) as i32;
        for y in peak..bottom {
            for x in columns.clone() {
                blend(canvas, x, y, if y < peak + edge { color } else { faded(color) }, 1.0);
            }
        }
    }
}

/// A row of ten blocks along the bottom, each in the colour of its own level.
pub(super) fn draw_blocks(canvas: &mut RgbImage, percent: u8, levels: &[(u8, [u8; 3])], scale: f32) {
    for block in 0..10 {
        let left = 6 + block * 7;
        let color = level_color(levels, block as f32 * 10.0 + 5.0);
        let color = if percent as i32 > block * 10 { color } else { faded(color) };
        for y in at(61, scale)..at(69, scale) {
            for x in at(left, scale)..at(left + 5, scale) {
                blend(canvas, x, y, color, 1.0);
            }
        }
    }
}

/// A bar along the bottom in the colour of the level reached.
pub(super) fn draw_bar(canvas: &mut RgbImage, percent: u8, levels: &[(u8, [u8; 3])], scale: f32) {
    let (left, right) = (at(8, scale), canvas.width() as i32 - at(8, scale));
    let filled = (canvas.width() as f32 - 16.0 * scale) * percent as f32 / 100.0;
    let color = level_color(levels, percent as f32);
    for y in at(62, scale)..at(68, scale) {
        for x in left..right {
            blend(canvas, x, y, if (x - left) as f32 <= filled { color } else { METER_TRACK }, 1.0);
        }
    }
}

/// Draws one band tile: a dark background so it reads as information rather
/// than as a button, which it isn't.
pub fn band_picture(face: &Face, style: &KeyStyle) -> RgbImage {
    band_picture_sized(face, style, BAND_SIZE)
}

/// Draws one band tile `px` pixels across, everything on it grown to match.
/// A round tile has no corners to put things in, so what it shows is drawn
/// smaller in its middle; a ring or a gauge is round already and loses less.
pub fn band_picture_sized(face: &Face, style: &KeyStyle, px: u32) -> RgbImage {
    let background = Rgb(face.background.unwrap_or(BAND_BACKGROUND));
    let scale = px as f32 / BAND_SIZE as f32;
    let shape_style = band_shape(style);
    let round = shape_style.shape == KeyShape::Circle && face.picture.is_none();
    let mut canvas = if round {
        let share = match face.percent.map(|_| face.meter) {
            Some(MeterStyle::Ring | MeterStyle::Gauge) => 0.9,
            _ => 0.74,
        };
        let inner = ((px as f32 * share).round() as u32).max(1);
        let mut canvas = RgbImage::from_pixel(px, px, background);
        let offset = ((px - inner) / 2) as i64;
        image::imageops::overlay(&mut canvas, &tile_content(face, style, inner, background), offset, offset);
        canvas
    } else {
        tile_content(face, style, px, background)
    };
    cut_shape(&mut canvas, &shape_style, scale);
    canvas
}

/// What a tile shows, on its background, `px` pixels across and uncut.
fn tile_content(face: &Face, style: &KeyStyle, px: u32, background: Rgb<u8>) -> RgbImage {
    let mut canvas = RgbImage::from_pixel(px, px, background);
    let scale = px as f32 / BAND_SIZE as f32;
    if let Some(path) = &face.picture {
        draw_picture(&mut canvas, path, (0, px), face.picture_fit, scale);
        return canvas;
    }
    match face.percent.map(|percent| (percent, face.meter)) {
        Some((percent, MeterStyle::Gauge)) => draw_gauge(&mut canvas, face, style, percent, scale),
        Some((percent, MeterStyle::Ring)) => draw_ring(&mut canvas, face, style, percent, scale),
        Some((_, MeterStyle::Graph)) => draw_graph(&mut canvas, face, style, scale),
        _ => draw_stacked(&mut canvas, face, style, scale),
    }
    canvas
}

/// The shape band tiles are cut to: the keys', so they line up with them.
pub fn band_shape(style: &KeyStyle) -> KeyStyle {
    style.clone()
}

/// A name or icon on top, the value under it, and any bar or blocks below.
pub(super) fn draw_stacked(canvas: &mut RgbImage, face: &Face, style: &KeyStyle, scale: f32) {
    let size = canvas.width() as f32;
    let center = size / 2.0;
    let fitted = |text: &str, largest: f32, smallest: f32| fitted_pen(style, text, (largest * scale, smallest * scale), size - 8.0 * scale, scale);
    let number = face.percent.filter(|_| face.meter == MeterStyle::Number);
    let has_icon = face.icon.is_some() || face.symbol.is_some();
    let has_top = !face.label.is_empty() || has_icon;
    if let Some(icon) = face.icon {
        draw_band_icon(canvas, icon, (center, 18.0 * scale), 1.15 * scale, style.color);
    } else if let Some(symbol) = &face.symbol {
        draw_symbol(canvas, symbol, 24.0 * scale, (center, 20.0 * scale), style.color);
    } else if has_top {
        fitted(&face.label, 13.0, 9.0).draw(canvas, &face.label, center, 20.0 * scale);
    }
    if !face.value.is_empty() {
        let largest = match (number, has_icon) {
            (Some(_), _) => 30.0,
            (None, true) => 22.0,
            (None, false) => 26.0,
        };
        let mut pen = fitted(&face.value, largest, 11.0);
        let baseline = if has_top { 55.0 * scale } else { center + pen.size * 0.36 };
        let baseline = if !has_icon && has_top { 50.0 * scale } else { baseline };
        let baseline = if number.is_some() && has_top { 62.0 * scale } else { baseline };
        if let Some(percent) = number {
            pen.color = level_color(&face.levels, percent as f32);
        }
        pen.color = face.value_color.unwrap_or(pen.color);
        pen.draw(canvas, &face.value, center, baseline);
    }
    match face.percent.map(|percent| (percent, face.meter)) {
        Some((percent, MeterStyle::Bar)) => draw_bar(canvas, percent, &face.levels, scale),
        Some((percent, MeterStyle::Blocks)) => draw_blocks(canvas, percent, &face.levels, scale),
        _ => {}
    }
}

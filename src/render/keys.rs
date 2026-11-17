//! What a key shows, and its picture for the pads and the window.

use super::*;

/// Everything that decides how a key looks; a change means a new picture.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    /// Line breaks are kept; each line wraps to fit too.
    pub text: String,
    pub glyph: String,
    pub color: [u8; 3],
    pub image: Option<String>,
    pub image_changed: Option<SystemTime>,
    pub fit: PictureFit,
    pub style: KeyStyle,
    /// A switch drawn on the key, on or off.
    pub toggle: Option<bool>,
    /// The person's own colour for that switch's track.
    pub track: Option<[u8; 3]>,
    /// A slice of a Home Assistant camera's picture.
    pub camera: Option<CameraPart>,
    /// A sensor's recent past, drawn behind the text.
    pub graph: Option<KeyGraph>,
    /// Home Assistant can't be reached or is restarting: the key is its last
    /// look, dimmed, with a mark in the corner.
    pub unavailable: bool,
}

/// The icon in the corner of a key whose Home Assistant can't be reached.
pub const UNAVAILABLE_MARK: &str = "mdi:home-off";
/// The icon of a key showing "HA offline".
pub const OFFLINE_ICON: &str = "mdi:home-off-outline";
/// The background of a key showing "HA offline".
pub const OFFLINE_COLOR: [u8; 3] = [52, 56, 66];

/// One key's part of a camera picture. The version changes with each picture.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CameraPart {
    pub entity: String,
    pub version: u64,
    pub at: [u8; 2],
    pub span: [u8; 2],
    pub fit: PictureFit,
    /// The gaps between keys it allows for, or 0.
    pub gaps: [u8; 2],
    /// The key shape, which a stretched camera takes as one piece.
    pub shape: KeyShape,
    pub corner: u8,
    /// The colour outside that shape: black on a pad, the pad's front in the window.
    pub outside: [u8; 3],
}

/// When a picture file last changed, so an edited picture is drawn again.
/// Asked of the disk at most every 2 seconds per file: every key looks this up
/// each frame, and a picture on a slow or missing drive would hold them up.
pub fn modified(path: Option<&String>) -> Option<SystemTime> {
    static SEEN: Mutex<Option<HashMap<String, (std::time::Instant, Option<SystemTime>)>>> = Mutex::new(None);
    const FRESH: std::time::Duration = std::time::Duration::from_secs(2);
    let path = path?;
    if let Some((asked, when)) = lock(&SEEN).get_or_insert_with(HashMap::new).get(path) {
        if asked.elapsed() < FRESH {
            return *when;
        }
    }
    let when = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let mut seen = lock(&SEEN);
    let seen = seen.get_or_insert_with(HashMap::new);
    if seen.len() > 512 {
        seen.clear();
    }
    seen.insert(path.clone(), (std::time::Instant::now(), when));
    when
}

impl Look {
    /// What a key shows with Home Assistant running (the tests' shorthand).
    #[cfg(test)]
    pub fn of(key: &DockKey, style: &KeyStyle, entities: &Entities) -> Option<Look> {
        Look::of_with(key, style, entities, Availability::Ready)
    }

    /// What a key shows, with Home Assistant as `ha` says. A key that reads
    /// or calls Home Assistant keeps its last look, dimmed with a mark, while
    /// it can't be reached; or shows "HA offline" when that's the person's
    /// choice or there's no last look. A Restart Home Assistant key that was
    /// just pressed says "Restarting…".
    pub fn of_with(key: &DockKey, style: &KeyStyle, entities: &Entities, ha: Availability) -> Option<Look> {
        let mut look = Look::now(key, style, entities)?;
        let (Availability::Down { label, restarting }, Some(binding)) = (ha, key.ha_binding()) else {
            return Some(look);
        };
        if binding.entity.trim().is_empty() && binding.service.trim().is_empty() {
            return Some(look);
        }
        if restarting && binding.restarts_home_assistant() {
            look.text = "Restarting…".into();
            if look.image.is_none() {
                look.glyph = "mdi:restart".into();
            }
            return Some(look);
        }
        let known = binding.service_only() || entities.contains_key(binding.entity.trim());
        if label || !known {
            return Some(Look::offline(key, style));
        }
        look.unavailable = true;
        Some(look)
    }

    /// The plain "HA offline" key.
    fn offline(key: &DockKey, style: &KeyStyle) -> Look {
        let mut style = style.clone();
        style.color = [235, 235, 240];
        Look {
            text: "HA offline".into(),
            glyph: OFFLINE_ICON.into(),
            color: OFFLINE_COLOR,
            image: None,
            image_changed: None,
            fit: key.picture_fit,
            style,
            toggle: None,
            track: None,
            camera: None,
            graph: None,
            unavailable: false,
        }
    }

    /// What a key shows from the states known.
    fn now(key: &DockKey, style: &KeyStyle, entities: &Entities) -> Option<Look> {
        if let Some(a) = key.ha_binding() {
            let entity = entities.get(&a.entity);
            let on = entity.and_then(|e| e.is_on());
            let template = match on {
                Some(true) if !a.on_title.is_empty() => &a.on_title,
                Some(false) if !a.off_title.is_empty() => &a.off_title,
                _ => &a.title,
            };
            let image = match on {
                _ if key.no_icon => None,
                Some(true) => a.on_image.clone().or(key.image.clone()),
                Some(false) => a.off_image.clone().or(key.image.clone()),
                None => key.image.clone(),
            };
            let color = match on {
                Some(true) => a.on_color.unwrap_or(key.color),
                Some(false) => a.off_color.unwrap_or(key.color),
                None => key.color,
            };
            let color = entity.and_then(|e| a.color_for(&e.state)).unwrap_or(color);
            let mut style = style.clone();
            style.color = entity.and_then(|e| text_color_of_state(&a.state_colors, &e.state)).unwrap_or(style.color);
            // The person's own choice comes first, then the switch, then the
            // entity's icon, so there is always something unless they ask for nothing.
            let chosen = key.no_icon || image.is_some() || !key.glyph.is_empty();
            let graph = a.graph.and_then(|span| crate::ha::history::points(&a.entity, span)).and_then(|points| KeyGraph::of(&points));
            let toggle = if a.show_switch && !chosen && can_toggle(&a.entity) { on } else { None };
            let track = match toggle {
                Some(true) => a.on_track,
                Some(false) => a.off_track,
                None => None,
            };
            let glyph = if chosen {
                if key.no_icon || image.is_some() { String::new() } else { key.glyph.clone() }
            } else if toggle.is_some() || graph.is_some() {
                // The graph and the value say more than the entity's usual icon.
                String::new()
            } else {
                icons::of_entity(&a.entity, entity)
            };
            let text = if template.is_empty() {
                key.title.clone()
            } else {
                crate::ha::title(template, &a.entity, entity)
            };
            return Some(Look {
                text,
                glyph,
                color,
                image_changed: modified(image.as_ref()),
                image,
                fit: key.picture_fit,
                style,
                toggle,
                track,
                camera: None,
                graph,
                unavailable: false,
            });
        }
        let glyph = microphone_glyph(key).unwrap_or_else(|| key.glyph.clone());
        if key.title.is_empty() && glyph.is_empty() && key.image.is_none() {
            return None;
        }
        Some(Look {
            text: key.title.clone(),
            glyph,
            color: key.color,
            image_changed: modified(key.image.as_ref()),
            image: key.image.clone(),
            fit: key.picture_fit,
            style: style.clone(),
            toggle: None,
            track: None,
            camera: None,
            graph: None,
            unavailable: false,
        })
    }

    /// Whether this key is one slice of a camera covering several keys.
    pub fn is_stretched(&self) -> bool {
        self.camera.as_ref().is_some_and(|camera| camera.span != [1, 1])
    }

    /// What a cell of a page shows, including its slice of a stretched camera.
    #[allow(clippy::too_many_arguments)]
    pub fn of_cell(
        profile: &crate::model::Profile,
        page: usize,
        cell: usize,
        style: &KeyStyle,
        entities: &Entities,
        pad_gaps: [u8; 2],
        outside: [u8; 3],
        ha: Availability,
    ) -> Option<Look> {
        let cover = profile.cover(page, cell);
        let owner = cover.map_or(cell, |cover| cover.owner);
        let key = profile.pages.get(page)?.keys.get(owner)?;
        let mut look = Look::of_with(key, style, entities, ha)?;
        // "HA offline" has no camera picture to show.
        let offline = ha != Availability::Ready && look.glyph == OFFLINE_ICON && look.color == OFFLINE_COLOR;
        if let Some(entity) = key.camera().filter(|_| !offline) {
            let (at, span) = cover.map_or(([0, 0], [1, 1]), |cover| (cover.at, cover.span));
            let version = crate::camera::version(entity);
            let gaps = if key.camera_gaps { pad_gaps } else { [0, 0] };
            look.camera = Some(CameraPart {
                entity: entity.to_string(),
                version,
                at,
                span,
                fit: key.picture_fit,
                gaps,
                shape: style.shape,
                corner: style.corner,
                outside,
            });
            // One title can't sit across several keys.
            if span != [1, 1] {
                look.text.clear();
            }
        }
        Some(look)
    }
}

/// A key that mutes the microphone shows it crossed out while it's muted, as
/// the original app does. The person's own picture or icon wins.
fn microphone_glyph(key: &DockKey) -> Option<String> {
    let mute = Action::Media { key: MediaKey::MicMute };
    let own = key.no_icon || key.image.is_some() || !matches!(key.glyph.as_str(), "" | MIC_ICON | MIC_OFF_ICON);
    if key.actions.first() != Some(&mute) || own {
        return None;
    }
    let muted = crate::media::microphone_muted() == Some(true);
    Some(if muted { MIC_OFF_ICON } else { MIC_ICON }.into())
}

pub(super) fn draw_toggle(canvas: &mut RgbImage, on: bool, center: (f32, f32), width: f32, background: [u8; 3], chosen: Option<[u8; 3]>) {
    let height = width * 0.46;
    let radius = height / 2.0;
    let (left, right) = (center.0 - width / 2.0 + radius, center.0 + width / 2.0 - radius);
    // A colour the person picked is used as it is.
    let track = chosen.unwrap_or_else(|| visible_on(background, if on { SWITCH_ON } else { SWITCH_OFF }, SHAPE_CONTRAST));
    let knob_color = visible_on(track, [24, 26, 32], SHAPE_CONTRAST);
    let knob_x = if on { right } else { left };
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = px - px.clamp(left, right);
            let dy = py - center.1;
            let edge = radius + 0.5 - (dx * dx + dy * dy).sqrt();
            if edge > 0.0 {
                blend(canvas, x as i32, y as i32, track, edge.min(1.0));
            }
            let knob = (px - knob_x).hypot(dy);
            let edge = radius * 0.7 + 0.5 - knob;
            if edge > 0.0 {
                blend(canvas, x as i32, y as i32, knob_color, edge.min(1.0));
            }
        }
    }
}

/// Draws a key the way the pad will show it.
pub fn key_picture(look: &Look) -> RgbImage {
    key_picture_sized(look, KEY_SIZE)
}

/// Draws a key `px` pixels across. Everything on it grows with it, so a
/// bigger picture looks the same as the pad's, only sharper.
pub fn key_picture_sized(look: &Look, px: u32) -> RgbImage {
    let mut canvas = RgbImage::from_pixel(px, px, Rgb(look.color));
    let size = px as f32;
    let scale = size / KEY_SIZE as f32;
    let has_text = !look.text.trim().is_empty();
    let block = has_text.then(|| lay_out_text(look, size));
    // What's left for the picture once the text has its place.
    let (gap, least) = (2.0 * scale, 30.0 * scale);
    let (top, height) = match (&block, look.style.place) {
        (Some(b), TextPlace::Bottom) => (0.0, (b.top - gap).max(least)),
        (Some(b), TextPlace::Top) => (b.bottom + gap, (size - b.bottom - gap).max(least)),
        _ => (0.0, size),
    };
    let circle_inset = if look.style.shape == KeyShape::Circle && top == 0.0 { 7.0 * scale } else { 0.0 };

    // Only the whole picture keeps clear of the text; the others run under it.
    let under_text = look.fit != PictureFit::Fit;
    let picture_area = if under_text { (0, px) } else { (top as u32, height as u32) };
    let live = look.camera.as_ref().and_then(|camera| crate::camera::part(camera, px));
    let has_camera = live.is_some();
    if let Some(live) = live {
        canvas = live;
    }
    let has_picture = has_camera
        || look.image.as_ref().is_some_and(|path| draw_picture(&mut canvas, path, picture_area, look.fit, scale));
    let center = (size / 2.0, top + (height + circle_inset) / 2.0);
    if let (false, Some(graph)) = (has_picture, &look.graph) {
        draw_key_graph(&mut canvas, graph, look.color, scale);
    }
    if let (false, Some(on)) = (has_picture, look.toggle) {
        draw_toggle(&mut canvas, on, center, (size * 0.62).min(height * 1.6), look.color, look.track);
    } else if !has_picture {
        let symbol = if has_text { (height - circle_inset) * 0.6 } else { size * 0.58 };
        draw_symbol(&mut canvas, &look.glyph, symbol, center, visible_on(look.color, [255, 255, 255], SHAPE_CONTRAST));
    }
    // A stretched camera's picture already has its shape, across all its keys.
    if !(has_camera && look.is_stretched()) {
        cut_shape(&mut canvas, &look.style, scale);
    }

    if let Some(mut block) = block {
        block.pen.outline = Some(opposite(block.pen.color));
        let count = block.lines.len();
        let total = count as f32 * block.line_height;
        // Rounded keys and circles keep the text inside their shape instead.
        let (mut baseline, step) = if look.style.shape == KeyShape::Square {
            let (height, ascent) = block.pen.metrics();
            let (first, step) = title_lines(look.style.place, count as i32, height, ascent, px as i32);
            (first as f32, step as f32)
        } else {
            let first = match look.style.place {
                TextPlace::Bottom => block.bottom - total + block.pen.size,
                TextPlace::Top => block.top + block.pen.size,
                TextPlace::Middle => (size - total) / 2.0 + block.pen.size,
            };
            (first, block.line_height)
        };
        for line in &block.lines {
            block.pen.draw(&mut canvas, line, size / 2.0, baseline);
            baseline += step;
        }
    }
    if look.unavailable {
        dim(&mut canvas);
        draw_mark(&mut canvas, size);
    }
    canvas
}

/// How much of its brightness a key whose Home Assistant can't be reached keeps.
const DIMMED: f32 = 0.42;

fn dim(canvas: &mut RgbImage) {
    for pixel in canvas.pixels_mut() {
        for channel in pixel.0.iter_mut() {
            *channel = (*channel as f32 * DIMMED).round() as u8;
        }
    }
}

/// The mark: a dark disc with a home crossed out, in the top right, inside
/// even a round key.
fn draw_mark(canvas: &mut RgbImage, size: f32) {
    let center = (size * 0.77, size * 0.23);
    let radius = size * 0.135;
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let d = (x as f32 + 0.5 - center.0).hypot(y as f32 + 0.5 - center.1);
            let edge = radius + 0.5 - d;
            if edge > 0.0 {
                blend(canvas, x as i32, y as i32, [24, 26, 32], edge.min(1.0));
            }
        }
    }
    draw_symbol(canvas, UNAVAILABLE_MARK, radius * 1.45, center, [255, 176, 32]);
}

/// A plain empty key, for the window.
pub fn empty_picture(style: &KeyStyle) -> RgbImage {
    empty_picture_sized(style, KEY_SIZE)
}

/// A plain empty key, `px` pixels across.
pub fn empty_picture_sized(style: &KeyStyle, px: u32) -> RgbImage {
    let mut canvas = RgbImage::from_pixel(px, px, Rgb([30, 33, 41]));
    cut_shape(&mut canvas, style, px as f32 / KEY_SIZE as f32);
    canvas
}

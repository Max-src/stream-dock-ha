//! Fonts, and laying out and drawing text and symbols.

use super::*;

pub(super) fn built_in_fonts() -> &'static [FontRef<'static>] {
    static FONTS: OnceLock<Vec<FontRef<'static>>> = OnceLock::new();
    FONTS.get_or_init(|| {
        [
            epaint_default_fonts::UBUNTU_LIGHT,
            epaint_default_fonts::NOTO_EMOJI_REGULAR,
            epaint_default_fonts::EMOJI_ICON,
        ]
        .iter()
        .filter_map(|bytes| FontRef::try_from_slice(bytes).ok())
        .collect()
    })
}

/// A font from Windows' font folder, loaded once and kept for good.
pub(super) fn system_font(file: &str) -> Option<&'static FontRef<'static>> {
    static LOADED: Mutex<Option<HashMap<String, Option<&'static FontRef<'static>>>>> = Mutex::new(None);
    if file.is_empty() {
        return None;
    }
    let mut loaded = lock(&LOADED);
    let loaded = loaded.get_or_insert_with(HashMap::new);
    // Asked for at every title, so a font already loaded costs no copy of its name.
    if let Some(found) = loaded.get(file) {
        return *found;
    }
    // Each font is read once and kept for as long as the app runs.
    *loaded
        .entry(file.to_string())
        .or_insert_with(|| {
            let folder = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from);
            let bytes: &'static [u8] = Box::leak(std::fs::read(folder.join("Fonts").join(file)).ok()?.into_boxed_slice());
            Some(&*Box::leak(Box::new(FontRef::try_from_slice(bytes).ok()?)))
        })
}

/// The fonts installed in Windows, as (name, file), sorted by name.
pub fn installed_fonts() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list = Vec::new();
        let path = crate::win::wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts");
        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key) != ERROR_SUCCESS {
                return list;
            }
            let mut index = 0;
            loop {
                let mut name = [0u16; 256];
                let mut file = [0u16; 260];
                let mut name_len = name.len() as u32;
                let mut file_bytes = (file.len() * 2) as u32;
                let status = RegEnumValueW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    file.as_mut_ptr().cast(),
                    &mut file_bytes,
                );
                if status != ERROR_SUCCESS {
                    break;
                }
                index += 1;
                let name = String::from_utf16_lossy(&name[..name_len as usize]);
                let name = name.trim_end_matches(" (TrueType)").trim_end_matches(" (OpenType)").to_string();
                let chars = (file_bytes as usize / 2).min(file.len());
                let file = String::from_utf16_lossy(&file[..chars]).trim_end_matches('\0').to_string();
                let lower = file.to_ascii_lowercase();
                if !file.contains('\\') && (lower.ends_with(".ttf") || lower.ends_with(".otf")) {
                    list.push((name, file));
                }
            }
            RegCloseKey(key);
        }
        list.sort();
        list
    })
}

/// Windows' own symbol fonts, for the few symbols the built-in fonts lack.
pub(super) const WINDOWS_SYMBOLS: [&str; 2] = ["seguisym.ttf", "seguiemj.ttf"];

/// These icons fill less of their square than other symbols do.
pub(super) const ICON_SCALE: f32 = 1.3;

pub(super) fn font_for(c: char, chosen: Option<&'static FontRef<'static>>) -> Option<&'static FontRef<'static>> {
    chosen
        .filter(|f| f.glyph_id(c).0 != 0)
        .or_else(|| built_in_fonts().iter().find(|f| f.glyph_id(c).0 != 0))
        .or_else(|| WINDOWS_SYMBOLS.iter().filter_map(|file| system_font(file)).find(|f| f.glyph_id(c).0 != 0))
}

/// Text settings for one drawing.
#[derive(Clone, Copy)]
pub(super) struct Pen {
    pub(super) font: Option<&'static FontRef<'static>>,
    pub(super) size: f32,
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) color: [u8; 3],
    /// A stroke around the letters, in this colour.
    pub(super) outline: Option<[u8; 3]>,
    /// How much bigger than usual the picture is, which the stroke and the
    /// extra weight of bold letters follow.
    pub(super) scale: f32,
}

impl Pen {
    pub(super) fn of(style: &KeyStyle, size: f32, scale: f32) -> Pen {
        Pen {
            font: system_font(&style.font),
            size,
            bold: style.bold,
            italic: style.italic,
            color: style.color,
            outline: None,
            scale,
        }
    }

    /// How far bold letters are drawn a second time to the right.
    fn bold_shift(&self) -> f32 {
        if self.bold { 0.6 * self.scale } else { 0.0 }
    }

    /// The height of a line and how far the letters rise, in whole pixels.
    pub(super) fn metrics(&self) -> (i32, i32) {
        let Some(font) = self.font.or_else(|| built_in_fonts().first()) else {
            return (self.size as i32, self.size as i32);
        };
        let scaled = font.as_scaled(PxScale::from(self.size));
        let ascent = scaled.ascent().round() as i32;
        (ascent - scaled.descent().round() as i32, ascent)
    }

    pub(super) fn width(&self, text: &str) -> f32 {
        let extra = self.bold_shift();
        text.chars()
            .filter_map(|c| {
                let font = font_for(c, self.font)?;
                Some(font.as_scaled(PxScale::from(self.size)).h_advance(font.glyph_id(c)) + extra)
            })
            .sum()
    }

    pub(super) fn draw(&self, canvas: &mut RgbImage, text: &str, center_x: f32, baseline: f32) {
        if let Some(edge) = self.outline {
            let half = TITLE_OUTLINE * self.scale / 2.0;
            for dy in [-half, 0.0, half] {
                for dx in [-half, 0.0, half] {
                    self.stamp(canvas, text, (center_x + dx, baseline + dy), edge, 1.0);
                }
            }
        }
        // Thin letters wash out on the small screen, so they're drawn stronger.
        let strength = if self.font.is_none() { 1.35 } else { 1.1 };
        self.stamp(canvas, text, (center_x, baseline), self.color, strength);
    }

    pub(super) fn stamp(&self, canvas: &mut RgbImage, text: &str, at: (f32, f32), color: [u8; 3], strength: f32) {
        let (center_x, baseline) = at;
        let scale = PxScale::from(self.size);
        let mut x = center_x - self.width(text) / 2.0;
        for c in text.chars() {
            let Some(font) = font_for(c, self.font) else { continue };
            let id = font.glyph_id(c);
            let passes: &[f32] = if self.bold { &[0.0, self.bold_shift()] } else { &[0.0] };
            for offset in passes {
                let glyph = id.with_scale_and_position(scale, point(x + offset, baseline));
                if let Some(outline) = font.outline_glyph(glyph) {
                    let bounds = outline.px_bounds();
                    outline.draw(|gx, gy, cover| {
                        let y = bounds.min.y as i32 + gy as i32;
                        let lean = if self.italic { ((baseline - y as f32) * 0.2) as i32 } else { 0 };
                        let px = bounds.min.x as i32 + gx as i32 + lean;
                        blend(canvas, px, y, color, (cover * strength).min(1.0));
                    });
                }
            }
            x += font.as_scaled(scale).h_advance(id) + self.bold_shift();
        }
    }
}

/// Draws one symbol centred on a point, going by its real shape so symbols
/// from different fonts all sit in the middle.
pub(super) fn draw_symbol(canvas: &mut RgbImage, glyph: &str, size: f32, center: (f32, f32), color: [u8; 3]) {
    let symbol = if glyph.starts_with("mdi:") {
        icons::character(glyph).and_then(|c| Some((icons::font()?, c, size * ICON_SCALE)))
    } else {
        glyph.chars().next().and_then(|c| Some((font_for(c, None)?, c, size)))
    };
    let Some((font, c, size)) = symbol else { return };
    let scale = PxScale::from(size);
    let outlined = font.outline_glyph(font.glyph_id(c).with_scale_and_position(scale, point(0.0, 0.0)));
    let Some(outlined) = outlined else { return };
    let bounds = outlined.px_bounds();
    let left = center.0 - bounds.width() / 2.0;
    let top = center.1 - bounds.height() / 2.0;
    outlined.draw(|gx, gy, cover| {
        blend(canvas, left as i32 + gx as i32, top as i32 + gy as i32, color, cover);
    });
}

/// Where a square key's title goes: the first line's baseline and the step to
/// the next, in whole pixels, on a key `key` pixels across.
pub(super) fn title_lines(place: TextPlace, lines: i32, height: i32, ascent: i32, key: i32) -> (i32, i32) {
    let first = match place {
        TextPlace::Top if height / 7 > 1 => height / 7 + height / 2,
        TextPlace::Top => height / 2 + (2.0 * key as f32 / KEY_SIZE as f32).round() as i32,
        TextPlace::Bottom => height / 6 - height / 4 + key - height * (lines - 1),
        TextPlace::Middle => (key - height * lines) / 2 + ascent,
    };
    (first, height)
}

pub(super) fn wrap(text: &str, pen: &Pen, width: f32) -> (Vec<String>, bool) {
    let mut lines: Vec<String> = Vec::new();
    let mut fits = true;
    for paragraph in text.lines() {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let longer = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
            if pen.width(&longer) <= width {
                current = longer;
            } else {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                fits &= pen.width(word) <= width;
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    (lines, fits)
}

/// The text of a key, placed: the largest size up to the chosen one where it
/// all fits inside the shape.
pub(super) struct TextBlock {
    pub(super) lines: Vec<String>,
    pub(super) pen: Pen,
    pub(super) line_height: f32,
    pub(super) top: f32,
    pub(super) bottom: f32,
}

/// `size` is the key's width in pixels. The sizes tried are those of a
/// usual-size key, grown with it, so a bigger key wraps its title the same way.
pub(super) fn lay_out_text(look: &Look, size: f32) -> TextBlock {
    let style = &look.style;
    let scale = size / KEY_SIZE as f32;
    let most_lines = 3;
    let start = (style.size as u32).max(SMALLEST_TEXT);
    let mut last = None;
    for usual in (SMALLEST_TEXT..=start).rev() {
        let px = usual as f32 * scale;
        let pen = Pen::of(style, px, scale);
        let line_height = px * 1.14;
        // The shape decides the room, and the room depends on how many lines
        // there are, so the count is settled in two passes.
        let mut count = look.text.lines().filter(|l| !l.trim().is_empty()).count().clamp(1, most_lines);
        let mut result = None;
        for _ in 0..2 {
            let height = count as f32 * line_height;
            let (top, bottom) = match style.place {
                TextPlace::Bottom => {
                    let lift = lift_of(style) * scale;
                    (size - lift - height, size - lift)
                }
                TextPlace::Top => {
                    let lift = lift_of(style) * scale;
                    (lift, lift + height)
                }
                TextPlace::Middle => ((size - height) / 2.0, (size + height) / 2.0),
            };
            let inset = edge_inset(style, top, size).max(edge_inset(style, bottom, size));
            let width = size - 2.0 * inset - 6.0 * scale;
            let (lines, fits) = wrap(&look.text, &pen, width);
            count = lines.len().max(1);
            result = Some((lines, fits, top, bottom));
        }
        let Some((lines, fits, top, bottom)) = result else { continue };
        let good = fits && lines.len() <= most_lines && count * ((usual as f32 * 1.14) as usize + 1) < KEY_SIZE as usize;
        let block = TextBlock { lines, pen, line_height, top, bottom };
        if good {
            return block;
        }
        last = Some(block);
    }
    let mut block = last.expect("sizes were tried");
    block.lines.truncate(most_lines);
    let inset = edge_inset(style, block.top, size).max(edge_inset(style, block.bottom, size));
    let width = size - 2.0 * inset - 6.0 * scale;
    for line in &mut block.lines {
        while block.pen.width(line) > width && line.chars().count() > 1 {
            line.pop();
        }
    }
    block
}

/// How far a title at the top or bottom stays from the edge of a usual-size
/// key, so it clears the shape's corners.
fn lift_of(style: &KeyStyle) -> f32 {
    match style.shape {
        KeyShape::Square => 4.0,
        KeyShape::Rounded => 4.0 + style.corner as f32 * 0.16,
        KeyShape::Circle => 11.0,
    }
}

/// The largest size up to `largest` at which text fits across `room` pixels.
/// The sizes are already scaled; `scale` is for the pen's own details.
pub(super) fn fitted_pen(style: &KeyStyle, text: &str, (largest, smallest): (f32, f32), room: f32, scale: f32) -> Pen {
    let mut px = largest;
    while px > smallest && Pen::of(style, px, scale).width(text) > room {
        px -= scale;
    }
    Pen::of(style, px, scale)
}

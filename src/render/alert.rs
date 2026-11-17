//! The picture of an alert from Home Assistant: one picture laid across all
//! of a pad's screen keys, as a stretched camera is. Only the keys show it:
//! the gaps between them are hidden. So the icon and every word are placed
//! inside one key, and a line of text runs on from one key to the next.

use super::*;
use image::DynamicImage;

/// What an alert shows.
pub struct AlertContent<'a> {
    pub title: &'a str,
    pub message: &'a str,
    /// An icon written `mdi:name`, or nothing.
    pub icon: &'a str,
    pub color: [u8; 3],
    /// A camera's picture to fill it with instead.
    pub camera: Option<&'a RgbImage>,
}

/// The sizes a title is tried at, largest first, as parts of a key.
const TITLE_SIZES: [f32; 6] = [0.26, 0.23, 0.2, 0.17, 0.15, 0.13];
/// The same for the message.
const MESSAGE_SIZES: [f32; 6] = [0.21, 0.19, 0.17, 0.15, 0.13, 0.12];
/// The same for the title strip over a camera.
const STRIP_SIZES: [f32; 5] = [0.22, 0.19, 0.17, 0.15, 0.13];
/// How much of the top row of keys the title strip over a camera covers.
const STRIP: f32 = 0.42;
/// The room kept inside each key's edges, across and down, as parts of a key.
const MARGIN: [f32; 2] = [0.08, 0.07];

/// How big an alert's picture is for `span` keys across and down, each
/// `unit` pixels across, with `gaps` (a percentage of a key) between them.
/// It's the size a stretched camera is laid out at.
pub fn alert_size(span: [u8; 2], gaps: [u8; 2], unit: u32) -> (u32, u32) {
    let step = steps(gaps, unit);
    let keys = span.map(|keys| keys.max(1) as u32 - 1);
    (keys[0] * step[0] + unit, keys[1] * step[1] + unit)
}

/// From one key to the next in the picture, across then down, in pixels.
fn steps(gaps: [u8; 2], unit: u32) -> [u32; 2] {
    gaps.map(|gap| unit + unit * gap as u32 / 100)
}

/// Draws an alert over `span` keys across and down, each `unit` pixels
/// across, with `gaps` between them: its colour, its icon in one key, the
/// title along the top row and the message along the bottom. With a camera,
/// the camera's picture fills it, and the title stays as a strip along the
/// top.
pub fn alert_picture(alert: &AlertContent, span: [u8; 2], gaps: [u8; 2], unit: u32) -> RgbImage {
    let (width, height) = alert_size(span, gaps, unit);
    let mut canvas = RgbImage::from_pixel(width, height, Rgb(alert.color));
    // Words across a whole pad need more contrast than a symbol on a key.
    let ink = visible_on(alert.color, [255, 255, 255], 100.0);
    let heading = KeyStyle { bold: true, color: ink, ..KeyStyle::default() };
    let plain = KeyStyle { color: ink, ..KeyStyle::default() };
    let grid = Grid { unit: unit as f32, step: steps(gaps, unit).map(|step| step as f32) };
    let cols = span[0].max(1) as usize;
    if let Some(picture) = alert.camera {
        let shown = fit_picture(&DynamicImage::ImageRgb8(picture.clone()), width, height, PictureFit::Fill).to_rgb8();
        let (left, down) = ((width as i64 - shown.width() as i64) / 2, (height as i64 - shown.height() as i64) / 2);
        image::imageops::overlay(&mut canvas, &shown, left, down);
        if !alert.title.trim().is_empty() {
            let strip = (STRIP * grid.unit).min(height as f32) as u32;
            for y in 0..strip {
                for x in 0..width {
                    canvas.put_pixel(x, y, Rgb(alert.color));
                }
            }
            let runs = [Run { row: 0, from: 0, to: cols, band: (0.0, STRIP) }];
            place(alert.title, &heading, &STRIP_SIZES, &runs, 1, &grid).draw(&mut canvas);
        }
        return canvas;
    }
    let has = |text: &str| !text.trim().is_empty();
    let plan = plan(span, has(alert.title), has(alert.message), !alert.icon.is_empty());
    place(alert.title, &heading, &TITLE_SIZES, &plan.title, usize::MAX, &grid).draw(&mut canvas);
    place(alert.message, &plain, &MESSAGE_SIZES, &plan.message, usize::MAX, &grid).draw(&mut canvas);
    if let Some([col, row]) = plan.icon {
        let middle = (col as f32 * grid.step[0] + grid.unit / 2.0, row as f32 * grid.step[1] + grid.unit / 2.0);
        draw_symbol(&mut canvas, alert.icon, grid.unit * 0.6, middle, ink);
    }
    canvas
}

/// Where the keys are in the picture.
struct Grid {
    unit: f32,
    step: [f32; 2],
}

/// Keys `from..to` across one row that a text runs through, and the part of
/// their height it may use, from the top, as parts of a key.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    row: usize,
    from: usize,
    to: usize,
    band: (f32, f32),
}

/// Which keys hold an alert's icon, title and message.
#[derive(Debug, PartialEq)]
struct Plan {
    /// The key holding the icon, across then down.
    icon: Option<[usize; 2]>,
    title: Vec<Run>,
    message: Vec<Run>,
}

/// Lays an alert out on `span` keys across and down. With three rows or
/// more the title takes the top row, the icon the middle key under it and
/// the message the rows below. With two, the title and message have a row
/// each and the icon starts the title's row when that leaves it two keys.
/// With one, the title sits over the message in each key.
fn plan(span: [u8; 2], title: bool, message: bool, icon: bool) -> Plan {
    let (cols, rows) = (span[0].max(1) as usize, span[1].max(1) as usize);
    let full = (0.0, 1.0);
    let run = |row, from, band| Run { row, from, to: cols, band };
    let middle = (cols - 1) / 2;
    if !title && !message {
        let icon = icon.then_some([middle, (rows - 1) / 2]);
        return Plan { icon, title: Vec::new(), message: Vec::new() };
    }
    if rows >= 3 {
        let message = (2..rows).map(|row| run(row, 0, full)).collect();
        return Plan { icon: icon.then_some([middle, 1]), title: vec![run(0, 0, full)], message };
    }
    if rows == 2 && (!title || !message) {
        // The row the words leave empty has the icon in its middle.
        let icon = icon.then_some([middle, if title { 1 } else { 0 }]);
        return Plan { icon, title: vec![run(0, 0, full)], message: vec![run(1, 0, full)] };
    }
    let icon = (icon && cols >= 3).then_some([0, 0]);
    let from = usize::from(icon.is_some());
    if rows == 2 {
        return Plan { icon, title: vec![run(0, from, full)], message: vec![run(1, 0, full)] };
    }
    let (over, under) = if title && message { ((0.0, 0.45), (0.45, 1.0)) } else { (full, full) };
    Plan { icon, title: vec![run(0, from, over)], message: vec![run(0, from, under)] }
}

/// Runs a text's words through `keys` keys of `lines` lines each, `room`
/// wide: filling one key before the next, so each key reads as a short
/// phrase and the pad reads key by key. A line break in the text starts a
/// new key, and a word too wide for a key is split. Gives each key's lines,
/// and whether everything fitted without splitting a word.
fn flow(text: &str, width: &dyn Fn(&str) -> f32, room: f32, keys: usize, lines: usize) -> (Vec<Vec<String>>, bool) {
    let mut placed = vec![vec![String::new(); lines]; keys];
    let (mut key, mut line) = (0, 0);
    let mut whole = true;
    for paragraph in text.lines().filter(|paragraph| !paragraph.trim().is_empty()) {
        if placed.get(key).is_some_and(|lines| lines.iter().any(|words| !words.is_empty())) {
            (key, line) = (key + 1, 0);
        }
        for word in paragraph.split_whitespace() {
            let pieces = if width(word) <= room {
                vec![word.to_string()]
            } else {
                whole = false;
                split_word(word, width, room)
            };
            for piece in pieces {
                loop {
                    let Some(words) = placed.get_mut(key).and_then(|lines| lines.get_mut(line)) else {
                        return (placed, false);
                    };
                    let longer = if words.is_empty() { piece.clone() } else { format!("{words} {piece}") };
                    if words.is_empty() || width(&longer) <= room {
                        *words = longer;
                        break;
                    }
                    line += 1;
                    if line >= lines {
                        (key, line) = (key + 1, 0);
                    }
                }
            }
        }
    }
    (placed, whole)
}

/// A word in pieces that each fit `room`.
fn split_word(word: &str, width: &dyn Fn(&str) -> f32, room: f32) -> Vec<String> {
    let mut pieces = vec![String::new()];
    for c in word.chars() {
        let last = pieces.last_mut().expect("there's always a piece");
        let longer = format!("{last}{c}");
        if last.is_empty() || width(&longer) <= room {
            *last = longer;
        } else {
            pieces.push(c.to_string());
        }
    }
    pieces
}

/// A text placed in the picture: each key's words, centred there.
struct Placed {
    pen: Pen,
    words: Vec<(f32, f32, String)>,
}

impl Placed {
    fn draw(&self, canvas: &mut RgbImage) {
        for (middle, baseline, words) in &self.words {
            self.pen.draw(canvas, words, *middle, *baseline);
        }
    }
}

/// How many lines of a pen fit a run's height, up to `most`.
fn lines_in(run: &Run, line: f32, step: f32, unit: f32, most: usize) -> usize {
    let height = (run.band.1 - run.band.0) * unit - 2.0 * MARGIN[1] * unit;
    if height < line {
        return 0;
    }
    (((height - line) / step) as usize + 1).min(most)
}

/// Places a text through its runs at the largest of `sizes`, and the fewest
/// lines a key, where it fits whole, or else at the smallest size, cut short.
fn place(text: &str, style: &KeyStyle, sizes: &[f32], runs: &[Run], most: usize, grid: &Grid) -> Placed {
    let unit = grid.unit;
    let room = unit * (1.0 - 2.0 * MARGIN[0]);
    let keys: usize = runs.iter().map(|run| run.to - run.from).sum();
    let mut chosen = None;
    'sizes: for size in sizes {
        let pen = Pen::of(style, unit * size, unit / KEY_SIZE as f32);
        let (line, ascent) = pen.metrics();
        let step = line as f32 * 1.1;
        let fitting = runs.iter().map(|run| lines_in(run, line as f32, step, unit, most)).min().unwrap_or(0);
        for lines in 1..=fitting {
            let (placed, whole) = flow(text, &|words| pen.width(words), room, keys, lines);
            chosen = Some((pen, line as f32, ascent as f32, step, placed));
            if whole {
                break 'sizes;
            }
        }
    }
    let Some((pen, line, ascent, step, placed)) = chosen else {
        return Placed { pen: Pen::of(style, unit * 0.2, 1.0), words: Vec::new() };
    };
    let mut words = Vec::new();
    let mut keys = placed.into_iter();
    for run in runs {
        let run_keys: Vec<Vec<String>> = keys.by_ref().take(run.to - run.from).collect();
        let filled = |lines: &Vec<String>| lines.iter().rposition(|words| !words.is_empty()).map_or(0, |last| last + 1);
        let used_keys = run_keys.iter().rposition(|lines| filled(lines) > 0).map_or(0, |last| last + 1);
        let used_lines = run_keys.iter().map(filled).max().unwrap_or(0);
        if used_keys == 0 {
            continue;
        }
        // Lines start level across the row, and a short text sits in its middle keys.
        let block = (used_lines - 1) as f32 * step + line;
        let band = (run.band.1 - run.band.0) * unit;
        let top = run.row as f32 * grid.step[1] + run.band.0 * unit + (band - block) / 2.0;
        let shift = (run_keys.len() - used_keys) / 2;
        for (key, lines) in run_keys.iter().enumerate() {
            let middle = (run.from + shift + key) as f32 * grid.step[0] + unit / 2.0;
            for (index, text) in lines.iter().enumerate().filter(|(_, text)| !text.is_empty()) {
                words.push((middle, top + ascent + index as f32 * step, text.clone()));
            }
        }
    }
    Placed { pen, words }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten pixels a letter, so the sums are easy.
    fn letters(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn a_three_row_pad_has_the_title_on_top_the_icon_in_the_middle_and_the_message_below() {
        let plan = plan([5, 3], true, true, true);
        assert_eq!(plan.icon, Some([2, 1]));
        assert_eq!(plan.title, vec![Run { row: 0, from: 0, to: 5, band: (0.0, 1.0) }]);
        assert_eq!(plan.message, vec![Run { row: 2, from: 0, to: 5, band: (0.0, 1.0) }]);
        // A taller pad gives the message the extra rows.
        assert_eq!(super::plan([4, 5], true, true, true).message.iter().map(|run| run.row).collect::<Vec<_>>(), [2, 3, 4]);
    }

    #[test]
    fn a_two_row_pad_starts_the_title_row_with_the_icon_when_there_is_room() {
        let plan = plan([3, 2], true, true, true);
        assert_eq!(plan.icon, Some([0, 0]));
        assert_eq!((plan.title[0].row, plan.title[0].from, plan.title[0].to), (0, 1, 3));
        assert_eq!((plan.message[0].row, plan.message[0].from), (1, 0));
        // Two keys across leave no room, so there's no icon.
        assert_eq!(super::plan([2, 2], true, true, true).icon, None);
        // Without a message, the icon has the bottom row's middle.
        assert_eq!(super::plan([3, 2], true, false, true).icon, Some([1, 1]));
    }

    #[test]
    fn a_one_row_pad_puts_the_title_over_the_message() {
        let plan = plan([4, 1], true, true, true);
        assert_eq!(plan.icon, Some([0, 0]));
        assert_eq!((plan.title[0].from, plan.title[0].band), (1, (0.0, 0.45)));
        assert_eq!((plan.message[0].from, plan.message[0].band), (1, (0.45, 1.0)));
        // An icon alone sits in the middle.
        assert_eq!(super::plan([5, 3], false, false, true).icon, Some([2, 1]));
    }

    #[test]
    fn words_fill_one_key_then_the_next_and_never_cross_a_gap() {
        let (placed, whole) = flow("Someone is at the front door", &letters, 90.0, 3, 1);
        assert!(!whole, "one line a key isn't enough");
        assert_eq!(placed, [["Someone"], ["is at the"], ["front"]]);
        let (placed, whole) = flow("Someone is at the front door", &letters, 90.0, 3, 2);
        assert!(whole);
        assert_eq!(placed, [["Someone", "is at the"], ["front", "door"], ["", ""]]);
        assert!(placed.iter().flatten().all(|words| letters(words) <= 90.0));
    }

    #[test]
    fn a_line_break_starts_a_new_key_and_a_long_word_is_split() {
        let (placed, whole) = flow("Door
Open", &letters, 90.0, 3, 2);
        assert!(whole);
        assert_eq!(placed, [["Door", ""], ["Open", ""], ["", ""]]);
        let (placed, whole) = flow("Doorbellringing", &letters, 90.0, 2, 1);
        assert!(!whole);
        assert_eq!(placed, [["Doorbellr"], ["inging"]]);
    }

    #[test]
    fn text_that_runs_out_of_keys_says_so() {
        let (placed, whole) = flow("one two three four", &letters, 50.0, 2, 1);
        assert!(!whole);
        assert_eq!(placed, [["one"], ["two"]]);
        assert_eq!(flow("anything", &letters, 90.0, 0, 1), (Vec::<Vec<String>>::new(), false));
        assert_eq!(flow("", &letters, 90.0, 1, 2), (vec![vec![String::new(), String::new()]], true));
    }

    #[test]
    fn the_picture_is_the_size_a_stretched_camera_is_laid_out_at() {
        assert_eq!(alert_size([5, 3], [50, 40], 120), (4 * 180 + 120, 2 * 168 + 120));
        assert_eq!(alert_size([1, 1], [50, 40], 120), (120, 120));
    }

    /// Whether anything but the alert's colour is drawn in the gaps between keys.
    fn ink_in_gaps(picture: &RgbImage, gaps: [u8; 2], unit: u32, color: [u8; 3]) -> bool {
        let step = steps(gaps, unit);
        picture
            .enumerate_pixels()
            .any(|(x, y, pixel)| (x % step[0] >= unit || y % step[1] >= unit) && pixel.0 != color)
    }

    #[test]
    fn no_word_or_icon_falls_in_a_gap_on_any_shape_of_pad() {
        let color = [255, 170, 0];
        let alert = AlertContent {
            title: "Front door",
            message: "Someone is at the front door. The camera saw them a moment ago.",
            icon: "mdi:doorbell",
            color,
            camera: None,
        };
        for span in [[5, 3], [3, 2], [3, 3], [3, 1], [1, 1], [8, 4]] {
            let gaps = [40, 50];
            let picture = alert_picture(&alert, span, gaps, 120);
            assert!(!ink_in_gaps(&picture, gaps, 120, color), "{span:?}");
            assert!(picture.pixels().any(|pixel| pixel.0 != color), "{span:?} shows something");
        }
    }
}

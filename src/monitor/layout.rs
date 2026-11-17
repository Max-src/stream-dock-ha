//! How the shared Mini Monitor picture is cut into a pad's keys and band.

use super::*;

pub(super) const KEY_PIXELS: i32 = 85;
pub(super) const BAND_PIXELS: i32 = 80;
pub(super) const SMALL_KEY_PIXELS: i32 = 60;

/// One pad screen and the part of the shared picture it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub column: usize,
    pub row: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Tile {
    pub fn is_band(&self) -> bool {
        self.column == 5
    }
}

/// The gaps between keys, as a percentage of a key's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gaps {
    pub across: u8,
    pub down: u8,
}

/// How the shared picture is cut up. With gaps, the tiles sit apart in the
/// picture just as the keys do on the pad, so a straight line stays straight
/// and only what's under a gap is hidden. Without them nothing is hidden, but
/// everything jumps at each gap.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<Tile>,
}

impl Layout {
    pub fn of(area: MonitorArea, gaps: Option<Gaps>) -> Layout {
        let columns = if area == MonitorArea::KeysAndBand { 6 } else { 5 };
        let (across, down) = gaps.map_or((0, 0), |g| {
            (KEY_PIXELS * g.across as i32 / 100, KEY_PIXELS * g.down as i32 / 100)
        });
        // Notes on a similar pad put the band a little closer than the keys.
        let band_gap = across * 4 / 5;
        let band_left = KEY_PIXELS * 5 + across * 4 + band_gap;
        let mut tiles = Vec::with_capacity(columns * 3);
        for column in 0..columns {
            for row in 0..3 {
                let band = column == 5;
                tiles.push(Tile {
                    column,
                    row,
                    x: if band { band_left } else { column as i32 * (KEY_PIXELS + across) },
                    y: row as i32 * (KEY_PIXELS + down),
                    width: if band { BAND_PIXELS } else { KEY_PIXELS },
                    height: if band { BAND_PIXELS } else { KEY_PIXELS },
                });
            }
        }
        Layout {
            width: if columns == 6 { band_left + BAND_PIXELS } else { KEY_PIXELS * 5 + across * 4 },
            height: KEY_PIXELS * 3 + down * 2,
            tiles,
        }
    }

    /// The AKP03E's six screen keys, three across and two down.
    pub fn small(gaps: Option<Gaps>) -> Layout {
        let (across, down) = gaps.map_or((0, 0), |g| {
            (SMALL_KEY_PIXELS * g.across as i32 / 100, SMALL_KEY_PIXELS * g.down as i32 / 100)
        });
        let mut tiles = Vec::with_capacity(6);
        for row in 0..2 {
            for column in 0..3 {
                tiles.push(Tile {
                    column,
                    row,
                    x: column as i32 * (SMALL_KEY_PIXELS + across),
                    y: row as i32 * (SMALL_KEY_PIXELS + down),
                    width: SMALL_KEY_PIXELS,
                    height: SMALL_KEY_PIXELS,
                });
            }
        }
        Layout { width: SMALL_KEY_PIXELS * 3 + across * 2, height: SMALL_KEY_PIXELS * 2 + down, tiles }
    }

    /// A picture for lining the gaps up: when the settings match the pad, the
    /// diagonals run straight across the gaps and the circle looks round.
    pub fn alignment_pattern(&self) -> Vec<u8> {
        pattern_for(self.width, self.height, || self.draw_alignment_pattern())
    }

    fn draw_alignment_pattern(&self) -> Vec<u8> {
        let (width, height) = (self.width, self.height);
        let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
        let radius = height as f32 * 0.42;
        let mut canvas = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                let line = (x + y) % 90 < 3 || (x - y).rem_euclid(90) < 3;
                let distance = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let color = if (distance - radius).abs() < 2.5 {
                    [255, 196, 64]
                } else if line {
                    [235, 238, 245]
                } else {
                    [18, 28, 52]
                };
                canvas.extend_from_slice(&color);
            }
        }
        canvas
    }

    /// Copies one tile out of the shared RGB picture.
    pub fn crop(&self, canvas: &[u8], tile: &Tile) -> Vec<u8> {
        let mut out = Vec::with_capacity((tile.width * tile.height * 3) as usize);
        for y in tile.y..tile.y + tile.height {
            let start = ((y * self.width + tile.x) * 3) as usize;
            out.extend_from_slice(&canvas[start..start + (tile.width * 3) as usize]);
        }
        out
    }
}

/// The test picture for a layout, made once per size: it takes a square
/// root per pixel.
pub(super) fn pattern_for(width: i32, height: i32, make: impl FnOnce() -> Vec<u8>) -> Vec<u8> {
    static MADE: Mutex<Vec<((i32, i32), Arc<Vec<u8>>)>> = Mutex::new(Vec::new());
    let mut made = lock(&MADE);
    if let Some((_, canvas)) = made.iter().find(|(size, _)| *size == (width, height)) {
        return canvas.to_vec();
    }
    let canvas = Arc::new(make());
    if made.len() >= 4 {
        made.clear();
    }
    made.push(((width, height), canvas.clone()));
    canvas.to_vec()
}

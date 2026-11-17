//! What each pad shows, worked out in one place. The pads themselves, the
//! window's pad view, pads on screen and, later, Home Assistant's integration
//! all take their looks from here, so what one shows can't drift from another.
//!
//! Nothing here needs the window: a frame can be made on any thread that can
//! lock the settings and the entities.

use crate::band::Face;
use crate::ha::Entities;
use crate::model::layout::{layout_of, ControlType, Layout, EDGE};
use crate::model::{Config, KeyStyle, Shared};
use crate::render::{band_picture_sized, band_shape, cut_area, key_picture_sized, Look, BAND_SIZE, KEY_SIZE};
use image::{Rgb, RgbImage};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash};

/// What's round the keys of a snapshot of a pad with a see-through front.
pub const CLEAR_FRONT: [u8; 3] = [30, 32, 38];

/// One screen key of a pad: its control's name, the profile's cell it shows
/// and what it shows there. `hash` changes whenever the picture would.
#[derive(Clone, Debug, PartialEq)]
pub struct CellFrame {
    pub control: String,
    pub cell: usize,
    pub look: Option<Look>,
    pub hash: u64,
}

/// Everything a pad's screens show at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadFrame {
    /// Every screen key of the pad's layout, in the layout's order.
    pub cells: Vec<CellFrame>,
    /// The 293S's band tiles from the top, by control name, with a hash each.
    pub band: Vec<(String, Face, u64)>,
    /// What band tiles are drawn with: the app's own style, as on the pad.
    pub band_style: KeyStyle,
    /// Where every control is, on the profile shown.
    pub layout: Layout,
    /// Changes whenever anything above would look different.
    pub look: u64,
}

impl PadFrame {
    /// What a cell shows, if it's a screen key with something on it.
    pub fn look_of(&self, cell: usize) -> Option<&Look> {
        self.cells.iter().find(|shown| shown.cell == cell)?.look.as_ref()
    }

    /// What each cell from 0 up to `count` shows, for code that goes by cell.
    pub fn looks(&self, count: usize) -> Vec<Option<Look>> {
        let mut looks = vec![None; count];
        for shown in self.cells.iter().filter(|shown| shown.cell < count) {
            looks[shown.cell] = shown.look.clone();
        }
        looks
    }

    /// The band tiles' faces, from the top.
    pub fn band_faces(&self) -> Vec<Face> {
        self.band.iter().map(|(_, face, _)| face.clone()).collect()
    }
}

/// Which profile and page a frame is of, and the colour around a stretched
/// camera's shape: black on a pad's own screen, the pad's front in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Showing {
    pub profile: usize,
    pub page: usize,
    pub outside: [u8; 3],
}

impl Showing {
    /// What the pad shows right now: its running profile at its current page,
    /// with black between the keys as on the real screen.
    pub fn running(cfg: &Config, pad_id: &str) -> Option<Showing> {
        let profile = cfg.pad_profile_of(pad_id)?;
        Some(Showing { profile, page: cfg.profiles[profile].active_page, outside: [0, 0, 0] })
    }
}

fn hash_of(value: &impl Hash) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

/// What the pad shows right now, or nothing if there's no such pad.
pub fn pad_frame(shared: &Shared, cfg: &Config, entities: &Entities, pad_id: &str) -> Option<PadFrame> {
    pad_frame_showing(shared, cfg, entities, pad_id, Showing::running(cfg, pad_id)?)
}

/// What the pad would show on any of its profiles and pages, as the window's
/// editor needs for the page someone is working on.
pub fn pad_frame_showing(shared: &Shared, cfg: &Config, entities: &Entities, pad_id: &str, showing: Showing) -> Option<PadFrame> {
    let entry = cfg.pad(pad_id)?;
    let profile = cfg.profiles.get(showing.profile)?;
    // The AKP03E's titles are drawn larger, as its keys are shown smaller.
    let style = profile.style_on_pad(&cfg.style);
    let layout = layout_of(entry, profile);
    let screens = layout.controls.iter().filter(|control| control.screen);
    let ha = shared.ha.availability(cfg.ha_offline_label);
    // A key waiting for its second press says so.
    let waiting = crate::confirm::waiting(pad_id, showing.profile, showing.page);
    let mut cells: Vec<CellFrame> = screens
        .clone()
        .filter_map(|control| {
            let cell = control.cell?;
            let mut look = Look::of_cell(profile, showing.page, cell, &style, entities, entry.gaps, showing.outside, ha);
            if let (Some(slot), Some(shown)) = (waiting, look.as_mut()) {
                if profile.owner_of(showing.page, cell) == slot {
                    shown.text = crate::confirm::PROMPT.into();
                    shown.color = crate::confirm::PROMPT_COLOR;
                    shown.style.color = [255, 255, 255];
                    shown.image = None;
                    shown.camera = None;
                    shown.unavailable = false;
                }
            }
            Some(CellFrame { control: control.name.clone(), cell, hash: hash_of(&look), look })
        })
        .collect();
    let tiles: Vec<(String, usize)> = screens
        .filter(|control| control.kind == ControlType::Band)
        .filter_map(|control| Some((control.name.clone(), control.band?)))
        .collect();
    // The band shows the same on every profile, so it's only worked out for a pad that has one.
    let faces = if tiles.is_empty() { Vec::new() } else { crate::band::faces_now(shared, cfg, entities) };
    let band: Vec<(String, Face, u64)> = tiles
        .into_iter()
        .filter_map(|(name, tile)| {
            let face = faces.get(tile)?.clone();
            let hash = hash_of(&(&face, &cfg.style));
            Some((name, face, hash))
        })
        .collect();
    // An alert is one picture over every screen key, each cell taking its
    // part of it, so `cells` and `look` follow. It covers what the pad is on,
    // not another profile or page someone is editing in the window.
    let on_the_pad = Showing::running(cfg, pad_id).is_some_and(|now| (now.profile, now.page) == (showing.profile, showing.page));
    // A flash covers one key, whatever page the pad is on; an alert covers it in turn.
    for (cell, flash) in crate::flashes::on_pad(shared, pad_id).into_iter().filter(|_| on_the_pad) {
        if let Some(shown) = cells.iter_mut().find(|shown| shown.cell == cell) {
            let look = crate::flashes::look(&flash, pad_id, cell, &style, showing.outside);
            shown.hash = hash_of(&Some(&look));
            shown.look = Some(look);
        }
    }
    if let Some(alert) = crate::alerts::active(shared, pad_id).filter(|_| on_the_pad) {
        for (cell, look) in crate::alerts::looks(&alert, pad_id, &layout, entry.gaps, &style, showing.outside) {
            if let Some(shown) = cells.iter_mut().find(|shown| shown.cell == cell) {
                shown.hash = hash_of(&Some(&look));
                shown.look = Some(look);
            }
        }
    }
    let hashes = (cells.iter().map(|shown| shown.hash).collect::<Vec<_>>(), band.iter().map(|tile| tile.2).collect::<Vec<_>>());
    let look = hash_of(&(hashes, &layout.version));
    Some(PadFrame { cells, band, band_style: cfg.style.clone(), layout, look })
}

/// A screen key's picture, `KEY_SIZE * scale` pixels across. A key with
/// nothing on it is black, as a pad shows it.
pub fn cell_picture(shown: &CellFrame, scale: u32) -> RgbImage {
    let px = KEY_SIZE * scale.max(1);
    shown.look.as_ref().map_or_else(|| RgbImage::new(px, px), |look| key_picture_sized(look, px))
}

/// A band tile's picture, `BAND_SIZE * scale` pixels across, or nothing if
/// the frame has no such tile. An empty tile is black, as the pad shows it.
pub fn band_picture(frame: &PadFrame, tile: usize, scale: u32) -> Option<RgbImage> {
    let (_, face, _) = frame.band.get(tile)?;
    Some(band_tile(face, &frame.band_style, BAND_SIZE * scale.max(1)))
}

fn band_tile(face: &Face, style: &KeyStyle, px: u32) -> RgbImage {
    if *face == Face::default() {
        RgbImage::new(px, px)
    } else {
        band_picture_sized(face, style, px)
    }
}

/// A picture of the whole pad, laid out as its layout says with its `EDGE`
/// round it, on the pad's front colour (dark grey for a see-through front). A
/// key unit is `KEY_SIZE * scale` pixels, gaps included. Buttons and knobs are
/// plain circles, and the band's tiles sit on one black strip, as on the pad.
pub fn pad_snapshot(cfg: &Config, pad_id: &str, frame: &PadFrame, scale: u32) -> RgbImage {
    let unit = (KEY_SIZE * scale.max(1)) as f32;
    let front = cfg.pad(pad_id).and_then(|entry| entry.front()).unwrap_or(CLEAR_FRONT);
    let size = |value: f32| ((value * unit).round() as u32).max(1);
    let at = |value: f32| ((value + EDGE) * unit).round() as i64;
    let mut canvas = RgbImage::from_pixel(size(frame.layout.width + 2.0 * EDGE), size(frame.layout.height + 2.0 * EDGE), Rgb(front));
    let style = &frame.band_style;
    let bands: Vec<_> = frame.layout.controls.iter().filter(|control| control.kind == ControlType::Band).collect();
    if let (Some(first), Some(last)) = (bands.first(), bands.last()) {
        let mut strip = RgbImage::new(size(first.w), size(last.y + last.h - first.y));
        let shape = band_shape(style);
        cut_area(&mut strip, shape.shape, shape.corner as f32 * size(first.w) as f32 / BAND_SIZE as f32, front);
        image::imageops::overlay(&mut canvas, &strip, at(first.x), at(first.y));
    }
    let mut knobs_drawn: Vec<&str> = Vec::new();
    for control in &frame.layout.controls {
        let (x, y) = (at(control.x), at(control.y));
        let picture = match (control.kind, control.cell, control.band) {
            (ControlType::Key, Some(cell), _) => {
                let shown = frame.cells.iter().find(|shown| shown.cell == cell);
                let mut picture = shown.map_or_else(|| RgbImage::new(size(control.w), size(control.h)), |shown| cell_picture(shown, scale));
                // The pad's black round each key would show on its front, so
                // the corners take the front's colour. A stretched camera has
                // its shape across all its keys already.
                if !shown.and_then(|shown| shown.look.as_ref()).is_some_and(Look::is_stretched) {
                    cut_area(&mut picture, style.shape, style.corner as f32 * scale as f32, front);
                }
                picture
            }
            (ControlType::Band, _, Some(tile)) => {
                let Some((_, face, _)) = frame.band.get(tile) else { continue };
                let px = size(control.w);
                let mut picture = band_tile(face, style, px);
                let shape = band_shape(style);
                cut_area(&mut picture, shape.shape, shape.corner as f32 * px as f32 / BAND_SIZE as f32, front);
                picture
            }
            _ => {
                // A knob's three parts share one place: it's drawn once.
                if let Some(group) = control.group.as_deref() {
                    if knobs_drawn.contains(&group) {
                        continue;
                    }
                    knobs_drawn.push(group);
                }
                let center = ((control.x + EDGE + control.w / 2.0) * unit, (control.y + EDGE + control.h / 2.0) * unit);
                disc(&mut canvas, center, control.w.min(control.h) * unit / 2.0, front);
                continue;
            }
        };
        image::imageops::overlay(&mut canvas, &picture, x, y);
    }
    canvas
}

/// A button or knob: a grey circle that shows on any front, with a soft edge.
fn disc(canvas: &mut RgbImage, center: (f32, f32), radius: f32, front: [u8; 3]) {
    let light = front.iter().map(|value| *value as u32).sum::<u32>() > 3 * 128;
    let color = if light { [70, 74, 84] } else { [92, 98, 112] };
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        let distance = (x as f32 + 0.5 - center.0).hypot(y as f32 + 0.5 - center.1);
        let cover = (radius + 0.5 - distance).clamp(0.0, 1.0);
        if cover > 0.0 {
            for (value, wanted) in pixel.0.iter_mut().zip(color) {
                *value = (*value as f32 * (1.0 - cover) + wanted as f32 * cover).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{small_cell_of_key, Action, DockKey, HaAction, PadKind, SMALL_SCREEN_KEYS};

    fn camera_key() -> DockKey {
        DockKey {
            actions: vec![Action::HomeAssistant(HaAction { entity: "camera.frames_test".into(), ..HaAction::default() })],
            span: [2, 2],
            camera_gaps: true,
            ..DockKey::default()
        }
    }

    fn with_every_pad() -> Config {
        let mut cfg = Config::sample();
        cfg.add_small_pad("Small");
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        let main = cfg.pad_profile(PadKind::Main).unwrap();
        cfg.profiles[main].pages[0].keys[6] = camera_key();
        cfg.profiles[main].pages[0].keys[0].title = "Covered".into();
        let small = cfg.small_pad_index().unwrap();
        cfg.profiles[small].pages[0].keys[small_cell_of_key(1)].title = "Lamp".into();
        let dashboard = cfg.pad_profile_of(&wall).unwrap();
        cfg.profiles[dashboard].pages[0].keys[5].title = "Garden".into();
        cfg.pad_mut(&wall).unwrap().face = [90, 20, 20];
        cfg.repair();
        cfg
    }

    #[test]
    fn a_frame_shows_what_each_caller_drew_before() {
        crate::camera::store("camera.frames_test", image::RgbImage::new(64, 36));
        let cfg = with_every_pad();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let entities = Entities::new();
        let id = |kind| cfg.pad_of_kind(kind).unwrap().id.clone();

        // Without Home Assistant, its camera, never seen, says "HA offline".
        let offline = pad_frame(&shared, &cfg, &entities, &id(PadKind::Main)).unwrap();
        let camera = offline.look_of(12).unwrap();
        assert_eq!((camera.text.as_str(), camera.camera.is_none()), ("HA offline", true));
        shared.ha.pretend_ready();

        // The 293S, as its driver drew it: black round a stretched camera,
        // and every cell of the grid.
        let main = cfg.main_profile();
        let frame = pad_frame(&shared, &cfg, &entities, &id(PadKind::Main)).unwrap();
        assert_eq!(frame.cells.len(), main.rows * main.cols);
        for cell in 0..main.rows * main.cols {
            let before = Look::of_cell(main, main.active_page, cell, &cfg.style, &entities, cfg.gaps(PadKind::Main), [0, 0, 0], crate::ha::Availability::Ready);
            assert_eq!(frame.look_of(cell), before.as_ref(), "cell {cell}");
        }
        let stretched = frame.look_of(12).unwrap().camera.clone().unwrap();
        assert_eq!((stretched.at, stretched.span, stretched.gaps), ([1, 1], [2, 2], cfg.gaps(PadKind::Main)));
        assert_eq!(frame.band.iter().map(|tile| tile.0.as_str()).collect::<Vec<_>>(), ["band_1", "band_2", "band_3"]);

        // The AKP03E, in its own style, in the order of its screen keys.
        let index = cfg.small_pad_index().unwrap();
        let small = &cfg.profiles[index];
        let frame = pad_frame(&shared, &cfg, &entities, &id(PadKind::Akp03e)).unwrap();
        let style = small.style_on_pad(&cfg.style);
        assert_ne!(style, cfg.style);
        let before: Vec<Option<Look>> = (0..SMALL_SCREEN_KEYS)
            .map(|key| Look::of_cell(small, small.active_page, small_cell_of_key(key), &style, &entities, cfg.gaps(PadKind::Akp03e), [0, 0, 0], crate::ha::Availability::Ready))
            .collect();
        assert_eq!(frame.cells.iter().map(|shown| shown.look.clone()).collect::<Vec<_>>(), before);
        assert!(frame.band.is_empty());
        assert_eq!(frame.cells[1].control, "key_2");

        // A virtual pad in the window, with its front round the keys.
        let wall = cfg.pads.iter().find(|entry| entry.kind == PadKind::Dashboard).unwrap();
        let index = cfg.pad_profile_of(&wall.id).unwrap();
        let showing = Showing { profile: index, page: 0, outside: wall.face };
        let frame = pad_frame_showing(&shared, &cfg, &entities, &wall.id, showing).unwrap();
        let profile = &cfg.profiles[index];
        let before: Vec<Option<Look>> = (0..8).map(|cell| Look::of_cell(profile, 0, cell, &cfg.style, &entities, wall.gaps, wall.face, crate::ha::Availability::Ready)).collect();
        assert_eq!(frame.looks(8), before);
        assert_eq!(frame.look_of(5).map(|look| look.text.as_str()), Some("Garden"));
    }

    #[test]
    fn the_frames_look_changes_with_what_it_shows() {
        let mut cfg = with_every_pad();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let id = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        let before = pad_frame(&shared, &cfg, &Entities::new(), &id).unwrap();
        assert_eq!(before, pad_frame(&shared, &cfg, &Entities::new(), &id).unwrap());
        let main = cfg.pad_profile(PadKind::Main).unwrap();
        cfg.profiles[main].pages[0].keys[3].title = "Changed".into();
        let after = pad_frame(&shared, &cfg, &Entities::new(), &id).unwrap();
        assert_ne!(before.look, after.look);
        assert_ne!(before.cells[3].hash, after.cells[3].hash);
        assert_eq!(before.cells[4].hash, after.cells[4].hash);
        assert!(pad_frame(&shared, &cfg, &Entities::new(), "0000000000000000").is_none());
    }

    #[test]
    fn an_alert_covers_every_screen_key_and_leaves_the_band() {
        let cfg = with_every_pad();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        let before = pad_frame(&shared, &cfg, &Entities::new(), &main).unwrap();
        crate::alerts::show(&shared, &main, crate::alerts::from_command(&serde_json::json!({"title": "Door"})).unwrap());
        let covered = pad_frame(&shared, &cfg, &Entities::new(), &main).unwrap();
        assert_ne!(before.look, covered.look);
        assert!(covered.cells.iter().all(|shown| shown.look.as_ref().and_then(|look| look.camera.as_ref()).is_some_and(|part| part.entity.starts_with("alert:"))));
        assert_eq!(before.band, covered.band);
        // Every pad's screen shows it, the window's included.
        let index = cfg.pad_profile_of(&main).unwrap();
        let window = pad_frame_showing(&shared, &cfg, &Entities::new(), &main, Showing { profile: index, page: 0, outside: [9, 9, 9] }).unwrap();
        assert_eq!(window.look_of(0).unwrap().camera.as_ref().unwrap().outside, [9, 9, 9]);
        crate::alerts::clear(&shared, &main);
        assert_eq!(pad_frame(&shared, &cfg, &Entities::new(), &main).unwrap().look, before.look);
    }

    #[test]
    fn an_alert_leaves_other_profiles_and_pages_in_the_editor_alone() {
        let mut cfg = with_every_pad();
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        let running = cfg.pad_profile_of(&main).unwrap();
        cfg.profiles[running].add_page();
        cfg.profiles[running].active_page = 0;
        let other = cfg.new_profile_for(&main, "Other").unwrap();
        cfg.profiles.push(other);
        cfg.repair();
        let other = cfg.profiles.len() - 1;
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let editing = |profile, page| pad_frame_showing(&shared, &cfg, &Entities::new(), &main, Showing { profile, page, outside: [0, 0, 0] }).unwrap();
        let (before, second_page, other_profile) = (pad_frame(&shared, &cfg, &Entities::new(), &main).unwrap(), editing(running, 1), editing(other, 0));
        crate::alerts::show(&shared, &main, crate::alerts::from_command(&serde_json::json!({"title": "Door"})).unwrap());
        assert_eq!(editing(running, 1).look, second_page.look);
        assert_eq!(editing(other, 0).look, other_profile.look);
        // The page it's on still shows it.
        let covered = editing(running, 0);
        assert!(covered.look_of(0).and_then(|look| look.camera.as_ref()).is_some_and(|part| part.entity.starts_with("alert:")));
        crate::alerts::clear(&shared, &main);
        assert_eq!(pad_frame(&shared, &cfg, &Entities::new(), &main).unwrap().look, before.look);
    }

    #[test]
    fn a_cell_picture_is_the_keys_picture_at_any_scale() {
        let cfg = with_every_pad();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let frame = pad_frame(&shared, &cfg, &Entities::new(), &cfg.pads[0].id).unwrap();
        let shown = frame.cells.iter().find(|shown| shown.look.is_some()).unwrap();
        assert_eq!(cell_picture(shown, 1), crate::render::key_picture(shown.look.as_ref().unwrap()));
        assert_eq!(cell_picture(shown, 2).dimensions(), (170, 170));
        let empty = CellFrame { control: "key_1".into(), cell: 0, look: None, hash: 0 };
        let black = cell_picture(&empty, 3);
        assert_eq!(black.dimensions(), (255, 255));
        assert!(black.pixels().all(|pixel| pixel.0 == [0, 0, 0]));
        let tile = band_picture(&frame, 0, 2).unwrap();
        assert_eq!(tile.dimensions(), (160, 160));
        assert!(band_picture(&frame, 3, 1).is_none());
    }

    #[test]
    fn a_snapshot_lays_the_pad_out_on_its_front() {
        let mut cfg = with_every_pad();
        let main_id = cfg.pads[0].id.clone();
        cfg.pad_mut(&main_id).unwrap().face = [200, 40, 40];
        cfg.pad_mut(&main_id).unwrap().face_clear = false;
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let frame = pad_frame(&shared, &cfg, &Entities::new(), &main_id).unwrap();
        let picture = pad_snapshot(&cfg, &main_id, &frame, 2);
        let unit = 170.0;
        let size = |value: f32| (value * unit).round() as u32;
        assert_eq!(picture.dimensions(), (size(frame.layout.width + 2.0 * EDGE), size(frame.layout.height + 2.0 * EDGE)));
        let at = |value: f32| ((value + EDGE) * unit) as u32;
        // The edge and the space between the first two keys are the front.
        assert_eq!(picture.get_pixel(3, 3).0, [200, 40, 40]);
        let second = frame.layout.control("key_2").unwrap();
        assert_eq!(picture.get_pixel(at(second.x) - 3, at(0.0) + 60).0, [200, 40, 40]);
        // A key's middle is its own picture's middle.
        let shown = frame.cells.iter().find(|shown| shown.cell == 3).unwrap();
        let key = frame.layout.control("key_4").unwrap();
        let own = cell_picture(shown, 2);
        assert_eq!(picture.get_pixel(at(key.x) + 85, at(key.y) + 85), own.get_pixel(85, 85));
        // A band tile is there too, at its size, and black strip between the tiles.
        let band = frame.layout.control("band_1").unwrap();
        let tile = band_picture_sized(&frame.band[0].1, &frame.band_style, size(band.w));
        assert_eq!(picture.get_pixel(at(band.x) + 40, at(0.0) + 40), tile.get_pixel(40, 40));
        assert_eq!(picture.get_pixel(at(band.x) + 40, at(band.y + band.h) + 3).0, [0, 0, 0]);

        // A see-through front is dark grey, and the AKP03E's knobs are circles on it.
        let small_id = cfg.pad_of_kind(crate::model::PadKind::Akp03e).unwrap().id.clone();
        cfg.pad_mut(&small_id).unwrap().face_clear = true;
        let frame = pad_frame(&shared, &cfg, &Entities::new(), &small_id).unwrap();
        let picture = pad_snapshot(&cfg, &small_id, &frame, 1);
        assert_eq!(picture.get_pixel(picture.width() - 1, picture.height() - 1).0, CLEAR_FRONT);
        let knob = frame.layout.control("top_knob_press").unwrap();
        let middle = ((knob.x + EDGE + knob.w / 2.0) * 85.0, (knob.y + EDGE + knob.h / 2.0) * 85.0);
        assert_ne!(picture.get_pixel(middle.0 as u32, middle.1 as u32).0, CLEAR_FRONT);
    }
}

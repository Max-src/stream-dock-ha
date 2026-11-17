//! The physical Stream Dock 293S.
//!
//! Protocol knowledge comes from public, independent sources: the `mirajazz`
//! library and the OpenDeck plugin for this pad. Nothing here was taken from
//! the manufacturer's app, and its files are never touched.
//!
//! Two rules protect the person's existing setup:
//! - The pad is left alone while the original app runs: two programs writing
//!   to it at once would garble the screen.
//! - Only everyday commands are used (wake, brightness, set a key's picture,
//!   refresh, clear). The boot logo and power-off commands are never sent.

use crate::actions;
use crate::band::{self, Face};
use crate::frames::{self, PadFrame};
use crate::ha::Entities;
use crate::model::{config_path, lock, Config, MonitorArea, PadKind, PadStatus, Shared};
use crate::monitor::{self, Gaps, Layout, VirtualDisplay};
use crate::model::KeyStyle;
use crate::render::{band_picture, key_picture, Look, BAND_SIZE, KEY_SIZE};
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use mirajazz::device::{list_devices, Device, DeviceQuery};
use mirajazz::error::MirajazzError;
use mirajazz::state::{DeviceStateReader, DeviceStateUpdate};
use mirajazz::types::DeviceInput;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Notify;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// How the 293S shows up on USB: 5548:6670, on the vendor-defined HID page.
const PAD: DeviceQuery = DeviceQuery::new(65440, 1, 0x5548, 0x6670);
/// The 293S uses the oldest protocol: 512-byte packets and press-only reports.
const PROTOCOL: usize = 1;
/// The pad's numbering has 18 positions: 15 keys, then the 3 side band tiles.
const SLOTS: usize = 18;
const KEYS: u8 = 15;
const PANEL_WIDTH: u32 = 854;
const PANEL_HEIGHT: u32 = 480;
/// A picture sent to the pad starts at full quality, which is lowered until the
/// file fits.
const PICTURE_LIMIT: usize = 9000;
/// The limit for a key that keeps changing, such as an animation.
pub const MOVING_PICTURE_LIMIT: usize = 3000;
/// Mini Monitor pictures with nothing new, about a second's worth, after
/// which the screen is looked at less often until something changes.
pub const STILL_FRAMES: u32 = 20;
const FIRST_QUALITY: u8 = 100;
const QUALITY_STEP: u8 = 4;
const ORIGINAL_APP: &str = "streamdock.exe";

static QUIT: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static WAKE: Notify = Notify::const_new();

/// Where a key sits on the grid, as (row, column). The pad counts by column
/// from the right: its first key is at the top right.
fn grid_position(key: u8) -> (usize, usize) {
    let key = key as usize;
    (key % 3, 4 - key / 3)
}

/// Everything the current profile can show, to have it ready before it's needed.
fn every_look(cfg: &Config, entities: &Entities, ha: crate::ha::Availability) -> Vec<Look> {
    let profile = cfg.main_profile();
    let visible = profile.key_count();
    let keys = profile.pages.iter().flat_map(|page| page.keys.iter().take(visible));
    keys.filter_map(|key| Look::of_with(key, &cfg.style, entities, ha)).collect()
}

/// What each key should show right now, from what the pad's frame shows.
fn wanted_looks(cfg: &Config, frame: Option<&PadFrame>) -> Vec<(u8, Option<Look>)> {
    let profile = cfg.main_profile();
    (0..KEYS)
        .map(|device| {
            let (row, col) = grid_position(device);
            let cell = row * profile.cols + col;
            let look = (row < profile.rows && col < profile.cols).then(|| frame?.look_of(cell).cloned()).flatten();
            (device, look)
        })
        .collect()
}

/// The startup picture is stored as raw pixels, one column at a time, flipped
/// left to right, as the manufacturer's public kit does for this model.
fn panel_bytes(picture: &RgbImage) -> Vec<u8> {
    let (width, height) = picture.dimensions();
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for x in (0..width).rev() {
        for y in 0..height {
            let [r, g, b] = picture.get_pixel(x, y).0;
            bytes.extend_from_slice(&[b, g, r]);
        }
    }
    bytes
}

fn startup_header(length: usize) -> Vec<u8> {
    let mut header = vec![0x00, 0x43, 0x52, 0x54, 0x00, 0x00, 0x4c, 0x4f, 0x47];
    header.extend_from_slice(&(length as u32).to_be_bytes());
    header.push(0x01);
    header
}

/// Stores a picture in the pad's permanent memory, shown at power-on and when
/// no app is connected. Only ever runs after the person confirms it.
async fn send_startup_picture(device: &Device, path: &std::path::Path) -> Result<(), String> {
    let picture = image::open(path).map_err(|e| e.to_string())?;
    let picture = picture.resize_to_fill(PANEL_WIDTH, PANEL_HEIGHT, FilterType::Triangle).to_rgb8();
    let bytes = panel_bytes(&picture);
    let mut header = startup_header(bytes.len());
    device.write_extended_data(&mut header).await.map_err(|e| e.to_string())?;
    for chunk in bytes.chunks(512) {
        let mut report = Vec::with_capacity(513);
        report.push(0x00);
        report.extend_from_slice(chunk);
        report.resize(513, 0);
        device.write_data(&report).await.map_err(|e| e.to_string())?;
    }
    let mut refresh = vec![0x00, 0x43, 0x52, 0x54, 0x00, 0x00, 0x53, 0x54, 0x50];
    device.write_extended_data(&mut refresh).await.map_err(|e| e.to_string())
}

/// True while the manufacturer's app is running. Only the process name is
/// looked at, nothing inside the program.
pub fn original_is_running() -> bool {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let name = crate::win::from_wide(&entry.szExeFile);
            if name.eq_ignore_ascii_case(ORIGINAL_APP) {
                found = true;
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

/// A short record of what happened with the pad, next to the settings, so a
/// problem can be looked into without guessing.
pub fn note(text: &str) {
    // A test that didn't choose its own settings file must never write next
    // to the person's own settings.
    if cfg!(test) && std::env::var_os("STREAM_DOCK_HA_CONFIG").is_none() && crate::model::sandbox_folder().is_none() {
        return;
    }
    let path = config_path().with_file_name("pad-log.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::write(&path, "");
    }
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{seconds} {text}");
    }
}

fn set_status(shared: &Shared, status: PadStatus) {
    let mut current = lock(&shared.pad_status);
    if *current != status {
        note(&status.label());
        *current = status;
        drop(current);
        shared.touch();
    }
}

/// Turns the pad's raw report into which position was pressed. Its reports
/// count from 1, and 0 means nothing happened.
fn read_input(input: u8, _state: u8) -> Result<DeviceInput, MirajazzError> {
    let input = input as usize;
    if input == 0 || input > SLOTS {
        return Ok(DeviceInput::NoData);
    }
    let mut states = vec![false; SLOTS];
    states[input - 1] = true;
    Ok(DeviceInput::ButtonStateChange(states))
}

fn pressed(shared: &Arc<Shared>, device: u8) {
    if device >= KEYS {
        note(&format!("press reported from band position {device}"));
        return;
    }
    let (row, col) = grid_position(device);
    note(&format!("press: pad position {device}, row {row}, column {col}"));
    let cols = {
        let cfg = lock(&shared.cfg);
        let profile = cfg.main_profile();
        if row >= profile.rows || col >= profile.cols {
            return;
        }
        profile.cols
    };
    actions::press_control(shared, PadKind::Main, row * cols + col, actions::PressSource::Pad);
}

async fn listen(reader: &DeviceStateReader, shared: &Arc<Shared>) -> Result<(), MirajazzError> {
    loop {
        for update in reader.read(None).await? {
            if let DeviceStateUpdate::ButtonDown(device) = update {
                pressed(shared, device);
            }
        }
    }
}

enum Stop {
    OriginalStarted,
    Quitting,
}

/// A picture as this pad wants it: turned 270 degrees, then JPEG.
fn encoded(picture: DynamicImage) -> Result<Vec<u8>, MirajazzError> {
    fitted_jpeg(&image::imageops::rotate270(&picture.to_rgb8()), PICTURE_LIMIT)
}

/// The most bytes a key's picture may take. A live camera keeps changing.
pub fn limit_of(look: &Look) -> usize {
    if look.camera.as_ref().is_some_and(|camera| crate::camera::is_moving(&camera.entity)) { MOVING_PICTURE_LIMIT } else { PICTURE_LIMIT }
}

/// A JPEG small enough for a pad to take.
///
/// The qualities tried are 100, 96, 92… down to 4, and the
/// highest that fits wins, as when stepping down one at a time. Size goes
/// down with quality, so halving the range finds it in at most 6 encodes
/// instead of up to 25: most keys fit at the first.
pub fn fitted_jpeg(picture: &RgbImage, limit: usize) -> Result<Vec<u8>, MirajazzError> {
    let encode = |quality: u8| -> Result<Vec<u8>, MirajazzError> {
        let mut data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, quality).encode_image(picture)?;
        Ok(data)
    };
    let first = encode(FIRST_QUALITY)?;
    if first.len() <= limit {
        return Ok(first);
    }
    // Steps down from the first quality: 1 is 96, the last is the lowest.
    let quality = |step: u8| FIRST_QUALITY - step * QUALITY_STEP;
    let last = (FIRST_QUALITY - 1) / QUALITY_STEP;
    let (mut low, mut high) = (1, last);
    let mut best = None;
    let mut lowest = None;
    while low <= high {
        let middle = (low + high) / 2;
        let data = encode(quality(middle))?;
        if data.len() <= limit {
            best = Some(data);
            high = middle - 1;
        } else {
            if middle == last {
                lowest = Some(data);
            }
            low = middle + 1;
        }
    }
    // Nothing fits: the lowest quality, where stepping down would end.
    match (best, lowest) {
        (Some(data), _) | (None, Some(data)) => Ok(data),
        (None, None) => encode(quality(last)),
    }
}

fn black(size: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::new(size, size))
}

/// The most tiles sent before each refresh. The refresh itself costs the pad
/// about 37 ms and each tile about 31 ms, so sending everything at once would
/// show nothing new for half a second; small batches keep the screen moving.
const TILES_PER_REFRESH: usize = 6;

/// The pad showing an extra Windows display. Only tiles that visibly changed
/// are sent, and the ones that have waited longest go first, so every part of
/// the screen keeps up during full-screen movement.
/// The Mini Monitor settings that change how the picture is laid out.
#[derive(Clone, Copy, PartialEq)]
struct Shape {
    area: MonitorArea,
    balance: i8,
    gaps: Option<Gaps>,
}

impl Shape {
    fn of(cfg: &Config) -> Shape {
        let [across, down] = cfg.gaps(PadKind::Main);
        Shape {
            area: cfg.monitor_area,
            balance: cfg.band_balance,
            gaps: cfg.monitor_gaps.then_some(Gaps { across, down }),
        }
    }
}

struct Mirror {
    /// None while only the alignment picture is showing.
    display: Option<VirtualDisplay>,
    shape: Shape,
    layout: Layout,
    last: Vec<Vec<u8>>,
    waiting: Vec<Option<Instant>>,
    refreshes: u32,
    tiles_sent: u32,
    capture_time: Duration,
    captures: u32,
    since: Instant,
}

impl Mirror {
    fn start(shared: &Shared, shape: Shape, with_display: bool) -> Result<Mirror, String> {
        let display = if with_display {
            let display = VirtualDisplay::add()?;
            monitor::set_live_display(PadKind::Main, &display);
            note(&format!("Mini Monitor on: {}", display.name()));
            set_status(shared, PadStatus::MiniMonitor(display.name().to_string()));
            Some(display)
        } else {
            note("alignment picture on");
            None
        };
        let mut mirror = Mirror {
            display,
            shape,
            layout: Layout::of(shape.area, shape.gaps),
            last: Vec::new(),
            waiting: Vec::new(),
            refreshes: 0,
            tiles_sent: 0,
            capture_time: Duration::ZERO,
            captures: 0,
            since: Instant::now(),
        };
        mirror.arrange(shape);
        Ok(mirror)
    }

    /// Starts afresh with this layout, so every tile gets sent again.
    fn arrange(&mut self, shape: Shape) {
        self.shape = shape;
        self.layout = Layout::of(shape.area, shape.gaps);
        self.last = vec![Vec::new(); self.layout.tiles.len()];
        self.waiting = vec![None; self.layout.tiles.len()];
    }

    /// Sends the tiles most in need of it, and says how many went out.
    async fn show_frame(&mut self, device: &Device, pattern: bool) -> Result<usize, MirajazzError> {
        let (width, height) = (self.layout.width, self.layout.height);
        let capture_started = Instant::now();
        let canvas = if pattern {
            Some(Arc::new(self.layout.alignment_pattern()))
        } else {
            self.display.as_ref().and_then(|d| d.area()).and_then(|area| monitor::capture(PadKind::Main, &self.layout, area))
        };
        let Some(canvas) = canvas else {
            return Ok(0);
        };
        self.capture_time += capture_started.elapsed();
        self.captures += 1;
        if let Some(path) = std::env::var_os("STREAM_DOCK_HA_CANVAS") {
            if let Some(picture) = RgbImage::from_raw(width as u32, height as u32, canvas.to_vec()) {
                let _ = picture.save(path);
            }
        }

        let now = Instant::now();
        let mut changed: Vec<(Instant, usize, Vec<u8>)> = Vec::new();
        for (index, tile) in self.layout.tiles.iter().enumerate() {
            let pixels = self.layout.crop(&canvas, tile);
            if monitor::visibly_changed(&self.last[index], &pixels) {
                let since = *self.waiting[index].get_or_insert(now);
                changed.push((since, index, pixels));
            } else {
                self.waiting[index] = None;
            }
        }
        changed.sort_by_key(|(since, index, _)| (*since, *index));
        changed.truncate(TILES_PER_REFRESH);

        let count = changed.len();
        for (_, index, pixels) in changed {
            let tile = self.layout.tiles[index];
            let darker = match (tile.is_band(), self.shape.balance) {
                (true, b) if b < 0 => b.unsigned_abs(),
                (false, b) if b > 0 => b as u8,
                _ => 0,
            };
            // A copy only when it's darkened: the tile is kept as it came.
            let darkened = (darker > 0 && self.shape.area == MonitorArea::KeysAndBand).then(|| {
                let mut copy = pixels.clone();
                monitor::darken(&mut copy, darker);
                copy
            });
            let shown = darkened.as_ref().unwrap_or(&pixels);
            let position = if tile.is_band() {
                KEYS + tile.row as u8
            } else {
                (4 - tile.column) as u8 * 3 + tile.row as u8
            };
            if let Some(data) = monitor::encode_tile(&shown, tile.width as usize, tile.height as usize) {
                device.write_image(position, &data).await?;
            }
            self.last[index] = pixels;
            self.waiting[index] = None;
        }
        if count > 0 {
            device.flush().await?;
            self.refreshes += 1;
            self.tiles_sent += count as u32;
        }

        let seconds = self.since.elapsed().as_secs_f32();
        if seconds >= 10.0 && self.refreshes > 0 {
            let rate = self.refreshes as f32 / seconds;
            let tiles = self.tiles_sent as f32 / seconds;
            let capture = self.capture_time.as_millis() as u32 / self.captures.max(1);
            note(&format!(
                "Mini Monitor: {rate:.1} screen updates and {tiles:.0} tiles a second, capture {capture} ms"
            ));
            self.refreshes = 0;
            self.tiles_sent = 0;
            self.capture_time = Duration::ZERO;
            self.captures = 0;
            self.since = Instant::now();
        }
        Ok(count)
    }
}

/// Sends whichever band tiles changed, and says how many. They appear with
/// the next refresh.
async fn update_band(
    device: &Device,
    band_shown: &mut [Option<Face>],
    faces: Vec<Face>,
    blank_tile: &[u8],
    style: &KeyStyle,
) -> Result<usize, MirajazzError> {
    let mut changed = 0;
    for (tile, face) in faces.into_iter().enumerate() {
        if band_shown[tile].as_ref() == Some(&face) {
            continue;
        }
        let position = KEYS + tile as u8;
        if face == Face::default() {
            device.write_image(position, blank_tile).await?;
        } else {
            let drawn = DynamicImage::ImageRgb8(band_picture(&face, style));
            let picture = encoded(drawn)?;
            device.write_image(position, &picture).await?;
        }
        band_shown[tile] = Some(face);
        changed += 1;
    }
    Ok(changed)
}

/// Keeps the pad showing what the configuration says.
///
/// Everything goes out as one batch followed by a single refresh, so a whole
/// page changes at once. An empty key is sent as a black picture for that
/// reason: a clear command takes effect on its own, one key at a time.
async fn keep_in_step(device: &Device, shared: &Shared) -> Result<Stop, MirajazzError> {
    let mut shown: HashMap<u8, Option<Look>> = HashMap::new();
    let mut band_shown: Vec<Option<Face>> = vec![None; 3];
    let mut pictures: HashMap<Look, Vec<u8>> = HashMap::new();
    let blank_key = encoded(black(KEY_SIZE))?;
    let blank_tile = encoded(black(BAND_SIZE))?;
    let mut brightness = None;
    let mut next_check = Instant::now();
    let mut mirror: Option<Mirror> = None;
    // Mini Monitor pictures in a row with nothing to send.
    let mut still = 0u32;
    loop {
        if QUIT.load(Ordering::SeqCst) {
            // Hands the screen back to the pad, which then shows its startup picture.
            let mut let_go = vec![0x00, 0x43, 0x52, 0x54, 0x00, 0x00, 0x43, 0x4c, 0x45, 0x00, 0x00, 0x44, 0x43];
            device.write_extended_data(&mut let_go).await?;
            return Ok(Stop::Quitting);
        }
        if Instant::now() >= next_check {
            if original_is_running() {
                return Ok(Stop::OriginalStarted);
            }
            // Prepares the other pages now, so changing page has nothing to draw.
            // Mini Monitor covers the keys, so that waits until it ends.
            let upcoming: HashSet<Look> = if mirror.is_some() {
                HashSet::new()
            } else {
                let cfg = lock(&shared.cfg);
                every_look(&cfg, &lock(&shared.ha.entities), shared.ha.availability(cfg.ha_offline_label)).into_iter().collect()
            };
            if mirror.is_none() {
                pictures.retain(|look, _| upcoming.contains(look));
            }
            for look in upcoming {
                if !pictures.contains_key(&look) {
                    let drawn = DynamicImage::ImageRgb8(key_picture(&look));
                    let picture = encoded(drawn)?;
                    pictures.insert(look, picture);
                }
            }
            next_check = Instant::now() + Duration::from_secs(1);
        }

        let startup_picture = lock(&shared.startup_picture).take();
        if let Some(path) = startup_picture {
            note("sending the startup picture");
            shared.set_status("Storing the startup picture on the pad…");
            let result = send_startup_picture(device, &path).await;
            note(&format!("startup picture: {result:?}"));
            shared.set_status(match result {
                Ok(()) => "The startup picture was sent to the pad".to_string(),
                Err(e) => format!("The startup picture couldn't be sent: {e}"),
            });
            // Everything on the screen is drawn again after that.
            shown.clear();
            band_shown.fill(None);
            if let Some(active) = mirror.as_mut() {
                let shape = active.shape;
                active.arrange(shape);
            }
        }

        let (wanted, level, faces, monitor_on, shape, style, its_picture) = {
            let cfg = lock(&shared.cfg);
            let entities = lock(&shared.ha.entities);
            // While Mini Monitor stays on, its picture covers the keys: only the
            // band can need drawing.
            let covered = mirror.is_some() && cfg.monitor_on(PadKind::Main);
            let frame = (!covered).then(|| cfg.pad_of_kind(PadKind::Main).and_then(|entry| frames::pad_frame(shared, &cfg, &entities, &entry.id))).flatten();
            (
                wanted_looks(&cfg, frame.as_ref()),
                if crate::screen_off::asleep(&cfg, PadKind::Main) { 0 } else { cfg.brightness_of(PadKind::Main) },
                frame.map_or_else(|| band::faces_now(shared, &cfg, &entities), |frame| frame.band_faces()),
                cfg.monitor_on(PadKind::Main),
                Shape::of(&cfg),
                cfg.style.clone(),
                cfg.monitor_pad == PadKind::Main,
            )
        };
        if brightness != Some(level) {
            device.set_brightness(level).await?;
            brightness = Some(level);
        }

        let pattern = its_picture && shared.alignment_pattern.load(Ordering::SeqCst);
        // Turning Mini Monitor on or off during an alignment starts afresh.
        if mirror.as_ref().is_some_and(|m| m.display.is_some() != monitor_on) {
            mirror = None;
            shown.clear();
            band_shown.fill(None);
            set_status(shared, PadStatus::Connected);
        }
        if (monitor_on || pattern) && mirror.is_none() {
            match Mirror::start(shared, shape, monitor_on) {
                Ok(started) => mirror = Some(started),
                Err(problem) => {
                    note(&format!("Mini Monitor couldn't start: {problem}"));
                    lock(&shared.cfg).mini_monitor = false;
                    shared.save_now();
                    shared.set_status(problem);
                    continue;
                }
            }
        } else if !monitor_on && !pattern && mirror.is_some() {
            if mirror.take().is_some_and(|ended| ended.shape.area == MonitorArea::KeysAndBand) {
                band_shown.fill(None);
            }
            note("Mini Monitor off");
            // The keys are simply drawn over, so they come back in one go.
            shown.clear();
            set_status(shared, PadStatus::Connected);
        }
        if let Some(active) = mirror.as_mut() {
            if active.shape != shape {
                // Switching to the keys-only picture hands the band back.
                band_shown.fill(None);
                active.arrange(shape);
            }
            let tiles = active.show_frame(device, pattern).await?;
            if active.shape.area == MonitorArea::KeysOnly {
                let tiles_changed = update_band(device, &mut band_shown, faces, &blank_tile, &style).await?;
                if tiles_changed > 0 && tiles == 0 {
                    device.flush().await?;
                }
            }
            still = if tiles == 0 { still.saturating_add(1) } else { 0 };
            if tiles == 0 {
                // A screen that stayed still for a second is looked at less often.
                let wait = if still > STILL_FRAMES { 150 } else { 50 };
                tokio::select! {
                    _ = WAKE.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                }
            }
            continue;
        }

        let started = Instant::now();

        let mut keys_changed = 0;
        for (position, look) in wanted {
            if shown.get(&position) == Some(&look) {
                continue;
            }
            match &look {
                Some(look) => {
                    if !pictures.contains_key(look) {
                        let turned = image::imageops::rotate270(&key_picture(look));
                        pictures.insert(look.clone(), fitted_jpeg(&turned, limit_of(look))?);
                    }
                    device.write_image(position, &pictures[look]).await?;
                }
                None => device.write_image(position, &blank_key).await?,
            }
            shown.insert(position, look);
            keys_changed += 1;
        }

        let tiles_changed = update_band(device, &mut band_shown, faces, &blank_tile, &style).await?;

        if keys_changed + tiles_changed > 0 {
            device.flush().await?;
        }
        if keys_changed > 0 {
            let took = started.elapsed().as_millis();
            note(&format!("updated {keys_changed} keys and {tiles_changed} band tiles in {took} ms"));
        }

        // Sleeps until something changes; the timer covers edits and the clock.
        tokio::select! {
            _ = WAKE.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
        }
    }
}

async fn use_pad(device: Device, shared: &Arc<Shared>) -> Result<Stop, MirajazzError> {
    let level = lock(&shared.cfg).brightness_of(PadKind::Main);
    device.set_brightness(level).await?;
    device.clear_all_button_images().await?;
    device.flush().await?;
    set_status(shared, PadStatus::Connected);
    CONNECTED.store(true, Ordering::SeqCst);

    let reader = device.get_reader(read_input);
    tokio::select! {
        heard = listen(&reader, shared) => heard.map(|_| Stop::Quitting),
        stop = keep_in_step(&device, shared) => stop,
    }
}

async fn run(shared: Arc<Shared>) {
    while !QUIT.load(Ordering::SeqCst) {
        if original_is_running() {
            set_status(&shared, PadStatus::OriginalRunning);
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let found = list_devices(&[PAD]).await.ok().and_then(|d| d.into_iter().next());
        let Some(found) = found else {
            set_status(&shared, PadStatus::NotFound);
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        };
        match Device::connect(&found, PROTOCOL, SLOTS, 0).await {
            Ok(device) => {
                let result = use_pad(device, &shared).await;
                CONNECTED.store(false, Ordering::SeqCst);
                match result {
                    Ok(Stop::Quitting) => return,
                    Ok(Stop::OriginalStarted) => set_status(&shared, PadStatus::OriginalRunning),
                    Err(e) => set_status(&shared, PadStatus::Problem(e.to_string())),
                }
            }
            Err(e) => set_status(&shared, PadStatus::Problem(e.to_string())),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Starts looking after the pad, with or without the window.
pub fn start(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build();
        match runtime {
            Ok(runtime) => runtime.block_on(run(shared)),
            Err(e) => set_status(&shared, PadStatus::Problem(e.to_string())),
        }
    });
}

/// Tells the pad something changed, so a new page doesn't wait for the timer.
pub fn wake() {
    WAKE.notify_one();
}

/// Before the app ends: blanks the pad's keys so old buttons don't stay lit
/// with nothing behind them.
pub fn finish() {
    crate::small_pad::finish();
    QUIT.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_millis(1500);
    while CONNECTED.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table published for this pad, grid position to pad position.
    const PUBLISHED: [u8; 15] = [12, 9, 6, 3, 0, 13, 10, 7, 4, 1, 14, 11, 8, 5, 2];

    #[test]
    fn numbering_matches_the_published_table() {
        for row in 0..3 {
            for col in 0..5 {
                assert_eq!(grid_position(PUBLISHED[row * 5 + col]), (row, col));
            }
        }
    }

    #[test]
    fn reports_are_counted_from_one_and_ignored_when_strange() {
        let DeviceInput::ButtonStateChange(states) = read_input(1, 1).unwrap() else {
            panic!("a press should be reported");
        };
        assert!(states[0] && states.iter().filter(|s| **s).count() == 1);
        assert!(read_input(0, 1).unwrap().is_empty());
        assert!(read_input(19, 1).unwrap().is_empty());
    }

    #[test]
    fn a_page_reaches_the_right_pad_positions() {
        let cfg = Config::sample();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let frame = frames::pad_frame(&shared, &cfg, &Entities::new(), &cfg.pads[0].id);
        let looks = wanted_looks(&cfg, frame.as_ref());
        let at = |device: u8| looks[device as usize].1.as_ref().map(|l| l.text.clone());
        assert_eq!(at(12).as_deref(), Some("Calculator")); // top left
        assert_eq!(at(0).as_deref(), Some("Next page")); // top right
        assert_eq!(at(14).as_deref(), Some("Copy")); // bottom left
        assert_eq!(at(2), None); // bottom right is empty in the sample
        assert_eq!(looks.len(), 15); // the band is handled apart from the keys
    }

    #[test]
    fn any_key_leaves_mini_monitor_without_running_its_action() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-pad-test.json"));
        let mut cfg = Config::sample();
        cfg.mini_monitor = true;
        cfg.profiles[0].active_page = 1;
        let (shared, _opened) = Shared::new(cfg, true);
        pressed(&shared, 12);
        let cfg = lock(&shared.cfg);
        assert!(!cfg.mini_monitor);
        assert_eq!(cfg.profile().active_page, 1); // still on the same page
        assert!(lock(&shared.pad_press).is_none()); // and no key was run
    }

    #[test]
    fn other_pages_are_prepared_too() {
        let mut cfg = Config::sample();
        cfg.profiles[0].pages[1].keys[0].title = "Second page".into();
        assert!(every_look(&cfg, &Entities::new(), crate::ha::Availability::Ready).iter().any(|look| look.text == "Second page"));
    }

    #[test]
    fn band_tiles_are_the_manufacturers_size() {
        let cfg = Config::sample();
        for face in band::faces(&cfg, band::Stats { cpu: 42, memory: 93, gpu: 61 }, &Entities::new(), &band::History::default()) {
            assert_eq!(band_picture(&face, &cfg.style).dimensions(), (BAND_SIZE, BAND_SIZE));
        }
    }

    #[test]
    fn keys_are_drawn_at_the_pads_size_with_something_on_them() {
        let cfg = Config::sample();
        let look = Look::of(&cfg.profiles[0].pages[0].keys[2], &cfg.style, &Entities::new()).unwrap();
        let picture = key_picture(&look);
        assert_eq!(picture.dimensions(), (KEY_SIZE, KEY_SIZE));
        let white = picture.pixels().filter(|p| p.0 == [255, 255, 255]).count();
        assert!(white > 50, "only {white} white pixels");
    }

    #[test]
    fn startup_picture_header_states_the_exact_size() {
        assert_eq!(
            startup_header(854 * 480 * 3),
            vec![0, 0x43, 0x52, 0x54, 0, 0, 0x4c, 0x4f, 0x47, 0x00, 0x12, 0xc3, 0xc0, 0x01]
        );
    }

    #[test]
    fn startup_picture_is_flipped_and_sent_by_columns() {
        let mut picture = RgbImage::new(2, 2);
        picture.put_pixel(0, 0, image::Rgb([1, 2, 3]));
        picture.put_pixel(1, 0, image::Rgb([4, 5, 6]));
        picture.put_pixel(1, 1, image::Rgb([7, 8, 9]));
        assert_eq!(panel_bytes(&picture), vec![6, 5, 4, 9, 8, 7, 3, 2, 1, 0, 0, 0]);
    }

    /// A rough photo-like picture, hard to compress.
    fn noisy(size: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(size, size, |x, y| {
            let mix = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503)).wrapping_mul(2246822519);
            image::Rgb([(mix >> 24) as u8, (mix >> 16) as u8, (mix >> 8) as u8])
        }))
    }

    #[test]
    fn pictures_start_at_full_quality_and_shrink_to_fit() {
        let plain = encoded(black(KEY_SIZE)).unwrap();
        let mut full = Vec::new();
        let turned = image::imageops::rotate270(&black(KEY_SIZE).to_rgb8());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut full, 100).encode_image(&turned).unwrap();
        assert_eq!(plain, full);
        assert!(encoded(noisy(KEY_SIZE)).unwrap().len() <= PICTURE_LIMIT);
    }

    /// Halving the range picks the quality stepping down one at a time would.
    #[test]
    fn the_quick_search_picks_what_stepping_down_would() {
        let stepping = |picture: &RgbImage, limit: usize| {
            let mut quality = FIRST_QUALITY;
            loop {
                let mut data = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, quality).encode_image(picture).unwrap();
                if data.len() <= limit || quality <= QUALITY_STEP {
                    return data;
                }
                quality -= QUALITY_STEP;
            }
        };
        let noisy_key = noisy(KEY_SIZE).to_rgb8();
        for limit in [500, 3000, 6000, PICTURE_LIMIT, 20_000, 100_000] {
            assert_eq!(fitted_jpeg(&noisy_key, limit).unwrap(), stepping(&noisy_key, limit), "limit {limit}");
        }
        let plain = black(KEY_SIZE).to_rgb8();
        assert_eq!(fitted_jpeg(&plain, 10).unwrap(), stepping(&plain, 10));
    }

    #[test]
    fn moving_pictures_get_the_smaller_limit() {
        let moving = fitted_jpeg(&noisy(KEY_SIZE).to_rgb8(), MOVING_PICTURE_LIMIT).unwrap();
        assert!(moving.len() <= MOVING_PICTURE_LIMIT, "{}", moving.len());
    }

    #[test]
    fn only_camera_keys_count_as_moving() {
        let key = crate::model::DockKey { title: "Still".into(), ..Default::default() };
        let style = KeyStyle::default();
        let mut look = Look::of(&key, &style, &Entities::new()).unwrap();
        assert_eq!(limit_of(&look), PICTURE_LIMIT);
        look.camera = Some(crate::render::CameraPart {
            entity: "camera.door".into(),
            version: 1,
            at: [0, 0],
            span: [1, 1],
            fit: Default::default(),
            gaps: [0, 0],
            shape: Default::default(),
            corner: 0,
            outside: [0, 0, 0],
        });
        assert_eq!(limit_of(&look), MOVING_PICTURE_LIMIT);
    }

    /// The new encoder has to turn pictures exactly as the pad library did.
    #[test]
    fn pictures_face_the_same_way_as_with_the_pad_library() {
        use mirajazz::types::{ImageFormat, ImageMirroring, ImageMode, ImageRotation};
        let format = ImageFormat {
            mode: ImageMode::JPEG,
            size: (KEY_SIZE as usize, KEY_SIZE as usize),
            rotation: ImageRotation::Rot90,
            mirror: ImageMirroring::Both,
        };
        let split = KEY_SIZE / 2;
        let picture = DynamicImage::ImageRgb8(RgbImage::from_fn(KEY_SIZE, KEY_SIZE, |x, y| {
            image::Rgb(match (x < split, y < split) {
                (true, true) => [255, 0, 0],
                (false, true) => [0, 255, 0],
                (true, false) => [0, 0, 255],
                (false, false) => [255, 255, 0],
            })
        }));
        let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
        let theirs = runtime.block_on(mirajazz::images::convert_image_with_format(format, picture.clone())).unwrap();
        let theirs = image::load_from_memory(&theirs).unwrap().to_rgb8();
        let ours = image::load_from_memory(&encoded(picture).unwrap()).unwrap().to_rgb8();
        for (x, y) in [(10, 10), (75, 10), (10, 75), (75, 75)] {
            let (a, b) = (ours.get_pixel(x, y).0, theirs.get_pixel(x, y).0);
            assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) < 30), "at {x},{y}: {a:?} and {b:?}");
        }
    }

    /// Only looks at what Windows lists: `cargo test --release finds_the_pad -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the pad plugged in"]
    fn finds_the_pad() {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_time().build().unwrap();
        let found = runtime.block_on(list_devices(&[PAD])).unwrap();
        println!("found {} pad(s), original running: {}", found.len(), original_is_running());
        assert_eq!(found.len(), 1);
    }
}

//! Runs an AJAZZ AKP03E next to the main pad, from a profile of its own.
//!
//! Its 18 controls are the cells of that profile, so they're set up in the
//! window like any key: see `model::small_cell_of_key` for the layout. Only
//! everyday commands are used (wake, brightness, set a key's picture, refresh,
//! stay connected, clear), so nothing is stored on the pad.

use crate::actions;
use crate::model::{lock, small_cell_of_key, PadKind, Shared, SMALL_SCREEN_KEYS};
use crate::monitor::{self, Gaps, Layout, VirtualDisplay};
use crate::pad::{fitted_jpeg, limit_of, note, original_is_running, MOVING_PICTURE_LIMIT};
use crate::render::{key_picture, Look};
use image::imageops::FilterType;
use image::RgbImage;
use mirajazz::device::{list_devices, Device, DeviceQuery};
use mirajazz::error::MirajazzError;
use mirajazz::types::DeviceInput;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How the AKP03E shows itself on USB: AJAZZ's vendor 0x0300, product 0x1002.
pub const QUERY: DeviceQuery = DeviceQuery::new(65440, 1, 0x0300, 0x1002);
/// From the public opendeck-akp03 project, and confirmed on the real pad.
const PROTOCOL: usize = 2;
/// The six screen keys and the three plain buttons.
const POSITIONS: usize = 9;
const KNOBS: usize = 3;
const PICTURE_SIZE: u32 = 60;
const KEEP_ALIVE: Duration = Duration::from_secs(10);

static QUIT: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicBool = AtomicBool::new(false);
static PLUGGED_IN: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<String> = Mutex::new(String::new());
static PRESSED: Mutex<Option<(usize, Instant)>> = Mutex::new(None);
static CODES: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Whether Windows lists the pad. Looking at that list doesn't open the pad, so
/// this is safe while the StreamDock app is open.
pub fn is_plugged_in() -> bool {
    PLUGGED_IN.load(Ordering::SeqCst)
}

/// Whether the pad is running.
pub fn is_connected() -> bool {
    CONNECTED.load(Ordering::SeqCst)
}

/// Lights a control up in the window, as after a press.
pub fn mark_pressed(cell: usize) {
    *lock(&PRESSED) = Some((cell, Instant::now()));
}

/// Whether a cell's control was just used, so the window can light it up.
pub fn just_pressed(cell: usize) -> bool {
    lock(&PRESSED).is_some_and(|(pressed, at)| pressed == cell && at.elapsed().as_millis() < 300)
}

/// How it's going, in plain words, for the window.
pub fn status() -> String {
    let text = lock(&STATUS).clone();
    if text.is_empty() {
        "Looking for it…".to_string()
    } else {
        text
    }
}

fn set_status(shared: &Shared, text: &str) {
    let mut status = lock(&STATUS);
    if *status != text {
        *status = text.to_string();
        drop(status);
        note(&format!("AKP03E: {text}"));
        shared.touch();
    }
}

/// Which cell of the profile a code from the pad belongs to.
///
/// All 18 codes were seen on the real pad.
pub fn cell_of_code(code: u8) -> Option<usize> {
    Some(match code {
        1..=6 => small_cell_of_key(code as usize - 1),
        0x25 => 12,
        0x30 => 13,
        0x31 => 14,
        0x90 => 3,
        0x50 => 4,
        0x60 => 5,
        0x91 => 9,
        0x51 => 10,
        0x61 => 11,
        0x33 => 15,
        0x35 => 16,
        0x34 => 17,
        _ => return None,
    })
}

fn reported(code: u8, _state: u8) -> Result<DeviceInput, MirajazzError> {
    // Code 0 is the pad acknowledging a command, not a press.
    if code != 0 {
        lock(&CODES).push(code);
    }
    Ok(DeviceInput::NoData)
}

fn act(shared: &Arc<Shared>, code: u8) {
    let Some(cell) = cell_of_code(code) else {
        note(&format!("AKP03E: unknown code {code:#04x}"));
        return;
    };
    note(&format!("AKP03E: code {code:#04x}"));
    // Looks the profile up now: the profiles may have changed during the read.
    actions::press_control(shared, PadKind::Akp03e, cell, actions::PressSource::Pad);
}

/// What the screen keys should show, or nothing once the profile is gone.
/// While Mini Monitor `covered` them, no looks are made.
fn wanted(shared: &Shared, covered: bool) -> Option<(usize, Vec<Option<Look>>, u8)> {
    let cfg = lock(&shared.cfg);
    let index = cfg.small_pad_index()?;
    let entities = lock(&shared.ha.entities);
    let entry = cfg.pad_of_kind(PadKind::Akp03e)?;
    // The frame lists the screen keys in the pad's own order.
    let looks: Vec<Option<Look>> = if covered {
        Vec::new()
    } else {
        let frame = crate::frames::pad_frame(shared, &cfg, &entities, &entry.id)?;
        frame.cells.into_iter().map(|shown| shown.look).collect()
    };
    let level = if crate::screen_off::asleep(&cfg, PadKind::Akp03e) { 0 } else { cfg.brightness_of(PadKind::Akp03e) };
    Some((index, looks, level))
}

fn picture(look: &Option<Look>) -> Result<Vec<u8>, MirajazzError> {
    let small = match look {
        Some(look) => image::imageops::resize(&key_picture(look), PICTURE_SIZE, PICTURE_SIZE, FilterType::Triangle),
        None => RgbImage::new(PICTURE_SIZE, PICTURE_SIZE),
    };
    fitted_jpeg(&small, look.as_ref().map_or(MOVING_PICTURE_LIMIT, limit_of))
}

/// The pad showing an extra Windows display, or the picture for lining up its
/// gaps. Only keys that visibly changed are sent.
struct Mirror {
    /// None while only the alignment picture is showing.
    display: Option<VirtualDisplay>,
    gaps: Option<Gaps>,
    layout: Layout,
    last: Vec<Vec<u8>>,
}

impl Mirror {
    fn start(with_display: bool, gaps: Option<Gaps>) -> Result<Mirror, String> {
        let display = if with_display { Some(VirtualDisplay::add()?) } else { None };
        if let Some(display) = &display {
            monitor::set_live_display(PadKind::Akp03e, display);
        }
        let layout = Layout::small(gaps);
        let last = vec![Vec::new(); layout.tiles.len()];
        Ok(Mirror { display, gaps, layout, last })
    }

    fn arrange(&mut self, gaps: Option<Gaps>) {
        self.gaps = gaps;
        self.layout = Layout::small(gaps);
        self.last = vec![Vec::new(); self.layout.tiles.len()];
    }

    /// Sends the keys that changed, and says how many.
    async fn show_frame(&mut self, device: &Device, pattern: bool) -> Result<usize, MirajazzError> {
        let canvas = if pattern {
            Some(std::sync::Arc::new(self.layout.alignment_pattern()))
        } else {
            self.display.as_ref().and_then(|d| d.area()).and_then(|area| monitor::capture(PadKind::Akp03e, &self.layout, area))
        };
        let Some(canvas) = canvas else { return Ok(0) };
        let mut changed = 0;
        for (index, tile) in self.layout.tiles.iter().enumerate() {
            let pixels = self.layout.crop(&canvas, tile);
            if monitor::visibly_changed(&self.last[index], &pixels) {
                let picture = RgbImage::from_raw(PICTURE_SIZE, PICTURE_SIZE, pixels.clone());
                if let Some(picture) = picture {
                    let data = fitted_jpeg(&picture, MOVING_PICTURE_LIMIT)?;
                    device.write_image((tile.row * 3 + tile.column) as u8, &data).await?;
                    changed += 1;
                }
                self.last[index] = pixels;
            }
        }
        if changed > 0 {
            device.flush().await?;
        }
        Ok(changed)
    }
}

/// Whether this pad should show Mini Monitor, the alignment picture, and with
/// which gaps.
fn monitor_wish(shared: &Shared) -> (bool, bool, Option<Gaps>) {
    let cfg = lock(&shared.cfg);
    let [across, down] = cfg.gaps(PadKind::Akp03e);
    let gaps = Gaps { across, down };
    let pattern = cfg.monitor_pad == PadKind::Akp03e && shared.alignment_pattern.load(Ordering::SeqCst);
    (cfg.monitor_on(PadKind::Akp03e), pattern, cfg.monitor_gaps.then_some(gaps))
}

/// Any press during Mini Monitor only means "back to my keys".
fn leave_monitor(shared: &Shared) {
    lock(&shared.cfg).mini_monitor = false;
    shared.alignment_pattern.store(false, Ordering::SeqCst);
    note("AKP03E: press, leaving Mini Monitor");
    shared.save_now();
    shared.repaint();
}

async fn drive(device: Device, shared: &Arc<Shared>) -> Result<(), MirajazzError> {
    device.clear_all_button_images().await?;
    device.flush().await?;
    let reader = device.get_reader(reported);
    let mut shown: Vec<Option<Option<Look>>> = vec![None; SMALL_SCREEN_KEYS];
    let mut brightness = None;
    let mut kept_alive = Instant::now();
    let mut mirror: Option<Mirror> = None;
    // Mini Monitor pictures in a row with nothing to send.
    let mut still = 0u32;
    set_status(shared, "Connected");
    CONNECTED.store(true, Ordering::SeqCst);

    while !QUIT.load(Ordering::SeqCst) {
        let (monitor_on, pattern, gaps) = monitor_wish(shared);
        let covered = mirror.as_ref().is_some_and(|m| m.display.is_some()) && monitor_on;
        let Some((_, looks, level)) = wanted(shared, covered) else { break };
        if brightness != Some(level) {
            device.set_brightness(level).await?;
            brightness = Some(level);
        }
        // Turning Mini Monitor on or off during an alignment starts afresh.
        if mirror.as_ref().is_some_and(|m| m.display.is_some() != monitor_on) || (!monitor_on && !pattern) {
            if mirror.take().is_some() {
                note("AKP03E: Mini Monitor off");
                set_status(shared, "Connected");
                shown.fill(None);
            }
        }
        if (monitor_on || pattern) && mirror.is_none() {
            match Mirror::start(monitor_on, gaps) {
                Ok(started) => {
                    note(&format!("AKP03E: picture on, {} tiles", started.layout.tiles.len()));
                    if let Some(display) = &started.display {
                        set_status(shared, &format!("Mini Monitor on {}", display.name()));
                    }
                    mirror = Some(started);
                }
                Err(problem) => {
                    note(&format!("AKP03E: Mini Monitor couldn't start: {problem}"));
                    lock(&shared.cfg).mini_monitor = false;
                    shared.save_now();
                    shared.set_status(problem);
                }
            }
        }
        if let Some(active) = mirror.as_mut() {
            if active.gaps != gaps {
                active.arrange(gaps);
            }
            let sent = active.show_frame(&device, pattern).await?;
            still = if sent == 0 { still.saturating_add(1) } else { 0 };
        } else {
            let mut changed = 0;
            for (key, look) in looks.iter().enumerate() {
                if shown[key].as_ref() != Some(look) {
                    device.write_image(key as u8, &picture(look)?).await?;
                    shown[key] = Some(look.clone());
                    changed += 1;
                }
            }
            if changed > 0 {
                device.flush().await?;
                note(&format!("AKP03E: updated {changed} keys"));
            }
        }
        // A still screen is looked at less often; presses still come at once.
        let wait = if mirror.is_some() && still <= crate::pad::STILL_FRAMES * 2 { 20 } else { 100 };
        reader.read(Some(Duration::from_millis(wait))).await?;
        for code in std::mem::take(&mut *lock(&CODES)) {
            if mirror.is_some() {
                leave_monitor(shared);
            } else {
                act(shared, code);
            }
        }
        if kept_alive.elapsed() > KEEP_ALIVE {
            device.keep_alive().await?;
            kept_alive = Instant::now();
        }
    }
    device.clear_all_button_images().await?;
    device.flush().await
}

async fn run(shared: Arc<Shared>) {
    while !QUIT.load(Ordering::SeqCst) {
        let found = list_devices(&[QUERY]).await.ok().and_then(|d| d.into_iter().next());
        if PLUGGED_IN.swap(found.is_some(), Ordering::SeqCst) != found.is_some() {
            shared.touch();
        }
        let in_use = lock(&shared.cfg).small_pad_index().is_some();
        if !in_use {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if original_is_running() {
            set_status(&shared, "Left alone while the StreamDock app is open");
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let Some(found) = found else {
            set_status(&shared, "Not plugged in");
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        };
        let result = match Device::connect(&found, PROTOCOL, POSITIONS, KNOBS).await {
            Ok(device) => drive(device, &shared).await,
            Err(e) => Err(e),
        };
        CONNECTED.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            set_status(&shared, &format!("Lost the connection: {e}"));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Starts watching for the pad, and runs it once a profile uses it.
pub fn start(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        // The pad library needs more than one worker to get an answer back.
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_time().build();
        if let Ok(runtime) = runtime {
            runtime.block_on(run(shared));
        }
    });
}

/// For the check that runs without clicks: waits until the pad has its pictures.
#[cfg(feature = "shots")]
pub fn wait_for_pad() {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !CONNECTED.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(1200));
}

/// Before the app ends: blanks the pad's keys, as the main pad does.
pub fn finish() {
    QUIT.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_millis(1500);
    while CONNECTED.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The codes seen on the real pad, with what the person was touching.
    #[test]
    fn every_code_seen_on_the_real_pad_reaches_its_own_cell() {
        let seen = [
            (1, 0), (2, 1), (3, 2), (4, 6), (5, 7), (6, 8),
            (0x25, 12), (0x30, 13), (0x31, 14),
            (0x90, 3), (0x91, 9), (0x50, 4), (0x51, 10), (0x60, 5), (0x61, 11),
            (0x33, 15), (0x35, 16), (0x34, 17),
        ];
        for (code, cell) in seen {
            assert_eq!(cell_of_code(code), Some(cell), "code {code:#04x}");
        }
    }

    #[test]
    fn all_eighteen_controls_have_a_cell_of_their_own() {
        let codes = [1, 2, 3, 4, 5, 6, 0x25, 0x30, 0x31, 0x90, 0x91, 0x50, 0x51, 0x60, 0x61, 0x33, 0x34, 0x35];
        let mut cells: Vec<usize> = codes.iter().filter_map(|code| cell_of_code(*code)).collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells, (0..18).collect::<Vec<_>>());
    }

    #[test]
    fn unknown_codes_do_nothing() {
        assert_eq!(cell_of_code(0), None);
        assert_eq!(cell_of_code(7), None);
        assert_eq!(cell_of_code(0x77), None);
    }

    #[test]
    fn pictures_are_the_size_this_pad_takes() {
        let data = picture(&None).unwrap();
        assert_eq!(image::load_from_memory(&data).unwrap().to_rgb8().dimensions(), (60, 60));
    }
}

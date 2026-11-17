//! Mini Monitor mode: Windows sees the pad as one more display.
//!
//! Windows only lists a display that a driver presents, so this uses the
//! Parsec Virtual Display driver, which is signed and needs no administrator
//! rights. How to talk to it comes from the open-source parsec-vdd project.
//! The driver unplugs a display about a second after it stops hearing from
//! us, so a crash can't leave a phantom monitor behind.

use crate::model::{lock, Config, MonitorArea, PadKind, Shared};
use std::sync::Mutex;
use crate::win::{from_wide, wide};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, POINT,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED,
    FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, ICONINFO,
};

mod capture;
mod driver;
mod layout;
pub use capture::*;
pub use driver::*;
pub use layout::*;

/// The latest Mini Monitor picture a pad got, so its picture on screen can
/// show the same.
pub struct Shown {
    pub layout: Layout,
    pub canvas: Arc<Vec<u8>>,
    /// Goes up with each new picture.
    pub version: u64,
    /// Where on the desktop the picture was taken; none for the test picture.
    pub from: Option<Area>,
}

impl Shown {
    /// The desktop point a point of the picture shows, or None on the bars
    /// round the display or on the test picture.
    pub fn desktop_point(&self, x: f32, y: f32) -> Option<(i32, i32)> {
        desktop_point(self.from?, (self.layout.width, self.layout.height), x, y)
    }
}

/// Like `desktop_point`, but always a point of the display: a point on the
/// bars round it, or beyond the picture, goes to the nearest edge, and the
/// result stays `inset` pixels inside the display.
pub fn desktop_point_clamped(from: Area, size: (i32, i32), x: f32, y: f32, inset: i32) -> (i32, i32) {
    let (left, top, width, height) = fit(from, size.0, size.1);
    let across = ((x - left as f32) / width as f32).clamp(0.0, 1.0);
    let down = ((y - top as f32) / height as f32).clamp(0.0, 1.0);
    let inset = inset.min(from.width / 2 - 1).min(from.height / 2 - 1).max(0);
    (
        (from.x + (across * from.width as f32) as i32).clamp(from.x + inset, from.x + from.width - 1 - inset),
        (from.y + (down * from.height as f32) as i32).clamp(from.y + inset, from.y + from.height - 1 - inset),
    )
}

/// The desktop point under a point of a picture `size` big, taken of the
/// display at `from`, or None on the bars round the display.
pub fn desktop_point(from: Area, size: (i32, i32), x: f32, y: f32) -> Option<(i32, i32)> {
    let (left, top, width, height) = fit(from, size.0, size.1);
    let across = (x - left as f32) / width as f32;
    let down = (y - top as f32) / height as f32;
    if !(0.0..1.0).contains(&across) || !(0.0..1.0).contains(&down) {
        return None;
    }
    Some((from.x + (across * from.width as f32) as i32, from.y + (down * from.height as f32) as i32))
}

/// Each pad's latest picture: its layout, the picture, its version, where on
/// the desktop it came from, and when it was taken.
type Kept = (Layout, Arc<Vec<u8>>, u64, Option<Area>, Instant);
static SHOWN: Mutex<[Option<Kept>; 2]> = Mutex::new([None, None]);

/// A picture this recent is used again rather than taken again: the pad and
/// its picture on screen share one capture when both want one.
const STILL_FRESH: Duration = Duration::from_millis(30);

/// Where a pad's picture is kept. A virtual pad never shows Mini Monitor.
fn slot(kind: PadKind) -> Option<usize> {
    match kind {
        PadKind::Main => Some(0),
        PadKind::Akp03e => Some(1),
        PadKind::Dashboard => None,
    }
}

/// Keeps the picture a pad was just given, laid out as on that pad.
pub fn publish(kind: PadKind, layout: &Layout, canvas: Vec<u8>, from: Option<Area>) {
    publish_shared(kind, layout, Arc::new(canvas), from);
}

fn publish_shared(kind: PadKind, layout: &Layout, canvas: Arc<Vec<u8>>, from: Option<Area>) {
    let Some(slot) = slot(kind) else { return };
    let mut shown = lock(&SHOWN);
    let version = shown[slot].as_ref().map_or(0, |kept| kept.2) + 1;
    shown[slot] = Some((layout.clone(), canvas, version, from, Instant::now()));
}

/// A picture of the display laid out for a pad, taken now unless one was
/// just taken for the same pad, layout and display. Kept for the pad's
/// picture on screen and dashboard cards too.
pub fn capture(kind: PadKind, layout: &Layout, area: Area) -> Option<Arc<Vec<u8>>> {
    if let Some(slot) = slot(kind) {
        if let Some((kept_layout, canvas, _, from, at)) = lock(&SHOWN)[slot].as_ref() {
            if kept_layout == layout && *from == Some(area) && at.elapsed() < STILL_FRESH {
                return Some(canvas.clone());
            }
        }
    }
    let canvas = Arc::new(grab(area, layout.width, layout.height)?);
    publish_shared(kind, layout, canvas.clone(), Some(area));
    Some(canvas)
}

/// The latest Mini Monitor picture for a pad, if there is one.
pub fn shown(kind: PadKind) -> Option<Shown> {
    lock(&SHOWN)[slot(kind)?].as_ref().map(|(layout, canvas, version, from, _)| Shown {
        layout: layout.clone(),
        canvas: canvas.clone(),
        version: *version,
        from: *from,
    })
}

/// What a click on a pad on screen's Mini Monitor does on the display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DisplayMouse {
    Left,
    Right,
    Middle,
    /// Turns the wheel; 120 is one notch, up when above zero.
    Wheel(i32),
}

/// A desktop point in the 0 to 65535 scale Windows takes for a mouse moved to
/// an exact place: `x × 65535 ÷ (width − 1)` rounded down, over the whole
/// desktop rather than the main screen alone, since the extra display is never
/// the main one.
pub fn absolute(point: (i32, i32), desktop: Area) -> (i32, i32) {
    let scale = |at: i32, start: i32, size: i32| ((at - start) as i64 * 0xffff / (size - 1).max(1) as i64) as i32;
    (scale(point.0, desktop.x, desktop.width), scale(point.1, desktop.y, desktop.height))
}

/// The whole desktop, every screen together, as Windows places them.
pub fn desktop() -> Area {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    unsafe {
        Area {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

/// The buttons of a click, sent down and up in the same input as the move to
/// the point (0x7 left, 0x19 right).
fn click_flags(what: DisplayMouse) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL,
    };
    match what {
        DisplayMouse::Left => MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_LEFTUP,
        DisplayMouse::Right => MOUSEEVENTF_RIGHTDOWN | MOUSEEVENTF_RIGHTUP,
        DisplayMouse::Middle => MOUSEEVENTF_MIDDLEDOWN | MOUSEEVENTF_MIDDLEUP,
        DisplayMouse::Wheel(_) => MOUSEEVENTF_WHEEL,
    }
}

/// Clicks or scrolls at a point of the extra display, as a mouse there would,
/// then puts the pointer back where it was, so it stays on the pad on screen.
/// It's one batch of input, so nothing lands between the steps.
pub fn mouse_on_display(at: (i32, i32), what: DisplayMouse) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let desktop = desktop();
        let mut before = POINT { x: 0, y: 0 };
        GetCursorPos(&mut before);
        let event = |point: (i32, i32), flags: u32, data: i32| {
            let (dx, dy) = absolute(point, desktop);
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx,
                        dy,
                        mouseData: data as u32,
                        dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        };
        let back = (before.x, before.y);
        // The wheel goes to whatever is under the pointer, so it moves there first.
        let data = if let DisplayMouse::Wheel(notches) = what { notches } else { 0 };
        let inputs = [event(at, click_flags(what), data), event(back, 0, 0)];
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// How a pad's Mini Monitor picture is cut up, from the settings.
pub fn layout_for(cfg: &Config, kind: PadKind) -> Layout {
    let [across, down] = cfg.gaps(kind);
    let gaps = cfg.monitor_gaps.then_some(Gaps { across, down });
    match kind {
        // A virtual pad never shows Mini Monitor; if asked, it's cut as the 293S's keys.
        PadKind::Main | PadKind::Dashboard => Layout::of(if kind == PadKind::Main { cfg.monitor_area } else { MonitorArea::KeysOnly }, gaps),
        PadKind::Akp03e => Layout::small(gaps),
    }
}

/// The extra display a real pad is showing, by pad, so its picture on screen
/// can capture the same display at its own pace.
static LIVE_DISPLAY: Mutex<[Option<String>; 2]> = Mutex::new([None, None]);

/// Notes that a real pad is showing this display.
pub fn set_live_display(kind: PadKind, display: &VirtualDisplay) {
    if let Some(slot) = slot(kind) {
        lock(&LIVE_DISPLAY)[slot] = Some(display.name().to_string());
    }
}

/// How often a pad on screen gets a new Mini Monitor picture: about 30 a
/// second. A real pad takes far fewer, since each key costs it about 30 ms,
/// but a window has no such limit.
const ON_SCREEN_EVERY: Duration = Duration::from_millis(33);

/// Whether the real pad is there to run Mini Monitor itself.
fn real_pad_running(shared: &Shared, kind: PadKind) -> bool {
    match kind {
        PadKind::Main => lock(&shared.pad_status).is_working(),
        PadKind::Akp03e => crate::small_pad::is_connected(),
        PadKind::Dashboard => false,
    }
}

/// Mini Monitor for pads on screen. Without the real pad it adds the extra
/// display itself; with the real pad it captures the display the pad is
/// showing. Either way it captures at its own pace, not the pad's.
pub fn start_on_screen(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        // The pad it's for, whether the display is there, and the display.
        let mut running: Option<(PadKind, bool, Option<VirtualDisplay>)> = None;
        loop {
            let wish = {
                let cfg = lock(&shared.cfg);
                // Pads on screen, and pads dashboard cards are showing. Cards
                // take only a few pictures a second, so for them alone the
                // display is captured at their pace.
                let kinds = |ids: Vec<String>| -> Vec<PadKind> { ids.iter().filter_map(|id| cfg.pad(id)).map(|entry| entry.kind).collect() };
                let on_screen = kinds(lock(&shared.on_screen).clone());
                let on_cards = kinds(shared.bridge.watched());
                let pattern = |kind: PadKind| cfg.monitor_pad == kind && shared.alignment_pattern.load(Ordering::SeqCst);
                let cards_every = Duration::from_millis(1000 / cfg.ha_monitor_fps.clamp(1, 15) as u64);
                PadKind::HARDWARE
                    .into_iter()
                    .find(|kind| (on_screen.contains(kind) || on_cards.contains(kind)) && (cfg.monitor_on(*kind) || pattern(*kind)))
                    .map(|kind| {
                        let every = if on_screen.contains(&kind) { ON_SCREEN_EVERY } else { cards_every };
                        (kind, cfg.monitor_on(kind), layout_for(&cfg, kind), every)
                    })
            };
            let Some((kind, with_display, layout, every)) = wish else {
                running = None;
                std::thread::sleep(Duration::from_millis(300));
                continue;
            };
            if real_pad_running(&shared, kind) {
                // The real pad has the display: take pictures of the same one.
                running = None;
                let live = slot(kind).and_then(|slot| lock(&LIVE_DISPLAY)[slot].clone());
                let area = live.filter(|_| with_display).and_then(|name| display_area(&name));
                if with_display {
                    // Shared with the pad, which may have just taken one.
                    if area.and_then(|area| capture(kind, &layout, area)).is_some() {
                        shared.repaint();
                    }
                } else {
                    publish(kind, &layout, layout.alignment_pattern(), None);
                    shared.repaint();
                }
                std::thread::sleep(every);
                continue;
            }
            if running.as_ref().is_none_or(|(was, had, _)| *was != kind || *had != with_display) {
                running = None;
                let display = if with_display {
                    match VirtualDisplay::add() {
                        Ok(display) => Some(display),
                        Err(problem) => {
                            lock(&shared.cfg).mini_monitor = false;
                            shared.save_now();
                            shared.set_status(problem);
                            continue;
                        }
                    }
                } else {
                    None
                };
                running = Some((kind, with_display, display));
            }
            let area = running.as_ref().and_then(|(_, _, display)| display.as_ref()).and_then(VirtualDisplay::area);
            let canvas = match (running.as_ref().and_then(|(_, _, display)| display.as_ref()), area) {
                (Some(_), Some(area)) => grab(area, layout.width, layout.height),
                (Some(_), None) => None,
                (None, _) => Some(layout.alignment_pattern()),
            };
            if let Some(canvas) = canvas {
                publish(kind, &layout, canvas, area);
                shared.repaint();
            }
            std::thread::sleep(every);
        }
    });
}

/// Screens never repeat exactly once video is involved, and sending a tile
/// costs the pad about 30 ms, so changes nobody can see don't count.
pub fn visibly_changed(old: &[u8], new: &[u8]) -> bool {
    old.len() != new.len() || old.iter().zip(new).any(|(a, b)| a.abs_diff(*b) > 6)
}

pub fn darken(pixels: &mut [u8], percent: u8) {
    let keep = 100 - percent.min(100) as u32;
    for value in pixels {
        *value = (*value as u32 * keep / 100) as u8;
    }
}

/// Turns a tile the way the pad needs it and encodes it. 4:2:0 colour takes
/// the pad about 11% less time to show than full colour, measured on a 293S.
pub fn encode_tile(pixels: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    let mut turned = vec![0u8; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let from = (y * width + x) * 3;
            let to = ((width - 1 - x) * height + y) * 3;
            turned[to..to + 3].copy_from_slice(&pixels[from..from + 3]);
        }
    }
    rgb_jpeg(&turned, height, width, 85, SamplingFactor::R_4_2_0)
}

/// RGB pixels, `width` × `height`, as a JPEG. The one encoder for every
/// picture that isn't a pad key's, which keep the usual quality steps.
pub fn rgb_jpeg(pixels: &[u8], width: usize, height: usize, quality: u8, sampling: SamplingFactor) -> Option<Vec<u8>> {
    let (width, height) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
    if pixels.len() != width as usize * height as usize * 3 {
        return None;
    }
    let mut data = Vec::with_capacity(4096);
    let mut encoder = Encoder::new(&mut data, quality);
    encoder.set_sampling_factor(sampling);
    encoder.encode(pixels, width, height, ColorType::Rgb).ok()?;
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_of_the_picture_is_the_same_point_of_the_display() {
        // A 1280 × 720 display to the right of a 1920-wide one, shown in a
        // 425 × 255 picture: it fits 425 across, with bars above and below.
        let layout = Layout::of(MonitorArea::KeysOnly, None);
        let from = Area { x: 1920, y: 0, width: 1280, height: 720 };
        let shown = Shown { layout, canvas: Arc::new(Vec::new()), version: 1, from: Some(from) };
        let (_, top, _, height) = fit(from, 425, 255);
        assert_eq!(shown.desktop_point(0.0, top as f32), Some((1920, 0)));
        let middle = shown.desktop_point(212.5, top as f32 + height as f32 / 2.0).unwrap();
        assert!((middle.0 - 2560).abs() <= 2 && (middle.1 - 360).abs() <= 2, "{middle:?}");
        // The bars round the display aren't part of it.
        assert_eq!(shown.desktop_point(100.0, top as f32 - 1.0), None);
        assert_eq!(shown.desktop_point(425.0, 100.0), None);
        // Nor is the test picture, which shows no display.
        let pattern = Shown { from: None, ..shown };
        assert_eq!(pattern.desktop_point(100.0, 100.0), None);
    }

    #[test]
    fn the_mouse_goes_to_exact_points_across_all_displays() {
        let desktop = Area { x: -1920, y: 0, width: 3840, height: 1080 };
        // The flags for a click: 0x7 left, 0x19 right, both with the move.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_MOVE;
        assert_eq!(click_flags(DisplayMouse::Left) | MOUSEEVENTF_MOVE, 0x7);
        assert_eq!(click_flags(DisplayMouse::Right) | MOUSEEVENTF_MOVE, 0x19);
        assert_eq!(click_flags(DisplayMouse::Middle) | MOUSEEVENTF_MOVE, 0x61);
        assert_eq!(absolute((-1920, 0), desktop), (0, 0));
        assert_eq!(absolute((1919, 1079), desktop), (65535, 65535));
        // Windows turns the scale back into the same pixel.
        for x in [-1920, -1, 0, 640, 1919] {
            let (scaled, _) = absolute((x, 0), desktop);
            let back = desktop.x + (scaled as i64 * (desktop.width - 1) as i64 / 65535) as i32;
            assert!((back - x).abs() <= 1, "{x} came back as {back}");
        }
    }

    #[test]
    fn keys_only_covers_the_whole_picture_without_overlap() {
        let layout = Layout::of(MonitorArea::KeysOnly, None);
        let mut seen = vec![0u8; (layout.width * layout.height) as usize];
        assert_eq!(layout.tiles.len(), 15);
        for tile in &layout.tiles {
            for y in tile.y..tile.y + tile.height {
                for x in tile.x..tile.x + tile.width {
                    seen[(y * layout.width + x) as usize] += 1;
                }
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn with_gaps_the_keys_sit_apart_and_stay_inside_the_picture() {
        let gaps = Some(Gaps { across: 52, down: 56 });
        for area in [MonitorArea::KeysOnly, MonitorArea::KeysAndBand] {
            let layout = Layout::of(area, gaps);
            for tile in &layout.tiles {
                assert!(tile.x + tile.width <= layout.width && tile.y + tile.height <= layout.height);
                assert!(!layout.crop(&vec![0; (layout.width * layout.height * 3) as usize], tile).is_empty());
            }
        }
        let layout = Layout::of(MonitorArea::KeysOnly, gaps);
        let second = layout.tiles.iter().find(|t| t.column == 1 && t.row == 0).unwrap();
        assert_eq!(second.x, 85 + 44);
        let below = layout.tiles.iter().find(|t| t.column == 0 && t.row == 1).unwrap();
        assert_eq!(below.y, 85 + 47);
    }

    #[test]
    fn with_these_gaps_the_keys_are_close_to_widescreen() {
        let layout = Layout::of(MonitorArea::KeysOnly, Some(Gaps { across: 52, down: 56 }));
        let shape = layout.width as f32 / layout.height as f32;
        assert!((shape - 16.0 / 9.0).abs() < 0.07, "shape {shape}");
    }

    #[test]
    fn a_display_keeps_its_shape_in_the_picture() {
        let area = Area { x: 0, y: 0, width: 1280, height: 720 };
        assert_eq!(fit(area, 640, 360), (0, 0, 640, 360));
        let (left, top, width, height) = fit(area, 425, 255);
        assert_eq!((left, width), (0, 425));
        assert_eq!(height, 239);
        assert_eq!(top, 8);
    }

    #[test]
    fn the_alignment_pattern_fills_the_picture() {
        let layout = Layout::of(MonitorArea::KeysOnly, Some(Gaps { across: 52, down: 56 }));
        assert_eq!(layout.alignment_pattern().len(), (layout.width * layout.height * 3) as usize);
    }

    #[test]
    fn keys_and_band_adds_the_three_band_tiles() {
        let layout = Layout::of(MonitorArea::KeysAndBand, None);
        assert_eq!(layout.tiles.len(), 18);
        assert_eq!(layout.tiles.iter().filter(|t| t.is_band()).count(), 3);
        assert!(layout.tiles.iter().filter(|t| t.is_band()).all(|t| t.width == 80 && t.height == 80));
    }

    #[test]
    fn tiles_turn_the_same_way_the_keys_do() {
        let width = 3u32;
        let pixels: Vec<u8> = (0..width * 2 * 3).map(|v| v as u8).collect();
        let picture = image::RgbImage::from_raw(width, 2, pixels.clone()).unwrap();
        let expected = image::imageops::rotate270(&picture);
        let mut turned = vec![0u8; pixels.len()];
        for y in 0..2usize {
            for x in 0..3usize {
                let from = (y * 3 + x) * 3;
                let to = ((3 - 1 - x) * 2 + y) * 3;
                turned[to..to + 3].copy_from_slice(&pixels[from..from + 3]);
            }
        }
        assert_eq!(turned, expected.into_raw());
    }

    #[test]
    fn encoded_tiles_are_real_pictures_of_the_right_size() {
        let data = encode_tile(&vec![120u8; 85 * 85 * 3], 85, 85).unwrap();
        assert_eq!(image::load_from_memory(&data).unwrap().to_rgb8().dimensions(), (85, 85));
        let data = encode_tile(&vec![120u8; 80 * 80 * 3], 80, 80).unwrap();
        assert_eq!(image::load_from_memory(&data).unwrap().to_rgb8().dimensions(), (80, 80));
    }

    #[test]
    fn tiny_changes_do_not_count() {
        assert!(!visibly_changed(&[100, 100, 100], &[104, 96, 100]));
        assert!(visibly_changed(&[100, 100, 100], &[100, 120, 100]));
        assert!(visibly_changed(&[], &[1, 2, 3]));
    }

    #[test]
    fn darkening_keeps_the_right_share() {
        let mut pixels = [200u8, 100, 0];
        darken(&mut pixels, 25);
        assert_eq!(pixels, [150, 75, 0]);
    }

    #[test]
    fn a_tile_is_cut_from_the_right_place() {
        let layout = Layout::of(MonitorArea::KeysOnly, None);
        let mut canvas = vec![0u8; (layout.width * layout.height * 3) as usize];
        let start = ((85 * layout.width + 170) * 3) as usize;
        canvas[start] = 200; // top left corner of the key at row 1, column 2
        let tile = layout.tiles.iter().find(|t| t.row == 1 && t.column == 2).unwrap();
        let cut = layout.crop(&canvas, tile);
        assert_eq!(cut.len(), 85 * 85 * 3);
        assert_eq!(cut[0], 200);
    }

    #[test]
    fn remove_index_is_sent_high_byte_first() {
        assert_eq!(remove_data(1), [0, 1]);
    }

    /// Only asks the driver if it's there; nothing is added.
    /// Captures part of the main screen twice, so the second reuses the
    /// kept surfaces. Nothing is saved or shown.
    #[test]
    #[ignore = "reads the screen"]
    fn a_kept_capture_gives_whole_pictures_with_black_bars() {
        let area = Area { x: 0, y: 0, width: 200, height: 100 };
        for _ in 0..2 {
            let canvas = grab(area, 100, 100).expect("a picture");
            assert_eq!(canvas.len(), 100 * 100 * 3);
            // A wide area in a square picture: bars above and below.
            assert!(canvas[..100 * 20 * 3].iter().all(|value| *value == 0));
            assert!(canvas[100 * 80 * 3..].iter().all(|value| *value == 0));
        }
        // Another size makes new surfaces.
        assert_eq!(grab(area, 60, 30).expect("a picture").len(), 60 * 30 * 3);
    }

    #[test]
    #[ignore = "needs the Parsec driver"]
    fn driver_can_be_reached() {
        let driver = open_driver().expect("the driver should open without administrator rights");
        let version = control(driver, 0x0022e010, &[]);
        unsafe { CloseHandle(driver) };
        println!("driver minor version: {version:?}");
        assert!(version.is_some());
    }
}

//! Live pictures from Home Assistant cameras, for keys that show one.
//!
//! A pad takes about 30 ms per key, so true video isn't possible: the camera's
//! latest still picture is fetched about once a second, only while a pad is on
//! a page that shows it. Pictures are kept in memory and never saved.

use crate::ha;
use crate::model::{lock, Config, PictureFit, Shared};
use crate::pad::note;
use crate::render::{cut_area, CameraPart, KEY_SIZE};
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ureq::tls::{TlsConfig, TlsProvider};

const REFRESH: Duration = Duration::from_secs(1);
const WAIT_FOR_CAMERA: Duration = Duration::from_secs(5);
/// Enough for a camera stretched over every key of a pad.
const LONGEST_SIDE: u32 = 960;
const MOST_BYTES: u64 = 16 * 1024 * 1024;
/// More key areas than any pad shows at once, so the scaled pictures stay few.
const MOST_AREAS: usize = 16;

struct Frame {
    version: u64,
    picture: Arc<RgbImage>,
    /// Whether a new one keeps coming, as from a camera.
    moving: bool,
}

static FRAMES: Mutex<BTreeMap<String, Frame>> = Mutex::new(BTreeMap::new());
/// What decides how a picture is laid out over a key area, and its keys' size.
type AreaKey = (String, [u8; 2], u8, [u8; 2], (u8, u8), [u8; 3], u32);

/// The latest picture scaled for a key area, so its keys share the work.
static SCALED: Mutex<BTreeMap<AreaKey, (u64, Arc<RgbImage>)>> = Mutex::new(BTreeMap::new());

/// Goes up with each new picture, so the keys showing it know to redraw.
pub fn version(entity: &str) -> u64 {
    lock(&FRAMES).get(entity).map_or(0, |frame| frame.version)
}

/// A camera's latest picture, as it came.
pub fn picture(entity: &str) -> Option<Arc<RgbImage>> {
    lock(&FRAMES).get(entity).map(|frame| frame.picture.clone())
}

/// Whether pictures under this name keep changing, so keys showing them get
/// the pads' smaller limit for moving pictures. Cameras do; a still alert or
/// flash picture doesn't.
pub fn is_moving(entity: &str) -> bool {
    lock(&FRAMES).get(entity).map_or_else(|| entity.starts_with("camera."), |frame| frame.moving)
}

/// Keeps a camera's latest picture.
pub fn store(entity: &str, picture: RgbImage) {
    store_as(entity, picture, entity.starts_with("camera."));
}

/// Keeps a picture, saying whether it keeps changing.
pub fn store_as(entity: &str, picture: RgbImage, moving: bool) {
    let picture = if picture.width().max(picture.height()) > LONGEST_SIDE {
        DynamicImage::ImageRgb8(picture).resize(LONGEST_SIDE, LONGEST_SIDE, FilterType::Triangle).to_rgb8()
    } else {
        picture
    };
    // One count for every picture: a name that was forgotten and comes back,
    // such as the next alert on a pad, never repeats an earlier version, which
    // would bring back that picture from the caches keyed by version.
    static NEXT_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let version = NEXT_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut frames = lock(&FRAMES);
    frames.insert(entity.to_string(), Frame { version, picture: Arc::new(picture), moving });
}

/// Drops a picture no longer shown, such as a flash's once it's gone.
pub fn forget(entity: &str) {
    lock(&FRAMES).remove(entity);
    lock(&SCALED).retain(|kept, _| kept.0 != entity);
}

/// Puts a picture into a key area the way the person chose, as for any picture.
fn laid_out(picture: &RgbImage, width: u32, height: u32, fit: PictureFit) -> RgbImage {
    let source = DynamicImage::ImageRgb8(picture.clone());
    let shown = crate::render::fit_picture(&source, width, height, fit).to_rgb8();
    let mut area = RgbImage::new(width, height);
    let (left, down) = ((width - shown.width()) as i64 / 2, (height - shown.height()) as i64 / 2);
    image::imageops::overlay(&mut area, &shown, left, down);
    area
}

/// The distance from one key to the next in the shared picture, across then
/// down, for keys `px` pixels across. `gaps` is a percentage of a key, and 0
/// runs the keys together.
fn steps(gaps: [u8; 2], px: u32) -> [u32; 2] {
    gaps.map(|gap| px + px * gap as u32 / 100)
}

fn scaled(camera: &CameraPart, px: u32) -> Option<Arc<RgbImage>> {
    let (version, picture) = {
        let frames = lock(&FRAMES);
        let frame = frames.get(&camera.entity)?;
        (frame.version, frame.picture.clone())
    };
    let mut cache = lock(&SCALED);
    let key = (
        camera.entity.clone(),
        camera.span,
        camera.fit as u8,
        camera.gaps,
        (camera.shape as u8, camera.corner),
        camera.outside,
        px,
    );
    if let Some((kept, scaled)) = cache.get(&key) {
        if *kept == version {
            return Some(scaled.clone());
        }
    }
    let [across, down] = steps(camera.gaps, px);
    let width = (camera.span[0] as u32 - 1) * across + px;
    let height = (camera.span[1] as u32 - 1) * down + px;
    let mut filled = laid_out(&picture, width, height, camera.fit);
    // Stretched, the key shape belongs to the whole picture, not to each key.
    if camera.span != [1, 1] {
        let corner = camera.corner as f32 * px as f32 / KEY_SIZE as f32;
        cut_area(&mut filled, camera.shape, corner, camera.outside);
    }
    let filled = Arc::new(filled);
    // Older pictures and settings no longer shown would only take up memory.
    cache.retain(|kept, (kept_version, _)| kept.0 != key.0 || *kept_version == version);
    if cache.len() >= MOST_AREAS {
        cache.clear();
    }
    cache.insert(key, (version, filled.clone()));
    Some(filled)
}

/// One key's slice of a camera's picture, laid out over the whole key area,
/// for a key `px` pixels across.
pub fn part(camera: &CameraPart, px: u32) -> Option<RgbImage> {
    let filled = scaled(camera, px)?;
    let [across, down] = steps(camera.gaps, px);
    let (x, y) = (camera.at[0] as u32 * across, camera.at[1] as u32 * down);
    Some(image::imageops::crop_imm(&*filled, x, y, px, px).to_image())
}

/// Web requests through Windows' own secure connections, as the Home Assistant
/// connection uses. Without naming them, an https address stops the app, and
/// without Windows' own trusted certificates Windows refuses some sites.
pub fn secure_agent(wait: Duration) -> ureq::Agent {
    let secure = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder().tls_config(secure).timeout_global(Some(wait)).build().new_agent()
}

/// One agent for every camera fetch, so its connections are kept and reused
/// instead of a new secure connection each second.
pub fn agent() -> ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| secure_agent(WAIT_FOR_CAMERA)).clone()
}

/// Fetches a camera's latest still picture.
pub fn fetch(address: &str, token: &str) -> Result<RgbImage, String> {
    fetch_with(address, Some(token))
}

/// Fetches a picture, with Home Assistant's token only when given one.
pub fn fetch_with(address: &str, token: Option<&str>) -> Result<RgbImage, String> {
    let request = agent().get(address);
    let request = match token {
        Some(token) => request.header("Authorization", format!("Bearer {token}")),
        None => request,
    };
    let mut reply = request.call().map_err(|e| e.to_string())?;
    let bytes = reply.body_mut().with_config().limit(MOST_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    Ok(image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgb8())
}

/// The cameras on the pages the pads are showing.
pub fn wanted(cfg: &Config) -> Vec<String> {
    let mut cameras = Vec::new();
    for profile in cfg.pads.iter().filter_map(|entry| cfg.pad_profile_of(&entry.id)).map(|i| &cfg.profiles[i]) {
        let keys = &profile.pages[profile.active_page].keys;
        cameras.extend(keys.iter().filter_map(|key| key.camera()).map(str::to_string));
    }
    cameras.sort();
    cameras.dedup();
    cameras
}

fn watch(shared: Arc<Shared>) {
    let mut last_problem = String::new();
    loop {
        let started = Instant::now();
        let mut cameras = wanted(&lock(&shared.cfg));
        // An alert or a flash can show a camera too.
        cameras.extend(crate::alerts::cameras(&shared));
        cameras.extend(crate::flashes::cameras(&shared));
        cameras.sort();
        cameras.dedup();
        // Pictures of cameras no longer shown would only take up memory.
        // Alert and flash pictures have names of their own and go with them.
        let shown = |name: &String| !name.starts_with("camera.") || cameras.binary_search(name).is_ok();
        lock(&FRAMES).retain(|name, _| shown(name));
        lock(&SCALED).retain(|kept, _| shown(&kept.0));
        if cameras.is_empty() {
            std::thread::sleep(REFRESH);
            continue;
        }
        let (base, token) = {
            let cfg = lock(&shared.cfg);
            (ha::rest_address(&cfg.ha_url), ha::unprotect(&cfg.ha_token).unwrap_or_default())
        };
        for camera in cameras.iter().filter(|_| !token.is_empty()) {
            match fetch(&format!("{base}/api/camera_proxy/{camera}"), &token) {
                Ok(picture) => {
                    store(camera, picture);
                    last_problem.clear();
                    crate::pad::wake();
                    shared.touch_soon();
                }
                Err(problem) => {
                    let problem = format!("Couldn't get a picture from {camera}: {problem}");
                    if problem != last_problem {
                        note(&problem);
                        shared.set_status(problem.clone());
                        last_problem = problem;
                    }
                }
            }
        }
        std::thread::sleep(REFRESH.saturating_sub(started.elapsed()));
    }
}

/// Starts watching for keys that show a camera.
pub fn start(shared: Arc<Shared>) {
    std::thread::spawn(move || watch(shared));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyShape;
    use std::io::{Read, Write};

    fn slice(entity: &str, at: [u8; 2], span: [u8; 2], fit: PictureFit, gaps: [u8; 2]) -> Option<RgbImage> {
        part(&camera(entity, at, span, fit, gaps, KeyShape::Square), KEY_SIZE)
    }

    fn camera(entity: &str, at: [u8; 2], span: [u8; 2], fit: PictureFit, gaps: [u8; 2], shape: KeyShape) -> CameraPart {
        let (entity, version) = (entity.to_string(), 1);
        CameraPart { entity, version, at, span, fit, gaps, shape, corner: 16, outside: [0, 0, 0] }
    }

    fn white_slice(entity: &str, at: [u8; 2], span: [u8; 2], shape: KeyShape) -> RgbImage {
        store(entity, RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255])));
        part(&camera(entity, at, span, PictureFit::Stretch, [0, 0], shape), KEY_SIZE).unwrap()
    }

    #[test]
    fn a_rounded_camera_only_rounds_the_corners_of_the_whole_picture() {
        let left = white_slice("camera.test_rounded", [0, 0], [2, 1], KeyShape::Rounded);
        let right = white_slice("camera.test_rounded", [1, 0], [2, 1], KeyShape::Rounded);
        let last = KEY_SIZE - 1;
        // The outer corners are cut...
        assert!(!lit(&left, 0, 0) && !lit(&left, 0, last));
        assert!(!lit(&right, last, 0) && !lit(&right, last, last));
        // ...and where the two keys meet, nothing is.
        assert!(lit(&left, last, 0) && lit(&left, last, last));
        assert!(lit(&right, 0, 0) && lit(&right, 0, last));
    }

    #[test]
    fn a_circle_camera_is_one_circle_across_its_keys() {
        let at = |at: [u8; 2]| white_slice("camera.test_circle", at, [2, 2], KeyShape::Circle);
        let last = KEY_SIZE - 1;
        // The middle of the circle is where the four keys meet.
        assert!(lit(&at([0, 0]), last, last) && lit(&at([1, 1]), 0, 0));
        assert!(lit(&at([1, 0]), 0, last) && lit(&at([0, 1]), last, 0));
        // Only the far corners fall outside it.
        assert!(!lit(&at([0, 0]), 0, 0) && !lit(&at([1, 1]), last, last));
        assert!(!lit(&at([1, 0]), last, 0) && !lit(&at([0, 1]), 0, last));
    }

    #[test]
    fn a_circle_over_a_row_of_keys_has_round_ends() {
        let first = white_slice("camera.test_pill", [0, 0], [3, 1], KeyShape::Circle);
        let middle = white_slice("camera.test_pill", [1, 0], [3, 1], KeyShape::Circle);
        let last = KEY_SIZE - 1;
        assert!(!lit(&first, 0, 0));
        assert!(lit(&middle, 0, 0) && lit(&middle, last, last));
    }

    #[test]
    fn a_square_camera_has_nothing_cut() {
        let left = white_slice("camera.test_square", [0, 0], [2, 1], KeyShape::Square);
        assert!(lit(&left, 0, 0));
    }

    #[test]
    fn outside_the_shape_takes_the_colour_of_the_pad() {
        store("camera.test_face", RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0])));
        let mut white_pad = camera("camera.test_face", [0, 0], [2, 1], PictureFit::Stretch, [0, 0], KeyShape::Rounded);
        white_pad.outside = [255, 255, 255];
        assert!(lit(&part(&white_pad, KEY_SIZE).unwrap(), 0, 0));
    }

    #[test]
    fn one_key_keeps_its_own_cut() {
        let alone = white_slice("camera.test_alone", [0, 0], [1, 1], KeyShape::Rounded);
        // The key's own shape is cut later, with everything else on the key.
        assert!(lit(&alone, 0, 0));
    }

    fn gradient(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 40])
        })
    }

    #[test]
    fn a_camera_stretched_over_keys_is_cut_into_its_slices() {
        store("camera.test_slices", gradient(1600, 900));
        let left = slice("camera.test_slices", [0, 0], [2, 1], PictureFit::Fill, [0, 0]).unwrap();
        let right = slice("camera.test_slices", [1, 0], [2, 1], PictureFit::Fill, [0, 0]).unwrap();
        assert_eq!(left.dimensions(), (KEY_SIZE, KEY_SIZE));
        // The picture runs on from one key to the next.
        let (end, start) = (left.get_pixel(KEY_SIZE - 1, 40).0[0] as i32, right.get_pixel(0, 40).0[0] as i32);
        assert!((end - start).abs() < 6, "{end} {start}");
        assert!(right.get_pixel(KEY_SIZE - 1, 40).0[0] > 200);
    }

    /// White, wider than a key, so it shows where each mode puts the picture.
    fn wide() -> RgbImage {
        RgbImage::from_pixel(330, 100, image::Rgb([255, 255, 255]))
    }

    fn lit(picture: &RgbImage, x: u32, y: u32) -> bool {
        picture.get_pixel(x, y).0[0] > 128
    }

    #[test]
    fn the_whole_picture_keeps_its_shape_with_bars() {
        let area = laid_out(&wide(), 85, 85, PictureFit::Fit);
        assert!(lit(&area, 42, 42) && lit(&area, 1, 42));
        assert!(!lit(&area, 42, 5) && !lit(&area, 42, 80));
    }

    #[test]
    fn fill_and_stretch_leave_no_bars() {
        for fit in [PictureFit::Fill, PictureFit::Stretch] {
            let area = laid_out(&wide(), 85, 85, fit);
            assert!(lit(&area, 42, 5) && lit(&area, 42, 80) && lit(&area, 1, 42), "{fit:?}");
        }
    }

    #[test]
    fn original_size_shows_the_middle_of_a_big_picture() {
        let mut picture = wide();
        for x in 0..330 {
            for y in 0..100 {
                if x < 150 {
                    picture.put_pixel(x, y, image::Rgb([0, 0, 0]));
                }
            }
        }
        let area = laid_out(&picture, 85, 85, PictureFit::Original);
        assert!(lit(&area, 80, 42) && !lit(&area, 2, 42));
    }

    #[test]
    fn a_picture_shorter_than_the_keys_stays_in_the_middle_of_them() {
        store("camera.test_bars", wide());
        let top = slice("camera.test_bars", [1, 0], [3, 2], PictureFit::Fit, [0, 0]).unwrap();
        let bottom = slice("camera.test_bars", [1, 1], [3, 2], PictureFit::Fit, [0, 0]).unwrap();
        assert!(!lit(&top, 42, 2) && lit(&top, 42, 84));
        assert!(lit(&bottom, 42, 2) && !lit(&bottom, 42, 84));
    }

    #[test]
    fn allowing_for_the_gaps_skips_what_falls_under_them() {
        store("camera.test_gaps", gradient(1600, 800));
        let edge = |at: [u8; 2], gaps, x| slice("camera.test_gaps", at, [2, 1], PictureFit::Stretch, gaps).unwrap().get_pixel(x, 40).0[0] as i32;
        let touching = edge([1, 0], [0, 0], 0) - edge([0, 0], [0, 0], KEY_SIZE - 1);
        let apart = edge([1, 0], [50, 50], 0) - edge([0, 0], [50, 50], KEY_SIZE - 1);
        assert!(touching.abs() < 6, "{touching}");
        // Half a key of picture is hidden, out of two and a half keys.
        assert!((35..=68).contains(&apart), "{apart}");
    }

    #[test]
    fn a_new_picture_changes_the_version() {
        let before = version("camera.test_versions");
        store("camera.test_versions", gradient(32, 32));
        assert!(version("camera.test_versions") > before);
    }

    /// The next alert on a pad must not reuse the last one's version, or the
    /// caches keyed by it would show the old picture.
    #[test]
    fn a_forgotten_picture_never_comes_back_with_an_old_version() {
        store("alert:test_versions", gradient(32, 32));
        let first = version("alert:test_versions");
        forget("alert:test_versions");
        store("alert:test_versions", gradient(16, 16));
        assert!(version("alert:test_versions") > first);
    }

    #[test]
    fn big_pictures_are_kept_smaller() {
        store("camera.test_big", gradient(3840, 2160));
        assert_eq!(lock(&FRAMES)["camera.test_big"].picture.width(), LONGEST_SIDE);
    }

    /// Without the right settings the request panics before it goes anywhere.
    #[test]
    fn secure_addresses_fail_politely_rather_than_stopping_the_app() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || drop(listener.accept()));
        assert!(fetch(&format!("https://{address}/api/camera_proxy/camera.door"), "token").is_err());
        server.join().unwrap();
    }

    #[test]
    fn nothing_shows_before_the_first_picture() {
        assert!(slice("camera.never_seen", [0, 0], [1, 1], PictureFit::Fit, [0, 0]).is_none());
        assert_eq!(version("camera.never_seen"), 0);
    }

    #[test]
    fn fetches_a_picture_with_the_token_and_reports_a_refusal() {
        let mut encoded = std::io::Cursor::new(Vec::new());
        gradient(16, 9).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        let body = encoded.into_inner();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let size = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..size]).to_lowercase();
                if request.contains("authorization: bearer good") {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    stream.write_all(head.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
            }
        });
        let url = format!("http://{address}/api/camera_proxy/camera.door");
        assert_eq!(fetch(&url, "good").unwrap().dimensions(), (16, 9));
        let refused = fetch(&url, "wrong").unwrap_err();
        assert!(refused.contains("401"), "{refused}");
        assert!(!refused.contains("wrong"), "{refused}");
        server.join().unwrap();
    }
}

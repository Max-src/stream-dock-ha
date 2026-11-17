//! Draws keys and band tiles. The window and the pad both use these same
//! pictures, so what the window shows is exactly what the pad gets.

use crate::band::{BandIcon, Face};
use crate::ha::{can_toggle, Availability, Entities};
use crate::icons;
use crate::model::{
    lock, text_color_of_state, Action, DockKey, KeyShape, KeyStyle, MediaKey, MeterStyle, PictureFit, TextPlace, MIC_ICON, MIC_OFF_ICON,
};
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::imageops::FilterType;
use image::{Rgb, RgbImage};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
};

mod text;
mod paint;
mod keys;
mod tiles;
mod alert;
mod graph;
pub use text::*;
pub use paint::*;
pub use keys::*;
pub use tiles::*;
pub use alert::*;
pub use graph::KeyGraph;
use graph::draw_key_graph;

pub const KEY_SIZE: u32 = 85;
pub const BAND_SIZE: u32 = 80;
const SMALLEST_TEXT: u32 = 8;
/// A band tile's own background, unless the person chose another.
pub const BAND_BACKGROUND: [u8; 3] = [18, 20, 26];
/// The empty part of a meter.
const METER_TRACK: [u8; 3] = [52, 57, 70];
/// The colour of a meter that has no levels.
const METER_PLAIN: [u8; 3] = [79, 140, 255];
/// The drawn switch's usual track.
pub const SWITCH_ON: [u8; 3] = [240, 180, 20];
pub const SWITCH_OFF: [u8; 3] = [92, 98, 112];

/// A picture sized for an area the way the person chose. It can come out
/// smaller than the area, and is then centred in it.
pub fn fit_picture(picture: &image::DynamicImage, width: u32, height: u32, fit: PictureFit) -> image::DynamicImage {
    match fit {
        PictureFit::Fit => picture.resize(width, height, FilterType::Triangle),
        PictureFit::Fill => picture.resize_to_fill(width, height, FilterType::Triangle),
        PictureFit::Stretch => picture.resize_exact(width, height, FilterType::Triangle),
        PictureFit::Original => {
            let (shown_x, shown_y) = (picture.width().min(width), picture.height().min(height));
            picture.crop_imm((picture.width() - shown_x) / 2, (picture.height() - shown_y) / 2, shown_x, shown_y)
        }
    }
}

#[cfg(test)]
mod tests;

//! A pad's firmware version: one HID "get input report" request on the pad's
//! vendor channel (usage page 0xFFA0), with the version as text from byte 1 of
//! the answer.
//!
//! It only reads, and sends no command to the pad. This runs only when the
//! person clicks for it, and never while the StreamDock app is open.

use crate::model::{lock, PadKind};
use std::sync::Mutex;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetInputReport, HidD_GetPreparsedData, HidP_GetCaps,
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};

/// The pads' vendor channel, where the version is read.
const VENDOR_USAGE_PAGE: u16 = 0xFFA0;

/// Each real pad's USB vendor and product. A virtual pad has no hardware.
pub fn usb_id(kind: PadKind) -> Option<(u16, u16)> {
    match kind {
        PadKind::Main => Some((0x5548, 0x6670)),
        PadKind::Akp03e => Some((0x0300, 0x1002)),
        PadKind::Dashboard => None,
    }
}

/// The version text in an input report: from byte 1 up to the first zero,
/// None if there's no readable text there.
pub fn version_from_report(report: &[u8]) -> Option<String> {
    let text = report.get(1..)?;
    let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
    let version: String = text[..end].iter().map(|b| *b as char).collect();
    let version = version.trim();
    (!version.is_empty() && version.chars().all(|c| c.is_ascii_graphic() || c == ' ')).then(|| version.to_string())
}

/// The last version read for each pad, or why it couldn't be: the 293S, then
/// the AKP03E.
static FOUND: Mutex<[Option<Result<String, String>>; 2]> = Mutex::new([None, None]);

fn slot(kind: PadKind) -> Option<usize> {
    match kind {
        PadKind::Main => Some(0),
        PadKind::Akp03e => Some(1),
        PadKind::Dashboard => None,
    }
}

pub fn found(kind: PadKind) -> Option<Result<String, String>> {
    lock(&FOUND)[slot(kind)?].clone()
}

/// Reads a pad's firmware version in the background, after the person asked.
pub fn read_in_background(kind: PadKind, done: impl FnOnce() + Send + 'static) {
    let (Some(slot), Some((vid, pid))) = (slot(kind), usb_id(kind)) else { return };
    lock(&FOUND)[slot] = Some(Err(READING.into()));
    std::thread::spawn(move || {
        let result = if crate::pad::original_is_running() {
            Err("Close the StreamDock app first".to_string())
        } else {
            read_version(vid, pid)
        };
        crate::pad::note(&format!("{}: firmware version {result:?}", kind.label()));
        lock(&FOUND)[slot] = Some(result);
        done();
    });
}

/// Shown while a read is under way.
pub const READING: &str = "Reading…";

/// Finds the pad's vendor channel and asks it for one input report.
pub fn read_version(vid: u16, pid: u16) -> Result<String, String> {
    unsafe {
        let mut guid = std::mem::zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1 || set == 0 {
            return Err("Windows couldn't list the USB devices".into());
        }
        let mut result = Err("The pad isn't plugged in".to_string());
        let mut interface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
        interface.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let mut index = 0;
        while SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, index, &mut interface) != 0 {
            index += 1;
            let mut size = 0u32;
            SetupDiGetDeviceInterfaceDetailW(set, &interface, std::ptr::null_mut(), 0, &mut size, std::ptr::null_mut());
            let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &interface, detail, size, &mut size, std::ptr::null_mut()) == 0 {
                continue;
            }
            let path = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            if let Some(answer) = ask(path, vid, pid) {
                result = answer;
                break;
            }
        }
        SetupDiDestroyDeviceInfoList(set);
        result
    }
}

/// One HID interface: None if it isn't the pad's vendor channel, otherwise
/// the version or why it couldn't be read.
unsafe fn ask(path: *const u16, vid: u16, pid: u16) -> Option<Result<String, String>> {
    let handle = CreateFileW(
        path,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        std::ptr::null(),
        OPEN_EXISTING,
        0,
        std::ptr::null_mut(),
    );
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let answer = (|| {
        let mut attributes: HIDD_ATTRIBUTES = std::mem::zeroed();
        attributes.Size = std::mem::size_of::<HIDD_ATTRIBUTES>() as u32;
        if !HidD_GetAttributes(handle, &mut attributes) || attributes.VendorID != vid || attributes.ProductID != pid {
            return None;
        }
        let mut preparsed = 0;
        if !HidD_GetPreparsedData(handle, &mut preparsed) {
            return None;
        }
        let mut caps: HIDP_CAPS = std::mem::zeroed();
        let status = HidP_GetCaps(preparsed, &mut caps);
        HidD_FreePreparsedData(preparsed);
        if status != HIDP_STATUS_SUCCESS || caps.UsagePage != VENDOR_USAGE_PAGE {
            return None;
        }
        // Report 0, the whole input report.
        let mut report = vec![0u8; (caps.InputReportByteLength as usize).max(2)];
        if !HidD_GetInputReport(handle, report.as_mut_ptr().cast(), report.len() as u32) {
            return Some(Err("The pad didn't answer".to_string()));
        }
        Some(version_from_report(&report).ok_or_else(|| "The pad's answer had no version in it".to_string()))
    })();
    CloseHandle(handle);
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_the_text_after_the_report_number() {
        let mut report = vec![0u8; 513];
        report[1..13].copy_from_slice(b"V1.293.001Z ");
        assert_eq!(version_from_report(&report).as_deref(), Some("V1.293.001Z"));
        // Nothing, or something that isn't text, is no version.
        assert_eq!(version_from_report(&[0u8; 513]), None);
        assert_eq!(version_from_report(&[0, 0xFF, 0x01, 0]), None);
        assert_eq!(version_from_report(&[0]), None);
    }

    #[test]
    fn each_pad_has_its_usb_id() {
        assert_eq!(usb_id(PadKind::Main), Some((0x5548, 0x6670)));
        assert_eq!(usb_id(PadKind::Akp03e), Some((0x0300, 0x1002)));
        assert_eq!(usb_id(PadKind::Dashboard), None);
        assert_eq!(found(PadKind::Dashboard), None);
    }
}

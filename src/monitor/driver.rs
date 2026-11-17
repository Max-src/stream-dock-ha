//! The Parsec Virtual Display driver: adding, sizing and removing the
//! extra display, and finding where Windows put it.

use super::*;

pub(super) const ADAPTER: GUID = GUID::from_u128(0x00b41627_04c4_429e_a26e_0265cf50c8fa);
pub(super) const ADAPTER_NAME: &str = "Parsec Virtual Display";
pub(super) const CONTROL_ADD: u32 = 0x0022e004;
pub(super) const CONTROL_REMOVE: u32 = 0x0022a008;
pub(super) const CONTROL_UPDATE: u32 = 0x0022a00c;
/// The driver's smallest built-in size, so things stay readable on the pad.
pub(super) const DISPLAY_WIDTH: u32 = 1280;
pub(super) const DISPLAY_HEIGHT: u32 = 720;

/// Whether Windows has the display driver Mini Monitor needs. It only looks:
/// nothing is opened or added.
pub fn driver_present() -> bool {
    unsafe {
        let set = SetupDiGetClassDevsW(&ADAPTER, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set as isize == -1 || set as isize == 0 {
            return false;
        }
        let mut interface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
        interface.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let found = SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &ADAPTER, 0, &mut interface) != 0;
        windows_sys::Win32::Devices::DeviceAndDriverInstallation::SetupDiDestroyDeviceInfoList(set);
        found
    }
}

pub(super) fn open_driver() -> Option<HANDLE> {
    unsafe {
        let set = SetupDiGetClassDevsW(
            &ADAPTER,
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set as isize == -1 || set as isize == 0 {
            return None;
        }
        let mut interface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
        interface.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let mut found = None;
        let mut index = 0;
        while found.is_none()
            && SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &ADAPTER, index, &mut interface) != 0
        {
            index += 1;
            let mut size = 0u32;
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                std::ptr::null_mut(),
                0,
                &mut size,
                std::ptr::null_mut(),
            );
            let mut buffer = vec![0u64; (size as usize).div_ceil(8).max(1)];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let ok = SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                detail,
                size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if ok == 0 {
                continue;
            }
            let handle = CreateFileW(
                std::ptr::addr_of!((*detail).DevicePath) as *const u16,
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL
                    | FILE_FLAG_NO_BUFFERING
                    | FILE_FLAG_OVERLAPPED
                    | FILE_FLAG_WRITE_THROUGH,
                std::ptr::null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
                found = Some(handle);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
        found
    }
}

pub(super) fn control(driver: HANDLE, code: u32, data: &[u8]) -> Option<u32> {
    unsafe {
        let mut input = [0u8; 32];
        input[..data.len()].copy_from_slice(data);
        let mut output = 0u32;
        let mut overlapped: OVERLAPPED = std::mem::zeroed();
        overlapped.hEvent = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        DeviceIoControl(
            driver,
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            (&mut output as *mut u32).cast(),
            4,
            std::ptr::null_mut(),
            &mut overlapped,
        );
        let mut transferred = 0;
        let ok = GetOverlappedResultEx(driver, &overlapped, &mut transferred, 5000, 0);
        if ok == 0 {
            // The driver still holds `output` and `overlapped`, which live on
            // this stack, so the request has to end before they do.
            CancelIoEx(driver, &overlapped);
            GetOverlappedResult(driver, &overlapped, &mut transferred, 1);
        }
        CloseHandle(overlapped.hEvent);
        (ok != 0).then_some(output)
    }
}

/// The driver wants the index as two bytes, high byte first.
pub(super) fn remove_data(index: u32) -> [u8; 2] {
    (index as u16).to_be_bytes()
}

/// The names of the displays on the virtual adapter that are part of the desktop.
pub(super) fn virtual_displays() -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let mut index = 0;
        loop {
            let mut device: DISPLAY_DEVICEW = std::mem::zeroed();
            device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
            if EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) == 0 {
                return names;
            }
            index += 1;
            let attached = device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0;
            if attached && from_wide(&device.DeviceString).contains(ADAPTER_NAME) {
                names.push(from_wide(&device.DeviceName));
            }
        }
    }
}

/// Where a display is on the desktop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub(super) fn display_area(name: &str) -> Option<Area> {
    let name = wide(name);
    unsafe {
        let mut mode: DEVMODEW = std::mem::zeroed();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
            return None;
        }
        let position = mode.Anonymous1.Anonymous2.dmPosition;
        Some(Area {
            x: position.x,
            y: position.y,
            width: mode.dmPelsWidth as i32,
            height: mode.dmPelsHeight as i32,
        })
    }
}

pub(super) fn set_size(name: &str) {
    let name = wide(name);
    unsafe {
        let mut mode: DEVMODEW = std::mem::zeroed();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        mode.dmPelsWidth = DISPLAY_WIDTH;
        mode.dmPelsHeight = DISPLAY_HEIGHT;
        mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT;
        ChangeDisplaySettingsExW(
            name.as_ptr(),
            &mode,
            std::ptr::null_mut(),
            CDS_UPDATEREGISTRY,
            std::ptr::null(),
        );
    }
}

/// The extra Windows display. It's unplugged when this is dropped.
pub struct VirtualDisplay {
    driver: HANDLE,
    index: u32,
    name: String,
    stop: Arc<AtomicBool>,
    ping: Option<std::thread::JoinHandle<()>>,
}

impl VirtualDisplay {
    pub fn add() -> Result<VirtualDisplay, String> {
        let driver = open_driver().ok_or(
            "Mini Monitor needs the Parsec Virtual Display driver, which couldn't be reached",
        )?;
        let before = virtual_displays();
        let Some(index) = control(driver, CONTROL_ADD, &[]) else {
            unsafe { CloseHandle(driver) };
            return Err("The display driver didn't add a display".into());
        };
        control(driver, CONTROL_UPDATE, &[]);

        let stop = Arc::new(AtomicBool::new(false));
        let (stopping, raw_driver) = (stop.clone(), driver as usize);
        let ping = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                control(raw_driver as HANDLE, CONTROL_UPDATE, &[]);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mut display = VirtualDisplay {
            driver,
            index,
            name: String::new(),
            stop,
            ping: Some(ping),
        };

        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if let Some(name) = virtual_displays().into_iter().find(|n| !before.contains(n)) {
                set_size(&name);
                display.name = name;
                return Ok(display);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Dropping `display` here unplugs it again.
        Err("Windows didn't start using the extra display. Check that it's set to extend your desktop".into())
    }

    pub fn name(&self) -> &str {
        self.name.trim_start_matches(r"\\.\")
    }

    pub fn area(&self) -> Option<Area> {
        display_area(&self.name)
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        // Its pictures no longer map to a place on the desktop: another
        // screen may take that place, and the pointer must be left alone there.
        if let Some(area) = display_area(&self.name) {
            for (_, _, _, from, _) in lock(&SHOWN).iter_mut().flatten() {
                if *from == Some(area) {
                    *from = None;
                }
            }
        }
        for live in lock(&LIVE_DISPLAY).iter_mut() {
            if live.as_deref() == Some(self.name.as_str()) {
                *live = None;
            }
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(ping) = self.ping.take() {
            let _ = ping.join();
        }
        // Nothing captures this display any more.
        forget_surfaces();
        control(self.driver, CONTROL_REMOVE, &remove_data(self.index));
        control(self.driver, CONTROL_UPDATE, &[]);
        unsafe { CloseHandle(self.driver) };
    }
}

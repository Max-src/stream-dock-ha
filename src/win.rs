//! Text in the form Windows' own functions take and give back.

/// A string as Windows wants it: UTF-16, ending with a zero.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Text Windows wrote into a buffer, up to its first zero.
pub fn from_wide(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Brings a window in front of the others without making it stay there, as
/// the original app does: above every window for a
/// moment, then back among the ordinary ones, keeping its place and size.
/// Windows sometimes refuses a plain request to come forward; this it allows.
pub fn bring_to_front(window: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };
    if window.is_null() {
        return;
    }
    // 0x43.
    let keep = SWP_NOSIZE | SWP_NOMOVE | SWP_SHOWWINDOW;
    unsafe {
        SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, keep);
        SetWindowPos(window, HWND_NOTOPMOST, 0, 0, 0, 0, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_flags_are_the_originals() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW};
        assert_eq!(SWP_NOSIZE | SWP_NOMOVE | SWP_SHOWWINDOW, 0x43);
        bring_to_front(std::ptr::null_mut());
    }

    #[test]
    fn text_goes_there_and_back() {
        let mut buffer = wide("Stream Dock HA");
        assert_eq!(buffer.last(), Some(&0));
        buffer.extend([65, 66]);
        assert_eq!(from_wide(&buffer), "Stream Dock HA");
        assert_eq!(from_wide(&[72, 105]), "Hi");
    }
}

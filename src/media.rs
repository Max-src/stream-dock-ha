//! Media keys when simulated key presses aren't allowed.
//!
//! Windows refuses simulated keys while the screen is locked, so media actions
//! go straight to the services behind them: the audio system for volume, and
//! the media session service (the one behind the lock screen's music controls)
//! for play, pause and tracks.

use crate::model::{lock, Action, Config, MediaKey, Shared, MIC_STEP};
use crate::pad::note;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::Result;
use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Sessions;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APPCOMMAND};

fn speakers() -> Result<IAudioEndpointVolume> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
        device.Activate(CLSCTX_ALL, None)
    }
}

/// Windows' default microphone, the one apps record from unless told otherwise,
/// as the original app uses.
fn microphone() -> Result<IAudioEndpointVolume> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        device.Activate(CLSCTX_ALL, None)
    }
}

/// A microphone turn:
/// the volume as a whole percent, plus or minus the step, kept to 0 to 100.
/// It's muted at 0, and unmuted again by turning it up.
pub fn microphone_step(level: f32, change: i32) -> (i32, bool) {
    let percent = ((level * 100.0).round() as i32 + change).clamp(0, 100);
    (percent, percent == 0)
}

fn change_microphone(key: MediaKey) -> Result<()> {
    let microphone = microphone()?;
    unsafe {
        let muted = microphone.GetMute()?.as_bool();
        if key == MediaKey::MicMute {
            microphone.SetMute(!muted, std::ptr::null())?;
        } else {
            let change = if key == MediaKey::MicUp { MIC_STEP } else { -MIC_STEP };
            let (percent, mute) = microphone_step(microphone.GetMasterVolumeLevelScalar()?, change);
            if mute != muted {
                microphone.SetMute(mute, std::ptr::null())?;
            }
            microphone.SetMasterVolumeLevelScalar(percent as f32 / 100.0, std::ptr::null())?;
        }
        remember_microphone(microphone.GetMute()?.as_bool());
    }
    Ok(())
}

/// Whether the microphone is muted, as last seen: 0 not known yet, 1 on, 2 muted.
static MICROPHONE_MUTED: AtomicU8 = AtomicU8::new(0);

fn remember_microphone(muted: bool) -> bool {
    let value = if muted { 2 } else { 1 };
    MICROPHONE_MUTED.swap(value, Ordering::SeqCst) != value
}

/// Whether the microphone is muted, for keys that show it. None until it's known.
pub fn microphone_muted() -> Option<bool> {
    match MICROPHONE_MUTED.load(Ordering::SeqCst) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// How often a microphone key checks whether the microphone is muted, so it
/// also follows a mute from Windows or another app.
const MICROPHONE_CHECK: Duration = Duration::from_millis(700);

/// Keeps microphone keys showing whether the microphone is muted, as the
/// original app does. It only
/// asks Windows while some key mutes the microphone.
pub fn watch_microphone(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        loop {
            // Home Assistant's integration shows it too, as a sensor of the PC.
            if uses_microphone(&lock(&shared.cfg)) || shared.bridge.is_connected() {
                let muted = microphone().and_then(|microphone| unsafe { microphone.GetMute() });
                if let Ok(muted) = muted {
                    if remember_microphone(muted.as_bool()) {
                        crate::pad::wake();
                        shared.touch();
                    }
                }
            }
            std::thread::sleep(MICROPHONE_CHECK);
        }
    });
}

fn uses_microphone(cfg: &Config) -> bool {
    let mute = Action::Media { key: MediaKey::MicMute };
    cfg.profiles.iter().flat_map(|profile| &profile.pages).flat_map(|page| &page.keys).any(|key| key.actions.contains(&mute))
}

fn change_volume(key: MediaKey) -> Result<()> {
    let volume = speakers()?;
    unsafe {
        match key {
            MediaKey::VolumeUp => volume.VolumeStepUp(std::ptr::null()),
            MediaKey::VolumeDown => volume.VolumeStepDown(std::ptr::null()),
            _ => volume.SetMute(!volume.GetMute()?.as_bool(), std::ptr::null()),
        }
    }
}

/// Asks whichever app Windows counts as playing. False means nothing took it.
fn control_session(key: MediaKey) -> Result<bool> {
    let session = Sessions::RequestAsync()?.get()?.GetCurrentSession()?;
    match key {
        MediaKey::PlayPause => session.TryTogglePlayPauseAsync()?.get(),
        MediaKey::NextTrack => session.TrySkipNextAsync()?.get(),
        MediaKey::PreviousTrack => session.TrySkipPreviousAsync()?.get(),
        _ => session.TryStopAsync()?.get(),
    }
}

/// For apps that don't use the media session: tells the Windows shell directly.
fn tell_shell(key: MediaKey) -> bool {
    let command: isize = match key {
        MediaKey::PlayPause => 14,
        MediaKey::NextTrack => 11,
        MediaKey::PreviousTrack => 12,
        _ => 13,
    };
    let name = crate::win::wide("Shell_TrayWnd");
    unsafe {
        let shell = FindWindowW(name.as_ptr(), std::ptr::null());
        !shell.is_null() && PostMessageW(shell, WM_APPCOMMAND, shell as usize, command << 16) != 0
    }
}

fn press_on_this_thread(key: MediaKey) -> std::result::Result<(), String> {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match key {
        _ if key.is_microphone() => {
            change_microphone(key).map_err(|e| format!("The microphone couldn't be changed: is one plugged in and turned on in Windows? ({e})"))?;
            note(&format!("microphone: {}", key.label()));
            Ok(())
        }
        MediaKey::VolumeUp | MediaKey::VolumeDown | MediaKey::Mute => {
            change_volume(key).map_err(|e| format!("The volume couldn't be changed: {e}"))?;
            note(&format!("media without keys: {} through the audio system", key.label()));
            Ok(())
        }
        _ => match control_session(key) {
            Ok(true) => {
                note(&format!("media without keys: {} through the media session", key.label()));
                Ok(())
            }
            other => {
                note(&format!("media without keys: session answered {other:?}, trying the shell"));
                if tell_shell(key) {
                    Ok(())
                } else {
                    Err("No media app could be reached".into())
                }
            }
        },
    }
}

/// Carries out a media key without touching the keyboard. It runs on its own
/// short-lived thread because these services need their own thread setup.
pub fn press_without_keyboard(key: MediaKey) -> std::result::Result<(), String> {
    std::thread::spawn(move || press_on_this_thread(key))
        .join()
        .unwrap_or_else(|_| Err("The media action stopped unexpectedly".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_microphone_turn_follows_the_originals_rule() {
        // 2 percent a click, on the rounded percent.
        assert_eq!(microphone_step(0.50, MIC_STEP), (52, false));
        assert_eq!(microphone_step(0.503, -MIC_STEP), (48, false));
        // Kept to 0 to 100, and muted only at 0.
        assert_eq!(microphone_step(0.99, MIC_STEP), (100, false));
        assert_eq!(microphone_step(0.01, -MIC_STEP), (0, true));
        assert_eq!(microphone_step(0.0, MIC_STEP), (2, false));
    }

    /// Mutes and unmutes the real microphone and moves its volume a step up
    /// and back, leaving it as it was.
    #[test]
    #[ignore = "briefly changes the real microphone"]
    fn the_microphone_answers_and_is_left_as_it_was() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let microphone = microphone().unwrap();
        let (level, muted) = unsafe { (microphone.GetMasterVolumeLevelScalar().unwrap(), microphone.GetMute().unwrap().as_bool()) };
        change_microphone(MediaKey::MicMute).unwrap();
        assert_eq!(microphone_muted(), Some(!muted));
        change_microphone(MediaKey::MicMute).unwrap();
        assert_eq!(microphone_muted(), Some(muted));
        let up = if level > 0.9 { MediaKey::MicDown } else { MediaKey::MicUp };
        change_microphone(up).unwrap();
        let moved = unsafe { microphone.GetMasterVolumeLevelScalar() }.unwrap();
        unsafe {
            microphone.SetMasterVolumeLevelScalar(level, std::ptr::null()).unwrap();
            microphone.SetMute(muted, std::ptr::null()).unwrap();
        }
        println!("microphone {level:.3}, one step {moved:.3}, muted {muted}");
        assert!(((moved - level).abs() * 100.0 - MIC_STEP as f32).abs() <= 1.0);
        assert_eq!(unsafe { microphone.GetMute() }.unwrap().as_bool(), muted);
    }

    /// Changes the volume by one step and back, and asks if a session exists,
    /// without playing or pausing anything.
    #[test]
    #[ignore = "briefly changes the real volume"]
    fn services_answer_without_any_key_press() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let volume = speakers().unwrap();
        let level = unsafe { volume.GetMasterVolumeLevelScalar() }.unwrap();
        let muted = unsafe { volume.GetMute() }.unwrap().as_bool();
        change_volume(MediaKey::VolumeUp).unwrap();
        let raised = unsafe { volume.GetMasterVolumeLevelScalar() }.unwrap();
        change_volume(MediaKey::VolumeDown).unwrap();
        change_volume(MediaKey::Mute).unwrap();
        let flipped = unsafe { volume.GetMute() }.unwrap().as_bool();
        change_volume(MediaKey::Mute).unwrap();
        unsafe { volume.SetMasterVolumeLevelScalar(level, std::ptr::null()) }.unwrap();
        println!("volume {level:.3}, one step up {raised:.3}, mute {muted} then {flipped}");
        assert!(raised > level || level >= 0.999);
        assert_ne!(muted, flipped);

        let manager = Sessions::RequestAsync().unwrap().get().unwrap();
        match manager.GetCurrentSession() {
            Ok(session) => println!("media session from: {}", session.SourceAppUserModelId().unwrap()),
            Err(_) => println!("no media session right now"),
        }
    }
}

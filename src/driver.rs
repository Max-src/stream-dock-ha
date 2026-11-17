//! The display driver Mini Monitor needs: the free Parsec Virtual Display
//! Driver 0.45, which Windows already trusts (Microsoft-signed). It turns a pad
//! into an extra Windows display. Nothing else outside the program is needed:
//! the C runtime is built in (.cargo/config.toml).
//!
//! Parsec's driver isn't shipped inside the setup file, since it's Parsec's to
//! hand out. Instead the setup, or Mini Monitor's tab, downloads Parsec's own
//! installer from Parsec's site, checks that Windows says it's validly signed
//! by Parsec Cloud, Inc., and runs it. Windows asks the person for
//! administrator rights first, as for any driver.

use std::path::Path;
use std::time::{Duration, Instant};

/// Parsec's own download of the driver's installer.
pub const DRIVER_URL: &str = "https://builds.parsec.app/vdd/parsec-vdd-0.45.0.0.exe";
/// Who must have signed it, as Windows reads the signature.
const SIGNER: &str = "CN=\"Parsec Cloud, Inc.\"";
/// The installer is about half a megabyte; anything far larger isn't it.
const MOST_BYTES: u64 = 20 * 1024 * 1024;

/// Whether Mini Monitor's driver is on this PC.
pub fn present() -> bool {
    crate::monitor::driver_present()
}

/// The last answer to `present`, for the window: it asks again in the
/// background every 5 seconds, so the window never waits on Windows.
pub fn present_lately() -> Option<bool> {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Mutex;
    static ANSWER: AtomicU8 = AtomicU8::new(0);
    static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
    static ASKING: AtomicBool = AtomicBool::new(false);
    let stale = crate::model::lock(&ASKED).is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
    if stale && !ASKING.swap(true, Ordering::SeqCst) {
        *crate::model::lock(&ASKED) = Some(Instant::now());
        std::thread::spawn(|| {
            ANSWER.store(if present() { 2 } else { 1 }, Ordering::SeqCst);
            ASKING.store(false, Ordering::SeqCst);
        });
    }
    match ANSWER.load(Ordering::SeqCst) {
        2 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// How long the whole download may take.
const DOWNLOAD_TIME: Duration = Duration::from_secs(120);

/// What an install is doing, for the person to see.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Step {
    #[default]
    Idle,
    Working(String),
    Done,
    Failed(String),
}

/// The words for a download under way, counting up from 0.
pub fn downloading(got: u64, total: Option<u64>) -> String {
    match total.filter(|total| *total > 0) {
        Some(total) => format!("Downloading the driver… {}%", (got * 100 / total).min(100)),
        None => "Downloading the driver…".to_string(),
    }
}

/// The words once it's downloaded, before the installer runs.
pub const DOWNLOADED: &str = "Downloaded. Installing… Windows asks for permission.";

/// Web requests through Windows' own secure connections and its own trusted
/// certificates: ureq's default list made Windows refuse Parsec's site
/// ("unable to find any user-specified roots in the final cert chain").
fn agent() -> ureq::Agent {
    crate::camera::secure_agent(DOWNLOAD_TIME)
}

/// Downloads, checks and runs Parsec's driver installer, then waits for the
/// driver to appear. Windows asks for administrator rights on the way. Each
/// stage is told to `report` as it starts.
pub fn install(report: &dyn Fn(String)) -> Result<(), String> {
    use std::io::Read;
    if present() {
        return Ok(());
    }
    let folder = std::env::temp_dir().join("StreamDockHA-driver");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let file = folder.join("parsec-vdd-0.45.0.0.exe");
    report(downloading(0, None));
    let mut reply = agent().get(DRIVER_URL).call().map_err(|e| format!("Parsec's site couldn't be reached ({e})"))?;
    let total = reply.body().content_length();
    let mut reader = reply.body_mut().with_config().limit(MOST_BYTES).reader();
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).map_err(|e| format!("The download didn't finish ({e})"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        report(downloading(bytes.len() as u64, total));
    }
    if total.is_some_and(|total| total != bytes.len() as u64) {
        return Err("The download didn't finish".into());
    }
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    report(DOWNLOADED.to_string());
    let result = check_signature(&file).and_then(|()| run_as_administrator(&file, "/S"));
    let _ = std::fs::remove_file(&file);
    result?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if present() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("The driver was installed but hasn't started yet. Restart Windows, then turn Mini Monitor on.".into())
}

/// True if Windows says this file is validly signed by Parsec.
fn check_signature(file: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let script = format!(
        "$s = Get-AuthenticodeSignature -LiteralPath '{}'; if ($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject.StartsWith('{}')) {{ exit 0 }} else {{ exit 1 }}",
        file.display().to_string().replace('\'', "''"),
        SIGNER.replace('\'', "''"),
    );
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The downloaded driver isn't signed by Parsec, so it wasn't run".into())
    }
}

/// Runs a program with administrator rights, after Windows asks, and waits.
fn run_as_administrator(file: &Path, arguments: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let (verb, path, arguments) = (crate::win::wide("runas"), crate::win::wide(&file.display().to_string()), crate::win::wide(arguments));
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = path.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = 1;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return Err("The driver needs administrator rights, and Windows didn't give them".into());
        }
        let waited = WaitForSingleObject(info.hProcess, 5 * 60 * 1000);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 {
            return Err("The driver's installer didn't finish".into());
        }
        if code != 0 {
            return Err(format!("The driver's installer stopped with code {code}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_download_says_how_far_it_is() {
        assert_eq!(downloading(0, None), "Downloading the driver…");
        assert_eq!(downloading(258_628, Some(517_256)), "Downloading the driver… 50%");
        assert_eq!(downloading(517_256, Some(517_256)), "Downloading the driver… 100%");
        assert_eq!(downloading(10, Some(0)), "Downloading the driver…");
    }

    /// Parsec's site answers through the same connection the download uses.
    /// The setup once failed here on a certificate error.
    #[test]
    #[ignore = "needs the internet"]
    fn parsecs_site_is_reached() {
        let reply = agent().head(DRIVER_URL).call().expect("Parsec's site");
        assert!(reply.status().is_success());
    }

    #[test]
    fn the_driver_comes_from_parsec_itself() {
        assert!(DRIVER_URL.starts_with("https://builds.parsec.app/"));
        assert!(SIGNER.contains("Parsec Cloud, Inc."));
    }

    /// Only looks: on this PC the driver is installed.
    #[test]
    #[ignore = "depends on this PC"]
    fn the_driver_is_found_here() {
        assert!(present());
    }

    /// A file Parsec's installer put on this PC passes the signature check,
    /// and a file of this app's doesn't.
    #[test]
    #[ignore = "depends on this PC"]
    fn only_parsecs_files_pass() {
        let parsec = Path::new(r"C:\Program Files\Parsec Virtual Display Driver\uninstall.exe");
        assert!(check_signature(parsec).is_ok());
        assert!(check_signature(&std::env::current_exe().unwrap()).is_err());
    }
}

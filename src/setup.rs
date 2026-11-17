//! The program is its own installer: a copy named "... Setup ..." offers to
//! install itself for this Windows account. No administrator rights are
//! needed, and it can be removed again from Windows' Apps list.
//!
//! Settings aren't part of the setup file, so a new PC starts with the sample
//! keys. Profiles move across with Save to file and Open file.
//!
//! Everything the program needs is inside it, except Mini Monitor's display
//! driver, which is Parsec's: the setup offers to fetch and install it
//! (driver.rs), which needs administrator rights for that step only.

use crate::model::{lock, APP_NAME};
use crate::{tray, ui};
use eframe::egui::{self, RichText};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink, KNOWN_FOLDER_FLAG,
};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO,
};

const EXE_NAME: &str = "Stream Dock HA.exe";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\StreamDockHA";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where things go. Tests point all of it at a folder of their own, so they
/// leave the real Start Menu and Apps list alone.
#[derive(Clone)]
struct Places {
    folder: PathBuf,
    start_menu: Option<PathBuf>,
    desktop: Option<PathBuf>,
    uninstall_key: String,
    sandbox: bool,
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        text.map(PathBuf::from)
    }
}

fn places() -> Places {
    let link = format!("{APP_NAME}.lnk");
    if let Some(root) = std::env::var_os("STREAM_DOCK_HA_INSTALL_ROOT") {
        let root = PathBuf::from(root);
        return Places {
            folder: root.join("Programs").join(APP_NAME),
            start_menu: Some(root.join("Start Menu").join(&link)),
            desktop: Some(root.join("Desktop").join(&link)),
            uninstall_key: String::new(),
            sandbox: true,
        };
    }
    let local = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    Places {
        folder: local.join("Programs").join(APP_NAME),
        start_menu: known_folder(&FOLDERID_Programs).map(|p| p.join(&link)),
        desktop: known_folder(&FOLDERID_Desktop).map(|p| p.join(&link)),
        uninstall_key: UNINSTALL_KEY.to_string(),
        sandbox: false,
    }
}

fn write_text(key: &str, name: &str, value: &str) -> Result<(), String> {
    let (key, name, value) = (crate::win::wide(key), crate::win::wide(name), crate::win::wide(value));
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err("Windows refused the Apps list entry".into())
    }
}

fn write_number(key: &str, name: &str, value: u32) -> Result<(), String> {
    let (key, name) = (crate::win::wide(key), crate::win::wide(name));
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_DWORD,
            (&value as *const u32).cast(),
            4,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err("Windows refused the Apps list entry".into())
    }
}

fn make_shortcut(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(folder) = link.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    let run = || -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shortcut.SetPath(&HSTRING::from(target.as_os_str()))?;
            if let Some(folder) = target.parent() {
                shortcut.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
            }
            shortcut.SetDescription(&HSTRING::from("Programmable buttons for your Stream Dock"))?;
            let file: IPersistFile = shortcut.cast()?;
            file.Save(&HSTRING::from(link.as_os_str()), true)
        }
    };
    run().map_err(|e| e.to_string())
}

/// Replacing a running program fails, so an older copy is asked to close first.
fn copy_program(from: &Path, to: &Path, sandbox: bool) -> Result<(), String> {
    if !sandbox {
        tray::ask_running_copy_to_quit();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("{APP_NAME} couldn't be copied. Quit it from its icon and try again ({e})")),
        }
    }
}

#[derive(Clone, Copy)]
struct Choices {
    desktop: bool,
    with_windows: bool,
    start_now: bool,
    /// Also installs Mini Monitor's display driver.
    driver: bool,
}

fn install(choices: Choices) -> Result<PathBuf, String> {
    let places = places();
    let this = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&places.folder).map_err(|e| e.to_string())?;
    let target = places.folder.join(EXE_NAME);
    copy_program(&this, &target, places.sandbox)?;

    if let Some(link) = &places.start_menu {
        make_shortcut(&target, link)?;
    }
    if let (true, Some(link)) = (choices.desktop, &places.desktop) {
        make_shortcut(&target, link)?;
    }

    let size_kb = std::fs::metadata(&target).map_or(0, |m| m.len() / 1024) as u32;
    let texts = [
        ("DisplayName", APP_NAME.to_string()),
        ("DisplayVersion", VERSION.to_string()),
        ("Publisher", APP_NAME.to_string()),
        ("InstallLocation", places.folder.display().to_string()),
        ("DisplayIcon", target.display().to_string()),
        ("UninstallString", format!("\"{}\" --uninstall", target.display())),
    ];
    let numbers = [("NoModify", 1), ("NoRepair", 1), ("EstimatedSize", size_kb)];
    if places.sandbox {
        // A test records what would go in the Apps list, in its own folder.
        let lines: Vec<String> = texts
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .chain(numbers.iter().map(|(name, value)| format!("{name}={value}")))
            .collect();
        let entry = places.folder.join("apps-list-entry.txt");
        std::fs::write(entry, lines.join("\n")).map_err(|e| e.to_string())?;
    } else {
        let key = &places.uninstall_key;
        for (name, value) in &texts {
            write_text(key, name, value)?;
        }
        for (name, value) in numbers {
            write_number(key, name, value)?;
        }
    }

    if choices.with_windows && !places.sandbox {
        tray::set_autostart_for(&target)?;
    }
    Ok(target)
}

fn start(target: &Path) -> Result<(), String> {
    std::process::Command::new(target).arg("--show").spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Removes everything but the person's buttons and settings.
fn uninstall(places: &Places) {
    if !places.sandbox {
        tray::ask_running_copy_to_quit();
        let _ = tray::set_autostart(false);
        std::thread::sleep(Duration::from_millis(1500));
    }
    for link in [&places.start_menu, &places.desktop].into_iter().flatten() {
        let _ = std::fs::remove_file(link);
    }
    if !places.sandbox {
        let key = crate::win::wide(&places.uninstall_key);
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
    }
    // A running program can't delete itself, so a short-lived helper finishes
    // the job once this one has ended.
    let folder = places.folder.display().to_string();
    let _ = std::process::Command::new("cmd.exe")
        .raw_arg(format!("/c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{folder}\""))
        .creation_flags(0x0800_0000)
        .spawn();
}

fn ask(text: &str) -> bool {
    let (text, title) = (crate::win::wide(text), crate::win::wide(APP_NAME));
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

fn tell(text: &str) {
    let (text, title) = (crate::win::wide(text), crate::win::wide(APP_NAME));
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

#[derive(Clone, PartialEq)]
enum Progress {
    Choosing,
    Working(String),
    /// Installed; with a note if the display driver didn't go in.
    Done(Option<String>),
    Failed(String),
}

struct SetupApp {
    choices: Choices,
    progress: Arc<Mutex<Progress>>,
    shot: Option<PathBuf>,
    frames: u32,
    /// Whether Mini Monitor's display driver was already there.
    driver_there: bool,
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let progress = lock(&self.progress).clone();
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::same(22)))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("Install {APP_NAME}")).size(22.0).strong());
                ui.label(RichText::new(format!("Version {VERSION}, for your Windows account.")).size(13.0));
                ui.add_space(12.0);
                match &progress {
                    Progress::Choosing => {
                        ui.checkbox(&mut self.choices.desktop, "Desktop icon");
                        ui.checkbox(&mut self.choices.with_windows, "Start with Windows");
                        ui.checkbox(&mut self.choices.start_now, "Open it when done");
                        if self.driver_there {
                            ui.label(RichText::new("Mini Monitor's display driver is already installed.").size(12.5));
                        } else {
                            ui.checkbox(&mut self.choices.driver, "Display driver for Mini Monitor")
                                .on_hover_text("Parsec's free Virtual Display Driver, downloaded from Parsec. Windows asks for administrator rights for this step only.");
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new("A new PC starts with sample keys. Move your profiles with Save to file and Open file.").size(12.5));
                        ui.add_space(12.0);
                        if ui.button(RichText::new("  Install  ").size(15.0)).clicked() {
                            *lock(&self.progress) = Progress::Working("Installing…".into());
                            let (choices, progress) = (self.choices, self.progress.clone());
                            let repaint = ctx.clone();
                            std::thread::spawn(move || {
                                let finished = install(choices).and_then(|target| {
                                    let mut note = None;
                                    if choices.driver && !places().sandbox {
                                        let report = |words: String| {
                                            *lock(&progress) = Progress::Working(words);
                                            repaint.request_repaint();
                                        };
                                        note = crate::driver::install(&report).err();
                                    }
                                    if choices.start_now {
                                        start(&target)?;
                                    }
                                    Ok(note)
                                });
                                *lock(&progress) = match finished {
                                    Ok(note) => Progress::Done(note),
                                    Err(e) => Progress::Failed(e),
                                };
                                repaint.request_repaint();
                            });
                        }
                    }
                    Progress::Working(what) => {
                        ui.spinner();
                        ui.label(what.as_str());
                    }
                    Progress::Done(note) => {
                        ui.label(RichText::new("Installed.").size(16.0).strong());
                        ui.label("It's in the Start Menu, with its icon near the clock. Remove it from Apps in Windows Settings.");
                        if let Some(note) = note {
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("Mini Monitor's driver isn't installed: {note}. You can install it later from the Monitor tab.")).size(12.5));
                        }
                        ui.add_space(12.0);
                        if ui.button("  Close  ").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    Progress::Failed(why) => {
                        ui.label(RichText::new("The installation didn't finish.").size(16.0).strong());
                        ui.label(why);
                        ui.add_space(12.0);
                        if ui.button("  Try again  ").clicked() {
                            *lock(&self.progress) = Progress::Choosing;
                        }
                    }
                }
            });

        self.frames += 1;
        if let Some(path) = self.shot.clone() {
            ctx.request_repaint();
            if self.frames == 60 {
                ctx.request_screenshot(move |image| {
                    let code = i32::from(ui::save_png(&path, &image).is_err());
                    std::process::exit(code);
                });
            }
        }
    }
}

fn show_setup_window(shot: Option<PathBuf>) {
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("{APP_NAME} Setup"))
        .with_inner_size([520.0, 360.0])
        .with_resizable(false)
        .with_icon(Arc::new(egui::IconData { rgba: tray::icon_rgba(64), width: 64, height: 64 }));
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let driver_there = crate::driver::present() && std::env::var_os("STREAM_DOCK_HA_SHOT_NO_DRIVER").is_none();
    let app = SetupApp {
        choices: Choices { desktop: false, with_windows: false, start_now: true, driver: !driver_there },
        progress: Arc::new(Mutex::new(Progress::Choosing)),
        shot,
        frames: 0,
        driver_there,
    };
    let result = eframe::run_native(
        "Stream Dock HA Setup",
        options,
        Box::new(move |cc| {
            ui::apply_theme(&cc.egui_ctx, crate::model::ThemeChoice::Windows);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        tell(&format!("Setup couldn't open its window: {e}"));
    }
}

/// Handles the setup and uninstall ways of starting. True if it did.
pub fn run_if_asked() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--uninstall") {
        if has("--quiet") || ask(&format!("Remove {APP_NAME} from this PC? Your buttons and settings stay in case you install it again.")) {
            uninstall(&places());
            if !has("--quiet") {
                tell(&format!("{APP_NAME} has been removed."));
            }
        }
        return true;
    }
    if has("--install-quiet") {
        // A quiet install never asks for administrator rights, so no driver.
        let choices = Choices { desktop: true, with_windows: false, start_now: false, driver: false };
        return match install(choices) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }
    let name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    if has("--setup") || (name.contains("setup") && !args.iter().any(|a| a == "--hidden")) {
        show_setup_window(screenshot);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::tray::icon_rgba;

    /// The icon for Home Assistant's integration, the same as on the PC:
    /// `cargo test -- --ignored make_brand_icons` writes target/brand.
    #[test]
    #[ignore = "writes target/brand"]
    fn make_brand_icons() {
        std::fs::create_dir_all("target/brand").unwrap();
        for (size, name) in [(256u32, "icon.png"), (512, "icon@2x.png")] {
            let picture = image::RgbaImage::from_raw(size, size, icon_rgba(size as usize)).unwrap();
            picture.save(format!("target/brand/{name}")).unwrap();
        }
    }

    #[test]
    #[ignore = "writes assets/icon.ico"]
    fn make_icon_file() {
        let sizes = [16u32, 24, 32, 48, 64, 128, 256];
        let mut pictures = Vec::new();
        for size in sizes {
            let picture = image::RgbaImage::from_raw(size, size, icon_rgba(size as usize)).unwrap();
            let mut bytes = std::io::Cursor::new(Vec::new());
            picture.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            pictures.push((size, bytes.into_inner()));
        }
        let mut file = vec![0u8, 0, 1, 0, sizes.len() as u8, 0];
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (size, data) in &pictures {
            let side = if *size == 256 { 0 } else { *size as u8 };
            file.extend_from_slice(&[side, side, 0, 0, 1, 0, 32, 0]);
            file.extend_from_slice(&(data.len() as u32).to_le_bytes());
            file.extend_from_slice(&offset.to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in &pictures {
            file.extend_from_slice(data);
        }
        std::fs::create_dir_all("assets").unwrap();
        std::fs::write("assets/icon.ico", file).unwrap();
    }
}

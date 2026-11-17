//! Where settings and saved profiles live on disk, including those from the app's earlier name.

use super::*;

/// The settings folder.
pub(super) const SETTINGS_FOLDER: &str = "StreamDockHA";

/// The settings folders under the app's earlier names, newest first: Stream
/// Dock Max, then Tray Dock. They are read when the new one is missing, and
/// never changed.
pub(super) const OLD_SETTINGS_FOLDERS: [&str; 2] = ["StreamDockMax", "TrayDock"];

pub(super) fn app_data() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// The folder of a test copy, when `STREAM_DOCK_HA_SANDBOX` names one. A test
/// copy keeps everything there and leaves the pads, the shortcuts, Windows'
/// start-up list and the tray to the copy the person uses, so the two can run
/// side by side.
pub fn sandbox_folder() -> Option<PathBuf> {
    std::env::var_os("STREAM_DOCK_HA_SANDBOX").filter(|folder| !folder.is_empty()).map(PathBuf::from)
}

pub fn config_path() -> PathBuf {
    chosen_settings_file(sandbox_folder(), std::env::var_os("STREAM_DOCK_HA_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| app_data().join(SETTINGS_FOLDER).join("config.json"))
}

/// The settings file a test copy or a test chose, if any: a test copy's
/// folder comes first, so it can never reach the person's own settings.
pub(super) fn chosen_settings_file(sandbox: Option<PathBuf>, own_file: Option<PathBuf>) -> Option<PathBuf> {
    sandbox.map(|folder| folder.join("config.json")).or(own_file)
}

/// The newest settings saved under an earlier name, unless a test chose its
/// own file or folder.
pub(super) fn old_config_path() -> Option<PathBuf> {
    if std::env::var_os("STREAM_DOCK_HA_CONFIG").is_some() || sandbox_folder().is_some() {
        return None;
    }
    OLD_SETTINGS_FOLDERS.iter().map(|folder| app_data().join(folder).join("config.json")).find(|path| path.exists())
}

/// Copies the old settings to the new place if only the old ones exist. The
/// old file stays as it was, so going back to an older copy still works.
pub(super) fn copy_settings(old: &std::path::Path, new: &std::path::Path) -> bool {
    if new.exists() || !old.exists() {
        return false;
    }
    new.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok()) && std::fs::copy(old, new).is_ok()
}

/// The first time Stream Dock HA runs, it brings over the settings saved
/// while it was called Stream Dock Max or Tray Dock.
pub fn adopt_old_settings() {
    if let Some(old) = old_config_path() {
        copy_settings(&old, &config_path());
    }
}

/// Loads the saved setup. A file that can't be understood is set aside
/// rather than overwritten, and one that can't be read right now is left
/// alone and not saved over, so a problem never destroys someone's buttons.
/// A read-only run, such as --screenshot, never moves anything.
pub fn load(read_only: bool) -> Loaded {
    let mut path = config_path();
    // Before the first save under the new name, read the old settings in place.
    if !path.exists() {
        if let Some(old) = old_config_path().filter(|old| old.exists()) {
            path = old;
        }
    }
    let fresh = |problem: Option<String>, can_save: bool| Loaded { cfg: settled(Config::sample()), problem, can_save };
    let mut attempt = 0;
    let bytes = loop {
        match std::fs::read(&path) {
            Ok(bytes) => break bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fresh(None, true),
            // Another program, such as a backup or sync tool, may have it open.
            Err(_) if attempt < 10 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                return fresh(
                    Some(format!("Your saved buttons couldn't be read ({e}). They were left as they are and nothing will be saved until Stream Dock HA is restarted.")),
                    false,
                )
            }
        }
    };
    match parse_settings(bytes) {
        Ok(cfg) => Loaded { cfg: settled(cfg), problem: None, can_save: true },
        Err(_) if read_only || path != config_path() => fresh(None, !read_only),
        Err(why) => {
            // The reason a settings file can't be read is kept.
            crate::pad::note(&format!("settings couldn't be read: {why}"));
            let keep = path.with_extension("unreadable.json");
            if std::fs::rename(&path, &keep).is_err() {
                return fresh(Some(format!("Your saved buttons couldn't be read ({why}), and couldn't be set aside, so nothing will be saved until Stream Dock HA is restarted.")), false);
            }
            fresh(
                Some(format!(
                    "Your saved buttons couldn't be read ({why}), so they were kept in {} and the sample set was loaded.",
                    keep.display()
                )),
                true,
            )
        }
    }
}

/// Settings made consistent, and with an instance id that belongs to this PC.
fn settled(mut cfg: Config) -> Config {
    cfg.repair();
    cfg.adopt_machine(&machine_hash());
    cfg
}

/// The settings from a file's bytes, or why they can't be read, with the line
/// and column where it went wrong.
pub(super) fn parse_settings(bytes: Vec<u8>) -> Result<Config, String> {
    let text = String::from_utf8(bytes).map_err(|_| "the file isn't text".to_string())?;
    serde_json::from_str::<Config>(&text).map_err(|e| e.to_string())
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

pub(super) const PROFILE_FORMAT: &str = "stream-dock-ha-profile";

/// Profiles saved under the app's earlier names still open.
pub(super) const OLD_PROFILE_FORMATS: [&str; 2] = ["stream-dock-max-profile", "tray-dock-profile"];

#[derive(Serialize, Deserialize)]
pub(super) struct ProfileFile {
    pub(super) format: String,
    pub(super) version: u32,
    pub(super) profile: Profile,
}

/// Saves one profile as a file of its own.
pub fn export_profile(profile: &Profile, path: &std::path::Path) -> Result<(), String> {
    let file = ProfileFile {
        format: PROFILE_FORMAT.into(),
        version: 1,
        profile: profile.clone(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// The endings of the original StreamDock app's profile files, which this app
/// can't read.
pub(super) const ORIGINAL_PROFILE_ENDINGS: [&str; 2] = ["streamDockProfile", "monstardeckProfile"];

/// Reads a saved profile. Nothing else is touched if the file isn't one.
pub fn import_profile(path: &std::path::Path) -> Result<Profile, String> {
    let ending = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ORIGINAL_PROFILE_ENDINGS.iter().any(|known| known.eq_ignore_ascii_case(ending)) {
        return Err("that is a profile from the StreamDock app, which Stream Dock HA can't open yet. Set its keys up here instead".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProfileFile =
        serde_json::from_str(&text).map_err(|_| "that file isn't a saved profile".to_string())?;
    if file.format != PROFILE_FORMAT && !OLD_PROFILE_FORMATS.contains(&file.format.as_str()) {
        return Err("that file isn't a saved profile".into());
    }
    let mut profile = file.profile;
    profile.repair();
    profile.forget_remote_allows();
    Ok(profile)
}

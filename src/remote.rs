//! Lets Home Assistant drive Stream Dock HA, and tells it about presses.
//!
//! Commands arrive as `stream_dock_ha_command` events over the connection Stream Dock HA
//! already keeps open to Home Assistant, so no port is opened on this PC. Each
//! one is answered with a `stream_dock_ha_result` event, and presses go out as
//! `stream_dock_ha_pressed` events. It all stays off until the person turns it on,
//! and adding keys that run programs needs a second switch.
//! docs/home-assistant-control.md is the reference for all of this.

use crate::actions::{self, PressSource};
use crate::model::layout::{cell_of_name, control_name};
use crate::model::{lock, Action, Config, DockKey, PadKind, Profile, RemoteAccess, Shared};
use crate::pad::note;
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, Weak};

pub const COMMAND_EVENT: &str = "stream_dock_ha_command";
pub const RESULT_EVENT: &str = "stream_dock_ha_result";
pub const PRESS_EVENT: &str = "stream_dock_ha_pressed";
/// The same events under the app's earlier names, Stream Dock Max and then
/// Tray Dock, as (command, result, press). A command under an old name is
/// still answered under that name. Presses go out under the current name
/// only: three events for each press tripled the traffic, and no released
/// version used the others.
pub const OLD_EVENTS: [(&str, &str, &str); 2] = [
    ("stream_dock_max_command", "stream_dock_max_result", "stream_dock_max_pressed"),
    ("tray_dock_command", "tray_dock_result", "tray_dock_pressed"),
];

/// The command event under every name, the current one first.
pub fn command_events() -> [&'static str; 3] {
    [COMMAND_EVENT, OLD_EVENTS[0].0, OLD_EVENTS[1].0]
}

/// What a command did, once it's been applied to the settings.
#[derive(Debug, Default, PartialEq)]
pub struct Done {
    pub reply: Value,
    pub changed: bool,
    /// What's left to do beyond the settings, once they're saved.
    pub effect: Option<Effect>,
}

/// What a command does outside the settings, which `apply` can't do itself.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// Press a key: the cell, on that profile and page of that pad.
    Press { pad_id: String, profile: usize, page: usize, cell: usize },
    /// Turn a pad's screen off or back on.
    Screen { pad_id: String, kind: PadKind, on: bool },
    /// A picture of the whole pad, a key `85 * scale` pixels across.
    Snapshot { pad_id: String, scale: u32 },
    ShowAlert { pad_id: String, alert: crate::alerts::Alert },
    ClearAlert { pad_id: String },
    /// Show something on one key, by cell, for a while.
    ShowOnKey { pad_id: String, cell: usize, flash: crate::flashes::Flash },
    /// Take that off one key, or off all the pad's keys.
    ClearOnKey { pad_id: String, cell: Option<usize> },
    /// The mouse on the pad's Mini Monitor, from a dashboard card.
    Mouse { kind: PadKind, request: crate::remote_mouse::Request },
}

fn changed(reply: Value) -> Done {
    Done { reply, changed: true, effect: None }
}

fn effect(reply: Value, effect: Effect) -> Done {
    Done { reply, changed: false, effect: Some(effect) }
}

/// Commands Home Assistant may send even with control turned off: they only read.
const READING: [&str; 3] = ["state", "get_key", "snapshot"];

pub const CONTROL_OFF: &str = "Home Assistant control is turned off in Stream Dock HA's settings";

/// What a command or press event calls a pad: `main` and `akp03e` for the
/// physical pads, as before pads had ids, and its id for a virtual pad.
fn pad_name(cfg: &Config, pad_id: &str) -> String {
    match cfg.pad(pad_id).map(|entry| entry.kind) {
        Some(PadKind::Main) => "main".into(),
        Some(PadKind::Akp03e) => "akp03e".into(),
        Some(PadKind::Dashboard) | None => pad_id.to_string(),
    }
}

/// The pad a command means, by id: its `pad` is a pad's id, or `main` or
/// `akp03e` for the first pad of that kind. Without one, the 293S.
fn pad_of(cfg: &Config, data: &Value) -> Result<String, String> {
    let wanted = data.get("pad").and_then(Value::as_str).unwrap_or("main");
    let kind = match wanted {
        "main" => PadKind::Main,
        "akp03e" => PadKind::Akp03e,
        id => {
            return match cfg.pad(id) {
                Some(entry) if cfg.in_use(id) => Ok(entry.id.clone()),
                Some(_) => Err("That pad isn't in use yet".into()),
                None => Err(format!("There's no pad called \"{id}\": use \"main\", \"akp03e\" or a pad's id")),
            }
        }
    };
    cfg.pad_of_kind(kind).map(|entry| entry.id.clone()).ok_or(format!("The {wanted} pad isn't in use yet"))
}

fn text<'a>(data: &'a Value, field: &str) -> Result<&'a str, String> {
    data.get(field).and_then(Value::as_str).ok_or(format!("This needs \"{field}\""))
}

fn number(data: &Value, field: &str) -> Result<i64, String> {
    data.get(field).and_then(Value::as_i64).ok_or(format!("This needs \"{field}\" as a number"))
}

/// The profile a command means: the one it names, or the one its pad is on.
fn profile_of(cfg: &Config, data: &Value) -> Result<usize, String> {
    let pad_id = pad_of(cfg, data)?;
    match data.get("profile").and_then(Value::as_str) {
        Some(name) => cfg
            .profiles_of(&pad_id)
            .into_iter()
            .find(|index| cfg.profiles[*index].name == name)
            .ok_or(format!("{} has no profile called \"{name}\"", pad_name(cfg, &pad_id))),
        None => cfg.pad_profile_of(&pad_id).ok_or(format!("The {} pad isn't in use yet", pad_name(cfg, &pad_id))),
    }
}

/// The page a command means, counted from 1, or the one showing.
fn page_of(profile: &Profile, data: &Value) -> Result<usize, String> {
    match data.get("page").and_then(Value::as_i64) {
        Some(page) if page >= 1 && page as usize <= profile.pages.len() => Ok(page as usize - 1),
        Some(page) => Err(format!("There's no page {page}: this profile has {}", profile.pages.len())),
        None => Ok(profile.active_page),
    }
}

/// The cell a command means, from `key` as a number or `control` as a name.
fn cell_of(profile: &Profile, data: &Value, field: &str) -> Result<usize, String> {
    let name = match data.get(field) {
        Some(Value::Number(key)) => format!("key_{key}"),
        Some(Value::String(name)) => name.clone(),
        _ => match data.get("control").and_then(Value::as_str).filter(|_| field == "key") {
            Some(name) => name.to_string(),
            None => return Err(format!("This needs \"{field}\", or \"control\" for a button or knob")),
        },
    };
    cell_of_name(profile.pad, profile, &name).ok_or(format!("This pad has no control called \"{name}\""))
}

/// Actions that act on this PC rather than on the pad or Home Assistant.
pub fn acts_on_this_pc(action: &Action) -> bool {
    matches!(
        action,
        Action::Open { .. } | Action::Website { .. } | Action::Shortcut { .. } | Action::TypeText { .. }
    )
}

/// A key with some fields changed, as long as what's changing is allowed.
/// Whether Home Assistant may press a key is only the person's to say, and
/// new steps are a new key as far as that goes, so it goes back to automatic.
fn with_changes(key: &DockKey, changes: &serde_json::Map<String, Value>, programs: bool) -> Result<DockKey, String> {
    if changes.contains_key("remote") {
        return Err("Only the person at the PC can choose whether Home Assistant may press a key".into());
    }
    let mut merged = serde_json::to_value(key).map_err(|e| e.to_string())?;
    for (field, value) in changes {
        merged[field] = value.clone();
    }
    let mut new: DockKey = serde_json::from_value(merged).map_err(|e| format!("That key isn't valid: {e}"))?;
    if !programs && changes.contains_key("actions") && new.actions.iter().any(acts_on_this_pc) {
        return Err("Stream Dock HA's settings don't let Home Assistant add keys that open programs or press keys".into());
    }
    if new.actions != key.actions {
        new.remote = RemoteAccess::Auto;
    }
    Ok(new)
}

/// Whether a press from this source may run this key: the one rule for every
/// way Home Assistant can press (events, actions, entities, cards). The
/// person's own presses, on the pad, on screen or by shortcut, always may.
pub fn may_press(cfg: &Config, key: &DockKey, source: PressSource) -> Result<(), String> {
    if source.is_local() {
        return Ok(());
    }
    if !cfg.ha_control {
        return Err(CONTROL_OFF.into());
    }
    match key.remote {
        RemoteAccess::Allow => Ok(()),
        RemoteAccess::Deny => Err("That key is set so Home Assistant can't press it".into()),
        RemoteAccess::Auto if key.actions.iter().any(acts_on_this_pc) => Err(
            "That key opens programs or presses keys, which Home Assistant isn't allowed to press. In Stream Dock HA, set its \"Pressing from Home Assistant\" to Always to allow it".into(),
        ),
        RemoteAccess::Auto => Ok(()),
    }
}

/// The screen key a command means on its pad, whatever profile and page that
/// pad shows: its pad, its cell and its control's name. `key` is a number
/// (also as text, as forms send it) or a control's name; `control` a name.
fn screen_key_of(cfg: &Config, data: &Value) -> Result<(String, usize, String), String> {
    let pad_id = pad_of(cfg, data)?;
    let index = cfg.pad_profile_of(&pad_id).ok_or(format!("The {} pad isn't in use yet", pad_name(cfg, &pad_id)))?;
    let profile = &cfg.profiles[index];
    let mut data = data.clone();
    if let Some(number) = data.get("key").and_then(Value::as_str).and_then(|key| key.trim().parse::<u64>().ok()) {
        data["key"] = json!(number);
    }
    let cell = cell_of(profile, &data, "key")?;
    let entry = cfg.pad(&pad_id).ok_or("That pad isn't there any more")?;
    let layout = crate::model::layout::layout_of(entry, profile);
    let control = layout
        .controls
        .iter()
        .find(|control| control.cell == Some(cell) && control.screen && control.kind == crate::model::layout::ControlType::Key)
        .ok_or("That control has no screen: choose a key")?;
    Ok((pad_id, cell, control.name.clone()))
}

/// The pad a command means, with its kind.
fn pad_and_kind(cfg: &Config, data: &Value) -> Result<(String, PadKind), String> {
    let pad_id = pad_of(cfg, data)?;
    let kind = cfg.pad(&pad_id).map_or(PadKind::Main, |entry| entry.kind);
    Ok((pad_id, kind))
}

fn key_summary(profile: &Profile, page: usize, cell: usize) -> Value {
    let key = &profile.pages[page].keys[cell];
    json!({ "control": control_name(profile.pad, profile, cell), "key": key })
}

/// A pad's profiles, as `state` lists them.
fn profiles_state(cfg: &Config, pad_id: &str) -> Vec<Value> {
    let running = cfg.pad_profile_of(pad_id);
    cfg.profiles_of(pad_id)
        .into_iter()
        .map(|index| {
            let profile = &cfg.profiles[index];
            json!({
                "name": profile.name,
                "pages": profile.pages.len(),
                "page": profile.active_page + 1,
                "running": Some(index) == running,
            })
        })
        .collect()
}

/// Every pad's profiles, brightness and Mini Monitor: under `main` and
/// `akp03e`, as before pads had ids, and in `pads` for all of them.
fn state(cfg: &Config) -> Value {
    let old = |kind: PadKind| {
        let profiles = cfg.pad_of_kind(kind).map_or(Vec::new(), |entry| profiles_state(cfg, &entry.id));
        json!({ "profiles": profiles, "brightness": cfg.brightness_of(kind), "mini_monitor": cfg.monitor_on(kind) })
    };
    let pads: Vec<Value> = cfg
        .pads_in_use()
        .map(|entry| {
            let mini_monitor = entry.kind.has_mini_monitor().then(|| cfg.monitor_on(entry.kind));
            json!({
                "id": entry.id,
                "kind": entry.kind,
                "name": entry.name,
                "profiles": profiles_state(cfg, &entry.id),
                "brightness": entry.brightness,
                "mini_monitor": mini_monitor,
            })
        })
        .collect();
    json!({ "main": old(PadKind::Main), "akp03e": old(PadKind::Akp03e), "pads": pads })
}

fn turn_page(profile: &mut Profile, command: &str, data: &Value) -> Result<(), String> {
    let count = profile.pages.len();
    profile.active_page = match command {
        "next_page" => (profile.active_page + 1) % count,
        "previous_page" => (profile.active_page + count - 1) % count,
        _ => {
            number(data, "page")?;
            page_of(profile, data)?
        }
    };
    Ok(())
}

fn add_profile(cfg: &mut Config, data: &Value) -> Result<Done, String> {
    let pad_id = pad_of(cfg, data)?;
    let wanted = data.get("name").and_then(Value::as_str).unwrap_or("Profile");
    let profile = cfg.new_profile_for(&pad_id, wanted).ok_or("That pad isn't there any more")?;
    let name = profile.name.clone();
    cfg.profiles.push(profile);
    Ok(changed(json!({ "profile": name })))
}

fn delete_profile(cfg: &mut Config, data: &Value) -> Result<Done, String> {
    let index = profile_of(cfg, data)?;
    let count = cfg.pad_of_profile(index).map_or(0, |entry| cfg.profiles_of(&entry.id).len());
    // The AKP03E can go back to not being in use; the others always keep one.
    match cfg.profiles[index].pad {
        PadKind::Main if count < 2 => return Err("The Stream Dock keeps at least one profile".into()),
        PadKind::Dashboard if count < 2 => return Err("A virtual pad keeps at least one profile".into()),
        PadKind::Main | PadKind::Akp03e | PadKind::Dashboard => {}
    }
    let name = cfg.profiles[index].name.clone();
    cfg.remove_profile(index);
    Ok(changed(json!({ "deleted": name })))
}

fn copy_key(cfg: &mut Config, data: &Value, cut: bool) -> Result<Done, String> {
    let from_profile = profile_of(cfg, data)?;
    let from_page = page_of(&cfg.profiles[from_profile], data)?;
    let from = cell_of(&cfg.profiles[from_profile], data, "key")?;
    let to = data.get("to").ok_or("This needs \"to\", with where the key goes")?;
    let to_profile = profile_of(cfg, to)?;
    let to_page = page_of(&cfg.profiles[to_profile], to)?;
    let to_cell = cell_of(&cfg.profiles[to_profile], to, "key")?;
    let key = cfg.profiles[from_profile].pages[from_page].keys[from].clone();
    if key.is_empty() {
        return Err("That key is empty".into());
    }
    if cut {
        cfg.profiles[from_profile].pages[from_page].keys[from] = DockKey::default();
    }
    cfg.profiles[to_profile].pages[to_page].keys[to_cell] = if cut { key } else { key.copied() };
    let to = &cfg.profiles[to_profile];
    Ok(changed(json!({ "control": control_name(to.pad, to, to_cell) })))
}

/// Carries out a command on the settings. Nothing here touches a pad or the
/// disk, so anything it does can be checked without either.
pub fn apply(cfg: &mut Config, data: &Value) -> Result<Done, String> {
    let command = text(data, "command")?;
    let programs = cfg.ha_control_programs;
    match command {
        "state" => Ok(Done { reply: state(cfg), ..Done::default() }),
        "show_profile" => {
            let index = profile_of(cfg, data)?;
            text(data, "profile")?;
            cfg.run_profile(index);
            Ok(changed(json!({ "profile": cfg.profiles[index].name })))
        }
        "next_profile" | "previous_profile" | "first_profile" => {
            let pad_id = pad_of(cfg, data)?;
            let index = match command {
                "next_profile" => cfg.profile_along_pad(&pad_id, 1),
                "previous_profile" => cfg.profile_along_pad(&pad_id, -1),
                _ => cfg.profiles_of(&pad_id).first().copied(),
            };
            let index = index.ok_or(format!("The {} pad isn't in use yet", pad_name(cfg, &pad_id)))?;
            cfg.run_profile(index);
            Ok(changed(json!({ "profile": cfg.profiles[index].name })))
        }
        "add_profile" => add_profile(cfg, data),
        "rename_profile" => {
            let index = profile_of(cfg, data)?;
            let name = text(data, "name")?;
            if name.trim().is_empty() || cfg.profiles.iter().any(|profile| profile.name == name) {
                return Err("That profile name is empty or already used".into());
            }
            let old = std::mem::replace(&mut cfg.profiles[index].name, name.to_string());
            cfg.rename_profile_links(&old, name);
            Ok(changed(json!({ "profile": name })))
        }
        "duplicate_profile" => {
            let index = profile_of(cfg, data)?;
            let wanted = data.get("name").and_then(Value::as_str);
            let mut copy = cfg.profiles[index].clone();
            copy.name = cfg.free_name(wanted.unwrap_or(&format!("{} copy", copy.name)));
            let name = copy.name.clone();
            cfg.profiles.push(copy);
            Ok(changed(json!({ "profile": name })))
        }
        "delete_profile" => {
            text(data, "profile")?;
            delete_profile(cfg, data)
        }
        "show_page" | "next_page" | "previous_page" => {
            let index = profile_of(cfg, data)?;
            turn_page(&mut cfg.profiles[index], command, data)?;
            Ok(changed(json!({ "page": cfg.profiles[index].active_page + 1 })))
        }
        "add_page" => {
            let index = profile_of(cfg, data)?;
            let shown = cfg.profiles[index].active_page;
            cfg.profiles[index].add_page();
            cfg.profiles[index].active_page = shown;
            Ok(changed(json!({ "page": cfg.profiles[index].pages.len() })))
        }
        "delete_page" => {
            let index = profile_of(cfg, data)?;
            let profile = &mut cfg.profiles[index];
            number(data, "page")?;
            let page = page_of(profile, data)?;
            if profile.pages.len() < 2 {
                return Err("A profile keeps at least one page".into());
            }
            profile.pages.remove(page);
            profile.repair();
            Ok(changed(json!({ "pages": profile.pages.len() })))
        }
        "get_key" => {
            let profile = &cfg.profiles[profile_of(cfg, data)?];
            let page = page_of(profile, data)?;
            let cell = profile.owner_of(page, cell_of(profile, data, "key")?);
            Ok(Done { reply: key_summary(profile, page, cell), ..Done::default() })
        }
        "set_key" | "add_key" => {
            let index = profile_of(cfg, data)?;
            let profile = &mut cfg.profiles[index];
            let page = page_of(profile, data)?;
            let cell = if command == "add_key" {
                let count = profile.key_count();
                profile.free_cell(page, count).ok_or("There's no free key on this page")?
            } else {
                profile.owner_of(page, cell_of(profile, data, "key")?)
            };
            let changes = data.get("set").and_then(Value::as_object).ok_or("This needs \"set\", with what to change")?;
            profile.pages[page].keys[cell] = with_changes(&profile.pages[page].keys[cell], changes, programs)?;
            Ok(changed(key_summary(profile, page, cell)))
        }
        "clear_key" => {
            let index = profile_of(cfg, data)?;
            let profile = &mut cfg.profiles[index];
            let page = page_of(profile, data)?;
            let cell = profile.owner_of(page, cell_of(profile, data, "key")?);
            profile.pages[page].keys[cell] = DockKey::default();
            Ok(changed(json!({ "control": control_name(profile.pad, profile, cell) })))
        }
        "swap_keys" => {
            let index = profile_of(cfg, data)?;
            let profile = &mut cfg.profiles[index];
            let page = page_of(profile, data)?;
            let (first, second) = (cell_of(profile, data, "key")?, cell_of(profile, data, "with")?);
            if profile.cover(page, first).is_some() || profile.cover(page, second).is_some() {
                return Err("A stretched camera can't be swapped this way".into());
            }
            profile.pages[page].keys.swap(first, second);
            Ok(changed(json!({})))
        }
        "copy_key" => copy_key(cfg, data, false),
        "move_key" => copy_key(cfg, data, true),
        "press_key" => {
            let index = profile_of(cfg, data)?;
            let profile = &cfg.profiles[index];
            let page = page_of(profile, data)?;
            let cell = profile.owner_of(page, cell_of(profile, data, "key")?);
            let press = Effect::Press { pad_id: profile.pad_id.clone(), profile: index, page, cell };
            Ok(effect(json!({ "control": control_name(profile.pad, profile, cell) }), press))
        }
        "set_screen" => {
            let (pad_id, kind) = pad_and_kind(cfg, data)?;
            let on = data.get("on").and_then(Value::as_bool).ok_or("This needs \"on\", true or false")?;
            Ok(effect(json!({ "on": on }), Effect::Screen { pad_id, kind, on }))
        }
        "snapshot" => {
            let pad_id = pad_of(cfg, data)?;
            let scale = match data.get("scale") {
                None | Some(Value::Null) => 1,
                Some(scale) => scale.as_u64().filter(|scale| (1..=3).contains(scale)).ok_or("\"scale\" should be 1, 2 or 3")? as u32,
            };
            Ok(effect(Value::Null, Effect::Snapshot { pad_id, scale }))
        }
        "show_alert" => {
            let pad_id = pad_of(cfg, data)?;
            let alert = crate::alerts::from_command(data)?;
            Ok(effect(json!({}), Effect::ShowAlert { pad_id, alert }))
        }
        "clear_alert" => {
            let pad_id = pad_of(cfg, data)?;
            Ok(effect(json!({}), Effect::ClearAlert { pad_id }))
        }
        "show_on_key" => {
            let (pad_id, cell, control) = screen_key_of(cfg, data)?;
            let flash = crate::flashes::from_command(data)?;
            Ok(effect(json!({ "control": control }), Effect::ShowOnKey { pad_id, cell, flash }))
        }
        "clear_on_key" => {
            let pad_id = pad_of(cfg, data)?;
            let named = ["key", "control"].iter().any(|field| !matches!(data.get(*field), None | Some(Value::Null)));
            let cell = if named { Some(screen_key_of(cfg, data)?.1) } else { None };
            Ok(effect(json!({}), Effect::ClearOnKey { pad_id, cell }))
        }
        "set_brightness" => {
            let (pad_id, level) = (pad_of(cfg, data)?, number(data, "level")?);
            if let Some(entry) = cfg.pad_mut(&pad_id) {
                entry.brightness = level.clamp(0, 100) as u8;
            }
            Ok(changed(json!({ "level": level.clamp(0, 100) })))
        }
        "change_brightness" => {
            let pad_id = pad_of(cfg, data)?;
            cfg.adjust_pad_brightness(&pad_id, number(data, "by")?.clamp(-100, 100) as i32);
            Ok(changed(state(cfg)))
        }
        "mini_monitor" => {
            let pad_id = pad_of(cfg, data)?;
            let kind = cfg.pad(&pad_id).map_or(PadKind::Main, |entry| entry.kind);
            if !kind.has_mini_monitor() {
                return Err("A virtual pad can't show Mini Monitor".into());
            }
            if cfg.pad_profile_of(&pad_id).is_none() {
                return Err(format!("The {} pad isn't in use yet", pad_name(cfg, &pad_id)));
            }
            cfg.monitor_pad = kind;
            cfg.mini_monitor = data.get("on").and_then(Value::as_bool).ok_or("This needs \"on\", true or false")?;
            Ok(changed(json!({ "on": cfg.mini_monitor })))
        }
        "mouse" => {
            if !cfg.ha_mouse {
                return Err(crate::remote_mouse::MOUSE_OFF.into());
            }
            let pad_id = pad_of(cfg, data)?;
            let kind = cfg.pad(&pad_id).map_or(PadKind::Main, |entry| entry.kind);
            let request = crate::remote_mouse::request_of(data)?;
            Ok(effect(json!({}), Effect::Mouse { kind, request }))
        }
        other => Err(format!("There's no command called \"{other}\"")),
    }
}

/// Carries out a command from Home Assistant, by whichever route it came,
/// and gives what to answer: applies it to the settings, saves them, and then
/// does what's left, such as pressing a key or turning a screen off.
/// `source` says who a key press is for.
pub fn carry_out(shared: &Arc<Shared>, data: &Value, source: PressSource) -> Result<Value, String> {
    let command = data.get("command").and_then(Value::as_str).unwrap_or("nothing").to_string();
    let outcome = {
        let mut cfg = lock(&shared.cfg);
        if cfg.ha_control || READING.contains(&command.as_str()) {
            let outcome = apply(&mut cfg, data);
            cfg.repair();
            outcome
        } else {
            Err(CONTROL_OFF.to_string())
        }
    };
    let outcome = outcome.and_then(|done| after_command(shared, done, source));
    match &outcome {
        // A card's mouse sends many moves a second: not worth a line each.
        Ok(_) if command == "mouse" => {}
        Ok(_) => note(&format!("Home Assistant command: {command}")),
        Err(problem) => note(&format!("Home Assistant command {command} refused: {problem}")),
    }
    outcome
}

/// What every command does once the settings have taken it.
fn after_command(shared: &Arc<Shared>, done: Done, source: PressSource) -> Result<Value, String> {
    if done.changed {
        shared.save_now();
        crate::tray::reload();
        crate::pad::wake();
        shared.repaint();
    }
    let mut reply = done.reply;
    match done.effect {
        None => {}
        Some(Effect::Press { pad_id, profile, page, cell }) => {
            if actions::press(shared, &pad_id, cell, Some((profile, page)), source)? {
                reply["dismissed"] = json!(true);
            }
        }
        Some(Effect::Screen { pad_id, kind, on }) => {
            crate::screen_off::set_screen(&pad_id, kind, on);
            crate::pad::wake();
            shared.touch();
        }
        Some(Effect::Snapshot { pad_id, scale }) => reply = crate::bridge::snapshot(shared, &pad_id, scale)?,
        Some(Effect::ShowAlert { pad_id, alert }) => crate::alerts::show(shared, &pad_id, alert),
        Some(Effect::ClearAlert { pad_id }) => {
            crate::alerts::clear(shared, &pad_id);
        }
        Some(Effect::ShowOnKey { pad_id, cell, flash }) => crate::flashes::show(shared, &pad_id, cell, flash)?,
        Some(Effect::ClearOnKey { pad_id, cell }) => {
            reply = json!({ "cleared": crate::flashes::clear(shared, &pad_id, cell) });
        }
        Some(Effect::Mouse { kind, request }) => {
            // Only from Home Assistant: on this PC, the person has the mouse itself.
            if source.is_local() {
                return Err("The mouse command only comes from Home Assistant".into());
            }
            crate::remote_mouse::act(kind, request)?;
        }
    }
    Ok(reply)
}

/// A command event waiting its turn: for whom, what it says, and which of
/// `command_events` it came under.
type Waiting = (Weak<Shared>, Value, usize);

/// The thread that carries out command events, one at a time in the order
/// they came, started with the first.
static WORKER: Mutex<Option<Sender<Waiting>>> = Mutex::new(None);

/// Hands a command event to the worker thread. Some commands take a while
/// (a `snapshot` draws and encodes a picture), and the connection's own
/// thread has to keep reading Home Assistant meanwhile.
pub fn hand_over(shared: &Arc<Shared>, data: Value, name: usize) {
    let mut worker = lock(&WORKER);
    let sender = worker.get_or_insert_with(start_worker);
    if let Err(lost) = sender.send((Arc::downgrade(shared), data, name)) {
        // The thread can only have gone if a command panicked: start another.
        let sender = worker.insert(start_worker());
        let _ = sender.send(lost.0);
    }
}

fn start_worker() -> Sender<Waiting> {
    let (sender, receiver) = channel::<Waiting>();
    std::thread::spawn(move || {
        for (shared, data, name) in receiver {
            if let Some(shared) = shared.upgrade() {
                received(&shared, &data, name);
            }
        }
    });
    sender
}

/// Handles a command event, and sends the answer back as an event.
/// `name` says which of `command_events` it came under, so it's answered alike.
pub fn received(shared: &Arc<Shared>, data: &Value, name: usize) {
    let command = data.get("command").and_then(Value::as_str).unwrap_or("nothing").to_string();
    let mut reply = json!({ "command": command, "id": data.get("id").cloned().unwrap_or(Value::Null) });
    // An event can claim anything about where it came from, so its presses
    // always count as Home Assistant's own.
    match carry_out(shared, data, PressSource::Action) {
        Ok(result) => {
            reply["ok"] = json!(true);
            reply["result"] = result;
        }
        Err(problem) => {
            reply["ok"] = json!(false);
            reply["error"] = json!(problem);
        }
    }
    let answer = match name {
        0 => RESULT_EVENT,
        old => OLD_EVENTS[(old - 1).min(OLD_EVENTS.len() - 1)].1,
    };
    shared.ha.send(json!({ "type": "fire_event", "event_type": answer, "event_data": reply }));
}

/// Tells Home Assistant that a control on a pad was used.
pub fn announce(shared: &Shared, pad_id: &str, cell: usize) {
    let event = {
        let cfg = lock(&shared.cfg);
        let Some(profile) = cfg.pad_profile_of(pad_id).map(|i| &cfg.profiles[i]).filter(|_| cfg.ha_control) else { return };
        let title = profile.pages[profile.active_page].keys.get(cell).map_or("", |key| key.title.as_str());
        let mut event = json!({
            "pad": pad_name(&cfg, pad_id),
            "pad_id": pad_id,
            "profile": profile.name,
            "page": profile.active_page + 1,
            "control": control_name(profile.pad, profile, cell),
            "title": title.replace('\n', " "),
        });
        // Where a screen key is, counted from 0 across and down.
        if profile.pad.cell_has_screen(cell) {
            event["column"] = json!(cell % profile.cols);
            event["row"] = json!(cell / profile.cols);
        }
        event
    };
    shared.ha.send(json!({ "type": "fire_event", "event_type": PRESS_EVENT, "event_data": event }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::small_cell_has_screen;

    fn both_pads() -> Config {
        let mut cfg = Config::default();
        cfg.profiles[0].name = "Main".into();
        cfg.add_small_pad("Small");
        cfg.choose_profile(0);
        cfg
    }

    #[test]
    fn a_press_says_where_a_screen_key_is_as_the_original_does() {
        let mut cfg = both_pads();
        cfg.ha_control = true;
        let (shared, _opened) = Shared::new(cfg, true);
        let mut sent = shared.ha.dispatch.open();
        let place = |message: &Value| (message["event_data"]["column"].clone(), message["event_data"]["row"].clone());
        // Each press goes out once, under the current name.
        let id = |kind| lock(&shared.cfg).pad_of_kind(kind).unwrap().id.clone();
        let (main, small) = (id(PadKind::Main), id(PadKind::Akp03e));
        let mut pressed = |pad_id: &str, cell| {
            announce(&shared, pad_id, cell);
            let message = sent.try_recv().unwrap().into_message();
            assert_eq!(message["event_type"], PRESS_EVENT);
            assert!(sent.try_recv().is_err(), "only one event for a press");
            message
        };
        assert_eq!(place(&pressed(&main, 7)), (json!(2), json!(1)));
        assert_eq!(place(&pressed(&small, 6)), (json!(0), json!(1)));
        let knob = pressed(&small, 16);
        assert!(knob["event_data"].get("column").is_none());
        assert_eq!((knob["event_data"]["pad"].clone(), knob["event_data"]["control"].clone()), (json!("akp03e"), json!("top_knob_press")));
        assert_eq!(knob["event_data"]["pad_id"], json!(small));
    }

    #[test]
    fn every_control_has_a_name_that_leads_back_to_it() {
        let cfg = both_pads();
        let small = &cfg.profiles[1];
        for cell in 0..18 {
            let name = control_name(PadKind::Akp03e, small, cell);
            assert_eq!(cell_of_name(PadKind::Akp03e, small, &name), Some(cell), "{name}");
        }
        assert_eq!(control_name(PadKind::Akp03e, small, 16), "top_knob_press");
        assert_eq!(control_name(PadKind::Akp03e, small, 3), "lower_left_knob_left");
        assert_eq!(control_name(PadKind::Akp03e, small, 12), "button_1");
        assert_eq!(control_name(PadKind::Akp03e, small, 6), "key_4");
        assert_eq!(control_name(PadKind::Main, &cfg.profiles[0], 14), "key_15");
        assert!(small_cell_has_screen(cell_of_name(PadKind::Akp03e, &cfg.profiles[1], "key_6").unwrap()));
    }

    #[test]
    fn unknown_controls_and_commands_are_refused() {
        let mut cfg = both_pads();
        assert!(cell_of_name(PadKind::Main, &cfg.profiles[0], "key_16").is_none());
        assert!(cell_of_name(PadKind::Main, &cfg.profiles[0], "button_1").is_none());
        assert!(cell_of_name(PadKind::Akp03e, &cfg.profiles[1], "key_7").is_none());
        assert!(apply(&mut cfg, &json!({"command": "explode"})).unwrap_err().contains("explode"));
        assert!(apply(&mut cfg, &json!({})).is_err());
        assert!(apply(&mut cfg, &json!({"command": "state", "pad": "nope"})).is_ok());
        assert!(apply(&mut cfg, &json!({"command": "next_page", "pad": "nope"})).is_err());
    }

    #[test]
    fn profiles_can_be_added_shown_renamed_copied_and_deleted() {
        let mut cfg = both_pads();
        apply(&mut cfg, &json!({"command": "add_profile", "name": "Games"})).unwrap();
        assert_eq!(apply(&mut cfg, &json!({"command": "show_profile", "profile": "Games"})).unwrap().changed, true);
        assert_eq!(cfg.main_profile().name, "Games");
        apply(&mut cfg, &json!({"command": "rename_profile", "name": "Play"})).unwrap();
        assert_eq!(cfg.main_profile().name, "Play");
        apply(&mut cfg, &json!({"command": "duplicate_profile", "name": "Play two"})).unwrap();
        assert!(cfg.profiles.iter().any(|profile| profile.name == "Play two"));
        apply(&mut cfg, &json!({"command": "delete_profile", "profile": "Play two"})).unwrap();
        assert!(!cfg.profiles.iter().any(|profile| profile.name == "Play two"));
        assert!(apply(&mut cfg, &json!({"command": "show_profile", "profile": "Nope"})).is_err());
    }

    #[test]
    fn the_last_profile_and_page_stay() {
        let mut cfg = both_pads();
        assert!(apply(&mut cfg, &json!({"command": "delete_profile", "profile": "Main"})).is_err());
        assert!(apply(&mut cfg, &json!({"command": "delete_page", "page": 1})).is_err());
        assert!(apply(&mut cfg, &json!({"command": "delete_profile", "pad": "akp03e", "profile": "Small"})).is_ok());
        assert_eq!(cfg.small_pad_index(), None);
    }

    #[test]
    fn each_command_stays_on_its_own_pad() {
        let mut cfg = both_pads();
        apply(&mut cfg, &json!({"command": "add_page", "pad": "akp03e"})).unwrap();
        apply(&mut cfg, &json!({"command": "next_page", "pad": "akp03e"})).unwrap();
        assert_eq!((cfg.profiles[0].active_page, cfg.profiles[1].active_page), (0, 1));
        apply(&mut cfg, &json!({"command": "set_brightness", "pad": "akp03e", "level": 80})).unwrap();
        let levels = |cfg: &Config| (cfg.brightness_of(PadKind::Main), cfg.brightness_of(PadKind::Akp03e));
        assert_eq!(levels(&cfg), (25, 80));
        apply(&mut cfg, &json!({"command": "change_brightness", "by": 5})).unwrap();
        assert_eq!(levels(&cfg), (30, 80));
    }

    #[test]
    fn keys_can_be_set_up_a_field_at_a_time() {
        let mut cfg = both_pads();
        let set = json!({"command": "set_key", "key": 3, "set": {"title": "Door", "color": [10, 20, 30]}});
        apply(&mut cfg, &set).unwrap();
        apply(&mut cfg, &json!({"command": "set_key", "key": 3, "set": {"glyph": "mdi:door"}})).unwrap();
        let key = &cfg.profiles[0].pages[0].keys[2];
        assert_eq!((key.title.as_str(), key.color, key.glyph.as_str()), ("Door", [10, 20, 30], "mdi:door"));
        let found = apply(&mut cfg, &json!({"command": "get_key", "key": 3})).unwrap();
        assert_eq!(found.reply["key"]["title"], "Door");
        apply(&mut cfg, &json!({"command": "clear_key", "key": 3})).unwrap();
        assert!(cfg.profiles[0].pages[0].keys[2].is_empty());
    }

    #[test]
    fn a_new_key_goes_in_the_first_free_place() {
        let mut cfg = both_pads();
        cfg.profiles[0].pages[0].keys[0].title = "Taken".into();
        let done = apply(&mut cfg, &json!({"command": "add_key", "set": {"title": "New"}})).unwrap();
        assert_eq!(done.reply["control"], "key_2");
    }

    #[test]
    fn knobs_can_be_set_up_by_name() {
        let mut cfg = both_pads();
        let set = json!({"command": "set_key", "pad": "akp03e", "control": "top_knob_press",
            "set": {"title": "Scene", "actions": [{"kind": "home_assistant", "entity": "scene.movie", "service": "turn_on"}]}});
        apply(&mut cfg, &set).unwrap();
        assert_eq!(cfg.profiles[1].pages[0].keys[16].title, "Scene");
        assert_eq!(cfg.profiles[1].pages[0].keys[16].actions.len(), 1);
    }

    #[test]
    fn keys_that_act_on_the_pc_need_the_second_switch() {
        let mut cfg = both_pads();
        let set = json!({"command": "set_key", "key": 1, "set": {"actions": [{"kind": "open", "target": "calc.exe"}]}});
        assert!(apply(&mut cfg, &set).unwrap_err().contains("settings"));
        assert!(cfg.profiles[0].pages[0].keys[0].actions.is_empty());
        cfg.ha_control_programs = true;
        apply(&mut cfg, &set).unwrap();
        assert_eq!(cfg.profiles[0].pages[0].keys[0].actions.len(), 1);
        // Changing the title of such a key isn't adding one.
        cfg.ha_control_programs = false;
        apply(&mut cfg, &json!({"command": "set_key", "key": 1, "set": {"title": "Calc"}})).unwrap();
    }

    #[test]
    fn a_bad_key_changes_nothing() {
        let mut cfg = both_pads();
        let before = cfg.clone();
        let set = json!({"command": "set_key", "key": 1, "set": {"color": "green"}});
        assert!(apply(&mut cfg, &set).is_err());
        assert_eq!(cfg, before);
    }

    #[test]
    fn keys_travel_between_pads() {
        let mut cfg = both_pads();
        cfg.profiles[0].pages[0].keys[0] = DockKey { title: "Mic".into(), hotkey: "Ctrl+Alt+1".into(), ..DockKey::default() };
        let copy = json!({"command": "copy_key", "key": 1, "to": {"pad": "akp03e", "key": 2}});
        apply(&mut cfg, &copy).unwrap();
        assert_eq!(cfg.profiles[1].pages[0].keys[1].title, "Mic");
        assert!(cfg.profiles[1].pages[0].keys[1].hotkey.is_empty());
        let moved = json!({"command": "move_key", "key": 1, "to": {"key": 5}});
        apply(&mut cfg, &moved).unwrap();
        assert!(cfg.profiles[0].pages[0].keys[0].is_empty());
        assert_eq!(cfg.profiles[0].pages[0].keys[4].hotkey, "Ctrl+Alt+1");
    }

    #[test]
    fn pressing_a_key_asks_for_its_steps_to_run() {
        let mut cfg = both_pads();
        cfg.profiles[0].pages[0].keys[1].title = "Go".into();
        cfg.profiles[0].pages[0].keys[1].actions = vec![Action::NextPage];
        let done = apply(&mut cfg, &json!({"command": "press_key", "key": 2})).unwrap();
        let pad_id = cfg.pads[0].id.clone();
        assert_eq!(done.effect, Some(Effect::Press { pad_id, profile: 0, page: 0, cell: 1 }));
        assert!(!done.changed);
    }

    #[test]
    fn mini_monitor_needs_a_pad_in_use() {
        let mut cfg = Config::default();
        assert!(apply(&mut cfg, &json!({"command": "mini_monitor", "pad": "akp03e", "on": true})).is_err());
        apply(&mut cfg, &json!({"command": "mini_monitor", "on": true})).unwrap();
        assert!(cfg.monitor_on(PadKind::Main));
    }

    #[test]
    fn the_state_lists_both_pads() {
        let mut cfg = both_pads();
        let done = apply(&mut cfg, &json!({"command": "state"})).unwrap();
        assert_eq!(done.reply["main"]["profiles"][0]["name"], "Main");
        assert_eq!(done.reply["akp03e"]["profiles"][0]["running"], true);
        assert!(!done.changed);
    }

    /// The 293S with "Work", "Home" and "Games", running "Games", a dashboard
    /// pad with "Hall", "Morning" and "Night", running "Night", and the window
    /// on the AKP03E.
    fn three_pads_each_running_their_second_profile() -> (Config, String) {
        let mut cfg = both_pads();
        cfg.profiles[0].name = "Work".into();
        for name in ["Home", "Games"] {
            let profile = cfg.new_profile_for(&cfg.pads[0].id.clone(), name).unwrap();
            cfg.profiles.push(profile);
        }
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        let first = cfg.pad_profile_of(&wall).unwrap();
        cfg.profiles[first].name = "Hall".into();
        for name in ["Morning", "Night"] {
            apply(&mut cfg, &json!({"command": "add_profile", "pad": wall, "name": name})).unwrap();
        }
        cfg.repair();
        for name in ["Games", "Night"] {
            let index = cfg.profiles.iter().position(|p| p.name == name).unwrap();
            cfg.run_profile(index);
        }
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        cfg.show_pad_id(&small);
        (cfg, wall)
    }

    fn running(cfg: &Config, pad_id: &str) -> String {
        cfg.profiles[cfg.pad_profile_of(pad_id).unwrap()].name.clone()
    }

    #[test]
    fn deleting_a_profile_leaves_every_pad_on_what_it_runs() {
        let (mut cfg, wall) = three_pads_each_running_their_second_profile();
        let main = cfg.pads[0].id.clone();
        assert_eq!((running(&cfg, &main), running(&cfg, &wall), cfg.profile().name.clone()), ("Games".into(), "Night".into(), "Small".into()));
        apply(&mut cfg, &json!({"command": "delete_profile", "profile": "Work"})).unwrap();
        assert_eq!((running(&cfg, &main), running(&cfg, &wall), cfg.profile().name.clone()), ("Games".into(), "Night".into(), "Small".into()));
        apply(&mut cfg, &json!({"command": "delete_profile", "pad": wall, "profile": "Hall"})).unwrap();
        assert_eq!((running(&cfg, &main), running(&cfg, &wall), cfg.profile().name.clone()), ("Games".into(), "Night".into(), "Small".into()));
        // The window's own profile going leaves it on the same pad.
        let night = cfg.profiles.iter().position(|p| p.name == "Night").unwrap();
        cfg.choose_profile(night);
        apply(&mut cfg, &json!({"command": "delete_profile", "pad": wall, "profile": "Night"})).unwrap();
        assert_eq!((running(&cfg, &main), cfg.profile().name.clone(), cfg.shown_pad_id()), ("Games".into(), "Morning".into(), wall));
    }

    #[test]
    fn command_events_are_carried_out_off_the_connection_in_the_order_they_came() {
        let (shared, _opened) = Shared::new(both_pads(), true);
        let mut sent = shared.ha.dispatch.open();
        // While a command is held up, here by the settings being in use,
        // handing more over still returns at once.
        let busy = lock(&shared.cfg);
        let (done, finished) = channel();
        let handler = shared.clone();
        std::thread::spawn(move || {
            for id in 1..=3 {
                hand_over(&handler, json!({"command": "state", "id": id}), 0);
            }
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(std::time::Duration::from_secs(2)).is_ok(), "the handler waited for the command");
        drop(busy);
        let mut answered = Vec::new();
        let started = std::time::Instant::now();
        while answered.len() < 3 && started.elapsed() < std::time::Duration::from_secs(5) {
            match sent.try_recv() {
                Ok(message) => {
                    let message = message.into_message();
                    assert_eq!(message["event_type"], RESULT_EVENT);
                    answered.push(message["event_data"]["id"].clone());
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        assert_eq!(answered, [json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn an_akp03e_with_no_profiles_left_is_like_one_never_added() {
        let mut cfg = both_pads();
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        apply(&mut cfg, &json!({"command": "delete_profile", "pad": "akp03e", "profile": "Small"})).unwrap();
        let done = apply(&mut cfg, &json!({"command": "state"})).unwrap();
        assert_eq!(done.reply["akp03e"]["profiles"], json!([]));
        assert_eq!(done.reply["pads"].as_array().unwrap().len(), 1);
        for pad in [json!("akp03e"), json!(small)] {
            let problem = apply(&mut cfg, &json!({"command": "set_brightness", "pad": pad, "level": 60})).unwrap_err();
            assert!(problem.contains("isn't in use"), "{problem}");
        }
    }

    #[test]
    fn a_pad_can_be_named_by_its_id() {
        let mut cfg = both_pads();
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        cfg.choose_profile(0);
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        apply(&mut cfg, &json!({"command": "set_key", "pad": wall, "key": 8, "set": {"title": "Lamp"}})).unwrap();
        let index = cfg.pad_profile_of(&wall).unwrap();
        assert_eq!(cfg.profiles[index].pages[0].keys[7].title, "Lamp");
        assert!(apply(&mut cfg, &json!({"command": "set_key", "pad": wall, "key": 9, "set": {}})).is_err());
        // An id and the old name mean the same pad.
        apply(&mut cfg, &json!({"command": "set_brightness", "pad": small, "level": 60})).unwrap();
        assert_eq!(cfg.brightness_of(PadKind::Akp03e), 60);
        let done = apply(&mut cfg, &json!({"command": "add_profile", "pad": wall, "name": "Evening"})).unwrap();
        assert_eq!(done.reply["profile"], "Evening");
        let evening = cfg.profiles.iter().find(|p| p.name == "Evening").unwrap();
        assert_eq!((evening.pad, evening.pad_id.as_str(), evening.key_count()), (PadKind::Dashboard, wall.as_str(), 8));
        apply(&mut cfg, &json!({"command": "next_profile", "pad": wall})).unwrap();
        assert_eq!(cfg.profiles[cfg.pad_profile_of(&wall).unwrap()].name, "Evening");
        assert!(apply(&mut cfg, &json!({"command": "next_page", "pad": "0123456789abcdef"})).unwrap_err().contains("0123456789abcdef"));
    }

    #[test]
    fn a_dashboard_pad_keeps_a_profile_and_has_no_mini_monitor() {
        let mut cfg = both_pads();
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        let only = cfg.profiles[cfg.pad_profile_of(&wall).unwrap()].name.clone();
        assert!(apply(&mut cfg, &json!({"command": "delete_profile", "pad": wall, "profile": only})).is_err());
        assert!(apply(&mut cfg, &json!({"command": "mini_monitor", "pad": wall, "on": true})).is_err());
        assert!(!cfg.mini_monitor);
    }

    #[test]
    fn the_state_lists_every_pad_with_its_id() {
        let mut cfg = both_pads();
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        let done = apply(&mut cfg, &json!({"command": "state"})).unwrap();
        let pads = done.reply["pads"].as_array().unwrap();
        assert_eq!(pads.len(), 3);
        assert_eq!(pads[0]["kind"], "main");
        assert_eq!(pads[1]["mini_monitor"], false);
        assert_eq!((pads[2]["id"].clone(), pads[2]["kind"].clone(), pads[2]["name"].clone()), (json!(wall), json!("dashboard"), json!("Wall")));
        assert_eq!(pads[2]["mini_monitor"], Value::Null);
        assert_eq!(pads[2]["profiles"][0]["running"], true);
        // The fields from before pad ids are still there.
        assert_eq!(done.reply["main"]["profiles"][0]["name"], "Main");
    }

    #[test]
    fn a_press_on_a_dashboard_pad_says_which_one() {
        let mut cfg = both_pads();
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        cfg.ha_control = true;
        let (shared, _opened) = Shared::new(cfg, true);
        let mut sent = shared.ha.dispatch.open();
        announce(&shared, &wall, 5);
        let event = sent.try_recv().unwrap().into_message()["event_data"].clone();
        assert_eq!((event["pad"].clone(), event["pad_id"].clone(), event["control"].clone()), (json!(wall), json!(wall), json!("key_6")));
        assert_eq!((event["column"].clone(), event["row"].clone()), (json!(1), json!(1)));
    }

    fn calculator() -> DockKey {
        DockKey { title: "Calc".into(), actions: vec![Action::Open { target: "calc.exe".into(), args: String::new() }], ..DockKey::default() }
    }

    #[test]
    fn home_assistant_may_press_only_what_the_key_allows() {
        let mut cfg = both_pads();
        cfg.ha_control = true;
        let plain = DockKey { actions: vec![Action::NextPage], ..DockKey::default() };
        let pc = calculator();
        for source in [PressSource::Dashboard, PressSource::Action] {
            assert!(may_press(&cfg, &plain, source).is_ok());
            assert!(may_press(&cfg, &pc, source).unwrap_err().contains("isn't allowed"));
            assert!(may_press(&cfg, &DockKey { remote: RemoteAccess::Allow, ..pc.clone() }, source).is_ok());
            assert!(may_press(&cfg, &DockKey { remote: RemoteAccess::Deny, ..plain.clone() }, source).is_err());
        }
        // The person's own presses always may.
        for source in [PressSource::Pad, PressSource::Screen, PressSource::Hotkey] {
            assert!(may_press(&cfg, &DockKey { remote: RemoteAccess::Deny, ..pc.clone() }, source).is_ok());
        }
        // And with control off, Home Assistant can press nothing.
        cfg.ha_control = false;
        assert_eq!(may_press(&cfg, &plain, PressSource::Dashboard).unwrap_err(), CONTROL_OFF);
    }

    #[test]
    fn home_assistant_cant_choose_what_it_may_press() {
        let mut cfg = both_pads();
        cfg.ha_control_programs = true;
        cfg.profiles[0].pages[0].keys[0] = DockKey { remote: RemoteAccess::Allow, ..calculator() };
        let sneaky = json!({"command": "set_key", "key": 1, "set": {"remote": "allow"}});
        assert!(apply(&mut cfg, &sneaky).unwrap_err().contains("Only the person"));
        // A new title keeps the person's choice; new steps don't.
        apply(&mut cfg, &json!({"command": "set_key", "key": 1, "set": {"title": "Calculator"}})).unwrap();
        assert_eq!(cfg.profiles[0].pages[0].keys[0].remote, RemoteAccess::Allow);
        let steps = json!({"command": "set_key", "key": 1, "set": {"actions": [{"kind": "open", "target": "notepad.exe"}]}});
        apply(&mut cfg, &steps).unwrap();
        assert_eq!(cfg.profiles[0].pages[0].keys[0].remote, RemoteAccess::Auto);
        // The same steps again aren't a change.
        cfg.profiles[0].pages[0].keys[0].remote = RemoteAccess::Allow;
        apply(&mut cfg, &steps).unwrap();
        assert_eq!(cfg.profiles[0].pages[0].keys[0].remote, RemoteAccess::Allow);
    }

    #[test]
    fn the_mouse_needs_its_own_switch_and_reads_where_to_go() {
        let mut cfg = both_pads();
        cfg.ha_control = true;
        let click = json!({"command": "mouse", "pad": "main", "action": "click", "x": 0.25, "y": 0.75});
        assert_eq!(apply(&mut cfg, &click).unwrap_err(), crate::remote_mouse::MOUSE_OFF);
        cfg.ha_mouse = true;
        let done = apply(&mut cfg, &click).unwrap();
        assert!(!done.changed, "the mouse changes no setting");
        let Some(Effect::Mouse { kind, request }) = done.effect else { panic!("not a mouse effect") };
        assert_eq!(kind, PadKind::Main);
        assert_eq!(request.at, (0.25, 0.75));
        let wrong = json!({"command": "mouse", "pad": "main", "action": "click", "x": 2, "y": 0});
        assert!(apply(&mut cfg, &wrong).is_err());
    }

    #[test]
    fn the_choice_is_saved_only_when_it_isnt_automatic() {
        let auto = serde_json::to_value(DockKey::default()).unwrap();
        assert!(auto.get("remote").is_none());
        let never = serde_json::to_value(DockKey { remote: RemoteAccess::Deny, ..DockKey::default() }).unwrap();
        assert_eq!(never["remote"], "deny");
        // Keys saved before the choice existed are automatic.
        let old: DockKey = serde_json::from_value(json!({"title": "Old", "actions": []})).unwrap();
        assert_eq!(old.remote, RemoteAccess::Auto);
        let back: DockKey = serde_json::from_value(never).unwrap();
        assert_eq!(back.remote, RemoteAccess::Deny);
    }

    #[test]
    fn an_event_can_only_press_what_the_key_allows_and_reads_need_no_control() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-remote-test.json"));
        let mut cfg = both_pads();
        cfg.ha_control = true;
        cfg.profiles[0].pages[0].keys[0] = calculator();
        let (shared, _opened) = Shared::new(cfg, true);
        // An event saying it's the pad is still Home Assistant's press.
        let press = json!({"command": "press_key", "key": 1, "source": "pad"});
        assert!(carry_out(&shared, &press, PressSource::Action).unwrap_err().contains("isn't allowed"));
        lock(&shared.cfg).profiles[0].pages[0].keys[0].remote = RemoteAccess::Deny;
        assert!(carry_out(&shared, &press, PressSource::Action).is_err());
        assert!(lock(&shared.last_press).is_none(), "a refused press isn't a press");
        lock(&shared.cfg).ha_control = false;
        assert!(carry_out(&shared, &json!({"command": "state"}), PressSource::Action).is_ok());
        assert!(carry_out(&shared, &json!({"command": "get_key", "key": 1}), PressSource::Action).is_ok());
        let snapshot = carry_out(&shared, &json!({"command": "snapshot"}), PressSource::Action).unwrap();
        assert_eq!(snapshot["content_type"], "image/jpeg");
        assert_eq!(carry_out(&shared, &json!({"command": "next_page"}), PressSource::Action).unwrap_err(), CONTROL_OFF);
        assert_eq!(carry_out(&shared, &json!({"command": "clear_alert"}), PressSource::Action).unwrap_err(), CONTROL_OFF);
    }

    #[test]
    fn alerts_and_screens_are_commands_too() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-remote-test.json"));
        let mut cfg = both_pads();
        cfg.ha_control = true;
        let wall = cfg.add_dashboard_pad("Wall", 2, 2);
        let (shared, _opened) = Shared::new(cfg, true);
        let alert = json!({"command": "show_alert", "pad": wall, "title": "Door", "color": [200, 0, 0]});
        carry_out(&shared, &alert, PressSource::Action).unwrap();
        assert_eq!(crate::alerts::active(&shared, &wall).unwrap().color, [200, 0, 0]);
        assert!(carry_out(&shared, &json!({"command": "show_alert", "pad": wall, "color": "red"}), PressSource::Action).is_err());
        carry_out(&shared, &json!({"command": "clear_alert", "pad": wall}), PressSource::Action).unwrap();
        assert!(crate::alerts::active(&shared, &wall).is_none());
        let off = carry_out(&shared, &json!({"command": "set_screen", "pad": wall, "on": false}), PressSource::Action).unwrap();
        assert_eq!(off["on"], false);
        assert!(crate::screen_off::pad_asleep(&lock(&shared.cfg), &wall));
        // A press only wakes it, and says so.
        let pressed = carry_out(&shared, &json!({"command": "press_key", "pad": wall, "key": 1}), PressSource::Dashboard).unwrap();
        assert_eq!(pressed["dismissed"], true);
        assert!(!crate::screen_off::pad_asleep(&lock(&shared.cfg), &wall));
        assert!(lock(&shared.last_press).is_some());
        assert!(apply(&mut lock(&shared.cfg), &json!({"command": "set_screen", "pad": wall})).is_err());
        assert!(apply(&mut lock(&shared.cfg), &json!({"command": "snapshot", "pad": wall, "scale": 4})).is_err());
    }

    #[test]
    fn something_shown_on_a_key_goes_with_a_press_on_that_key_only() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-remote-test.json"));
        let mut cfg = both_pads();
        cfg.ha_control = true;
        let wall = cfg.add_dashboard_pad("Wall", 2, 2);
        let (shared, _opened) = Shared::new(cfg, true);
        let show = |data: Value| carry_out(&shared, &data, PressSource::Action);
        // By number, by number as text, and by name.
        assert_eq!(show(json!({"command": "show_on_key", "pad": wall, "key": 2, "title": "Door"})).unwrap()["control"], "key_2");
        show(json!({"command": "show_on_key", "pad": wall, "key": "3", "title": "Washer"})).unwrap();
        show(json!({"command": "show_on_key", "pad": wall, "control": "key_4", "title": "Rain"})).unwrap();
        assert_eq!(crate::flashes::on_pad(&shared, &wall).len(), 3);
        assert!(show(json!({"command": "show_on_key", "pad": wall, "key": 9})).is_err());
        assert!(show(json!({"command": "show_on_key", "pad": wall, "key": 1, "camera": "light.x"})).is_err());
        // A press on that key only takes it away and says so...
        let pressed = carry_out(&shared, &json!({"command": "press_key", "pad": wall, "key": 2}), PressSource::Dashboard).unwrap();
        assert_eq!(pressed["dismissed"], true);
        assert!(crate::flashes::active(&shared, &wall, 1).is_none());
        // ...and a press on another key doesn't.
        let other = carry_out(&shared, &json!({"command": "press_key", "pad": wall, "key": 1}), PressSource::Dashboard).unwrap();
        assert!(other.get("dismissed").is_none());
        assert_eq!(crate::flashes::on_pad(&shared, &wall).len(), 2);
        assert_eq!(show(json!({"command": "clear_on_key", "pad": wall, "key": 3})).unwrap()["cleared"], 1);
        assert_eq!(show(json!({"command": "clear_on_key", "pad": wall})).unwrap()["cleared"], 1);
        // It needs control on, as alerts do.
        lock(&shared.cfg).ha_control = false;
        assert!(show(json!({"command": "show_on_key", "pad": wall, "key": 1})).is_err());
    }

}

# Integration work: app-side design

Developer notes for the work that makes Stream Dock HA talk to its Home Assistant integration. The wire contract is [integration-protocol.md](integration-protocol.md). This file says how the app is reshaped to meet it. Work happens in the phases below, in order. Each phase ends with the full test suite passing and a local commit.

## Working rules (every phase)

- **The person's app is running** from `target\debug\stream-dock-ha.exe`.
  - Never close it. Never run `shots\quit-app.ps1`.
  - Build and test with `--target-dir target/work`.
  - Never use the plain `target/debug` or `target/release` folders.
- **Never touch the real pads.** No test or run may open a USB pad: the person's app owns them. Check things through unit tests and `--screenshot` runs, which don't start the pad drivers.
- **Never read or write the person's real settings** (`%APPDATA%\StreamDockHA`, `%APPDATA%\TrayDock`, `%APPDATA%\StreamDockMax`). Never use their Home Assistant address or token.
- **Match the surrounding code.**
  - Comments are plain English, explain *why*, and talk about "the person" or what someone sees.
  - User-facing text is short and plain.
  - Functions stay small, and new modules follow the folder split already there (`src/model`, `src/render`, `src/ui`, `src/ha`).
- **Keep old settings loading.** Every settings change needs `#[serde(default)]` and a migration test: an old settings JSON in, the right new shape out. Settings written by this version may be unreadable by older versions. That's acceptable, because the app keeps its own backup copy when it can't read a file.
- **Commits.** Commit locally with a plain sentence as the subject, like the existing history. Never add "Co-Authored-By" lines; this is the person's standing rule. Don't push. Before committing, check staged files for addresses, tokens and local paths.
- **Tests.**
  - `cargo test --target-dir target/work` must pass before each commit. The baseline is 248 passed, 15 ignored.
  - New behaviour gets unit tests close to the code, as the codebase already does.
  - UI changes get a `--screenshot` run with made-up settings: see `shots/docs-shots.sh` and the `STREAM_DOCK_HA_SHOT_*` variables in `src/main.rs`.

## Phase 1: pads become instances with ids

Today a pad is a `PadKind` (`Main` = 293S, `Akp03e`). Per-pad settings are split across `Config` fields (`brightness` / `small_brightness`, `gap_*` / `small_gap_*`, `main_face` / `small_face`, `*_face_clear`, `last_main_profile` / `last_small_profile`).

### Target model

```rust
// src/model/pads.rs
pub enum PadKind { Main, Akp03e, Dashboard }   // serde: "main", "akp03e", "dashboard"

pub struct PadEntry {
    pub id: String,          // 16 lowercase hex, random (model::new_id())
    pub kind: PadKind,
    pub name: String,        // "Stream Dock 293S", "AJAZZ AKP03E", "Virtual pad"; editable
    pub brightness: u8,
    pub gaps: [u8; 2],       // across, down, percent of a key
    pub face: [u8; 3],
    pub face_clear: bool,
    pub last_profile: usize, // what last_main_profile / last_small_profile were
}

// Config gains
pub pads: Vec<PadEntry>,
pub instance_id: String,
pub instance_machine: String,
pub pc_name: String,          // empty = use COMPUTERNAME
pub ha_integration: bool,     // default true: report to the integration when it's there

// Profile gains
pub id: String,
pub pad_id: String,           // the PadEntry it belongs to; `pad: PadKind` stays and is kept equal to that entry's kind
```

### Rules

- `Config::repair()` keeps everything consistent:
  - every profile has an `id` and a `pad_id` that names an existing entry;
  - every entry has at least one profile;
  - `profile.pad == entry.kind`;
  - there's at most one entry of kind `Main` and one of kind `Akp03e`, since the drivers handle one of each today;
  - `Dashboard` entries can be any number;
  - ids are unique; a duplicate gets a new id.
- **Migration from old settings:**
  - Make one `Main` entry from the old main fields.
  - Make one `Akp03e` entry from the old small fields if any profile has `pad: akp03e`.
  - Point every profile at the entry of its kind.
  - The old fields are read (with `#[serde(default)]`) and no longer written: use `skip_serializing`.
- **Lookups go by pad id.**
  - `cfg.pad(id)`, `cfg.pad_mut(id)`, `cfg.pad_of_kind(kind) -> Option<&PadEntry>` (the first of that kind), `cfg.pad_profile_of(pad_id)`, `cfg.profiles_of(pad_id)`.
  - The existing `pad_profile(kind)`, `brightness_of(kind)`, `gaps(kind)` and similar stay as thin wrappers over the first entry of that kind, so the drivers (`pad.rs`, `small_pad.rs`) and Mini Monitor change as little as possible.
  - Code that's really about "this pad" (the window's pad tabs, the pad view, pads on screen, remote commands) moves to pad ids.
- `PadKind::Dashboard` behaves like `Main` without hardware, Mini Monitor, band or shortcuts. Its grid is the profile's `rows` × `cols` (1–8 × 1–10, as `Main` already allows). It shows in the window's pad tabs and pads on screen, and can be edited like any pad.
- **Instance id:** `model::machine_hash()` computes the `machine` value of the protocol (section 2). At load:
  - if `instance_id` is empty, make one;
  - if `instance_machine` differs from `machine_hash()`, make a new `instance_id` and new ids for every pad entry, re-pointing profiles.

  `Config::renew_instance()` and `Config::renew_pad(id) -> new_id` do this and are reused by the bridge.
- **Layout as data:** `model::layout::layout_of(entry: &PadEntry, profile: &Profile) -> Layout`. Its fields match the protocol's Layout: controls with name, type, group, x, y, w, h, screen, press, and the cell each maps to.
  - Control names come from it. `remote::control_name` and `cell_of_name` move onto it, with the same names as today: `key_N`, `button_N`, `<knob>_knob_<part>`, `band_N`.
  - Geometry uses the pad's gaps. The AKP03E's buttons and knobs sit where the window's pad view draws them, so read `ui/pad_view.rs` for its proportions.
- **Remote commands:** `pad` accepts a pad id, or the old names `main` and `akp03e` (the first entry of that kind).
  - `state` keeps its old `main` and `akp03e` fields, for existing automations, and adds `pads: [...]` (id, kind, name, profiles, brightness, mini_monitor).
  - Profile names are unique per app today (`free_name`), so commands keep using names.
- **UI** (Pads section, Settings):
  - list the pads with their names (rename in place);
  - **Add a virtual pad** (name, rows, columns);
  - remove a virtual pad, with its profiles, after a confirmation;
  - the window's pad tabs come from `cfg.pads`.

  Keep it compact and in the style of the existing Pads section.

## Phase 2: the shared pieces

1. **Change signal.**
   - `Shared` gains `changes: tokio::sync::watch::Sender<u64>` and `Shared::touch()`, which bumps it and also asks for a repaint.
   - Call `touch()` wherever something the integration reports can change:
     - settings saves (`save_now`, `save_now_with`);
     - HA entity updates (`store_states` and `handle`);
     - pad connection status changes;
     - Mini Monitor and screen-off changes;
     - microphone mute changes;
     - camera pictures;
     - alerts;
     - presses.
   - Stats changing every 3 s may touch too: the bridge throttles.
2. **One producer of what a pad shows** (`src/frames.rs`).
   - `frames::pad_frame(shared, cfg, entities, pad_id) -> PadFrame`, where `PadFrame` holds:
     - `cells: Vec<CellFrame { control: String, cell: usize, look: Option<Look>, hash: u64 }>` for every `screen` control of the layout;
     - `band: Vec<(String, Face, u64)>`;
     - `look: u64`, a hash over everything.
   - It folds in the running profile and page, stretched cameras, the active alert, and the AKP03E style.
   - Every place that builds `Look::of_cell` for a pad today uses it instead: `pad.rs:83`, `small_pad.rs:130`, `ui/pad_view.rs:652`, `ui/virtual_pad.rs:442`. Watch the `face` and `gaps` arguments those callers pass.
   - Behaviour must not change. Existing render, pad and UI tests must still pass, plus a test that the producer gives the same looks the old code did.
3. **Draw at any size.**
   - `render::key_picture_sized(look, px)` and `render::band_picture_sized(face, style, px)`.
   - `key_picture(look)` stays as the 85 px case, and `band_picture` as the 80 px case.
   - Text, icons, switch, meters and pictures scale with the size. `KEY_SIZE` and `BAND_SIZE` stay as the defaults.
   - Test: an 85 px picture is unchanged byte for byte, and a 170 px one has the same layout (compare a downscale).
   - `frames::cell_picture(&CellFrame, scale) -> RgbImage` and `frames::pad_snapshot(..., scale) -> RgbImage` (the whole pad laid out per the layout) build on it.
4. **Requests and replies on the HA connection** (`src/ha/`).
   - Replace the fixed id ranges in `session()` with a small dispatcher:
     - `Hub::request(message) -> oneshot` result;
     - `Hub::subscribe(message, handler)`, which gets the subscription's events and returns a handle that unsubscribes when dropped or cancelled;
     - all sharing the one increasing id counter.
   - The existing calls (get_states, state_changed subscription, registries, the three command event subscriptions, service calls, pings, fire_event) move onto it.
   - The stand-in server in `ha/tests.rs` must still pass, including the keep-alive test.
5. **Sandbox mode for end-to-end tests.** With `STREAM_DOCK_HA_SANDBOX=<folder>` set, the app:
   - keeps its settings in that folder;
   - uses its own single-instance mutex (`Local\StreamDockHASandbox`);
   - doesn't start the pad drivers, hotkeys, autostart registration or the tray icon;
   - doesn't open the window unless `--show`;
   - otherwise runs normally, including the Home Assistant connection and the integration bridge.

   This is what lets a second copy run next to the person's without touching their pads or settings. It never changes anything outside that folder.

## Phase 3: Home Assistant features that need no integration

1. **History graphs.**
   - Band meters and keys bound to a numeric HA sensor can show a graph of the last 1 h, 6 h or 24 h, taken from Home Assistant's recorder. Keep "live" (the current 64 readings every 3 s) as the default.
   - Use `history/history_during_period` (`minimal_response`, `no_attributes`, `significant_changes_only: false`) for 1 h, and `recorder/statistics_during_period` (period `5minute`, types `mean`) for 6 h and 24 h. Fall back to history if there are no statistics.
   - Resample to 64 points (the last value carried forward), and refresh every period/64 or so.
   - Keys get a "Graph" option in the key editor (off, 1 h, 6 h, 24 h) that draws the graph behind the value, like the band's graph meter look.
2. **Full templates.**
   - A title (or on/off title) is *simple* when every `{{ … }}` body is a bare identifier. The app fills simple ones itself, as now.
   - Anything else (filters, functions, `states(...)`, `{% %}`) is rendered by Home Assistant with the `render_template` subscription. Pass `variables`: `entity_id`, `state`, `friendly_name`, `unit` and every attribute of the key's entity.
   - Results are cached per (template, entity) and used by `ha::fill` and the renderer. Until a result arrives, or if HA refuses (not admin, an error), the local fill is used, so nothing goes blank.
   - Subscriptions follow the templates actually in the settings (subscribe new, cancel removed) after each change.

## Phase 4: the bridge, security, alerts, PC sensors

1. **`src/bridge.rs`.** It does the integration side of the protocol over the dispatcher:
   - `connect`, with error handling as section 3.1 says;
   - debounced `state`;
   - `pressed`;
   - `pictures` for watched pads, from `frames` and `cell_picture`, JPEG at quality 90 with `jpeg-encoder`;
   - `reply`.

   Commands go through `remote::apply` and the same "after a command" path `remote::received` uses: saving, running keys, the brightness and Mini Monitor side effects.

   Its status ("Integration: connected as DESKTOP-MAX", "not installed", "update needed") shows in Settings > Home Assistant, where `pc_name` and `ha_integration` are set too.
2. **Security**, per protocol section 7:
   - `DockKey.remote: RemoteAccess { Auto, Allow, Deny }`, with an inspector control "Pressing from Home Assistant: Automatic / Always / Never" and a short hint;
   - one function, `remote::may_press(cfg, key, source) -> Result<(), String>`, used by every route;
   - `set_key` refuses `remote` changes and resets `remote` when `actions` change;
   - `docs/home-assistant-control.md` updated.
3. **Presses carry a source.** `actions::press_control(shared, pad_id or kind, cell, source)`:
   - announces to the legacy events (as now) and to the bridge;
   - an alert or a sleeping screen swallows the press, with `dismissed`;
   - `pc.last_press` is updated.
4. **Alerts**, per protocol section 6:
   - `Shared.alerts`, by pad id;
   - drawn by `frames` as one picture over the screen keys, using the stretched camera's gap logic;
   - `show_alert` and `clear_alert` commands;
   - expiry after `seconds`.
5. **New commands:** `set_screen`, which turns the screen off until it's turned on or pressed (built on `screen_off.rs`), and `snapshot`.
6. **PC values:**
   - `mic_muted` from `media::watch_microphone`'s state;
   - `cpu`, `memory` and `gpu` from `Shared.stats`;
   - `last_press`.

## Phase 5: release

- Bump `Cargo.toml` to 1.1.0.
- Add to `docs/release-notes.md`: what's new, and the security change. Keys that act on the PC now need "Always" before Home Assistant can press them.
- Update `README.md` to mention the integration. Update `docs/home-assistant-control.md`.
- Don't make the setup file or publish anything. The person does that.

# Controlling Stream Dock HA from Home Assistant

Home Assistant can do what you do in Stream Dock HA's window: change profiles and pages, create and edit keys, press them, set the brightness and turn Mini Monitor on or off. It also hears about every press on your pads, so a key or knob can start an automation.

It works through Home Assistant **events**, over the connection Stream Dock HA already keeps open. Nothing listens on your PC, so there's no port to open or firewall rule to add, and it works wherever Stream Dock HA can reach Home Assistant.

**Want devices, entities and a dashboard card?** The [Stream Dock HA integration](https://github.com/Max-src/stream-dock-ha-integration) gives each PC and pad a device in Home Assistant, with entities, actions with the same names as the commands below, device triggers for every press, and a card that shows a pad's keys and presses them. It follows the same settings and rules as this page. The events on this page keep working with or without it.

**Made automations under an earlier name?** Commands keep working: a `stream_dock_max_command` or `tray_dock_command` event is still acted on and answered under the same name, with `stream_dock_max_result` or `tray_dock_result`. Presses are announced only as `stream_dock_ha_pressed`: an automation made with a test build that waits for `stream_dock_max_pressed` or `tray_dock_pressed` needs that one name changed. New automations should use the names on this page.

## Turning it on

It's off until you turn it on.

1. Connect Stream Dock HA to Home Assistant, in **Settings > Home Assistant**.
2. Tick **Press keys, switch profiles and pages, and change keys**.
3. To check it, open **Developer tools > Events** in Home Assistant, type `stream_dock_ha_result` under **Listen to events** and start listening. Then in **Fire an event**, use the type `stream_dock_ha_command` and this data:

   ```yaml
   command: state
   ```

   A `stream_dock_ha_result` event appears, listing your pads and profiles.

Stream Dock HA has to be running, and connected, to act on a command. A command sent while it's closed is lost. Events need a token from an administrator user.

## What it's allowed to do

Anyone who can edit your Home Assistant automations can use this, so there are two switches.

| Setting | What it allows |
| --- | --- |
| **Press keys, switch profiles and pages, and change keys** | Everything on this page, except what's in the next row |
| **Add keys that open programs or press keys** | Adding keys with `open`, `website`, `shortcut` or `type_text` steps. Those act on your PC, so leave this off unless you need it |

Each key also has its own **Pressing from Home Assistant** choice, under **More for this key** in the key's settings:

| Choice | What Home Assistant can do with that key |
| --- | --- |
| **Automatic** | Press it, unless one of its steps opens a program or a website, presses a shortcut or types text |
| **Always** | Press it, whatever it does |
| **Never** | Nothing: only you can press it |

This covers `press_key` from events, the Stream Dock HA integration's actions and its dashboard cards. Presses on the pad, on a pad on screen and from keyboard shortcuts are yours, so they always work. A `press_key` event always counts as Home Assistant's press, even if its data says otherwise, such as `source: pad`.

Home Assistant can't change this choice:

- a `set_key` that includes `remote` is refused;
- a `set_key` that changes a key's steps puts it back to **Automatic**, while one that changes only its title, colour or picture keeps it;
- `get_key` shows it as `remote`: `allow` for Always, `deny` for Never, and nothing for Automatic.

`state`, `get_key` and `snapshot` only read, so they work even with control turned off.

## Sending a command

An automation or script fires the `stream_dock_ha_command` event:

```yaml
actions:
  - event: stream_dock_ha_command
    event_data:
      command: show_profile
      pad: main
      profile: Games
```

Every command is answered with a `stream_dock_ha_result` event:

| Field | Meaning |
| --- | --- |
| `command` | The command it answers |
| `id` | Whatever you put in the command's `id`, to match them up |
| `ok` | `true` or `false` |
| `result` | What came back, when `ok` is true |
| `error` | What went wrong in plain words, when `ok` is false |

### Saying which pad, profile, page and key

| Field | Meaning | If left out |
| --- | --- | --- |
| `pad` | `main` for the Stream Dock 293S, `akp03e`, or a pad's id from `state`, which is how a virtual pad is named | `main` |
| `profile` | A profile's name | The profile that pad is running |
| `page` | A page number, from 1 | The page showing |
| `key` | A key number, from 1, left to right and then down: 1 to 15 on the 293S, 1 to 6 on the AKP03E | |
| `control` | A control's name, for buttons and knobs | |

The AKP03E's controls are named like this:

| Control | Names |
| --- | --- |
| Screen keys | `key_1` to `key_6` |
| Round buttons | `button_1` to `button_3` |
| Top knob | `top_knob_left`, `top_knob_right`, `top_knob_press` |
| Lower left knob | `lower_left_knob_left`, `lower_left_knob_right`, `lower_left_knob_press` |
| Lower right knob | `lower_right_knob_left`, `lower_right_knob_right`, `lower_right_knob_press` |

## Commands

### Reading

| Command | Fields | Answers with |
| --- | --- | --- |
| `state` | | Each pad's profiles, its brightness and whether it's showing Mini Monitor. `main` and `akp03e` as before, and `pads` with every pad: its `id`, `kind`, `name`, `profiles`, `brightness` and `mini_monitor` (empty for a virtual pad) |
| `get_key` | `key` or `control`, and optionally `pad`, `profile`, `page` | The key's settings |
| `snapshot` | `pad`, and optionally `scale` from 1 to 3 | A picture of the whole pad as it looks now, alert included, as `content_type` (`image/jpeg`) and `data` in base64. A key is 85 pixels across at scale 1 |

### Profiles

| Command | Fields | What it does |
| --- | --- | --- |
| `show_profile` | `profile`, `pad` | Puts that pad on a profile |
| `next_profile`, `previous_profile`, `first_profile` | `pad` | Moves along that pad's profiles, wrapping round at the ends |
| `add_profile` | `name`, `pad` | Adds an empty profile |
| `rename_profile` | `name`, and `profile`, `pad` | Renames a profile, and any key that switches to it |
| `duplicate_profile` | `profile`, `pad`, and optionally `name` | Copies a profile |
| `delete_profile` | `profile`, `pad` | Deletes a profile. The Stream Dock keeps at least one |

### Pages

| Command | Fields | What it does |
| --- | --- | --- |
| `show_page` | `page`, and optionally `pad`, `profile` | Shows a page |
| `next_page`, `previous_page` | `pad`, `profile` | Turns the page, wrapping round at the ends |
| `add_page` | `pad`, `profile` | Adds a page, without showing it |
| `delete_page` | `page`, and optionally `pad`, `profile` | Deletes a page. A profile keeps at least one |

### Keys

| Command | Fields | What it does |
| --- | --- | --- |
| `set_key` | `key` or `control`, `set` | Changes the fields in `set` and leaves the rest |
| `add_key` | `set` | Fills the first free key on the page, and says which |
| `clear_key` | `key` or `control` | Empties a key |
| `press_key` | `key` or `control` | Runs a key's steps, if it may be pressed from Home Assistant (see above). With Mini Monitor on, it runs the key and leaves Mini Monitor on. An alert or a dark screen takes the press instead, and the answer says `dismissed` |
| `swap_keys` | `key`, `with` | Swaps two keys on a page |
| `copy_key` | `key`, `to` | Copies a key. `to` holds `key` or `control`, and optionally `pad`, `profile`, `page`. A copy doesn't take the key's shortcut |
| `move_key` | `key`, `to` | Moves a key, shortcut included |

These all take `pad`, `profile` and `page` too.

The `set` fields are:

| Field | Meaning | Example |
| --- | --- | --- |
| `title` | The title. `\n` starts a new line | `Door` |
| `color` | Background colour, as red, green and blue | `[46, 104, 214]` |
| `glyph` | An icon | `mdi:door` |
| `image` | A picture file's full path on the PC | `C:\Pictures\door.png` |
| `picture_fit` | `fit`, `fill`, `stretch` or `original` | `fill` |
| `no_icon` | Text only | `true` |
| `hotkey` | A shortcut, on the 293S | `Ctrl+Alt+1` |
| `actions` | What the key does, replacing its steps | See below |
| `span` | How many keys a camera covers, across then down | `[3, 1]` |
| `camera_gaps` | Whether a stretched camera allows for the gaps between keys | `true` |

### Steps in `actions`

| `kind` | Fields |
| --- | --- |
| `home_assistant` | `entity`, `service`, and optionally `data` as JSON text |
| `media` | `key`: `volume_up`, `volume_down`, `mute`, `play_pause`, `next_track` or `previous_track` |
| `wait` | `ms` |
| `next_page`, `previous_page` | |
| `go_to_page` | `page` |
| `next_profile`, `previous_profile`, `first_profile` | |
| `switch_profile` | `name` |
| `adjust_brightness` | `change` |
| `toggle_mini_monitor` | |
| `open`, `website`, `shortcut`, `type_text` | `target` and `args`, `url`, `keys`, and `text` with `press_enter` as true or false. These need the second switch |

### Pads

| Command | Fields | What it does |
| --- | --- | --- |
| `set_brightness` | `level` from 0 to 100, `pad` | Sets a pad's brightness |
| `change_brightness` | `by`, `pad` | Moves it up or down |
| `mini_monitor` | `on` as true or false, `pad` | Turns Mini Monitor on or off on a pad. See the note below |
| `set_screen` | `on` as true or false, `pad` | Turns a pad's screen off, dark until it's turned on again or pressed, or back on. Answers with `on` |
| `show_alert` | `pad`, and optionally `title`, `message`, `icon` (`mdi:…`, a bell if left out), `color` as red, green and blue (amber if left out), `seconds` (0, the default, keeps it until a press; at most a day), `camera` (a camera entity, such as `camera.front_door`, to fill it with) | Covers all of a pad's screen keys with one picture. The next press on that pad only takes it away; a keyboard shortcut runs as usual and leaves it up. A new alert replaces the old one |
| `clear_alert` | `pad` | Takes the alert away |
| `mouse` | `pad`, `action` (`move`, `down`, `up`, `click` or `scroll`), `x` and `y` from 0 to 1 on the Mini Monitor picture, optional `button` (`left`, `right` or `middle`) and `amount` (wheel notches) | The mouse on Mini Monitor's display, as the Mini Monitor dashboard card uses it. Needs **Cards can use the mouse**, in the app's **Monitor** tab, too |

An alert is one picture over the keys, so the gaps between them hide parts of it. Its icon and words are placed inside the keys: the title along the top row, the icon in a key of its own and the message along the bottom, running on from key to key. The words shrink to fit, and what still doesn't fit is cut short. With a camera, the camera fills the keys and the title is a strip along the top. A press on the pad, a pad on screen or a card takes an alert away and does nothing else. A keyboard shortcut isn't a press on the pad: it runs its key as usual and the alert stays.

Turning Mini Monitor on adds a display to Windows, which needs the Parsec Virtual Display driver.

## Knowing when a key is pressed

Every press on a pad fires a `stream_dock_ha_pressed` event, knob turns included, while control is turned on:

| Field | Meaning |
| --- | --- |
| `pad` | `main` or `akp03e`, or a virtual pad's id |
| `pad_id` | The pad's id, as `state` lists it |
| `profile` | The profile's name |
| `page` | The page number |
| `control` | The control's name, such as `key_3` or `top_knob_press` |
| `title` | The key's title |
| `column`, `row` | Where a screen key is, counted from 0 from the top left, as the original app tells its own actions. Buttons and knobs don't have them |

A press still runs the key's own steps, so a key with no steps can be left entirely to Home Assistant. Presses on a Stream Dock HA dashboard card count too; `press_key` commands and keyboard shortcuts don't fire it.

## Examples

### Turn on the lights from a knob

```yaml
alias: Top knob turns on the desk
triggers:
  - trigger: event
    event_type: stream_dock_ha_pressed
    event_data:
      pad: akp03e
      control: top_knob_press
actions:
  - action: light.toggle
    target:
      entity_id: light.desk
```

### Change profile when you start gaming

```yaml
alias: Games profile with the PC's game mode
triggers:
  - trigger: state
    entity_id: binary_sensor.gaming_pc_game_running
    to: "on"
actions:
  - event: stream_dock_ha_command
    event_data:
      command: show_profile
      profile: Games
```

### Put a camera on three keys when the doorbell rings

```yaml
alias: Show the door on the pad
triggers:
  - trigger: state
    entity_id: binary_sensor.doorbell
    to: "on"
actions:
  - event: stream_dock_ha_command
    event_data:
      command: set_key
      key: 1
      set:
        title: Door
        span: [3, 1]
        camera_gaps: true
        picture_fit: fill
        actions:
          - kind: home_assistant
            entity: camera.front_door
            service: ""
```

### Dim the pads at night

```yaml
alias: Dim the pads
triggers:
  - trigger: time
    at: "22:30:00"
actions:
  - event: stream_dock_ha_command
    event_data:
      command: set_brightness
      level: 10
  - event: stream_dock_ha_command
    event_data:
      command: set_brightness
      pad: akp03e
      level: 10
```

### See whether a command worked

```yaml
alias: Tell me if a Stream Dock HA command failed
triggers:
  - trigger: event
    event_type: stream_dock_ha_result
    event_data:
      ok: false
actions:
  - action: persistent_notification.create
    data:
      title: Stream Dock HA refused a command
      message: "{{ trigger.event.data.command }}: {{ trigger.event.data.error }}"
```

## If nothing happens

| What you see | What to check |
| --- | --- |
| No `stream_dock_ha_result` at all | Stream Dock HA isn't running or isn't connected: see **Settings > Home Assistant** |
| `Home Assistant control is turned off` | Tick the setting in **Settings > Home Assistant** |
| `don't let Home Assistant add keys` | That key opens a program or presses keys, which needs the second switch |
| `isn't allowed to press` or `can't press it` | That key's **Pressing from Home Assistant** is **Automatic** and it acts on the PC, or it's **Never**. Choose **Always** under **More for this key** if you want it pressed |
| `Only the person at the PC can choose whether Home Assistant may press a key` | A `set_key` included `remote`. Leave it out, and choose in the app instead |
| `dismissed` in a `press_key` answer | The pad had an alert or a dark screen, so the press only took that away. Press again to run the key |
| No `stream_dock_ha_pressed` | Control has to be on. Firing events also needs a token from an administrator user |

Stream Dock HA's `pad-log.txt`, next to its settings, lists each command it ran or refused.

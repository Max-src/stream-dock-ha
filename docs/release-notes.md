Stream Dock HA 1.1.1 works with its own Home Assistant integration: your PCs and pads become devices in Home Assistant, and a dashboard card shows a pad's keys live and presses them.

## Install or update

1. Download **Stream-Dock-HA-Setup.exe** below.
2. Run the file. Windows SmartScreen warns about it because it isn't signed: choose **More info**, then **Run anyway**.

Installing over 1.0 keeps your settings. Read the two changes below before you update.

## Before you update

**Home Assistant can press fewer keys.** Keys that open programs or websites, press shortcuts or type text can no longer be pressed from Home Assistant until you allow it. Each key has a new setting, **Pressing from Home Assistant**: **Automatic**, **Always** or **Never**. Automatic, the default, lets Home Assistant press a key unless it acts on your PC like that. To let Home Assistant press such a key, from an automation, an action or a dashboard card, set it to **Always**, under the key's **More for this key**. Home Assistant can't change this setting, and when it changes a key's steps the setting goes back to Automatic. Presses on the pad, on a pad on screen and from shortcuts work as before.

**Settings are converted to a new shape** the first time 1.1 starts, so that pads can have ids and names. Version 1.0 can't read them back. If you might go back to 1.0, first copy `%APPDATA%\StreamDockHA\config.json` somewhere safe.

## What's new

### The Stream Dock HA integration

A Home Assistant integration made for this app. [Install it](https://github.com/Max-src/stream-dock-ha-integration#install) from Home Assistant's Marketplace, from HACS or by copying it into Home Assistant, then tick **Press keys, switch profiles and pages, and change keys** in the app's **Settings > Home Assistant**. It needs a token from an administrator user. Without the integration, the app works as before.

- Each PC and each pad shows as a device.
- **Entities** for each pad: profile, page, screen on or off with its brightness, Mini Monitor, whether it's plugged in, presses, a picture of the pad and alerts. Each PC has the microphone's mute, processor, memory and graphics card use, the last press, and whether it's online.
- **Actions** for everything Home Assistant can ask the app: profiles, pages, keys, brightness, screen, Mini Monitor and alerts.
- **Device triggers** for every key, button and knob, so an automation can start from a press without writing any YAML.
- Blueprints for a knob that dims a light, a key that toggles an entity, an on-air light that follows the microphone, and a doorbell alert.
- **The dashboard card** shows a pad's keys as the pad shows them, live, and you tap a key to press it. With Mini Monitor on, the card shows the PC screen across its keys, as the pad does, a few pictures a second, or as one picture across the gaps (both in the **Monitor** tab). When the app reaches Home Assistant through the internet, those pictures go over Home Assistant's local address instead, if it has one. It's added to Home Assistant by the integration, and set up in the card editor by picking a pad. It looks like the pad in the app: the same front colour, or none, the same corners and the same space between keys.

### In the app

- **Virtual pads:** a grid of keys up to 10 across and 8 down, with no pad to plug in. Use it on a dashboard card or as a pad on screen. Add one with **Add a pad > Virtual pad**: two sliders set its size while the pad is drawn as it will be. **Edit pad** in the top bar renames it, resizes it (each key keeps its row and column, and the window shows any key that would be removed) or deletes it. Pads made as "Dashboard pad" take the new name.
- **Pad names:** rename any pad in **Settings > Pads**. Home Assistant uses the same names.
- **PC name:** the name this PC has in Home Assistant, in **Settings > Home Assistant**.
- **History graphs:** keys and band meters showing a Home Assistant sensor can graph its last hour, 6 hours or day, from Home Assistant's own records.
- **Full templates:** titles can use any Home Assistant template, such as `{{ states('sensor.outside') | round(1) }}`. Home Assistant works them out, which needs a token from an administrator user; the simple `{{state}}` kind works with any token, as before.
- **Alerts:** Home Assistant can cover a pad's keys with a title, a message and an icon, or a camera, until the next press or for a number of seconds.
- **The mouse on Mini Monitor from a dashboard card:** with **Cards can use the mouse** ticked in the **Monitor** tab (off by default), clicks, drags and the wheel on the Mini Monitor card move this PC's real pointer and act on the extra display. Only Home Assistant administrators can.
- **The pointer comes back from Mini Monitor** after a dashboard card used it: move your own mouse (that first move is used up), wait two minutes without the card, or press **Ctrl+Alt+Home**, which brings it back from the extra display whoever put it there.
- **Clearer Mini Monitor setting for cards:** the **Monitor** tab now says what the pictures a second on Home Assistant cards are for.
- **Something on one key:** Home Assistant can show words, an icon, a colour, a camera or a picture on a single key for a while (30 seconds unless told otherwise), with the new **Show on a key** action. Pressing that key takes it away and tells Home Assistant, so an automation can answer, and the other keys work as usual.
- **Restart Home Assistant key:** drag **Restart Home Assistant** from **Smart home** in the list of actions onto a key. It asks for a second press: the first shows **Press again** for 3 seconds, and only a second press within that time restarts Home Assistant. Then it says **Restarting…** until Home Assistant is back. It needs a token from an administrator user; with another, the app says so. Any Home Assistant step can ask for a second press, with **Press twice to confirm** in its settings, and a Home Assistant step can now call a service with no entity.
- **Home Assistant keys while it's unavailable:** while the app can't reach Home Assistant, or Home Assistant is restarting, its keys keep their last look, dimmed, with a small crossed-out house in the corner, instead of falling back to plain icons. A key with nothing known yet shows **HA offline**. To show **HA offline** on all of them instead, choose it under **While it's unavailable** in **Settings > Home Assistant**. They come back as they are once Home Assistant runs again, on the pads, pads on screen and dashboard cards alike.
- **Screen off from Home Assistant:** an automation can turn a pad's screen off and on, and a press wakes it.
- **Pictures of a pad:** Home Assistant can ask for a picture of a whole pad.
- **WebP pictures** work on keys, chosen from a file or sent by Home Assistant, as PNG and JPEG do.
- **Reconnects after Home Assistant restarts:** Home Assistant answers before its integrations are loaded, so the app now asks the integration again after a few seconds, instead of waiting for the next time it connects.
- **Band tile backgrounds:** each band tile can have its own background colour, under **Background** in the tile's settings.
- **A pad on screen has no frame, shadow or line round it** any more, see-through or not, also once it's been clicked.
- **Round band tiles:** with round keys, the band's tiles are round too, so they line up; what they show is drawn smaller to fit.
- **A setup wizard:** the setup file is now a standard installer with a desktop icon and start-up choice. It installs for your Windows account only, without administrator rights, updates the copy you have in place and closes it first. It also appears in Windows' Apps list for removal.
- **One look for every pad:** the pad in the window, a pad on screen, the dashboard card and Home Assistant's picture of a pad all have the same space between keys and round the edge, close to the real pad. The band's tiles sit on one black strip, like its side screen, centred beside the keys.
- The band tile's settings scroll in a small window.
- **A tidier window:**
  - One profile menu at the top switches profiles and adds, copies, renames, saves or deletes them.
  - A page is deleted by right-clicking its tab.
  - The bottom-right corner speaks only when something needs doing, such as a pad that isn't plugged in or Home Assistant refusing the token. Click it for the settings.
  - A key's shortcut and **Pressing from Home Assistant** are under **More for this key**. **Run** is now **Try it**.
  - A Home Assistant step offers its services in one list, with **Other service…** to type any other.
  - **Settings > Home Assistant** reads in order: the connection, what Home Assistant can do, the integration, and what keys show while it's unavailable.
  - Everything about Mini Monitor on dashboard cards is in the **Monitor** tab.
  - The **Look** tab says what's for this pad and what's for all pads.

[docs/home-assistant-control.md](https://github.com/Max-src/stream-dock-ha/blob/main/docs/home-assistant-control.md) has every command, with examples.

## 1.0.0

The first release of Stream Dock HA, a Windows tray app for the StreamDock 293S button pad. It was called Stream Dock Max, and Tray Dock before that: installing it brings your settings across.

### What's in it

- Profiles and pages of keys that open programs and websites, press shortcuts, type text and send media keys
- Drag keys to rearrange them, with a live preview, and cut, copy, paste or duplicate them between profiles and pads
- A side band showing the clock, processor, memory and graphics card use, sensors, text or pictures
- Home Assistant control, live states and an entity browser, and Home Assistant can control Stream Dock HA in return
- 7,447 searchable icons, or your own pictures with four display modes
- Key shapes, fonts, and light and dark themes
- Mini Monitor, which makes the pad an extra Windows display (needs the Parsec Virtual Display driver, not included)
- An AJAZZ AKP03E can run next to the 293S, with its own profiles, pages, keys, buttons and knobs
- Home Assistant cameras on keys, refreshed about once a second, which can stretch over several keys
- Mini Monitor on either pad
- Meters for the band's measurements: bar, blocks, gauge, ring, graph or number, with colours that change at levels you choose
- Home Assistant keys whose background and text follow a number, such as a temperature, or an exact state

See the [README](https://github.com/Max-src/stream-dock-ha#readme) for everything it does and its known limits.

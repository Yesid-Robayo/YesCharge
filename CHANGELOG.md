# Changelog

**English** · [Español](CHANGELOG.es.md)

What each YesCharge version added and fixed, newest first. Published versions have their exe, release notes in English
and Spanish and their SHA256 on the [Releases](https://github.com/Yesid-Robayo/YesCharge/releases) page.

When a new version is published, it is added at the top with the same content as the *What's new* section of its
release notes.

## 0.1.1

A maintenance version: how the app works doesn't change.

- **It really needs nothing installed.** The exe depended on `VCRUNTIME140.dll`, from the Microsoft Visual C++ Redistributable,
  even though it promised no runtime. On a PC without that package it wouldn't open. It now carries that code inside.
- **Safer DLL loading.** It only loads DLLs from the Windows system folder, so a file placed next to the exe, for example in
  Downloads, can't be loaded instead.
- **Each release includes its license and third-party notices**, and the list of bundled fonts is corrected.

## 0.1.0

First version. It's a personal project in beta: **you use it at your own risk**, and it may not work with your device.

### Adding devices

- **Step-by-step wizard**: pick your device from the list and YesCharge tests, in order, the battery Windows already
  knows (Bluetooth and Xbox-compatible controllers), the standard HID battery and what the device sends on its own.
- **Your own profile by listening**: for 2.4 GHz devices Windows doesn't know about, you follow five steps (no cable,
  plug it in, unplug it, turn it off, turn it on) and the app learns where charging, power and the percentage come from.
  The steps with the device off continue on their own, because nothing can be pressed.
- **Asking, only if you accept**: models that only tell their battery when asked can be asked, after a risk warning.
  It's off by default, per device, and limited to verified requests.
- **A test when asking too**: the wizard follows those same steps and tells you what it noticed, so you know what to
  expect before adding it.
- **The list doesn't repeat** what you already added, and a notice shows while a device is being added.
- **Send us your device**: if it can't be read, the app saves a report with no personal data to open an *issue*.

### Day to day

- **Tray icon** with the lowest level and a bolt while something charges; menu with every device's level.
- **Windows notifications** for low battery (repeated while it stays low) and fully charged.
- **Top-off margin** after 100 %, so you don't unplug it too soon.
- **Learned battery life**, a **24-hour and 7-day history** for each device, and the time left to charge when it can be
  estimated.
- **A list ordered your way**: what's on first, or the fixed order you choose by dragging each device.
- **Refresh button**: the list shows "Updating…" and comes back with the new data, so you can tell it did something.
- **Spanish and English**: it starts in Spanish and you can change it in the settings.
- **Quick panel**: left click the tray icon to see the levels without opening the window.
- **Two identical devices** can be told apart and show up numbered when adding them.
- **Mute the alerts** of each device in its details.
- **No old numbers**: if something charges without giving its percentage, "—" shows until there's a real reading.
- **Check for updates** and **Uninstall** in the settings, keeping or deleting your data.
- **Optional local API** on `127.0.0.1` (off by default), with `/v1/status` and the compact `/min` format.

### What each state shows

- **Charging with no number**: if a model doesn't give its percentage on the cable, an old one isn't shown as current.
- **Off** or **Asleep or off**: the latter when the device can't tell the two apart.
- **Last known**: the last thing known, when the device hasn't said anything or the PC has been unused for a while.

### Verified models

| Model | Percentage | Charging | Off | When charging ends |
|---|---|---|---|---|
| SteelSeries Arctis Nova 5 | Yes | Yes | Yes | Alerts on reaching 100 % |
| MCHOSE L7 Ultra+ | Yes, without cable | Yes | Yes | Alerts if the mouse says it finished |
| Mars68 (CAROTMAS) | Yes; rises about 13 points near the cable | Yes | Asleep or off | Alerts on reaching 100, after the margin |
| GameSir Nova 2 Lite | Yes, without cable | Yes | Yes | Can't be known |

### Security and privacy

- **Read-only by default**: devices are opened without write permission.
- **Asking has limits**: only exact requests built into the exe, at most once a minute, never while the PC is idle, and
  it stops by itself after 3 errors in a row.
- **No telemetry, no accounts and no automatic updates.**
- **It doesn't start with Windows** unless you turn that on in the settings.
- **Doesn't wake your devices** while the PC is idle: it then only checks what needs no asking.
- **Portable and light**: one ~5 MB exe, with no installer or runtime, and you can keep your data next to it; the window
  is a separate process that only exists while it's open.

### Known limitations

- Models that give no notice of their own (Mars68 and GameSir) take up to a minute to show a change.
- Playing only with a controller counts as an idle PC for Windows, so while you do that the app doesn't ask.
- The fully charged alert hasn't been tried with a real full charge yet.
- With two identical devices detected by their USB id, plugging one in can show both as charging.
- The exe isn't signed, so Windows SmartScreen may warn you the first time.

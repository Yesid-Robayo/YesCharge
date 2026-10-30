# Changelog

**English** · [Español](CHANGELOG.es.md)

What each YesCharge version added and fixed, newest first. Published versions have their exe, release notes in English
and Spanish and their SHA256 on the [Releases](https://github.com/Yesid-Robayo/YesCharge/releases) page.

When a new version is published, it is added at the top with the same content as the *What's new* section of its
release notes.

## 0.1.0 (not published yet)

First version.

- **Add your own device** with a wizard that tests, in order, the battery Windows already knows (Bluetooth and
  Xbox-compatible controllers), the standard HID battery and, if needed, **your own profile by listening** to what the
  device sends while you plug and unplug the cable.
- **Read-only by default**: devices are opened without write permission.
- **Asking, only if you accept**: supported models that only tell their battery when asked can be asked after a risk
  warning. It's off by default, per device, limited to verified requests, and it stops by itself after repeated errors.
  It covers 4 models.
- **The wizard tests asked devices too**: it follows the same cable and power steps and tells you what it noticed. Models
  that give no notice of their own are asked once a minute, the only way to notice those changes.
- **English and Spanish**, following the Windows language or the one you pick in the settings.
- **Tray icon** with the lowest level and a bolt while something charges; menu with every device's level.
- **Notifications** for low battery (repeated while it stays low) and fully charged.
- **Top-off margin** after 100 %, **learned battery life** and a **24-hour chart** for each device.
- **Doesn't wake your devices** while the PC is idle.
- **Optional local API** on `127.0.0.1` (off by default), with `/v1/status` and the compact `/min` format.
- **Light**: one ~5 MB exe, no runtime; the window is a separate process that only exists while it's open.

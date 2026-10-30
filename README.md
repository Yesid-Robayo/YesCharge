<div align="center">

# YesCharge

**See the battery of your wireless mice, keyboards, headsets and controllers in the Windows tray, without touching
your devices.**

![Version](https://img.shields.io/badge/version-0.1.0-43e5a0)
![Status](https://img.shields.io/badge/status-beta-f0c86a)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-2f5fb3)
![Size](https://img.shields.io/badge/size-~5%20MB%2C%20no%20runtime-12a8c9)
![Languages](https://img.shields.io/badge/languages-english%20%7C%20español-7ddc8f)

**English** · [Español](README.es.md) · [Changelog](CHANGELOG.md)

</div>

> [!WARNING]
> **Beta.** The app, the tray and the read-only engine are working, but very few devices have been tried yet (see
> [Testing status](#-testing-status)). **It may not be able to read your device.** If it can't, open an *issue* with the
> report the app saves for you: that is how support for new models grows.

You pick your device from a list and YesCharge tests it, looking for a **safe** way to read its battery. If it finds
one, it watches it from the tray, warns you when it's low and when it has finished charging, and learns how long the
battery lasts.

> [!NOTE]
> For now the app's interface is **in Spanish**. The button names below are given as they appear in the app, with
> their translation.

**By default YesCharge only reads.** Some models only tell their battery when asked: for those you can choose to let
YesCharge ask. That option is off, it's per device, and you have to accept a risk warning first.

## Contents

- [Features](#-features)
- [How it reads the battery](#-how-it-reads-the-battery)
- [How it protects your devices](#%EF%B8%8F-how-it-protects-your-devices)
- [Testing status](#-testing-status)
- [Install](#-install)
- [How to use it](#-how-to-use-it)
- [Settings](#%EF%B8%8F-settings)
- [Technology](#-technology)
- [Development](#-development)
- [Credits](#-credits)
- [License and contributions](#-license-and-contributions)

---

## ✨ Features

- **Add your own device** with a wizard: choose it, test it and confirm that the percentage matches what you see.
- **Your own profile by listening**: for 2.4 GHz devices Windows doesn't know about, YesCharge records what the device
  sends while you plug and unplug the charging cable, finds the bytes that change and builds a profile for it.
- **Asking, only if you accept**: supported models that only tell their battery when asked can be asked, after a risk
  warning you must accept. See the [list of models](docs/protocol-sources.md#models-in-the-catalog).
- **Send us your device**: if yours can't be read, the wizard saves a report you can attach to an *issue* so its model
  can be added.
- **Tray icon** that shows the lowest battery level, with a bolt while something is charging.
- **Windows notifications**: low battery (repeated every few minutes while it stays low) and "fully charged, you can
  unplug it".
- **Top-off margin**: after reaching 100 %, it keeps showing "charging" for the minutes you choose before saying it's
  full.
- **Learned battery life**: it measures how long each 10 % lasts and estimates the time left.
- **24-hour chart** for each device.
- **Doesn't wake your devices**: while the PC is idle it doesn't read anything.
- **Light**: one ~5 MB exe, no runtime to install. The tray part uses a few MB of memory; the window is a separate
  process that only exists while it's open.
- **Optional local API** (off by default) for external screens.

## 🔋 How it reads the battery

When you pick a device, YesCharge tries these methods in order. **The first three send nothing to the device.**

| # | Method | For | How |
|---|---|---|---|
| 1 | **What Windows already knows** | Bluetooth devices and Xbox-compatible controllers | Reads the battery value Windows shows in Settings (Bluetooth) or gives through XInput (4 levels). |
| 2 | **Standard HID battery** | Devices that declare it in their descriptor | Only a `GET_REPORT` read of the standard *Battery Strength* value, and only if the device itself declares it. |
| 3 | **Your listening profile** | 2.4 GHz devices that send their status on their own | Listens, read-only, while you plug and unplug the cable and turn it off and on, and learns where the charge, the power state and the percentage are. |
| 4 | **Asking (optional)** | Models on YesCharge's verified list | Sends that model's exact status request, only after you accept a risk warning. |

If none of them works, the app saves a **report** (no personal data) and helps you open an *issue* to ask for support
for that model.

> [!NOTE]
> Many 2.4 GHz devices only tell their battery when **asked**, and the question is different for each model. YesCharge
> only asks models on its verified list, and never tries commands on an unknown device.

## 🛡️ How it protects your devices

**By default it only reads:**

- **Devices are opened without write permission** (`GENERIC_READ`), so Windows would reject any attempt to send
  something. Editing the configuration file can't change that.
- **It only listens to the manufacturer's interfaces** (usage pages `0xFF00`–`0xFFFF`), never the keyboard or mouse
  ones: your keystrokes and mouse movements are never read or saved.
- **No administrator rights**, no drivers, nothing installed in the system.
- **No telemetry and no automatic updates.** The only network connection is the optional local API, which listens on
  `127.0.0.1` only.

**Asking is optional, and it's the one exception.** All of it lives in one file, [`core/src/ask.rs`](core/src/ask.rs):

- It's **off by default**, per device, and it needs you to accept a risk warning.
- It only sends **that model's exact status request**, built into the exe. It can't be changed from any file.
- It **never tries commands** on a device that isn't on the list, and never changes a setting.
- At most **once a minute**, and never while the PC is idle.
- After **3 errors in a row** it stops asking until you tell it to resume.

> [!CAUTION]
> Sending anything to a device is never 100 % safe: with a different firmware it could behave differently. Each request
> was checked on a single unit of that model. If you want no risk at all, don't turn asking on.

More details in [SECURITY.md](SECURITY.md).

## 🧪 Testing status

What has been tried on a real PC so far:

| Part | Status |
|---|---|
| Tray, notifications, window and settings | ✅ Working |
| Bluetooth battery from Windows | ✅ Read correctly on a Bluetooth device |
| Listening engine (opening read-only, capture, analysis) | ✅ Working; automatic tests for the analysis |
| Listening profiles with real 2.4 GHz devices | ⚠️ A 2.4 GHz headset gave its percentage, cable and power state (with its official software open; not tried with it closed). A 2.4 GHz mouse gives cable and link, but **not** its percentage. A keyboard is still to be tried |
| Asking (headset, mouse, keyboard, controller) | ⚠️ It worked on the mouse, which answered 53 % when its official reading was about 50 %. The headset, keyboard and controller requests come from the author's previous app and are still to be tried here |
| Xbox-compatible controllers (XInput) | ⚠️ Implemented, not tried with a controller yet |
| Standard HID battery | ⚠️ Implemented, no device that declares it has been tried |

If you try it with your device, an *issue* saying whether it worked (and which method it used) helps a lot.

## 📦 Install

1. Download `YesCharge.exe` from [Releases](https://github.com/Yesid-Robayo/YesCharge/releases).
2. *(Recommended)* Check that it's the official file. In PowerShell:
   ```powershell
   Get-FileHash .\YesCharge.exe -Algorithm SHA256
   ```
   It must match the `SHA256SUMS.txt` of the release. Every release is built by GitHub Actions from this repository's
   code and carries a [provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations)
   that you can verify with `gh attestation verify YesCharge.exe --repo Yesid-Robayo/YesCharge`.
3. Put it in a folder of your choice and open it. There is no installer.

> [!NOTE]
> The exe is not signed with a paid certificate, so Windows SmartScreen may warn you the first time. Choose **More
> info → Run anyway** if the SHA256 matches.

To uninstall: turn off *Iniciar con Windows* (*Start with Windows*) in the settings, right-click the tray icon →
**Salir** (*Exit*), and delete the exe and the `%APPDATA%\YesCharge` folder.

## 🎮 How to use it

1. Open `YesCharge.exe`. The first time, the **Añadir dispositivo** (*Add device*) wizard opens.
2. Turn on your device and use it for a moment so it's awake. Pick it from the list.
3. YesCharge tests the safe methods. If one works, it shows the percentage and asks you if it matches what you see on
   the device or in its official app. Give it a name and press **Sí, añadir** (*Yes, add*).
4. If it needs **your own profile**, turn it on, press **Ya está encendido: empezar a escuchar** (*Already on: start listening*) and follow the five steps
   (without cable → plug the cable → unplug it → turn it off → turn it on), then pick which value matches your battery.
5. From then on it lives in the **tray**: left click opens the window; right click shows each device's level and the
   options.

Closing the window doesn't close YesCharge: it keeps watching from the tray. To close it completely: right-click the
icon → **Salir** (*Exit*).

## ⚙️ Settings

| Setting | What it does |
|---|---|
| Low battery / Fully charged | Turn each notification on or off. |
| Repeat the low battery notice | Every how many minutes it reminds you while the battery stays low. |
| Read the battery every | 1, 2, 5, 10 or 15 minutes. |
| Idle | After this long without using the PC, it stops reading so it doesn't wake your devices. |
| Start with Windows | Starts in the tray, without opening the window. |
| Local API | Publishes the status on `http://127.0.0.1:<port>/v1/status` (and `/min`, a compact format). Off by default. |

Each device also has its own low battery threshold and top-off margin (in its **Details**).

**Files:** everything is kept in `%APPDATA%\YesCharge`: `config.json` (settings and devices), `status.json` (last
status), `history.json` and `model.json` (history and learned battery life), `yescharge.log` (the log to attach to a
bug report) and `captures\` (listening reports).

## 🔧 Technology

- **Rust**, a single exe with no runtime. The tray part uses the Windows API directly; the window uses
  [egui](https://github.com/emilk/egui) and is a separate process that only exists while it's open.
- Windows `hid.dll` + SetupAPI for HID; CfgMgr32 for the Bluetooth battery; XInput for controllers.
- Icons drawn in code (no image files): the exe icon is generated at build time.

The design is explained in [docs/architecture.md](docs/architecture.md) (Spanish).

## 💻 Development

Requirements: [Rust](https://rustup.rs) (stable, `x86_64-pc-windows-msvc`) and the Visual Studio Build Tools (C++).

```powershell
cargo run -p yescharge                         # service + window (debug)
cargo run -p yescharge -- --window             # window only
cargo test                                     # tests
cargo clippy --all-targets -- -D warnings      # lint
cargo build --release                          # target\release\YesCharge.exe
```

Diagnostics (read-only, like the app):

```powershell
cargo run -p yescharge-core --example diag -- scan                # HID interfaces, Bluetooth, XInput
cargo run -p yescharge-core --example diag -- candidates          # what the wizard would show
cargo run -p yescharge-core --example diag -- watch 120           # what the devices send, live
```

`YESCHARGE_DATA_DIR` changes the data folder (handy for trying things without touching your real configuration). In
debug builds, `YESCHARGE_DEMO_PAGE` (`detail`, `settings`, `probe`, `none`, `listen`, `review`, `empty`, `confirm`)
opens a screen with sample data.

## 🙏 Credits

- **[egui / eframe](https://github.com/emilk/egui)** (MIT or Apache-2.0), **[windows-sys](https://github.com/microsoft/windows-rs)**
  (MIT or Apache-2.0), **[serde](https://serde.rs)** (MIT or Apache-2.0) and the rest of the crates listed in
  [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).
- **YesCharge is an independent project**, not affiliated with or endorsed by SteelSeries, MCHOSE, CAROTMAS, GameSir or any
  other brand. Their names only say which devices it works with, and belong to their owners.
- Inspired by **BatteryInfo**, the author's previous app, which only worked with his own devices.
- **AI disclosure:** this app was developed with the help of AI tools.

---

## 📄 License and contributions

- YesCharge is licensed under the **[MIT License](LICENSE)**. It is provided **as is, without any warranty**.
- It's a personal project, shared in case it's useful to someone else: there's no support commitment, and *issues* get an
  answer when there's time.
- **Pull requests are not accepted** (they are closed automatically). Bugs, ideas and support requests for new devices
  go in an *issue*; see [CONTRIBUTING.md](CONTRIBUTING.md).
- Security problems are reported privately: [SECURITY.md](SECURITY.md).

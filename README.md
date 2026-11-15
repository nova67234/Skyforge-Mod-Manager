# Skyforge Mod Manager

A mod manager for **Skyrim Special Edition**, written in Rust with [egui](https://github.com/emilk/egui). It runs on Windows, Linux and macOS.

It installs mods from archives or straight from Nexus Mods, keeps the game folder clean, and tells you in plain language when something a mod needs is missing.

## Features

- **Install from anywhere** — drop a `.zip`, `.7z` or `.rar` archive, an extracted folder, or a single plugin or DLL onto the window.
- **Nexus Mods integration** — connect with your personal API key to search mods, browse their files, download, and check for newer versions. "Mod Manager Download" (`nxm://`) links on the Nexus site open directly in the app.
- **Non-destructive deployment** — mods are stored outside the game and hardlinked into `Data` (copied if linking isn't possible). Any vanilla file a mod overwrites is backed up, and one click restores the vanilla game.
- **Install order and load order** — reorder mods to decide who wins file conflicts, toggle plugins, and sort master files first. `Plugins.txt` is kept up to date for you.
- **Requirement checks** — works out what each mod needs from its own files: the masters a plugin is built on, and whether an SKSE plugin needs the script extender or Address Library. Mods built for a different game version are flagged.
- **Crash reports** — with Crash Logger or Trainwreck installed, the app finds the report Skyrim leaves behind and summarises what went wrong and which files were involved.
- **Game-root files** — ENB, SKSE loader and upscaler DLLs that belong next to `SkyrimSE.exe` are placed there instead of in `Data`.
- **Themes** — light, dark, or follow the system, plus **Liquid Glass**: the sidebar and page become panes of real frosted glass floating over slowly flowing colours. Pick one under Settings → Appearance.
- **Graphics info** — the Settings page shows your graphics card, driver version and the DLSS runtimes found in the game folder.

## Download

Grab the latest build from the [Releases page](https://github.com/nova67234/Skyforge-Mod-Manager/releases/latest):

| System | File | How to run |
| --- | --- | --- |
| Windows 10/11 | `Skyforge-Mod-Manager-windows-x64.exe` | Double-click it. Windows may show a SmartScreen warning because the app isn't signed; choose **More info → Run anyway**. |
| Linux (x64) | `Skyforge-Mod-Manager-linux-x64.tar.gz` | Extract it, then run `./Skyforge-Mod-Manager`. |
| macOS (Intel and Apple Silicon) | `Skyforge-Mod-Manager-macos-universal.tar.gz` | Extract it, then run `./Skyforge-Mod-Manager` from Terminal. The app isn't signed, so the first time you may need to allow it under **System Settings → Privacy & Security**. |

## Requirements

- Skyrim Special Edition (the app looks for your Steam install, or you can point it at the game folder)
- A free [Nexus Mods](https://www.nexusmods.com/) account, only if you want in-app search and downloads

## Platform notes

Windows is the main platform and the one the app is tested on. The Linux and macOS builds are compiled and unit-tested automatically, but have had no hands-on testing yet, so expect rough edges and please report what you find.

- **Linux** — Skyrim runs through Steam's Proton. The app finds the game in your Steam library and uses the game's Proton prefix for `Plugins.txt` and crash reports, so start the game once from Steam before managing mods. **Play** asks Steam to launch the game. Graphics card detection isn't available.
- **macOS** — Skyrim Special Edition has no Mac version, so the app is only useful with a copy of the game you run through a compatibility layer and point the app at yourself. "Mod Manager Download" links and graphics card detection aren't available.

## Building

You need a recent [Rust toolchain](https://rustup.rs/) (edition 2024, so Rust 1.85 or newer) and a C++ compiler. On Linux you also need the OpenSSL development package (`libssl-dev` on Debian/Ubuntu).

```sh
git clone https://github.com/nova67234/Skyforge-Mod-Manager.git
cd Skyforge-Mod-Manager
cargo run --release
```

The finished executable is at `target/release/Skyrim_Mod_Manager` (`.exe` on Windows).

Builds for all three systems are produced by GitHub Actions (`.github/workflows/release.yml`); pushing a version tag such as `v0.1.0` publishes them as a release.

Run the tests with:

```sh
cargo test
```

## Connecting Nexus Mods

1. Open the app's Nexus page and choose **Connect Nexus Mods**.
2. Follow the link to your Nexus account page and copy the **Personal API Key** at the bottom.
3. Paste it into the app.

The key is stored on your own computer only and is never part of this repository.

## Project layout

| Path | What it does |
| --- | --- |
| `src/app.rs` | The user interface and app state |
| `src/mods.rs` | Installing, storing and deploying mods |
| `src/game.rs` | Finding the game, reading plugins and `Plugins.txt` |
| `src/needs.rs` | Working out what each mod requires |
| `src/nexus.rs` | Nexus Mods API, downloads and `nxm://` links |
| `src/crash.rs` | Finding and summarising crash reports |
| `src/gpu.rs` | Graphics card, driver and DLSS detection |
| `src/archive.rs` | Extracting `.zip`, `.7z` and `.rar` archives |
| `src/ipc.rs` | Keeping a single running copy of the app |
| `src/theme.rs`, `src/widgets.rs` | Look and feel, custom controls |
| `tools/` | PowerShell helpers for UI testing (screenshots, clicks, fake test mods) |

## License

Released under the [MIT License](LICENSE).

## Disclaimer

This is an unofficial fan project. It is not affiliated with or endorsed by Bethesda Softworks or Nexus Mods. Back up your saves before modding.

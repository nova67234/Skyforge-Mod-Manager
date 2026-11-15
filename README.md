# Skyforge Mod Manager

A mod manager for **Skyrim Special Edition** on Windows, written in Rust with [egui](https://github.com/emilk/egui).

It installs mods from archives or straight from Nexus Mods, keeps the game folder clean, and tells you in plain language when something a mod needs is missing.

## Features

- **Install from anywhere** — drop a `.zip`, `.7z` or `.rar` archive, an extracted folder, or a single plugin or DLL onto the window.
- **Nexus Mods integration** — connect with your personal API key to search mods, browse their files, download, and check for newer versions. "Mod Manager Download" (`nxm://`) links on the Nexus site open directly in the app.
- **Non-destructive deployment** — mods are stored outside the game and hardlinked into `Data` (copied if linking isn't possible). Any vanilla file a mod overwrites is backed up, and one click restores the vanilla game.
- **Install order and load order** — reorder mods to decide who wins file conflicts, toggle plugins, and sort master files first. `Plugins.txt` is kept up to date for you.
- **Requirement checks** — works out what each mod needs from its own files: the masters a plugin is built on, and whether an SKSE plugin needs the script extender or Address Library. Mods built for a different game version are flagged.
- **Crash reports** — with Crash Logger or Trainwreck installed, the app finds the report Skyrim leaves behind and summarises what went wrong and which files were involved.
- **Game-root files** — ENB, SKSE loader and upscaler DLLs that belong next to `SkyrimSE.exe` are placed there instead of in `Data`.
- **Graphics info** — the Settings page shows your graphics card, driver version and the DLSS runtimes found in the game folder.

## Requirements

- Windows 10 or 11
- Skyrim Special Edition (the app looks for your Steam install, or you can point it at the game folder)
- A free [Nexus Mods](https://www.nexusmods.com/) account, only if you want in-app search and downloads

## Building

You need a recent [Rust toolchain](https://rustup.rs/) (edition 2024, so Rust 1.85 or newer).

```sh
git clone https://github.com/nova67234/Skyforge-Mod-Manager.git
cd Skyforge-Mod-Manager
cargo run --release
```

The finished executable is at `target/release/Skyrim_Mod_Manager.exe`.

Run the tests with:

```sh
cargo test
```

## Connecting Nexus Mods

1. Open the app's Nexus page and choose **Connect Nexus Mods**.
2. Follow the link to your Nexus account page and copy the **Personal API Key** at the bottom.
3. Paste it into the app.

The key is stored on your own PC only and is never part of this repository.

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

## Disclaimer

This is an unofficial fan project. It is not affiliated with or endorsed by Bethesda Softworks or Nexus Mods. Back up your saves before modding.

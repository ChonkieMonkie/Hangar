<p align="center">
  <img src="docs/images/hangar-icon.svg" width="96" height="96" alt="Hangar logo">
</p>

<h1 align="center">Hangar</h1>

<p align="center"><strong>A free, community-made updater and launcher for DCS World</strong><br>
Updates you can follow · mod switching · backups · readable logs · launch presets</p>

> **Not affiliated with or endorsed by Eagle Dynamics SA.**
> THIS MATERIAL IS NOT MADE OR SUPPORTED BY EAGLE DYNAMICS SA.

Hangar is an unofficial, open-source front end for the standalone version of DCS World on Windows. It does not replace Eagle Dynamics' updater. It starts ED's own `DCS_updater.exe` and `DCS.exe` as separate programs and adds the things the official tools don't show you.

<!-- Screenshot: add docs/images/overview.png (the Overview screen) and uncomment the line below.
     Use the blueprint fallback or a picture of your own, not ED artwork, and crop out personal paths. -->
<!-- <p align="center"><img src="docs/images/overview.png" alt="Hangar's Overview screen" width="900"></p> -->

## Features

- **Updates you can follow.** Start an update or repair and watch it live: progress for both download and unpacking, a graph of the updater's own network traffic, disk space, the current phase and a time estimate with a likely range. The estimate is our own. On the downloads recorded so far (6 runs) it was off by 5.6 % on average, against 42 % for the updater's built-in estimate. That is a small sample, so read it as a promising start, not a guarantee.
- **Mod manager.** Switch mods in Saved Games on and off with one click. Files move to Hangar's vault and back. Conflicts follow "switched on last wins", and the covered file comes back when you switch the mod off. Mod sets, and warnings when you have edited a mod's files. Add mods from a zip or a folder.
- **Backups.** Incremental, checksummed snapshots of your Saved Games folders (and Tacview, if you have it) to a folder you choose. You can verify, pin and restore everything, only your controls and options, or single files. Every restore first takes a snapshot of how things were, so it can be undone. A backup runs automatically before each update once you have chosen a backup folder.
- **Readable logs.** `dcs.log` grouped by message and by the script it came from, with mod errors picked out. A history of every updater run, kept longer than the updater's own two log files.
- **Launch presets.** Normal, VR (OpenXR) or flat screen, or your own command-line flags.
- **Hangar wall.** Your own screenshots on the start screen, from folders you pick.
- **Links to ED's news.** The changelog, newsletters and "what's new in this version" open in your browser.

## What Hangar doesn't do

- It doesn't log in to your ED account or ask for your password.
- It doesn't download DCS content itself. Updates and repairs are done by ED's own updater.
- It doesn't patch or inject into DCS. Changes to the install folder are left to ED's updater.
- It doesn't handle activation, keys or purchases.
- It has no telemetry, accounts, ads or donations.

## How it works

Hangar runs ED's programs the way you would from a shortcut, with command-line arguments, and watches them from the outside. It reads the updater's log, its window text (with Windows' read-only window APIs, without clicking anything), the process list and network counters.

Where it writes:

| Where | What | When |
|---|---|---|
| `%LOCALAPPDATA%\Hangar` | settings, mod vault, update recordings, an archive of updater logs | while it runs |
| `Saved Games\DCS*` | switching mods on and off, restoring a backup | only when you click |
| your backup folder | snapshots | when you back up, or before an update |

## Requirements

- Windows 10 or 11 (WebView2 is included in Windows 11)
- DCS World, standalone version. The Steam version has not been tested.

## Install

There is no release yet. Until there is, build it from source (below). Release builds will not be code-signed at first, so Windows SmartScreen may warn the first time you start Hangar.

## Building from source

You need [Rust](https://rustup.rs) (stable, MSVC toolchain), [Node.js](https://nodejs.org) LTS and the Microsoft C++ Build Tools.

```powershell
cd app
npm ci
npm run tauri dev                 # run in development mode
npm run check                     # type check the frontend
cd src-tauri; cargo test --lib    # Rust tests (they only write to temp folders)
cd ..; npx tauri build --no-bundle   # release exe in src-tauri/target/release/hangar.exe
```

The code is Tauri 2 (Rust) with a Svelte 5 frontend.

## Project status

Version 0.2.1, in active development, Windows only. Planned next: compression for backups and the mod vault, hiding the updater's window while Hangar shows its progress, and editing DCS graphics settings with presets.

## Contributing

Issues and pull requests are welcome. Please don't add any of ED's files, logos, images or text to the repository. Hangar must stay clearly separate from DCS and only use ED's programs from the outside. Contributions are accepted under the same license as the project.

## Trademarks

DCS World and Eagle Dynamics are trademarks of Eagle Dynamics SA. They are used here only to say which game Hangar works with.

## License

Copyright (C) 2026 ChonkieMonkie.

Hangar is free software: you can redistribute it and/or modify it under the terms of the [GNU General Public License](LICENSE) as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the license for details.

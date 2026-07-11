# Plutonium Controller Launcher

An open source alternative to the official `plutonium.exe` updater. Injects arrow key and controller navigation support into the launcher UI.

## Why

[Plutonium](https://plutonium.pw/) ships its own updater/launcher (`plutonium.exe`) that re-downloads and overwrites `launcher/assets/index.html` on every run, and its Ultralight-based launcher UI has no controller or keyboard navigation at all — mouse only.

This project replaces the updater entirely so it controls what gets written to disk, and injects a small JS layer (`controller-nav.js`) that adds spatial keyboard/controller navigation on top of the existing Vue UI, without forking or modifying Plutonium's own bundle.

The motivating use case is split-screen couch co-op on **Steam Deck / desktop Linux via [PartyDeck](https://github.com/partydeck/partydeck)**, where Steam Input is disabled (PartyDeck isolates each game instance with Bubblewrap + per-instance evdev masking, which is incompatible with Steam Input) — so each player needs a controller that works against the launcher UI directly, with no Steam Input translation layer in the loop.

## Features

- **Drop-in updater replacement** — reimplements Plutonium's CDN sync protocol (`prod.json` → per-manifest `info.json` → content-addressed file download + SHA1 verification), so it installs/updates the same way the official updater does.
- **Fast verify by default** — size-only file comparison (matches stock Plutonium's `fastVerify` behavior) instead of hashing every file on every run; `--full-verify` opts into full SHA1 checking.
- **Non-destructive patching** — intercepts exactly two files (`launcher/assets/index.html`, `launcher/assets/controller-nav.js`) before the CDN sync would overwrite them, injecting a single `<script>` tag into the *current* stock HTML rather than shipping a hand-maintained fork. Future Plutonium HTML updates are picked up automatically.
- **Spatial keyboard/controller navigation** (`assets/controller-nav.js`) — finds the launcher's clickable elements (this UI has no `<button>`/`[role=button]`/`[tabindex]` anywhere; everything is a plain `<div class="button">`/`.clickable`/`.row`) and drives a visible focus highlight between them using nearest-neighbor spatial scoring. Works around Ultralight not populating the modern `KeyboardEvent.key`/`.code` properties by mapping legacy numeric `keyCode`s instead.
- **Background gamepad helper** (`src/gamepad.rs`) — polls the controller via [`gilrs`](https://crates.io/crates/gilrs) (XInput backend) and injects the corresponding keyboard events via `SendInput`, active only while the launcher window has OS focus so a running game is never affected. Ultralight doesn't expose the browser Gamepad API, so this native bridge is required rather than polling from JS.

## Usage

```
plutonium.exe [OPTIONS]
```

| Flag | Effect |
|---|---|
| *(none)* | Update all files, write patched `index.html`, launch, run the controller helper. |
| `--no-update` | Skip the update/sync step and just re-patch + launch (fast path for repeated launches, e.g. one PartyDeck instance per player). |
| `--update-only` | Update + patch but don't launch (for pre-seeding an install). |
| `--install-dir <path>` | Override the install directory (default: `%ProgramData%\Plutonium`). |
| `--full-verify` | Full SHA1 verification instead of the size-only default. |

Controller mapping (while the launcher window is focused):

| Input | Action |
|---|---|
| D-pad / left stick | Move focus |
| A / Cross | Activate (Enter) |
| B / Circle | Back (Escape) |
| RB / R1 | Tab forward |
| LB / L1 | Shift+Tab (backward) |

## Building

```
cargo build --release
```

Windows only — the project depends on Win32 APIs (`SendInput`, `FindWindowW`, process enumeration) to drive the launcher window and requires spawning `plutonium-launcher-win32.exe` directly. This is expected to also run correctly as a Windows binary under Wine/Proton (the actual Steam Deck target), since `winebus` normalizes physical controllers (Xbox, PlayStation, generic) into the XInput surface this project already relies on.

CI builds and publishes a Windows release automatically on every push to `main`, with an auto-incremented patch version — see `.github/workflows/release.yml`.

## Project layout

```
src/
  main.rs      CLI entry point, flag parsing, launcher process lifecycle
  updater.rs   CDN sync protocol (prod.json/info.json, content-addressed download, SHA1 verify)
  manifest.rs  Manifest JSON types
  patch.rs     Injects controller-nav.js into index.html; intercepts those two files from the CDN sync
  gamepad.rs   Background thread: gilrs (XInput) polling + SendInput key injection
assets/
  controller-nav.js   Injected into the launcher UI; spatial nav + input handling
```

## Status

- ✅ Updater core, verified against the live CDN (full sync, retry logic, patch injection)
- ✅ Spatial keyboard/controller navigation, verified working end-to-end on real hardware (Xbox controller)
- ⬜ Real Proton/Steam Deck testing
- ⬜ PartyDeck integration (per-instance launch wiring)

## Known limitations

- `gilrs` is built XInput-only, not the broader `wgi` (Windows.Gaming.Input) backend gilrs defaults to on Windows. This is deliberate, not an oversight: WGI requires an in-focus window belonging to the *polling process*, but the controller helper runs inside this wrapper while OS focus belongs to the separately-spawned launcher process — WGI would not reliably receive events in that arrangement. XInput has no such restriction. A side effect: testing on native Windows with a controller that isn't natively XInput (e.g. a PlayStation pad with no translation layer) won't be detected — this doesn't affect the real target, since Wine's `winebus` already normalizes any physical controller into XInput before this binary sees it.

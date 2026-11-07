# Tuck

![Tuck showing media artwork](docs/screens/collapsed-artwork.png)

The screenshot uses test media data.

Tuck is a desktop island for Windows that puts media controls, dictation, and small tools at the top of your screen.

[Download the latest release](https://github.com/jojomensah89/tuck/releases/latest)

## Requirements

Windows 10 or 11 on x64, with Microsoft Edge WebView2 Runtime installed. Offline dictation needs an AVX2-capable CPU and a downloaded speech model. Model sizes vary; check the picker before downloading.

## Features

- Media playback controls and optional lyrics.
- Offline dictation with Whisper or Parakeet models.
- Clipboard history, timers, stopwatch, and Pomodoro.
- Battery and system resource information.
- Optional usage readings from supported installed AI clients.
- Tray controls, keyboard navigation, and optional startup at sign-in.

## Installation and updates

Download the installer from GitHub Releases. The release workflow signs updater packages, but does not apply Windows Authenticode signing. Windows SmartScreen may show an unknown-publisher warning. Check that the installer came from this repository before choosing More info and Run anyway.

Tuck checks for updates about 20 seconds after startup and every six hours. Settings > About and the tray can also check. Downloads start only after you choose Download and install. Update signatures are checked before installation. Signing does not remove SmartScreen warnings.

## Privacy

Speech recognition runs on your computer. Dictation settings and downloaded models are stored locally. Clipboard history is saved locally; it can contain sensitive copied text. Treat your Windows account and local app data as sensitive.

Tuck is not fully offline. Updates contact GitHub. Optional model downloads contact Hugging Face. Lyrics requests send track details to LRCLIB. Claude and Codex each start off; switches appear only when their executable is found. With Claude on, Tuck asks Claude Code for limits and the client may contact Anthropic; the optional Tuck status-line file is local. With Codex on, Tuck asks its app-server for limits and the client may contact OpenAI. Turning either off cancels its reader and removes its rings. Tuck reads no other app's credential files and sends no prompts or login requests. Antigravity is deferred because the researched T3 Code reader requires a token file. Network speeds use local Windows interface counters only while System is visible in the open island, without network requests.

Local processing does not mean local files are encrypted.

## Build from source

Install Bun (CI uses 1.4.2), stable Rust with the MSVC target, Microsoft C++ Build Tools with the Windows SDK, Python, WebView2, and CMake 3.22 or newer. Install CMake with `winget install Kitware.CMake` and make its bin directory available on PATH.

```powershell
bun install --frozen-lockfile
. .\scripts\prepare-native.ps1 -InstallBuildTools
bun run tauri dev
```

The script installs pinned libclang into the repository's ignored artifacts directory. In each new shell, dot-source it without `-InstallBuildTools` before running Cargo. `bun run dev` opens a browser demo with test data.

```powershell
. .\scripts\prepare-native.ps1
bun run tauri build
```

Installers are written to `src-tauri/target/release/bundle/nsis/`.

## Documentation

- [Contributing and checks](CONTRIBUTING.md)
- [Releasing and updates](docs/releasing.md)
- [Installed-client usage](docs/installed-client-usage.md)
- [Model attribution](THIRD_PARTY_NOTICES.md)
- [Support](SUPPORT.md) and [security reports](SECURITY.md)

## Licence

[MIT](LICENSE), copyright 2026 Tuck contributors. Third-party components and model weights retain their own terms.

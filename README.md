# Tuck

A Dynamic Island style desktop utility for Windows 10/11, built with Tauri 2, Rust and TypeScript.

## Develop

Install Bun, stable Rust with the MSVC target, Microsoft C++ Build Tools and WebView2. Then run:

```powershell
bun install --frozen-lockfile
.\scripts\prepare-native.ps1 -InstallBuildTools
bun run tauri dev
```

`bun run dev` runs a clearly labeled browser demo. The desktop app connects to Windows media and power services. Use the tray menu to open settings, focus the island for keyboard operation, or quit.

## Check

```powershell
bun test
bun run build
bun run test:ui
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

The rendered UI smoke uses installed Microsoft Edge in headless mode and intercepts build assets locally; no preview server is required. Native desktop and permission smoke checks are documented separately.

Offline speech adds CMake and libclang build prerequisites. `prepare-native.ps1` prepares them in the current PowerShell session; it downloads libclang only with `-InstallBuildTools`. It does not download a speech model or open a microphone. In a new shell, run the script again without that switch before Cargo. CI prepares its own pinned libclang dependency.

Phase-two work is tracked in [the approved scope](docs/live-island-phase-two.md) and [integration verification](docs/phase-two-verification.md). Its PR remains a draft until independent review and checks finish.

## Releases and Installation

Packaged installers and updates are published automatically on GitHub Releases.

> [!NOTE]
> **Windows SmartScreen Notice:** Tuck is an open-source project and is not signed with a paid Microsoft Authenticode certificate. On first install, Windows SmartScreen will display an *"Unknown Publisher"* warning. Click **More info** &rarr; **Run anyway** to proceed.

Tuck checks for updates at startup and every 6 hours, shows **Update available** in Settings and the tray menu, and downloads only when you click. See [Releasing Guide](docs/releasing.md) for full details on release automation and signing.

## Scope and setup

See [feature inventory](docs/feature-matrix.md), [Windows verification](docs/windows-verification.md), and [usage setup](docs/usage-setup.md), [installed-client sources](docs/installed-client-usage.md), and [v1 integrations](docs/v1-integrations.md). Optional monitoring and startup require opt-in. API budgets cover API spending, not subscription prices. ChatGPT counts are manual estimates.

No signing certificate, updater publication or release is configured by this work.

If another notch utility is already running, close it manually before testing Tuck; both target the same top-center screen position. The validation executable is a debug build and does not establish signing or public-release readiness.

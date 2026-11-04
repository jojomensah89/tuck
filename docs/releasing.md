# Releasing Tuck

This document explains the release pipeline, the in-app update mechanism, signing key management, and Windows installation behavior.

---

## 1. Automated Release Pipeline

Tuck uses an automated continuous release workflow defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml).

### Trigger and Lifecycle

1. **Trigger:** Every push or merge to the `main` branch.
2. **Concurrency:** Serialized with `concurrency: { group: "release", cancel-in-progress: false }` to prevent concurrent runs or version collisions.
3. **Branch Protection:** Queried via `gh api repos/jojomensah89/tuck/branches/main/protection`. Currently, `main` does not have branch protection rules enabled (`404 Branch not protected`). If branch protection is enabled in the future (such as requiring signed commits or blocking direct pushes), releases must use an administrative GitHub Personal Access Token (PAT) with repository write permissions or bypass rules for the GitHub Actions bot.

### Version Calculation & Conventional Commits

The next semantic version is determined automatically by [`scripts/release-metadata.ts`](../scripts/release-metadata.ts) from conventional commit subjects since the previous tag:

- `feat` / `feat(...)` &rarr; **Minor** release (`0.1.0` &rarr; `0.2.0`).
- `fix` / `perf` &rarr; **Patch** release (`0.1.0` &rarr; `0.1.1`).
- Breaking changes (`!` or `BREAKING CHANGE:` footer) &rarr; **Minor** while under `1.0.0` (as recommended by SemVer spec §4), and **Major** once `1.0.0` or higher.
- `docs`, `test`, `chore`, `ci`, `style`, `refactor` &rarr; **No release**.

If a push contains only non-release commits, the workflow terminates early with no tags or release artifacts created.

### Release Artifacts & Files Synced

When a release is triggered:
1. Version numbers are synchronized across:
   - `package.json`
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/Cargo.lock`
2. [`CHANGELOG.md`](../CHANGELOG.md) is updated in [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.
3. The bump is committed with `chore(release): vX.Y.Z [skip ci]` to ensure the commit itself does not trigger another workflow run.
4. Git tag `vX.Y.Z` is pushed.
5. `tauri-apps/tauri-action` builds the application on Windows (`windows-latest`).
6. A GitHub Release is published containing:
   - The NSIS executable installer (`tuck_<version>_x64-setup.exe`).
   - The updater signature (`tuck_<version>_x64-setup.exe.sig`).
   - `latest.json` updater manifest pointing to the release assets.

> [!NOTE]
> Tuck only produces single-file NSIS installers (`bundle.targets: ["nsis"]`). MSI bundles are intentionally disabled so all updates share one consistent target and installation path.

---

## 2. In-App Updates

Tuck includes an in-app updater powered by `@tauri-apps/plugin-updater`.

### Update Checks and Timing

- **Startup Check:** Tuck checks for new updates shortly after startup.
- **Cadence:** Updates are checked every 6 hours using a one-shot `setTimeout` timer. It sleeps without background polling or CPU wakeups between cycles.
- **Quiet Failures:** Background checks fail silently if the machine is offline or GitHub Releases cannot be reached. Error messages are only surfaced when the user initiates a manual check.
- **Settings UI:** The **About** view in Settings displays:
  - The current Tuck version.
  - A **Check for updates** button.
  - When an update is found, an **Update available** row expands, showing the new version tag, release notes, and a **Download and install** button with real-time download progress.
- **No Automatic Downloads:** Tuck *never* downloads an update payload without an explicit user click.
- **Tray Menu Status:** When an update is detected, the system tray menu item dynamically changes from `Check for updates` to `Update to vX.Y.Z`. Clicking this opens Settings directly to the update details.
- **Island Integrity:** The dynamic island notch does not display update banners or dialogs. All update controls are confined to Settings and the tray menu.

---

## 3. How to Skip a Release

If you want to merge commits to `main` without triggering a release:
1. Use non-release commit types: `docs:`, `test:`, `chore:`, or `ci:`.
2. Or include `[skip ci]` in the merge commit message.

---

## 4. Signing Keys and Secrets

Tauri updater packages are signed using `minisign` cryptography.

- **Updater Public Key:** Configured in `src-tauri/tauri.conf.json`:
  ```
  dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IENCRDgyNTk0N0I5NjY4RUQKUldUdGFKWjdsQ1hZeTFGQzhadW5WcVBvcUN4TnZvMFdvVGNOdDYwdUVFRWN2NURETFJ5YlUzK1IK
  ```
- **Private Key Storage (Developer Machine):**
  - Private key: `%USERPROFILE%\.tauri\tuck-updater.key`
  - Key password: `%USERPROFILE%\.tauri\tuck-updater.key.password`
- **GitHub Actions Secrets:**
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

> [!CAUTION]
> **CRITICAL: BACK UP YOUR SIGNING KEY!**
> The owner must securely back up `%USERPROFILE%\.tauri\tuck-updater.key` and its password. If this private key is lost or corrupted, existing installations of Tuck cannot verify new updates and will reject them. A lost key requires distributing a manual re-install to all users.
> Never print or log private keys in CI logs, scripts, or commit history.

---

## 5. Windows Installation Notes

### Microsoft SmartScreen Warning
Tuck is an open-source tool and is not signed with an expensive Microsoft Authenticode EV code-signing certificate. Consequently, Windows Defender SmartScreen will display an *"Unknown Publisher"* warning when running the installer for the first time:
- Users should click **More info** &rarr; **Run anyway**.

### Windows 11 System Tray Promotion
On Windows 11, newly installed application tray icons are automatically placed inside the **Taskbar Overflow Flyout** (`^` chevron). Windows APIs do not permit third-party applications to programmatically force promotion to the main visible taskbar.
- Users who want Tuck's tray icon permanently visible can drag the icon from the overflow menu onto the taskbar, or enable it in **Windows Settings &rarr; Personalization &rarr; Taskbar &rarr; Other system tray icons**.

### System Tray Testing & Screenshots
The system tray icon and its native Win32 context menu are rendered directly by the Windows Shell (`Shell_NotifyIconW`), completely outside the Chromium/Edge webview DOM. Consequently, the native tray menu cannot be screenshotted or automated with browser-based tools like Playwright. The web-based updates UI in Settings is verified and captured via Playwright in [`docs/screens/settings-updates.png`](./screens/settings-updates.png).

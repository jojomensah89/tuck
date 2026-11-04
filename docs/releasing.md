# Releasing Tuck

This document explains the release pipeline, the in-app update mechanism, signing key management, and Windows installation behavior.

---

## 1. Automated Release Pipeline

Tuck releases itself from [`.github/workflows/release.yml`](../.github/workflows/release.yml) on every push to `main`. The git and GitHub steps are in [`scripts/release-steps.sh`](../scripts/release-steps.sh), so [`tests/release-steps.test.ts`](../tests/release-steps.test.ts) can run the same commands against a local bare remote.

### One build, then a publish of that build

| Job | Runs on | Token | Signing key | What it does |
|---|---|---|---|---|
| `check` | Windows | read | no | Tests the commit: `bun test`, build, UI smoke, `cargo fmt`, `cargo test`, `clippy`. |
| `build` | Windows | read | yes | Bumps the version in a **local** commit, builds and signs that commit, checks the signature against `tauri.conf.json`, writes `latest.json`, and uploads everything as the `release-assets` workflow artifact. |
| `publish` | Ubuntu | write | no | Runs only for a live release. Steps below. |

`publish` does this, in this order:

1. Finishes any earlier release left as a draft (see [Recovery](#6-recovery)).
2. Stops successfully if `main` is no longer the commit this run tested.
3. Creates a **draft** release and uploads the installer, `.sig` and `latest.json`, with retries.
4. Runs `git push --atomic origin HEAD:main vX.Y.Z`. The commit and tag land together or not at all.
5. Publishes the draft and confirms `releases/latest/download/latest.json` serves the new version.

Installed copies only ever see a complete release, because `releases/latest` ignores drafts. The tag always points at the commit that was built, and that commit differs from the tested one only in `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `CHANGELOG.md`. The script enforces this.

### When main moves during a release

Runs share the `release` concurrency group, so one runs at a time and a newer waiting run replaces an older waiting one. A run whose commit is no longer the tip of `main` exits successfully without releasing. The newest run reads everything since the last tag, so nothing is lost. If the only newer commit has `[skip ci]`, the next merge picks it all up.

### Version calculation

[`scripts/release-metadata.ts`](../scripts/release-metadata.ts) reads the first-parent history since the last `vX.Y.Z` tag:

- Merge commits: this repository sets `merge_commit_message` to `PR_TITLE`, so the title is the first line of the commit **body**. A PR merge with no title stops the run.
- Squash commits: the title is the commit subject.

| Title | Release |
|---|---|
| `feat` | minor |
| `fix`, `perf` | patch |
| `!` after the type, or a `BREAKING CHANGE:` footer in the last paragraph | minor below 1.0.0, then major |
| `docs`, `test`, `chore`, `ci`, `style`, `refactor`, `build`, `revert` | none |
| Anything else | none, with a warning in the run log |

A commit reverted in the same range counts for neither the version nor the notes. PR titles become the changelog, so write them in plain language.

### Skipping a release

Use a non-release type in the PR title, or put `[skip ci]` in the merge commit message. Skipped commits are still covered by the next release.

### Files published

Tuck builds a single NSIS installer (`"bundle": { "targets": "nsis" }`, no MSI), so every update has one target and one install path. Each release holds:

- `tuck_<version>_x64-setup.exe`
- `tuck_<version>_x64-setup.exe.sig`
- `latest.json`, with `windows-x86_64` and `windows-x86_64-nsis` both pointing at the installer and carrying the contents of the `.sig`

### Rehearsing a release

Actions > Release > Run workflow, with `dry_run` on (the default), or:

```powershell
gh workflow run release.yml --ref main -f dry_run=true
```

This runs the checks and the real signed build, prints the version and notes, and uploads the `release-assets` artifact. The `publish` job does not run, and the `build` job's token cannot write. On any branch other than `main` every run is a dry run, whatever `dry_run` says. If the branch has no version tag, the job makes a **local** one at the merge base with `main`, so the range matches what a merge would release.

---

## 2. In-App Updates

Tuck includes an in-app updater powered by `@tauri-apps/plugin-updater`.

- **Checks:** 20 seconds after startup, then every 6 hours with a one-shot `setTimeout` rescheduled after each check. Nothing runs in between. Background failures stay quiet. Only a check you start reports an error.
- **Tray:** `Check for updates` becomes `Update available (vX.Y.Z)`. Clicking it opens Settings at **About** and checks again.
- **Settings:** **About** shows the current version and a **Check for updates** button. When an update is known, including one the background check finds while Settings is open, an **Update available** row shows the version, the notes and **Download and install**.
- **Downloads:** Only after that click. Tuck refuses while dictation is doing anything other than idle or error.
- **Install:** Tuck stops its background services and hands over to the installer. If the installer cannot start, Tuck restarts itself so those services come back.
- **Island:** No update UI at all.

---

## 3. First release

The first automatic release needs a baseline to count from. Immediately before merging the PR that adds this pipeline, tag the current `main`:

```powershell
git fetch origin
git tag v0.1.0 origin/main
git push origin v0.1.0
```

`v0.1.0` has no GitHub release, and the pipeline treats it as a baseline. The merge then releases `v0.2.0` with only that PR in its notes. Without the tag, the run stops with an error rather than releasing the whole history.

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

---

## 6. Recovery

Re-running the failed workflow is always safe, and it is the first thing to try. Use **Re-run all jobs**, or **Re-run failed jobs** if only `publish` failed.

| What happened | State | What to do |
|---|---|---|
| `check` or `build` failed | Nothing pushed, no release | Fix it. The next merge, or a re-run, starts clean. |
| Draft upload failed | A draft at most, no push | Re-run. Drafts whose tag never reached the remote are deleted, and a new one is created. |
| Push rejected because `main` moved | Nothing pushed, draft removed, run is green | Nothing. The newer run releases it. |
| Publish failed after the push | Commit and tag pushed, complete draft | Re-run, or merge anything. Either one publishes the draft without building again, before any newer version. |
| `<tag> was released from CI but has no GitHub release` | Someone deleted the draft or release | Rebuild from the tag, below. |
| `The <tag> draft is missing ...` | Someone edited the draft | Delete it, then rebuild from the tag, below. |
| `Latest tag ... does not match package.json version` | A tag or version was changed by hand | Make the newest `vX.Y.Z` tag on `main` equal `package.json`, then re-run. |
| `Merge commit ... has no PR title` | A merge message was edited or emptied, so history can't say what it contained | Release by hand once to move the range past it, below. |
| `Tag <tag> already exists but main has no release commit` | A tag was pushed by hand | If it was a mistake, delete it with `git push origin :refs/tags/<tag>` and re-run. |

### Releasing by hand past a bad merge

Set the new version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, add its section to `CHANGELOG.md`, and run `cargo update -p tuck --manifest-path src-tauri/Cargo.toml`. Then commit and push the commit and tag together, and rebuild from the tag as below:

```powershell
git add CHANGELOG.md package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(release): vX.Y.Z [skip ci]"
git tag vX.Y.Z
git push --atomic origin HEAD:main vX.Y.Z
```

### Rebuilding a release from its tag

This uses the signing key on the release machine, and needs the build tools from `scripts/prepare-native.ps1`.

```powershell
git switch --detach vX.Y.Z
bun install --frozen-lockfile
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\tuck-updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Get-Content -Raw "$env:USERPROFILE\.tauri\tuck-updater.key.password"
bun run tauri build
# Notes for the version, without its "## [X.Y.Z]" heading, in release-notes.md
bun run scripts/release-assets.ts --version X.Y.Z --notes-file release-notes.md --out release-assets
gh release create vX.Y.Z --verify-tag --title "Tuck vX.Y.Z" --notes-file release-assets/release-notes.md --latest `
  release-assets/tuck_X.Y.Z_x64-setup.exe release-assets/tuck_X.Y.Z_x64-setup.exe.sig release-assets/latest.json
```

Close that terminal afterwards so the key leaves its environment.

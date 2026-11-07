# Releasing Tuck

This document explains the release pipeline, the in-app update mechanism, signing key management, and Windows installation behavior.

---

## 1. Automated Release Pipeline

Tuck releases itself from [`.github/workflows/release.yml`](../.github/workflows/release.yml) on every push to `main`. The version decision and synchronization checks are in [`scripts/release-plan.ts`](../scripts/release-plan.ts); git and GitHub steps are in [`scripts/release-steps.sh`](../scripts/release-steps.sh). The focused fixtures in [`tests/release-plan.test.ts`](../tests/release-plan.test.ts) and [`tests/release-steps.test.ts`](../tests/release-steps.test.ts) exercise those paths against temporary repositories and a local bare remote.

### Version PR, then one build and publish

| Job | Runs on | Token | Signing key | What it does |
|---|---|---|---|---|
| `check` | Windows | read | no | Tests the commit: `bun test`, build, UI smoke, `cargo fmt`, `cargo test`, `clippy`. |
| `prepare` | Ubuntu | PR/write | no | Calculates the next version, opens an idempotent version-only PR, and dispatches Windows checks for that branch. |
| `build` | Windows | read | yes | Runs after the version PR is merged; builds and signs that exact `main` commit, checks the signature, writes `latest.json`, and uploads the assets. |
| `recover` | Ubuntu | release write | no | Finishes an earlier tagged draft after checks, even if no new build is needed. |
| `publish` | Ubuntu | tag/release write | no | Creates a draft, pushes only the version tag, publishes the draft and confirms the feed. |

The workflow has two stages; recovery also runs independently after checks:

1. Opens a version-only PR when feature/fix merges warrant a release. Review and merge it normally; automation does not approve or merge it.
2. After that PR is merged, checks, builds and signs the merged `main` commit without changing it.
3. Confirms no tracked files changed during the build.
4. Creates a **draft** release and uploads the installer, `.sig` and `latest.json`, with retries.
5. Pushes only `vX.Y.Z`, then publishes the draft and confirms the update feed.

The separate `recover` job can finish an already tagged draft on a no-release merge or full rerun, without rebuilding. Preparation, signing and publishing are restricted to their separate job permissions. Default repository token permissions stay read-only. GitHub Actions PR creation must be enabled in repository Actions settings; the release workflow never calls PR approval or merge. Windows checks are explicitly dispatched on each prepared branch.

Installed copies only ever see a complete release, because `releases/latest` ignores drafts. The tag points at the already merged commit that was built. Protected `main` is never written by the release workflow.

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

- `Tuck_<version>_Setup.exe`
- `Tuck_<version>_Setup.exe.sig`
- `latest.json`, with `windows-x86_64` and `windows-x86_64-nsis` both pointing at the installer and carrying the contents of the `.sig`

### Rehearsing a release

Actions > Release > Run workflow, with `dry_run` on (the default), or:

```powershell
gh workflow run release.yml --ref main -f dry_run=true
```

This runs the checks and the real signed build for the checked-out `main` commit, prints the version and notes, and uploads the `release-assets` artifact. The `publish` job does not run, and the `build` job's token cannot write. A dry run is useful even when the current version is already tagged; it validates the synchronized version files and rebuilds the current release without creating a PR, tag or release. A live manual run (`dry_run=false`) on main uses the same prepare-or-build decision as a main push. On a feature branch, only dry runs are supported.

---

## 2. In-App Updates

Tuck includes an in-app updater powered by `@tauri-apps/plugin-updater`.

- **Checks:** 20 seconds after startup, then every 6 hours with a one-shot `setTimeout` rescheduled after each check. Nothing runs in between. Background failures stay quiet. Only a check you start reports an error.
- **Tray:** `Check for updates` becomes `Update available (vX.Y.Z)`. Clicking it opens Settings at **About** and checks again.
- **Settings:** **About** shows the current version and a **Check for updates** button. When an update is known, including one the background check finds while Settings is open, an **Update available** row shows the version, the notes and **Download and install**.
- **Downloads:** Only after that click. Tuck refuses while dictation is recording, transcribing, reviewing or inserting, or while a speech model is downloading or being removed. A missing speech model does not block an update.
- **Install:** Tuck stops its background services and hands over to the installer. If the installer cannot start, Tuck restarts itself so those services come back.
- **Island:** No update UI at all.

---

## CPU requirement

Releases set `GGML_NATIVE=OFF`, so the speech engine targets AVX2 rather than whichever CPU the build machine has. Without this, a release built on an AVX-512 runner would crash on PCs without it. AVX2 covers most PCs from about 2013 onward; dictation will not run on older CPUs. The `Confirm the CPU baseline` step fails the release if these settings ever stop applying, and the build job has its own cache key because `whisper-rs-sys` does not rebuild when this variable changes.

## 3. First release

The first automatic release needs a baseline to count from. Immediately before merging the PR that adds this pipeline, tag the current `main`:

```powershell
git fetch origin
git tag v0.1.0 origin/main
git push origin v0.1.0
```

`v0.1.0` has no GitHub release, and the pipeline treats it as a baseline. The merge then opens a version PR for `v0.2.0`; merging that PR triggers its checked, signed release. Without the tag, the run stops with an error rather than releasing the whole history.

If you already merged, tag the commit before the merge, not `origin/main`, or the run finds nothing to release:

```powershell
git fetch origin
git tag v0.1.0 origin/main^1
git push origin v0.1.0
```

Then re-run the failed workflow.

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

Re-running the failed workflow is safe and is the first thing to try. If a newer merge is already queued, let that run go instead: it covers everything since the last tag. Use **Re-run all jobs**, or **Re-run failed jobs** if only `publish` failed.

| What happened | State | What to do |
|---|---|---|
| `check` or `build` failed | Nothing pushed, no release | Fix it. The next merge, or a re-run, starts clean. |
| Draft upload failed | A draft at most, no push | Re-run. Only the draft for the version being retried is replaced if its tag never reached the remote. |
| Push rejected because `main` moved | Nothing pushed, draft removed, run is green | Nothing. The newer run releases it. |
| Publish failed after the push | Only tag pushed, complete draft | Re-run, or merge anything. Either one publishes the draft without building again, before any newer version. |
| `<tag> was released from CI but has no GitHub release` | Someone deleted the draft or release | Rebuild from the tag, below. |
| `The <tag> draft is missing ...` | Someone edited the draft | Delete it, then rebuild from the tag, below. |
| Version files disagree, or package version is behind the latest tag | A version file or tag was changed by hand | Correct the version files through a PR. Do not move a published tag. |
| `Merge commit ... has no PR title` | A merge message was edited or emptied, so history can't say what it contained | Release by hand once to move the range past it, below. |
| Existing tag points at a different commit | A version was reused or tagged separately | Stop and inspect provenance. Never replace a published tag; use a new version PR if necessary. |
| Warning: `latest.json does not serve <version>` | The feed serves another version, because a release was pulled or marked as a pre-release, or the feed is stale | Nothing, if you pulled it: the warning doesn't block the next release. If you didn't, check which release is marked Latest. |

### Releasing by hand past a bad merge

For a bad merge that cannot be described by the normal Conventional Commit rules, make a small version-only PR. Set the new version in `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and the root `tuck` entry in `src-tauri/Cargo.lock`, add its section to `CHANGELOG.md`, and merge it through the protected `main` branch. Do not run `cargo update` for this recovery: it can rewrite unrelated dependencies. The next release workflow builds that merged commit and pushes only its tag.

```powershell
git add CHANGELOG.md package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git switch -c recovery/release-vX.Y.Z
git commit -m "chore(release): vX.Y.Z"
git push -u origin HEAD
# Open a PR into main, wait for checks, and merge it. The release workflow does the build and tag.
```

### Rebuilding a release from its tag

This uses the signing key on the release machine, and needs the build tools from `scripts/prepare-native.ps1`.

```powershell
git switch --detach vX.Y.Z
bun install --frozen-lockfile
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\tuck-updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Get-Content -Raw "$env:USERPROFILE\.tauri\tuck-updater.key.password"
# Without these, whisper.cpp targets this machine's CPU and the release can crash on other PCs.
# The clean matters: changing the variable doesn't rebuild an existing target directory.
$env:GGML_NATIVE = "OFF"
cargo clean --manifest-path src-tauri/Cargo.toml
bun run tauri build
bash scripts/release-steps.sh verify-cpu-baseline
# Notes for the version, without its "## [X.Y.Z]" heading, in release-notes.md
bun run scripts/release-assets.ts --version X.Y.Z --notes-file release-notes.md --out release-assets
gh release create vX.Y.Z --verify-tag --title "Tuck vX.Y.Z" --notes-file release-assets/release-notes.md --latest `
  release-assets/Tuck_X.Y.Z_Setup.exe release-assets/Tuck_X.Y.Z_Setup.exe.sig release-assets/latest.json
```

Close that terminal afterwards so the key leaves its environment.

### Installer branding

The NSIS installer and uninstaller use Tuck's folded-T icon. Release staging renames the built installer to `Tuck_<version>_Setup.exe` without changing its signed bytes. The signature filename and both updater platform URLs use the same public name. Recovery accepts the legacy `_x64-setup.exe` names for older drafts. Product name, app identifier and install path remain unchanged for existing users.

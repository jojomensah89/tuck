# Microsoft Store MSIX

Build on Windows with the README prerequisites and Windows SDK:

```powershell
bun install --frozen-lockfile
. ./scripts/prepare-native.ps1 -InstallBuildTools
bun run tauri build --no-bundle
$env:MSIX_IDENTITY_NAME = 'your Partner Center identity'
$env:MSIX_PUBLISHER = 'CN=your Partner Center publisher'
$env:MSIX_PUBLISHER_DISPLAY_NAME = 'Your publisher name'
bun run package:msix
```

The script emits `artifacts/msix/Tuck_0.3.0.0_x64.msix` for version 0.3.0.
It packages the release executable, not the NSIS installer. No signing key is
used: the Store signs submissions. The default identity in
`src-tauri/msix/identity.json` holds the product identity of **Tuck Island** in
Partner Center (Store ID 9NKJQ3M3XQHX), so a local build is ready to submit. CI
reads the same values from four repository variables: `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER` and
`MSIX_PUBLISHER_DISPLAY_NAME` from Partner Center > Product management > Product
identity, and `MSIX_STORE_NAME`, the product name you reserved, typed exactly as
reserved (the package's `Properties/DisplayName` must match it). Tuck still shows
as "Tuck" in the Start menu. If any value changes in Partner Center (for example
the publisher display name), update both identity.json and the variables.
Reserve an **MSIX or PWA app**, then upload the unsigned MSIX through its package
submission page. This is separate from Tauri's EXE/MSI submission route.

Suggested runFullTrust justification:

> Tuck is a Windows desktop utility that displays an interactive desktop island,
> controls media and volume, supports local speech dictation and user-enabled
> clipboard and notification tools. Full trust is needed for desktop window
> management, global keyboard shortcuts and integration with Windows desktop APIs.

Certification review notes:

- A low-level keyboard hook supports dictation shortcuts and volume keys.
- Clipboard history is off by default; notification listener access requires user permission.
- Dictation processing is local. Model downloads and optional online integrations
  still need network access; do not describe the entire application as offline.
- Packaged startup uses the declared `TuckStartup` task. Task Manager or policy
  can prevent enabling it; Settings displays the restriction.
- Updates come from the Microsoft Store. The website installer keeps its updater.
- Settings do not carry over from the website installation. MSIX redirects app data;
  test first-run and model storage in the installed package.
- Test Windows S mode certification restrictions, including blocked executables
  and WebView2 behavior; passing compilation alone is not certification evidence.

Local testing: turn on Developer Mode (Windows Settings > System > Advanced, or
For developers on older builds), then register the unpacked package. This needs no
certificate and is how the installed package was verified:

```powershell
$dir = "$env:LOCALAPPDATA\tuck-msix-test"
makeappx unpack /p .\artifacts\msix\Tuck_0.3.0.0_x64.msix /d $dir /o
Stop-Process -Name tuck -ErrorAction SilentlyContinue   # Tuck allows one copy at a time
Add-AppxPackage -Register "$dir\AppxManifest.xml"
Start-Process "shell:AppsFolder\JojoMensh.TuckIsland_kb81ffxhyes9c!Tuck"
# afterwards: Get-AppxPackage JojoMensh.TuckIsland | Remove-AppxPackage
```

Alternatively, for Windows 11 unsigned package installs, build a separate test package with:

```powershell
./scripts/package-msix.ps1 -OutputDirectory artifacts/msix-local-test `
  -Publisher 'CN=TuckPlaceholder, OID.2.25.311729368913984317654407730594956997722=1'
Add-AppxPackage -Path .\artifacts\msix-local-test\Tuck_0.3.0.0_x64.msix -AllowUnsigned
```

[Microsoft's unsigned package guide](https://learn.microsoft.com/en-us/windows/msix/package/unsigned-package)
requires that OID for local unsigned deployment and administrator privileges for
executable content. Do not use the test OID for Partner Center submissions.
If this deployment route is blocked, create a test code-signing certificate whose
subject exactly matches the normal Publisher, trust its public certificate in the
test machine's Trusted People store, sign a COPY with
`signtool sign /fd SHA256 /f test.pfx package.msix`, and install the copy.
Never upload the test-signed package or commit the private certificate.

Launch from Start; verify startup on/off, Task Manager disabled state, About,
absence of update controls, tray behavior, notification permission, dictation,
restart and uninstall. Run Windows App Certification Kit from the Windows SDK:
select the installed packaged application in **Windows App Cert Kit**, run all
applicable tests, and retain its report. Resolve certification failures before
submission. Store identity and certification approval remain owner actions.

References: [startup task schema](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap5-startuptask),
[Tauri Store distribution](https://v2.tauri.app/distribute/microsoft-store/).

## Implementation validation

See the PR for current executed check results. Screenshots in `docs/screens/msix`
use mocked native identity/startup responses and show the real built Settings UI;
they are not proof of installed-package behavior.

Executed on 2026-11-09:

- Release executable built with the prepared native environment; makeappx validated
  the manifest and created the unsigned 0.3.0.0 x64 MSIX.
- `Add-AppxPackage -AllowUnsigned` with the placeholder Store-style publisher
  failed with 0x80073D2C (publisher outside unsigned namespace).
- A separate local-test package with the documented unsigned OID also built, but
  deployment failed with 0x80073D2B (unsigned executable activations rejected).
- No certificate was installed.
- Installed package (Developer Mode, loose registration as above, Windows build
  26300): ran with package identity from the package folder; the island drew;
  Start with Windows set the `TuckStartup` task to Enabled (state 2) with no Run
  key written; Settings > About read "Updates come from the Microsoft Store";
  the tray had no update item; notification access was Allowed and a test toast
  appeared on the island. WACK has not been run.
- TypeScript, 236 Bun tests, frontend build, full UI smoke, packaged Settings
  smoke, cargo fmt, cargo check, strict Clippy and default cargo tests passed.
- The ignored symlink test failed locally with Windows error 1314 (no symlink
  privilege in that session); it passes in CI.
- Full NSIS build produced its executable/installer, then failed updater signing
  because the private signing key is unavailable locally. The MSIX does not need it.

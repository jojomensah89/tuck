# Native file-drop regression probe

This runs real OLE `DoDragDrop` with a CF_HDROP data object and `SendInput` mouse input. It is a Windows integration test, not a DOM drag simulation. The source logs each feedback effect, cursor HWND ancestry, `OleDropTargetInterface` presence (diagnostics only), and `WS_EX_TRANSPARENT`.

## Isolated setup

Do not launch the normal identifier alongside an installed Tuck. Use this checkout's config:

```powershell
$env:CARGO_TARGET_DIR = 'F:\tuck-agent-target'
. .\scripts\prepare-native.ps1 -InstallBuildTools
$probeData = Join-Path $env:APPDATA 'com.jojo.tuck.drop-probe'
New-Item -ItemType Directory -Force $probeData | Out-Null
Set-Content (Join-Path $probeData 'legacy-credentials-v1.done') '1'
Set-Content (Join-Path $probeData 'welcome-seen') '1'
Set-Content (Join-Path $probeData 'usage-providers.json') '{"claude":false,"codex":false,"antigravity":false}'
$env:TUCK_DROP_TRACE = '1'
bun run tauri dev --no-watch --config scripts/file-drop-probe.config.json
```

The identifier is `com.jojo.tuck.drop-probe`. Debug builds place this identifier's island 400 physical pixels below the monitor's top so it cannot overlap the installed island. Confirm two different process paths/PIDs and the probe's separate AppData directory. Do not stop the installed process. The migration marker suppresses credential cleanup; usage providers are all disabled.

In another PowerShell session:

```powershell
.\scripts\test-file-drop.ps1 -ProcessId <probe-PID>
```

The helper refuses processes outside `F:\tuck-agent-target\` and selects the exact `Tuck Drop Probe` window title. It cancels if the final target is outside that island's HWND ancestry. Keep the mouse untouched during the test. The default compiler setup is VS 2019 BuildTools; adjust the script's `vcvars` path for another installation.

Cases: 35-second idle then collapsed entry; a second file at the slim strip (80 px below the window top); fresh collapsed entry; an already-open island; and a late native child HWND. The four drag cases move slowly from the pill down into the island. The late-child case also asserts registration before entering the island, exercising the native creation hook independently of the activity refresh. The late-child test reports `REGISTRATION PASS` and destroys its synthetic child; it does not count as a shelf drop. Every successful drop must report `hr=40100 effect=1 foreign=0`, exit 0, and a corresponding `drop-shelf added=1 total=N` in the app trace.

To measure the original Wry registration path with the identical debug executable/config, set `$env:TUCK_DROP_BASELINE='1'` before launch. This bypass is restricted to debug builds with the probe identifier. Unset it before testing the repair. The suite deliberately stops on failure; baseline cases can be run individually with the compiled helper:

```powershell
artifacts\file-drop-probe\ole-drop-probe.exe <PID> <absolute-temp-file> 120 35
# Args: PID, file, destination Y relative to island, idle seconds, optional open/late-child
```

`TUCK_DROP_TRACE` adds debug-only native hit-size, trim and shelf diagnostics. No secrets or file contents are logged. `OleDropTargetInterface` is read only by the diagnostic helper; production code uses public OLE registration APIs.

## Cleanup

The test removes its generated input files in `finally`. Quit only the probe process by its verified F: executable path and stop its dev command. Delete only `com.jojo.tuck.drop-probe` under Roaming and Local AppData, plus the generated probe executables/objects in `artifacts/`. Preserve `F:\tuck-agent-target` as the build cache. Never delete the installed app's `com.jojo.tuck` data.

## Evidence boundary

See `file-drop-evidence.md` for measured results. The owner still needs to repeat a physical Explorer drag with the resulting build, including their normal monitor/DPI setup. The helper does not reproduce Explorer's full data-format set, shell extensions, or modifier-key behavior.

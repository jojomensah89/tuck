# File-drop evidence (Windows, 2026-11-08)

## Reproduction and cause

The original Wry path was reproduced with the isolated debug identifier `com.jojo.tuck.drop-probe`, using the same executable's debug-only baseline switch. The installed `C:\Users\Jojo\AppData\Local\tuck\tuck.exe` remained running as PID 55916. Baseline PID 65964 and repaired PIDs ran from `F:\tuck-agent-target\debug\tuck.exe`, with separate app data. The probe island was placed at `(700,400)-(1220,688)` to avoid the installed island.

The source uses a real CF_HDROP IDataObject, native OLE `DoDragDrop`, and `SendInput`. The reported runs use the corrected source, with no synthetic WM_MOUSEMOVE messages. Earlier harness experiments and runs where another window overlapped the target were excluded. The helper's release guard checks the actual cursor HWND ancestry before permitting a drop.

Baseline main-window sample:

```text
pid=40068 class=Chrome_RenderWidgetHostHWND registered=0 transparent=1
pid=65964 [ancestor owner] class=Chrome_WidgetWin_0 registered=nonzero transparent=0
pid=65964 class=Tauri Window registered=nonzero transparent=0
RESULT hr=40100 effect=0 foreign=0
```

The renderer itself was owned by WebView2 PID 40068; the ancestors above belong to Tuck PID 65964. The raw renderer HWND was `0x180E84`, its Chrome_WidgetWin_1 parent `0x5A16DE`, and the island root `0x61BC6`. Both renderer HWNDs lacked registrations; the older Chrome_WidgetWin_0 and top-level HWND had them. Native hit telemetry showed `hovered=true size=500x264` during the failed open-island run. The root's `WS_EX_TRANSPARENT` was clear over the island; the renderer's own transparent style is distinct from root click-through.

This confirms hypothesis A's late-child registration gap. The same failure occurred fresh and after idle, so lowering the memory target to LOW is **not established as the trigger**. LOW trimming remains enabled. The slow path did not demonstrate hypothesis B: the failure occurred while the island root was hit-testable and its expanded hit bounds were present. Fast motion, other monitor/DPI configurations and physical Explorer behavior still need owner confirmation.

## Change

Register CF_HDROP targets for the island's current descendants at startup/activity. An event-driven WinEvent hook catches descendant creation/show notifications, including children owned by WebView2 processes. Targets are cached per HWND so the opening activity update cannot replace a target between DragEnter and DragOver; destruction removes the cached target. All registration runs on the Tauri UI thread. Standard targeted Tauri drag events feed the existing Files UI and shelf commands.

There is no new timer, polling loop, renderer idle task, click-through policy, or memory-trim change. OLE storage is released with ReleaseStgMedium. Other windows, settings and outbound shelf dragging keep their existing implementations.

## Results

`effect=0` is NONE; `effect=1` is COPY. `hr=40100` is DRAGDROP_S_DROP. Baseline exit 1 means the test correctly rejected the unsuccessful drop.

| Case | Baseline | Repaired |
|---|---|---|
| Collapsed after >30 seconds idle | effect 0, exit 1 | effect 1, exit 0; shelf total 1 |
| Fresh collapsed, slow pill-to-zone move | effect 0, exit 1 | effect 1, exit 0 |
| Already open | effect 0, exit 1 | effect 1, exit 0 |
| Second file, slim strip at window Y+80 | Not seeded in failed baseline | effect 1, exit 0; shelf total 2 |
| Native child created while idle | Not measured | registered before island activity, exit 0 |

The repaired shelf trace increased by one per real drop. A repeated run verified totals 5 through 8 as well. The independent late-child registration test creates and destroys a synthetic child and does not count it as a shelf drop. It logged a non-null registered target before the cursor entered the island, exercising the lifecycle hook without the activity refresh. Native logs also confirmed LOW memory trimming after idle and collapsed hit geometry returning to 212x32.

The Windows integration runner is `scripts/test-file-drop.ps1`; setup and cleanup are documented in [file-drop-probe.md](file-drop-probe.md). No physical Explorer drag is claimed: the owner still needs to test an actual Explorer source with their shell extensions, modifier keys, monitor and DPI settings.


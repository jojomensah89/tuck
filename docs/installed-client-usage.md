# Installed Claude Code and Codex usage sources

Checked locally on 2026-11-02 at the user's explicit request to use the approach implemented in T3 Code and PandaCode.

## Observed implementation evidence

PandaCode source at `C:/Users/Jojo/Desktop/Projects/pandacode/src-tauri/src/agent_usage.rs` uses an installed Codex app-server client for `account/read` and `account/rateLimits/read` (around lines 636-662). Its Claude adapter reads the saved login and calls the fixed first-party OAuth usage endpoint (around lines 680-764). These are code observations, not evidence of live requests by PandaCode in this session.

The installed T3 Code Nightly executable reports product version 0.1.3.0. A read-only copy of its server.asar was extracted under the temporary `re/tuck-t3-usage` directory. In `binCli-COJdIRDQ.mjs`, `probeClaudeCapabilities` initializes the Claude SDK with a never-yielding prompt, disabled hooks, no tools and no MCP servers, then calls `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET({skipBehaviors:true})` (around lines 205760-205865). Its Codex provider calls `account/read` and `account/rateLimits/read` (around lines 229414-229458).

In the extracted `claudeHistoryWorker-DLr2Wa_H.mjs`, the SDK method sends a `control_request` with `subtype:get_usage` and `skip_behaviors:true` (around lines 42432-42437). Initialization and message envelopes are visible around lines 42033 and 42300. Tuck reimplements these small read-only protocol interactions; it does not depend on or modify the installed T3 bundles.

## Tuck behavior

The optional installed-account source starts disabled. Enable it in Settings. Tuck starts the actual installed `.exe` directly with hidden console windows, sends only initialization/account/usage control requests and rejects reverse requests. No user prompt, turn, tool action, login, credit reset or auth refresh request is sent. No token file is read by Tuck. The installed clients manage their own authentication.

Native polling has a five-minute interval, a manual refresh option, per-request and total deadlines, bounded output/frame counts, discarded stderr and a child-process cleanup guard. Only quota percentages, reset times and fixed diagnostic messages enter the webview. Codex account email/identity fields and raw control responses are discarded. Codex windows describe Codex subscription usage on the ChatGPT plan, not a consumer ChatGPT message cap. The separate manual ChatGPT counter remains explicitly an estimate.

The Claude get_usage control method is experimental and can be unavailable in older clients or unsupported account configurations. Tuck reports that state rather than pretending a zero quota. Status-line and Admin API adapters remain available as separate sources.

## Local evidence

### Claude unavailable readings (2026-11-06)

Claude status-line percentages retained from a stale or unavailable reading are not current subscription quota. The card, ring and tooltip suppress those percentages, while retaining the status explaining that current limits are unavailable. A fresh general five-hour or weekly window at 100% shows `Limit reached`; exhaustion of a model-specific window does not imply all Claude models are blocked. Codex's existing retained-reading behavior is unchanged.

The read-only installed Claude control probe reported `tokenSource:none` and `rate_limits_available:false` in this check. No prompt was sent and no login was changed. Being signed into Claude's website does not establish that this separate CLI connection is authenticated. Run `claude auth login --claudeai` interactively for the same subscription account, then refresh Tuck. Command availability was verified against the installed CLI's help. Current live quota remains unverified until that connection supplies limits.

- Live installed Codex app-server probe returned two quota windows.
- Installed Claude Code initialization and get_usage control requests succeeded. The connection reported tokenSource:none, no subscription metadata and no subscription quota windows; Tuck displays unavailable.
- No actual quota percentages, account identifiers or credentials were printed during the smoke check.
- Two deterministic parser tests check account type, percentage/reset validity and identity omission. The live probe is an explicitly ignored test so CI does not require the user's logged-in accounts.

# Claude status-line fallback

Enable Claude in Settings > AI usage first. Tuck reads only its own forwarded status-line file; no credential files are read. The installed-client reader is preferred when it reports limits.

## 4. Claude Code Statusline Setup Instructions

Tuck ships a PowerShell statusline forwarder at `scripts/tuck-statusline.ps1`.

### Setup Steps:
1. Ensure the target directory `%APPDATA%\Tuck` exists (the script creates it automatically if missing).
2. Configure Claude Code to use the Tuck statusline script by updating `~/.claude/settings.json`:
   ```json
   {
     "statusLine": {
       "type": "command",
       "command": "powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"C:\\path\\to\\tuck\\scripts\\tuck-statusline.ps1\""
     }
   }
   ```
3. Whenever Claude Code runs a session, it pipes its JSON payload to the script, which atomically saves it to `%APPDATA%\Tuck\claude-status.json` and outputs `Tuck`.
4. Tuck also checks the legacy guide fallback path `%APPDATA%\Notch\claude-status.json` if the Tuck directory is not yet present.

---

## 5. Coordinator & Native Worker Integration Guide

To wire `src-tauri/src/usage.rs` into the main application:

### Step 1: Update `src-tauri/Cargo.toml`
Add the following dependencies under `[dependencies]`:
```toml
chrono = { version = "0.4", features = ["serde"] }
keyring = { version = "3", features = ["windows-native"] }
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
```

### Step 2: Update `src-tauri/src/lib.rs`
1. Declare the module:
   ```rust
   pub mod usage;
   ```
2. Register commands in `tauri::Builder`:
   ```rust
   .invoke_handler(tauri::generate_handler![
       // existing commands...
       usage::usage_now,
       usage::set_secret,
   ])
   ```
3. Start the background service in the Tauri setup hook:
   ```rust
   .setup(|app| {
       // start usage polling service
       usage::start(app.handle().clone());
       Ok(())
   })
   ```

## Review corrections and validation

After independent review, the cache snapshot became memory-only. Keyring and HTTP calls execute outside its mutex; credential edits clear cached spending and increment a generation so in-flight old-key requests cannot overwrite the new state. UTC month changes hide old totals. Request query parameters are encoded, redirects disabled, each provider fetch limited to 30 seconds total, each response capped at 5 MB, and malformed/missing amounts, currencies or pagination fail explicitly. Valid empty results can report zero; malformed results cannot.

Raw external error details are withheld rather than passed through a best-effort token sanitizer. The statusline forwarder uses an atomic unique temporary-file replacement and leaves the existing file intact on a failed replacement.

Coordinator validation: 16 usage unit tests plus 10 native tests pass against the wired Windows application. Formatting and Clippy pass. No real provider credentials or organization billing responses were used for validation.

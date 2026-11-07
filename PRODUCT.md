# Tuck

A local Windows 10/11 utility that places an interactive black notch or floating pill at the top of the primary display. It provides media transport, time and power status, live activities, AI usage information and a screen snip action and file shelf without taking focus during ordinary mouse use.

The approved product and visual direction is the user-supplied Notch build guide, adapted to the existing Tuck Tauri 2 and Vite/TypeScript scaffold. This delivery includes the specified Windows app and v1 features. Later ideas live in docs/feature-matrix.md.

Privacy: Claude and Codex each start off; switches appear only when their executable is found. With Claude on, Tuck asks Claude Code for limits and the client may contact Anthropic; the optional Tuck status-line file is local. With Codex on, Tuck asks its app-server for limits and the client may contact OpenAI. Turning either off cancels its reader and removes its rings. Tuck reads no other app's credential files and sends no prompts or login requests. Antigravity is deferred because the researched T3 Code reader requires a token file. Network speeds use local Windows interface counters only while System is visible in the open island, without network requests.

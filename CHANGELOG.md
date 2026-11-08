# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-11-08

### Added
- A taller island with a five-tab rail: Media, Today, Dictation, Tools and Usage
- Update alerts: one peek when an update is ready, then an icon in the rail and Settings until you install it; download and restart are separate clicks
- AI usage per provider, off until you turn it on: Claude, Codex and Antigravity, each with a 5-hour and a 7-day line, in colour brand icons
- Antigravity quota, read only after you switch it on and confirm
- Network download and upload speed in the System view
- Shelf: drop files on the island to keep them handy, shown as tiles with Windows thumbnails; drag them out anywhere, click to open, or show in folder
- The whole drop zone appears as soon as you drag a file over the island, and you go back to where you were if you don't drop
- Clipboard history as visual cards with Text, Links, Images, Videos and Files filters, image previews and file thumbnails
- A short welcome on first run: start with Windows, AI usage and a dictation model
- Settings follows the Windows light or dark theme, with denser rows that use the full window

### Changed
- Dictation works like Hex: no live text while you speak; the text is typed when you stop. With no text box focused it is copied, and Tuck says "Copied, paste with Ctrl+V"
- Tuck asks before downloading a missing dictation model, and reuses models you already have
- Refresh buttons spin until the refresh finishes
- A motion and detail pass across every screen: shorter, calmer transitions, no animation while idle, and focus rings only for the keyboard
- Lower resource use: the cursor is checked less often when it isn't moving, and clipboard images are capped at 128 MiB
- Downloads, Quick Actions and Focus were removed; Screen snip moved to an icon in Tools

### Fixed
- Files dropped from File Explorer were refused ("not allowed")
- A fresh install could freeze, with Settings opening twice and blank
- Antigravity showed "quota unavailable"; when it fails, it now says which step failed

## [0.2.1] - 2026-11-06

### Fixed
- Remove selected island filler text
- Release through version PRs on protected main
- Brand Windows setup and simplify download names

## [0.2.0] - 2026-11-05

### Added
- Automatic updates and a fuller tray menu

## [0.1.0] - 2026-11-04

### Added
- Dynamic Island notch interface for Windows at the top of the screen
- System audio and lyrics integration with GSMTC and LRCLIB
- Real-time audio visualizer and volume/brightness level HUD
- Offline dictation powered by whisper.cpp and local speech models
- Clipboard history with privacy preservation and in-memory storage
- System tray integration with quick controls
- AI usage monitors and spend tracking for Claude and OpenAI
- Raycast-style settings window for desktop customization

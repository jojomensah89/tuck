# Microsoft Store listing text

Paste these into Partner Center > Tuck Island > Start your submission. Product name: **Tuck Island** (reserved). Privacy policy URL: `https://tuck.jojomensah.com/privacy` (live once the site deploys).

## Properties
- Category: **Productivity**
- Support contact: `https://github.com/jojomensah89/tuck/issues`
- Website: `https://tuck.jojomensah.com`
- Privacy policy: `https://tuck.jojomensah.com/privacy`
- Copyright and trademark info: `© 2026 Tuck contributors`
- Pricing: Free, all markets, no trial.

## Age rating questionnaire
No violence, sexual content, language, gambling, drugs or purchases. No user-generated content or chat. No location sharing. It does not collect personal information. Expected rating: 3+ (PEGI 3 / ESRB Everyone).

## Short description (up to 100 characters)
A tiny island at the top of your screen for music, lyrics, dictation, clipboard and timers.

## Description
Tuck hangs from the top edge of your screen like a notch and opens when you reach for it.

**Music and lyrics.** See what is playing in Spotify, Apple Music, your browser or Windows Media Player, with synced lyrics, artwork, and play, pause and skip controls.

**Offline dictation.** Record what you say and get the text back. Speech is turned into text on your own processor, so nothing is uploaded after the one-time download of the model you pick.

**Clipboard history and a file shelf.** Everything you copied is a click away: text, links, images and files. Drop files on the shelf to carry them between windows.

**Timers and the day.** A focus timer and the clock, right on the island.

**AI usage at a glance.** Rings show how much of your Claude and Codex limits are left.

**Light and private.** About 5 MB in Task Manager while it rests, no account, and no analytics. It hides when an app goes full screen, and clicks pass through everywhere outside the island.

Free and open source (MIT). Source and issues: https://github.com/jojomensah89/tuck

## What's new (first version)
First release on the Microsoft Store.

## Keywords (7)
dynamic island, notch, clipboard history, dictation, lyrics, file shelf, widgets

## Screenshots (1920x1080)
Six PNGs, in this order: media and lyrics, timers, offline dictation, file shelf, clipboard history, AI usage. They show the real island with demo data and public-domain songs.

## Capability justifications (paste when Partner Center asks)

**runFullTrust.** Tuck is a Tauri desktop app (Rust with the WebView2 runtime), which is a Win32 process and cannot run in the app container. It needs full trust to draw a transparent always-on-top window at the top of the screen, read global media sessions, record the microphone for dictation and watch the clipboard.

**userNotificationListener.** Used only by the optional Notification previews setting, which is off by default. When the user turns it on, Tuck asks Windows for notification access and shows incoming notifications on the island. Notifications are not stored or sent anywhere.

## Notes for certification
- No account or sign-in is needed. Everything works offline except lyrics and model downloads.
- To see notifications on the island, open Settings > Activities and turn on Notification previews, then allow access.
- The app starts at sign-in only if the user turns on Start with Windows. The startup task is declared disabled in the manifest and the first-run screen asks.
- The Store build does not update itself; the Store does.
- Source: https://github.com/jojomensah89/tuck

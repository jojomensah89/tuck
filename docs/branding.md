# Tuck icon

The user selected **B: Folded stem**: a white T with a teal lower fold on a charcoal tile.

- Editable app source: `src-tauri/icons/tuck.svg`.
- Web/settings source: `public/tuck.svg` (same artwork).
- Monochrome tray source: `src-tauri/icons/tray.svg`, with a dark outline to keep the white glyph visible against a light Windows tray.
- Windows/macOS/store raster assets are generated with `bun run tauri icon src-tauri/icons/tuck.svg`.
- Tray raster: `bun run tauri icon src-tauri/icons/tray.svg --output artifacts/tray-export --png 32`. Copy that PNG to `src-tauri/icons/tray.png` and export its 32×32 RGBA bytes to `tray.rgba`. Native code embeds those 4096 bytes directly, avoiding an additional runtime image decoder. Keep the SVG, PNG, and RGBA in sync.

The configured bundle ICO replaces the executable/taskbar/installer icon; the ICNS and store assets use the same mark. Settings and About use the SVG. The native tray uses its own monochrome asset and macOS template flag.

Generated assets were inspected at small sizes. Native Windows taskbar, tray, installer and shortcut presentation still require a fresh application/package build; an already running executable and Windows icon caches may keep the previous icon until refreshed. No installer was published or live app restarted for this change.

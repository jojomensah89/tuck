import { defineConfig } from "vite";
import { resolve } from "node:path";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;
const rootDir = typeof import.meta.dirname !== "undefined" ? import.meta.dirname : process.cwd();

// https://vite.dev/config/
export default defineConfig(() => ({
  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        index: resolve(rootDir, "index.html"),
        settings: resolve(rootDir, "settings.html"),
      },
    },
  },
}));

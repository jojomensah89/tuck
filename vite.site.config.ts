import { defineConfig } from "vite";
import { resolve } from "node:path";
import process from "node:process";

const rootDir = typeof import.meta.dirname !== "undefined" ? import.meta.dirname : process.cwd();

// The website frames the real island (index.html?embed) from site/island/.
// Relative base so it works under any path on Cloudflare Pages.
export default defineConfig({
  base: "./",
  // Provider marks and the icon come along; icons.ts resolves them against the base
  build: {
    outDir: resolve(rootDir, "site/island"),
    emptyOutDir: true,
    rollupOptions: { input: { index: resolve(rootDir, "index.html") } },
  },
});

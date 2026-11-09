import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import process from "node:process";

const rootDir = typeof import.meta.dirname !== "undefined" ? import.meta.dirname : process.cwd();

// The website's Stageshow walkthroughs (site-src/walkthroughs) as one script and stylesheet in site/walkthroughs/.
export default defineConfig({
  plugins: [react()],
  base: "./",
  publicDir: false,
  build: {
    outDir: resolve(rootDir, "site/walkthroughs"),
    emptyOutDir: true,
    cssCodeSplit: false,
    rollupOptions: {
      input: resolve(rootDir, "site-src/walkthroughs/main.tsx"),
      output: { entryFileNames: "walkthroughs.js", assetFileNames: "walkthroughs[extname]", format: "es" },
    },
  },
});

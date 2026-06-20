import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

// Tauri serves the built `dist/` and a dev server on a fixed port. We keep the
// screen un-cleared so Tauri's logs and Vite's stay interleaved during dev.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    // three.js (~520 kB) is a single lazy chunk, only fetched when a 3D
    // visualizer scene is opened — it can't be split further, so lift the
    // warning threshold above it (other oversized chunks still warn).
    chunkSizeWarningLimit: 700,
  },
});

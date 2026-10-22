import { defineConfig } from "vite";

export default defineConfig({
  root: ".",
  publicDir: "public",
  server: {
    port: 5173,
  },
  build: {
    outDir: "dist",
    target: "es2020",
  },
  // The firmware Worker (src/runtime/firmware-worker.ts) is a module worker
  // that imports the WASM bridge lazily, which needs code splitting; the
  // default "iife" worker format cannot split.
  worker: {
    format: "es",
  },
});

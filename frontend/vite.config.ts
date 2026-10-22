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
  // The dev dependency pre-bundle defines only process.env.NODE_ENV, so
  // fengari's luaconf.js, which reads process.env.FENGARICONF at module top
  // level with no `typeof process` guard, throws "process is not defined" in
  // the browser. The production build already replaces `process.env` with
  // `{}` (Vite's define plugin), so the pre-bundle gets the same define and
  // fengari's config resolves to `{}` in both. This must not go in the
  // top-level `define`: in dev that is assigned onto globalThis at runtime,
  // which would make `typeof process` true and turn on fengari's Node paths.
  optimizeDeps: {
    rolldownOptions: {
      transform: {
        define: {
          "process.env": "{}",
        },
      },
    },
  },
});

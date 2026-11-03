import type { IncomingMessage, ServerResponse } from "node:http";
import { defineConfig, type Plugin } from "vite";
import { MAX_BODY_BYTES, MemoryStore, handleSignal } from "./src/radio/signal-handler";

/**
 * Dev only (`bun run dev`): serves `/api/signal` with the same handler the
 * Vercel function (`api/signal.ts`) uses, over an in-memory store, so two
 * local windows can link without Vercel or Upstash. Nothing is logged.
 */
function devSignal(): Plugin {
  const store = new MemoryStore();
  return {
    name: "htn-dev-signal",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/api/signal", (req, res) => {
        void serve(req, res);
      });
    },
  };

  async function serve(req: IncomingMessage & { originalUrl?: string }, res: ServerResponse): Promise<void> {
    try {
      const chunks: Buffer[] = [];
      let total = 0;
      // Read the whole body (so the reply is not cut off), keeping at most the cap.
      for await (const chunk of req as AsyncIterable<Buffer>) {
        total += chunk.length;
        if (total <= MAX_BODY_BYTES) chunks.push(chunk);
      }
      if (total > MAX_BODY_BYTES) {
        // What the handler answers an oversized body.
        res.statusCode = 400;
        res.end();
        return;
      }
      const method = req.method ?? "GET";
      const request = new Request(new URL(req.originalUrl ?? req.url ?? "/", "http://localhost"), {
        method,
        body: method === "GET" || method === "HEAD" ? undefined : Buffer.concat(chunks),
      });
      const response = await handleSignal(request, store);
      res.statusCode = response.status;
      response.headers.forEach((value, key) => res.setHeader(key, value));
      res.end(Buffer.from(await response.arrayBuffer()));
    } catch {
      res.statusCode = 500;
      res.end();
    }
  }
}

export default defineConfig({
  root: ".",
  plugins: [devSignal()],
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

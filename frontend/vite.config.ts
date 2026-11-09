import type { IncomingMessage, ServerResponse } from "node:http";
import { defineConfig, type Plugin } from "vite";
import { MAX_BODY_BYTES, handleSignal, memoryDeps } from "./src/radio/signal-handler.ts";

/**
 * Dev only (`bun run dev`): serves `/api/signal` with the same handler the
 * Vercel function (`api/signal.ts`) uses, over an in-memory store and
 * in-memory limiters at the same caps, so two local windows can link
 * without Vercel or Upstash. Nothing is logged.
 */
function devSignal(): Plugin {
  const deps = memoryDeps();
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
      const body = await readCapped(req);
      if (body === null) {
        // What the handler answers an oversized body; the rest is never
        // read: the connection closes once the reply is flushed.
        res.statusCode = 400;
        res.setHeader("Connection", "close");
        res.end(() => req.destroy());
        return;
      }
      const method = req.method ?? "GET";
      const request = new Request(new URL(req.originalUrl ?? req.url ?? "/", "http://localhost"), {
        method,
        body: method === "GET" || method === "HEAD" ? undefined : body,
      });
      const response = await handleSignal(request, deps);
      res.statusCode = response.status;
      response.headers.forEach((value, key) => res.setHeader(key, value));
      res.end(Buffer.from(await response.arrayBuffer()));
    } catch {
      res.statusCode = 500;
      res.end();
    }
  }
}

/**
 * The request body, or null as soon as it passes `MAX_BODY_BYTES`: then
 * reading stops (the stream is paused, its listeners removed), and the
 * caller answers and destroys the request.
 */
export function readCapped(req: IncomingMessage): Promise<Buffer | null> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    const stop = () => {
      req.off("data", onData);
      req.off("end", onEnd);
      req.off("error", onError);
    };
    const onData = (chunk: Buffer) => {
      total += chunk.length;
      if (total > MAX_BODY_BYTES) {
        stop();
        req.pause();
        resolve(null);
      } else {
        chunks.push(chunk);
      }
    };
    const onEnd = () => {
      stop();
      resolve(Buffer.concat(chunks));
    };
    const onError = (e: Error) => {
      stop();
      reject(e);
    };
    req.on("data", onData);
    req.on("end", onEnd);
    req.on("error", onError);
  });
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

/**
 * The Vercel function serving `/api/signal` (Milestone 10 Task 10): the
 * WebRTC signaling mailbox of `../src/radio/signal-handler.ts`, over the
 * project's Upstash Redis (connected through the Vercel Marketplace, which
 * sets `KV_REST_API_URL` and `KV_REST_API_TOKEN`).
 *
 * The entry format is the `fetch` Web Standard export, which Vercel's
 * Node.js runtime calls for a file under `/api` in a project without a
 * framework router (here Vite, whose project root is `frontend/`): a
 * default export `{ fetch(request: Request) }` returning a `Response`, for
 * every method. See "Functions API Reference", section "`fetch` Web
 * Standard": https://vercel.com/docs/functions/functions-api-reference
 * (checked 2026-11-02).
 *
 * If either env var is missing it answers 500 with no body and logs
 * nothing; it never logs anything else either.
 */
import { UpstashStore, handleSignal } from "../src/radio/signal-handler.js";

type Env = Record<string, string | undefined>;

function env(): Env {
  // No Node types in this project: reach `process.env` without them.
  return (globalThis as { process?: { env?: Env } }).process?.env ?? {};
}

export default {
  fetch(request: Request): Promise<Response> | Response {
    const { KV_REST_API_URL: url, KV_REST_API_TOKEN: token } = env();
    if (!url || !token) return new Response(null, { status: 500, headers: { "Cache-Control": "no-store" } });
    return handleSignal(request, new UpstashStore(url, token));
  },
};

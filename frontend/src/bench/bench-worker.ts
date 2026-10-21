/**
 * The bench page's Worker: fetches the WASM, `factory.bin` and the hacker
 * fixture, runs `runRedrawBench` once, unpaced, and posts its log lines
 * and result to `./bench-page.ts`.
 */
import { createFirmwareEmulator, initCpuWasm } from "../cpu/bridge";
import { testIdentityUrl } from "../runtime/test-identities";
import type { BenchWorkerMessage } from "./bench-page";
import { runRedrawBench } from "./redraw-bench";
import { OWN_ROLE } from "./walk";

const post = (msg: BenchWorkerMessage) => self.postMessage(msg);

async function fetchOk(url: string): Promise<Response> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res;
}

async function main(): Promise<void> {
  const [, image, identity] = await Promise.all([
    initCpuWasm(),
    fetchOk("/firmware/factory.bin").then((r) => r.arrayBuffer()),
    fetchOk(testIdentityUrl(OWN_ROLE)).then((r) => r.text()),
  ]);
  const handle = createFirmwareEmulator(new Uint8Array(image));
  try {
    const result = runRedrawBench(handle, identity, () => performance.now(), (line) =>
      post({ kind: "log", line }),
    );
    post({ kind: "done", result });
  } finally {
    handle.dispose();
  }
}

main().catch((e: unknown) =>
  post({ kind: "error", message: e instanceof Error ? (e.stack ?? e.message) : String(e) }),
);

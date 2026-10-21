/**
 * The browser bench page (`bench.html`, dev server only): runs the redraw
 * benchmark unpaced in a dedicated Worker (`./bench-worker.ts`), the same
 * thread setup the app uses from Milestone 7's Worker task on, and writes
 * `JSON.stringify(result)` into `<pre id="result" data-done="true">`, or
 * the error into `data-error`, for a WebDriver or extension to read.
 */
import type { BenchResult } from "./redraw-bench";

export type BenchWorkerMessage =
  | { kind: "log"; line: string }
  | { kind: "done"; result: BenchResult }
  | { kind: "error"; message: string };

const result = document.getElementById("result")!;
const log = document.getElementById("log")!;
document.getElementById("host")!.textContent = navigator.userAgent;

function fail(message: string): void {
  result.dataset.error = message;
  result.textContent = message;
}

const worker = new Worker(new URL("./bench-worker.ts", import.meta.url), { type: "module" });
worker.onmessage = (e: MessageEvent<BenchWorkerMessage>) => {
  const msg = e.data;
  if (msg.kind === "log") {
    log.textContent += `${msg.line}\n`;
  } else if (msg.kind === "done") {
    result.textContent = JSON.stringify(msg.result);
    result.dataset.done = "true";
    worker.terminate();
  } else {
    fail(msg.message);
    worker.terminate();
  }
};
worker.onerror = (e: ErrorEvent) => fail(`worker error: ${e.message}`);

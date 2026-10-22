/**
 * `bun run bench:safari`: drives Safari through `safaridriver` (WebDriver
 * over HTTP) to the dev server's bench page and prints its result. Expects
 * `bun run dev` to be running (http://localhost:5173) and Safari's remote
 * automation to be enabled (Develop > Allow Remote Automation, or
 * `safaridriver --enable`). Uses a safaridriver already listening on port
 * 4444, or starts one and stops it afterwards.
 *
 * Loads the page `--runs N` times (default 3, as `bench:wasm` runs), one
 * fresh Worker per load, and prints each run's JSON plus a median summary.
 */
import { buildInfo } from "./bun-bench";
import { type BenchResult, formatRedraw, formatWalk, summarizeRuns } from "../src/bench/redraw-bench";

const DRIVER = "http://localhost:4444";
const PAGE = "http://localhost:5173/bench.html";
const TIMEOUT_MS = 15 * 60 * 1000;
const POLL_MS = 2000;

const runsArg = process.argv.indexOf("--runs");
const RUNS = runsArg >= 0 ? Number(process.argv[runsArg + 1]) : 3;

async function wd(method: string, path: string, body?: unknown): Promise<any> {
  const res = await fetch(`${DRIVER}${path}`, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = (await res.json()) as { value: any };
  if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${JSON.stringify(json.value)}`);
  return json.value;
}

async function driverUp(): Promise<boolean> {
  try {
    return (await fetch(`${DRIVER}/status`)).ok;
  } catch {
    return false;
  }
}

async function startDriver(): Promise<ReturnType<typeof Bun.spawn> | undefined> {
  if (await driverUp()) return undefined;
  const proc = Bun.spawn(["safaridriver", "-p", "4444"], { stdout: "inherit", stderr: "inherit" });
  for (let i = 0; i < 50; i++) {
    if (await driverUp()) return proc;
    await Bun.sleep(100);
  }
  proc.kill();
  throw new Error("safaridriver did not start on port 4444");
}

const READ_RESULT = `
  const r = document.getElementById("result");
  return r ? { done: r.dataset.done ?? null, error: r.dataset.error ?? null, text: r.textContent } : null;
`;

async function runOnce(session: string): Promise<BenchResult> {
  await wd("POST", `/session/${session}/url`, { url: PAGE });
  const deadline = Date.now() + TIMEOUT_MS;
  while (Date.now() < deadline) {
    const r = await wd("POST", `/session/${session}/execute/sync`, { script: READ_RESULT, args: [] });
    if (r?.error) throw new Error(`bench page error: ${r.error}`);
    if (r?.done === "true") return JSON.parse(r.text) as BenchResult;
    await Bun.sleep(POLL_MS);
  }
  throw new Error(`no result within ${TIMEOUT_MS / 60000} min`);
}

try {
  await fetch(PAGE);
} catch {
  console.error(`${PAGE} is not reachable: start \`bun run dev\` first`);
  process.exit(1);
}

const driver = await startDriver();
let session: string | undefined;
try {
  session = (
    await wd("POST", "/session", { capabilities: { alwaysMatch: { browserName: "safari" } } })
  ).sessionId as string;
  const ua = await wd("POST", `/session/${session}/execute/sync`, {
    script: "return navigator.userAgent;",
    args: [],
  });
  console.log(`BENCH host="Safari: ${ua}" ${buildInfo()}`);
  const results: BenchResult[] = [];
  for (let run = 1; run <= RUNS; run++) {
    console.log(`run ${run}/${RUNS}`);
    const result = await runOnce(session);
    console.log(JSON.stringify(result));
    for (const r of result.redraws) console.log(formatRedraw(r));
    console.log(formatWalk(result.walk));
    results.push(result);
  }
  for (const line of summarizeRuns(results)) console.log(line);
} finally {
  if (session !== undefined) await wd("DELETE", `/session/${session}`).catch(() => {});
  driver?.kill();
}

// Repro: a large script return value with NO return file rides the done
// message as one unchunkable WS message, then a single connectrpc message.
// Neither layer surfaces an error to the run:
//   - >64MB: exceeds the plugin-WS tungstenite default; the read error is
//     swallowed by plugin_ws.rs's silent `_ => {}` arm — no log, no
//     disconnect, the run hangs forever.
//   - >4MB: passes the WS hop but exceeds connectrpc's 4MB default on the
//     backend->master relay; the master's relay loop treats the decode error
//     as backend death.
//
// Expected behavior: both sizes either work or fail fast with a clear error.
import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");

const procs: Bun.Subprocess[] = [];

function cleanup(port: number) {
  Bun.spawnSync(["pkill", "-f", `__master --port ${port}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${port + 1}`]);
  killLaunchedStudios(port, STARTED);
}

afterAll(() => {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  cleanup(47330);
  cleanup(47334);
});

async function runWithReturnSize(port: number, mb: number) {
  const proc = Bun.spawn(
    [
      RODEO, "run", "--port", String(port), "--place",
      "-s", `return string.rep("x", ${mb} * 1024 * 1024)`,
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  // Generous budget: Studio launch (~10-30s) + run + teardown.
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(120_000).then(() => ({ exited: false, code: -1 })),
  ]);
  const stderr = await new Response(proc.stderr as ReadableStream).text().catch(() => "");
  return { ...result, stderr };
}

test("80MB in-wire return (over WS cap) fails fast instead of hanging", async () => {
  const r = await runWithReturnSize(47330, 80);
  expect(r.exited).toBe(true); // FAILS today: silent WS read error, run hangs
  expect(r.code).not.toBe(0);
}, 150_000);

test("8MB in-wire return (over connectrpc cap) fails fast with a clear error", async () => {
  const r = await runWithReturnSize(47334, 8);
  expect(r.exited).toBe(true);
  expect(r.code).not.toBe(0);
}, 150_000);

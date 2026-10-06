// Repro: killing a `rodeo run --place` client mid-run leaves its
// auto-spawned serve (__master / __studio-backend) running, squatting the
// port and breaking later runs that need it.
//
// The serve children are spawned with `--ppid <run-pid>`, so a parent-exit
// watch is supposed to reap them when the client dies — this repro verifies
// whether that binding actually works, for both SIGTERM and SIGKILL.
import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");

const procs: Bun.Subprocess[] = [];

function serveAlive(port: number): boolean {
  const r = Bun.spawnSync(["pgrep", "-f", `__master --port ${port}`]);
  return r.exitCode === 0;
}

async function waitForServe(port: number, timeoutMs: number): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (serveAlive(port)) {
      // Children exist; give the ppid watch a beat to arm before we kill.
      await Bun.sleep(2_000);
      return true;
    }
    await Bun.sleep(250);
  }
  return false;
}

function cleanup(port: number) {
  Bun.spawnSync(["pkill", "-f", `__master --port ${port}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${port + 1}`]);
  killLaunchedStudios(port, STARTED);
}

afterAll(() => {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  cleanup(47320);
  cleanup(47324);
});

async function killAndCheck(port: number, signal: number): Promise<boolean> {
  const proc = Bun.spawn(
    [RODEO, "run", "--port", String(port), "--place", "-s", "task.wait(120) return 1"],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  expect(await waitForServe(port, 60_000)).toBe(true);

  proc.kill(signal);
  await proc.exited;

  // Give the --ppid parent-exit watch ample time to reap the children.
  const start = Date.now();
  while (Date.now() - start < 15_000) {
    if (!serveAlive(port)) return true;
    await Bun.sleep(500);
  }
  return false;
}

test("SIGTERMed run client's serve children are reaped", async () => {
  const reaped = await killAndCheck(47320, 15);
  expect(reaped).toBe(true);
}, 120_000);

test("SIGKILLed run client's serve children are reaped", async () => {
  const reaped = await killAndCheck(47324, 9);
  expect(reaped).toBe(true);
}, 120_000);

// Repro (issue #30): a `rodeo run --place` submitted right after `rodeo serve`
// starts failed with "no studio backend registered". The serve's master
// answers about 100 ms before its studio backend registers, and the run
// checked for a backend once instead of waiting.
//
// Also covers the master-alive, backend-dead case from issue #7: the run must
// fail with the recovery commands rather than a bare "no studio backend".
//
// Neither case launches Studio: the place file is invalid, so the launch fails
// in the backend's place prep once a backend is there to receive it.
import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { tmpdir } from "os";
import { randomUUID } from "crypto";
import { rmSync } from "fs";

const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");

const procs: Bun.Subprocess[] = [];
const files: string[] = [];

afterAll(async () => {
  // SIGTERM first: a studio backend removes its plugin file on a clean exit.
  for (const p of procs) {
    try { p.kill("SIGTERM"); } catch {}
  }
  await Promise.race([Promise.all(procs.map((p) => p.exited)), Bun.sleep(5_000)]);
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  for (const f of files) rmSync(f, { force: true });
});

async function badPlace(): Promise<string> {
  const path = join(tmpdir(), `rodeo-repro-${randomUUID()}.rbxl`);
  await Bun.write(path, "this is not a place file");
  files.push(path);
  return path;
}

function spawnMaster(port: number): Bun.Subprocess {
  const p = Bun.spawn([RODEO, "__master", "--port", String(port)], { stdout: "ignore", stderr: "ignore" });
  procs.push(p);
  return p;
}

function spawnStudioBackend(masterPort: number): Bun.Subprocess {
  const p = Bun.spawn(
    [RODEO, "__studio-backend", "--port", String(masterPort + 1), "--master-host", "localhost", "--master-port", String(masterPort)],
    { stdout: "ignore", stderr: "ignore" },
  );
  procs.push(p);
  return p;
}

// The run must find the master already up, or it starts a serve of its own.
async function waitForMaster(port: number) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const r = Bun.spawnSync([RODEO, "state", "--port", String(port), "--json"], { stdout: "ignore", stderr: "ignore" });
    if (r.exitCode === 0) return;
    await Bun.sleep(100);
  }
  throw new Error(`master on ${port} never answered`);
}

async function runPlace(port: number, place: string) {
  const proc = Bun.spawn([RODEO, "run", "--port", String(port), "--place", place, "-s", "return 1"], {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, NO_COLOR: "1" },
  });
  procs.push(proc);
  return proc;
}

test("a run waits for a studio backend that registers after the master answers", async () => {
  const port = 47500;
  spawnMaster(port);
  await waitForMaster(port);

  const started = Date.now();
  const proc = await runPlace(port, await badPlace());
  // The backend comes up after the run has already asked for it.
  await Bun.sleep(1_000);
  spawnStudioBackend(port);

  const code = await Promise.race([proc.exited, Bun.sleep(30_000).then(() => "timeout" as const)]);
  const stderr = await new Response(proc.stderr).text();
  expect(code, stderr).not.toBe("timeout");
  expect(code, stderr).not.toBe(0);
  // The launch reached the backend: it failed on the place file, not on a
  // missing backend.
  expect(stderr).not.toContain("no studio backend");
  expect(stderr).toContain("not a valid rbxl/rbxlx");
  expect(Date.now() - started).toBeGreaterThanOrEqual(1_000);
}, 45_000);

test("a serve whose studio backend is gone names the recovery commands", async () => {
  const port = 47502;
  spawnMaster(port);
  await waitForMaster(port);

  const started = Date.now();
  const proc = await runPlace(port, await badPlace());
  const code = await Promise.race([proc.exited, Bun.sleep(30_000).then(() => "timeout" as const)]);
  const stderr = await new Response(proc.stderr).text();
  expect(code, stderr).not.toBe("timeout");
  expect(code, stderr).not.toBe(0);
  expect(stderr).toContain(`the rodeo serve on localhost:${port} has no studio backend`);
  expect(stderr).toContain(`rodeo serve --port ${port}`);
  expect(stderr).toContain(`rodeo serve --studio --master-port ${port} --port ${port + 1}`);
  // It waited for a backend before giving up.
  expect(Date.now() - started).toBeGreaterThanOrEqual(5_000);
}, 45_000);

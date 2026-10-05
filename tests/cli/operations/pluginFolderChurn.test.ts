// Plugin-folder churn does not reach runs in other Studios.
//
// Every studio backend installs its own `rodeo-<build>-<port>.rbxm` into the
// one local plugins folder Studio watches (~/Documents/Roblox/Plugins) when it
// starts, and removes it when it stops. Studio hot-loads changes in that
// folder into every open Studio, which is how a hand-opened Studio joins a new
// serve. The question this guards is whether such a change also disturbs the
// plugin an OTHER serve already has loaded in its own Studio: a reload there
// tears down the plugin VM and kills any run executing in it ("dom <id>
// disconnected while the run was active").
//
// Measured 2026-09-18 on Studio 0.739 / macOS: it does not. A run in serve A's
// Studio survives, and A's edit DOM never reconnects, through another serve
// starting and stopping, another project's full one-shot `rodeo run --place`
// (ephemeral serve, Studio open and close, file removed), an unrelated file,
// a trivial `.lua` plugin, and that plugin in a subfolder. An earlier flake
// while writing multiServe.test.ts had been read as such a reload; DESIGN.md
// was corrected to match this measurement. If Studio ever starts reloading
// siblings, this file is what fails.
//
// Ports 47360/47370 are this file's own; the paths in `--ppid` cleanup are too.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..", "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
const PORT_A = 47360; // the serve whose run must survive
const PORT_B = 47370; // the serve whose start/stop is the perturbation
const PLUGINS = join(homedir(), "Documents", "Roblox", "Plugins");
const RUN_SECONDS = 20;

const procs: Bun.Subprocess[] = [];
let studioId = "";

// Serve logs go to files so a plugin reload is visible (a "dom disconnected"
// / "dom connected" pair on A) even when the run happens to survive it.
const LOG_A = join(tmpdir(), "rodeo-churn-repro-serve-a.log");
const LOG_B = join(tmpdir(), "rodeo-churn-repro-serve-b.log");

function spawn(args: string[], log?: string): Bun.Subprocess {
  const p = Bun.spawn([RODEO, ...args, "--ppid", String(process.pid)], {
    stdout: log ? Bun.file(log) : "pipe",
    stderr: log ? Bun.file(log) : "pipe",
    stdin: "ignore",
  });
  procs.push(p);
  return p;
}

// How many times A's edit DOM (re)connected so far: a reload shows up as an
// extra "dom connected" for the same Studio.
function domConnects(): number {
  try {
    return (readFileSync(LOG_A, "utf8").match(/dom connected/g) ?? []).length;
  } catch {
    return 0;
  }
}

async function text(stream: ReadableStream<Uint8Array> | null | number): Promise<string> {
  return typeof stream === "object" && stream ? await new Response(stream).text() : "";
}

function stateJson(port: number): any {
  const r = Bun.spawnSync([RODEO, "state", "--port", String(port), "--json"]);
  try {
    return JSON.parse(r.stdout.toString());
  } catch {
    return null;
  }
}

async function waitFor(pred: () => boolean, ms: number, what: string): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < ms) {
    if (pred()) return;
    await Bun.sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

// A run in serve A's Studio that sleeps RUN_SECONDS, then returns. Resolves to
// its combined output and exit code.
function startLongRun(): { done: Promise<{ code: number; out: string }> } {
  const p = spawn([
    "run", "--port", String(PORT_A), "--studio-id", studioId, "--show-return",
    "--source", `task.wait(${RUN_SECONDS}) return "survived"`,
  ]);
  const done = (async () => {
    const [code, so, se] = await Promise.all([p.exited, text(p.stdout), text(p.stderr)]);
    return { code, out: so + se };
  })();
  return { done };
}

async function waitRunning(): Promise<void> {
  await waitFor(() => JSON.stringify(stateJson(PORT_A) ?? {}).includes('"running"'), 30_000, "the long run to be running");
  await Bun.sleep(1500); // let the script settle inside the plugin VM
}

// Perturb the plugins folder while the run is live, then let the run finish.
async function survives(perturb: () => Promise<void>): Promise<{ code: number; out: string; reconnects: number }> {
  const run = startLongRun();
  await waitRunning();
  const before = domConnects();
  await perturb();
  const result = await run.done;
  const reconnects = domConnects() - before;
  console.info(`  A's DOMs (re)connected ${reconnects} time(s) during the perturbation`);
  return { ...result, reconnects };
}

beforeAll(async () => {
  rmSync(LOG_A, { force: true });
  rmSync(LOG_B, { force: true });
  spawn(["serve", "--port", String(PORT_A)], LOG_A);
  await waitFor(() => stateJson(PORT_A) !== null, 30_000, "serve A");
  // A persistent Studio on A: a `run --place` with no source holds it open for
  // as long as this process lives (the same hold the CLI tests use).
  spawn(["run", "--port", String(PORT_A), "--place"]);
  await waitFor(() => (stateJson(PORT_A)?.studios ?? []).some((s: any) => s.sessionId), 90_000, "A's launched Studio");
  studioId = stateJson(PORT_A).studios.find((s: any) => s.sessionId).studioId;
}, 150_000);

afterAll(async () => {
  for (const p of procs.reverse()) {
    try { p.kill(); } catch {}
  }
  await Promise.all(procs.map((p) => p.exited.catch(() => 0)));
  for (const f of ["rodeoChurnProbe.txt", "rodeoChurnProbe.lua"]) rmSync(join(PLUGINS, f), { force: true });
  rmSync(join(PLUGINS, "rodeoChurnProbe"), { recursive: true, force: true });
});

test("a run survives another serve starting and stopping (the bug)", async () => {
  const r = await survives(async () => {
    const b = spawn(["serve", "--port", String(PORT_B)], LOG_B);
    await waitFor(() => stateJson(PORT_B) !== null, 30_000, "serve B");
    await Bun.sleep(3000); // B's plugin file is installed and loaded by then
    b.kill(); // graceful: B removes its plugin file on exit
    await b.exited;
    await Bun.sleep(3000);
  });
  expect(r.out).not.toContain("disconnected while the run was active");
  expect(r.out).toContain("survived");
  expect(r.code).toBe(0);
}, 120_000);

test("a run survives another project's one-shot `run --place` (ephemeral serve + Studio open/close)", async () => {
  const r = await survives(async () => {
    // Nothing listens on PORT_B, so this spawns an ephemeral serve, installs
    // its plugin file, launches a Studio, runs, closes the Studio, stops the
    // serve and removes the file: the whole lifecycle another project's
    // one-shot command goes through.
    const one = spawn(["run", "--port", String(PORT_B), "--place", "--source", "return 1"], LOG_B);
    await one.exited;
    await Bun.sleep(3000);
  });
  expect(r.out).not.toContain("disconnected while the run was active");
  expect(r.out).toContain("survived");
  expect(r.code).toBe(0);
}, 150_000);

test("diagnostic: an unrelated non-plugin file (foo.txt) added to the folder", async () => {
  const path = join(PLUGINS, "rodeoChurnProbe.txt");
  const r = await survives(async () => {
    writeFileSync(path, "not a plugin\n");
    await Bun.sleep(3000);
    rmSync(path, { force: true });
    await Bun.sleep(3000);
  });
  expect(r.out).not.toContain("disconnected while the run was active");
  expect(r.out).toContain("survived");
}, 120_000);

test("diagnostic: a trivial .lua plugin added to the folder", async () => {
  const path = join(PLUGINS, "rodeoChurnProbe.lua");
  const r = await survives(async () => {
    writeFileSync(path, "-- rodeo churn probe: an unrelated local plugin\nreturn nil\n");
    await Bun.sleep(3000);
    rmSync(path, { force: true });
    await Bun.sleep(3000);
  });
  expect(r.out).not.toContain("disconnected while the run was active");
  expect(r.out).toContain("survived");
}, 120_000);

test("diagnostic: the same .lua plugin inside its own subfolder", async () => {
  const dir = join(PLUGINS, "rodeoChurnProbe");
  const r = await survives(async () => {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "plugin.lua"), "-- rodeo churn probe, in a subfolder\nreturn nil\n");
    await Bun.sleep(3000);
    rmSync(dir, { recursive: true, force: true });
    await Bun.sleep(3000);
  });
  expect(existsSync(dir)).toBe(false);
  expect(r.out).not.toContain("disconnected while the run was active");
  expect(r.out).toContain("survived");
}, 120_000);

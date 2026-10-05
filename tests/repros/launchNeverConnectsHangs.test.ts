// Repro for #13: when a Studio that `rodeo run --place` launched never loads
// the rodeo plugin, the run waits forever. The reported trigger is a signed-out
// Studio, which stops at the sign-in prompt before loading plugins. Signing out
// would sign out every Studio on the machine, so this repro keeps the plugin
// from loading by deleting this serve's plugin file before the launch: same
// state (Studio running, its edit DOM never registers).
//
// Expected: the launch fails after the backend's launch deadline (shortened
// here with RODEO_LAUNCH_TIMEOUT), the run exits non-zero naming the timeout,
// and the launched Studio is closed.
import { test, expect, afterAll } from "bun:test";
import { existsSync, readFileSync, unlinkSync } from "node:fs";
import { pluginFileFor, waitForHealthy, waitUntil } from "../cli/helpers.js";

const PORT = 46350;
const LAUNCH_TIMEOUT_S = 20;
const BOUND_MS = 60_000;

const procs: Bun.Subprocess[] = [];
afterAll(async () => {
  for (const p of procs) p.kill();
  await Promise.all(procs.map((p) => p.exited));
});

function spawnRodeo(args: string[], pipe: boolean): Bun.Subprocess {
  const proc = Bun.spawn(["rodeo", ...args, "--ppid", String(process.pid)], {
    env: { ...process.env, RODEO_LAUNCH_TIMEOUT: String(LAUNCH_TIMEOUT_S) },
    stdout: pipe ? "pipe" : "inherit",
    stderr: pipe ? "pipe" : "inherit",
    stdin: "ignore",
  });
  procs.push(proc);
  return proc;
}

function backendRegistered(): boolean {
  const r = Bun.spawnSync(["rodeo", "state", "--json", "--port", String(PORT)]);
  return r.exitCode === 0 && (JSON.parse(r.stdout.toString()).backends?.length ?? 0) > 0;
}

// `rodeo state` lists a Studio only once its plugin connects, so find the one
// this serve launched by its RunScript bootstrap, which stamps the port.
function launchedSession(): string | undefined {
  const ps = Bun.spawnSync(["ps", "-axo", "command="]).stdout.toString();
  for (const line of ps.split("\n")) {
    const m = /-runScriptFile (\S+rodeo-bootstrap-([0-9a-f-]+)\.luau)/.exec(line);
    if (m && existsSync(m[1]) && readFileSync(m[1], "utf8").includes(`"rodeoPort", ${PORT + 1})`)) return m[2];
  }
  return undefined;
}

function studioRunning(session: string): boolean {
  return Bun.spawnSync(["pgrep", "-f", `rodeo-bootstrap-${session}`]).exitCode === 0;
}

test("a launched Studio that never connects fails the run instead of hanging", async () => {
  spawnRodeo(["serve", "--port", String(PORT)], false);
  await waitForHealthy(PORT);
  // A run sent before the studio backend registers fails outright (#30).
  await waitUntil(backendRegistered, 10_000, "the studio backend to register");
  const plugin = pluginFileFor(PORT);
  await waitUntil(() => existsSync(plugin), 10_000, "serve to install its plugin");
  unlinkSync(plugin);

  const started = Date.now();
  const run = spawnRodeo(["run", "--port", String(PORT), "--place", "--source", "return true"], true);

  let session: string | undefined;
  await waitUntil(() => (session = launchedSession()) !== undefined, 30_000, "the launched Studio to appear");
  console.log(`launched Studio session ${session}`);
  expect(studioRunning(session!)).toBe(true);

  const exited = await Promise.race([run.exited.then(() => true), Bun.sleep(BOUND_MS).then(() => false)]);
  const elapsedS = Math.round((Date.now() - started) / 1000);
  console.log(`run ${exited ? `exited with ${run.exitCode}` : "still waiting"} after ${elapsedS}s`);
  expect(exited).toBe(true);

  const output = (await new Response(run.stdout as ReadableStream).text()) + (await new Response(run.stderr as ReadableStream).text());
  console.log(output.trim());
  expect(run.exitCode).not.toBe(0);
  expect(output).toContain(`(session ${session!.slice(0, 8)}) did not connect within ${LAUNCH_TIMEOUT_S}s`);

  await waitUntil(() => !studioRunning(session!), 15_000, "the launched Studio to close");
}, BOUND_MS + 60_000);

test("a launched Studio that connects outlives the launch deadline", async () => {
  const port = PORT + 2;
  const started = Date.now();
  const run = spawnRodeo(
    ["run", "--port", String(port), "--place", "--show-return", "--source", `task.wait(${LAUNCH_TIMEOUT_S + 10}) return "survived"`],
    true,
  );
  await run.exited;
  const output = (await new Response(run.stdout as ReadableStream).text()) + (await new Response(run.stderr as ReadableStream).text());
  expect(output).toContain("survived");
  expect(run.exitCode).toBe(0);
  expect(Date.now() - started).toBeGreaterThan((LAUNCH_TIMEOUT_S + 10) * 1000);
}, BOUND_MS + 60_000);

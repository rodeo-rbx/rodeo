// Repro (issue #9): a file entrypoint whose transitive bundle exceeded the
// run-submission payload limit used to exit 2 with EMPTY stdout/stderr — the
// single-message submission blew the master's receive cap, the stream
// dropped, and the client swallowed the transport error twice over.
//
// Scripts over SCRIPT_CHUNK_SIZE are now split across ScriptChunk messages on
// every hop (client->master reassembled at the master; master->plugin
// reassembled in the plugin, each chunk its own WS frame), so an oversized
// bundle RUNS. The fixture generates a ~24MB module tree on the fly
// (pseudo-random string literals so nothing compresses or dedups) plus a
// small-bundle control; both must execute and print. The client also emits a
// large-bundle advisory on stderr — data that size belongs in baked files,
// not compiled source.
import { test, expect, afterAll } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");
const PORT = 47410;
const FIXTURE_DIR = "/tmp/rodeo-oversized-bundle-repro";

const procs: Bun.Subprocess[] = [];

function cleanup() {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
  killLaunchedStudios(PORT, STARTED);
  try { rmSync(FIXTURE_DIR, { recursive: true, force: true }); } catch {}
}

afterAll(cleanup);

// Deterministic pseudo-random printable ASCII so the bundled source cannot
// shrink below the payload limit through literal dedup or compression. The
// alphabet (48..90: digits, punctuation, uppercase) deliberately excludes
// `"` and `\` so the output is always a valid Luau string literal.
function chunkOfJunk(seedStart: number, bytes: number): string {
  let state = 0x6d2b79f5 ^ seedStart;
  const chars: string[] = [];
  for (let i = 0; i < bytes; i++) {
    state ^= state << 13; state >>>= 0;
    state ^= state >> 17;
    state ^= state << 5; state >>>= 0;
    chars.push(String.fromCharCode(48 + (state % 43)));
  }
  return chars.join("");
}

function writeFixtures(moduleCount: number, bytesPerModule: number) {
  rmSync(FIXTURE_DIR, { recursive: true, force: true });
  mkdirSync(join(FIXTURE_DIR, "fixtures"), { recursive: true });
  // Bundling a file script resolves requires through luau configuration;
  // outside a project a .luaurc must exist next to the entrypoint. An
  // explicit index file (not init.luau) sidesteps the issue #6
  // directory-require limitation.
  writeFileSync(join(FIXTURE_DIR, ".luaurc"), "{}\n");
  const indexLines: string[] = ["return {"];
  for (let i = 0; i < moduleCount; i++) {
    writeFileSync(
      join(FIXTURE_DIR, "fixtures", `data${i}.luau`),
      `return "${chunkOfJunk(i, bytesPerModule)}"\n`,
    );
    indexLines.push(`\trequire("./data${i}"),`);
  }
  indexLines.push("}");
  writeFileSync(join(FIXTURE_DIR, "fixtures", "index.luau"), indexLines.join("\n") + "\n");
  writeFileSync(
    join(FIXTURE_DIR, "entry.luau"),
    `local fixtures = require("./fixtures/index")\nprint("fixtures loaded", #fixtures)\nreturn #fixtures\n`,
  );
  writeFileSync(
    join(FIXTURE_DIR, "control.luau"),
    `local one = require("./fixtures/data0")\nprint("control loaded", #one)\nreturn true\n`,
  );
}

async function runRodeo(args: string[], timeoutMs: number) {
  const proc = Bun.spawn([RODEO, ...args], { stdout: "pipe", stderr: "pipe" });
  procs.push(proc);
  const stdoutPromise = new Response(proc.stdout as ReadableStream).text();
  const stderrPromise = new Response(proc.stderr as ReadableStream).text();
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(timeoutMs).then(() => ({ exited: false, code: -1 })),
  ]);
  if (!result.exited) {
    proc.kill(9);
    await proc.exited.catch(() => {});
  }
  const [stdout, stderr] = await Promise.all([
    stdoutPromise.catch(() => ""),
    stderrPromise.catch(() => ""),
  ]);
  return { ...result, stdout, stderr };
}

test("oversized bundle chunks through and runs", async () => {
  cleanup();
  // 24 x 1MB modules ≈ 24MB of bundled source — over the 16MiB envelope.
  writeFixtures(24, 1_000_000);

  // Persistent serve: an implicit (run-spawned) serve exits with its run,
  // and the oversized run must hit a live master to exercise the transport.
  const serve = Bun.spawn([RODEO, "serve", "--port", String(PORT)], {
    stdout: "ignore",
    stderr: "ignore",
  });
  procs.push(serve);
  for (let i = 0; i < 60; i++) {
    const probe = Bun.spawnSync([RODEO, "state", "--port", String(PORT)]);
    if (probe.exitCode === 0) break;
    await Bun.sleep(1000);
  }

  // Control below the limit: same fixture tree, one module — must run.
  const control = await runRodeo(
    ["run", `${FIXTURE_DIR}/control.luau`, "--place", "--detach", "--port", String(PORT)],
    180_000,
  );
  expect(control.code, `${control.stdout}\n${control.stderr}`).toBe(0);
  expect(control.stdout).toContain("control loaded");

  // Oversized bundle against the same live serve/Studio: chunked submission
  // means it executes end-to-end.
  const big = await runRodeo(
    ["run", `${FIXTURE_DIR}/entry.luau`, "--port", String(PORT)],
    180_000,
  );

  expect(big.exited, "oversized run hung").toBe(true);
  expect(big.code, `${big.stdout}\n${big.stderr}`).toBe(0);
  expect(big.stdout).toContain("fixtures loaded 24");
  // The advisory still steers toward baking data instead of bundling it.
  expect(big.stderr).toMatch(/large bundle/i);
}, 400_000);

// Repro (issue #8): passing script arguments after `--` disables the script's
// `@rodeo run ...` directive.
//
// The directive splice appends the directive's flag tokens to the END of the
// user argv — after the user's `--` — where clap's `last = true` script_args
// captures them as raw script arguments instead of parsing them as flags. So
// `rodeo run script.luau -- fixture` on a script with `-- @rodeo run --place`
// never applies --place: the run waits indefinitely for a resident Studio,
// and the directive's flag tokens leak into process.args.
//
// Expected behavior: the directive provides the base run configuration and
// script args; CLI flags override directive flags per-flag, and a CLI `--`
// tail replaces the directive's script args wholesale — neither layer
// disables the other.
import { test, expect, afterAll } from "bun:test";
import { rmSync, writeFileSync } from "fs";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");
const PORT = 47346;
const SCRIPT = "/tmp/rodeo-directive-args-repro.luau";

const procs: Bun.Subprocess[] = [];

function cleanup() {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
  killLaunchedStudios(PORT, STARTED);
  try { rmSync(SCRIPT, { force: true }); } catch {}
}

afterAll(cleanup);

test("script args after -- do not disable @rodeo run directives", async () => {
  cleanup();

  writeFileSync(
    SCRIPT,
    [
      "-- @rodeo run --place",
      'local process = require("@rodeo/process")',
      'print("ARGS:[" .. table.concat(process.args, ",") .. "]")',
      "return true",
    ].join("\n"),
  );

  // No serve is running on PORT: the directive's --place is the only thing
  // that can make this run succeed (it must bootstrap a serve + Studio).
  const proc = Bun.spawn(
    [RODEO, "run", "--port", String(PORT), SCRIPT, "--", "fixture-name"],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  const stdoutPromise = new Response(proc.stdout as ReadableStream).text();
  const stderrPromise = new Response(proc.stderr as ReadableStream).text();
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(120_000).then(() => ({ exited: false, code: -1 })),
  ]);
  if (!result.exited) {
    proc.kill(9);
    await proc.exited.catch(() => {});
  }
  const [stdout, stderr] = await Promise.all([
    stdoutPromise.catch(() => ""),
    stderrPromise.catch(() => ""),
  ]);

  // Directive applied → the run bootstraps its own place and completes.
  expect(result.exited, "run hung waiting for a Studio — directive --place was not applied").toBe(true);
  expect(result.code, `${stdout}\n${stderr}`).toBe(0);
  // CLI `--` tail populates script args; directive flag tokens must not leak in.
  expect(stdout).toContain("ARGS:[fixture-name]");
}, 150_000);

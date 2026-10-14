// Lute adapter conformance: lute's own (verbatim) tests for its standard
// library and built-in modules, run through `rodeo run --adapter lute`
// against a resident Studio. See tests/adapters/lute/README.md for provenance.
//
// Every *.test.luau under lute/tests must be classified by the manifest below:
// an unclassified file fails the suite, so upstream additions force a
// conscious triage instead of silently not running.
//
//   run          — executes via `rodeo run`; every case passes except the
//                  ones CASE_GAPS lists, which must fail (a listed case that
//                  starts passing fails the test, so the list stays honest)
//   gap          — a test file that can't run under rodeo; skipped with reason
//   out-of-scope — needs a built-in rodeo can't provide; skipped with reason
import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { appendFileSync, cpSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { cliStudioHandle } from "../cli/helpers.js";

const PORT = 46290;
const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
// Holds `tests/` laid out as in lute's repo: some tests read data at paths
// relative to lute's repo root (e.g. tests/std/JSONParsingTestSuite).
const SUITE_DIR = join(import.meta.dir, "lute");

// ---------------------------------------------------------------------------
// Manifest (paths relative to lute/tests)
// ---------------------------------------------------------------------------

const RUN: string[] = [
  "lute/crypto.test.luau",
  "lute/debugger.test.luau",
  "lute/net.test.luau",
  "lute/task.test.luau",
  "lute/vm.test.luau",
  "std/fs.test.luau",
  "std/io.test.luau",
  "std/json.test.luau",
  "std/luau.test.luau",
  "std/net.test.luau",
  "std/path/path.posix.test.luau",
  "std/path/path.win32.test.luau",
  "std/process.test.luau",
  "std/signal.test.luau",
  "std/stringext.test.luau",
  "std/syntax/parser.test.luau",
  "std/syntax/printer.test.luau",
  "std/syntax/query.test.luau",
  "std/syntax/utils.test.luau",
  "std/tableext.test.luau",
  "std/test.test.luau",
  "std/time.test.luau",
];

// Known-failing cases of a `run` file ("Suite.case" → reason): cases that
// can't pass under rodeo.
const TYPEOF = "asserts the typeof of a lute value; Studio's typeof reports a stand-in for one as userdata";
const UPGRADE_HANDLER =
  "its websocket upgrade handler is a function of the script's, which lute calls without letting it wait for Studio";
const allFor = (reason: string, cases: string[]) =>
  Object.fromEntries(cases.map((name) => [name, reason]));

const CASE_GAPS: Record<string, Record<string, string>> = {
  "lute/debugger.test.luau": allFor(TYPEOF, [
    "LuteDebugLibrarySuite.launchAndHitBreakpoint",
    "LuteDebugLibrarySuite.multisource",
    "LuteDebugLibrarySuite.setAndRemove",
    "LuteDebugLibrarySuite.stopOnBp",
  ]),
  "lute/net.test.luau": allFor(UPGRADE_HANDLER, [
    "LuteNetSuite.clientInitiatedCloseRunsOnclose",
    "LuteNetSuite.clientWebsocketCallbacksSurviveGc",
    "LuteNetSuite.serveBinaryWebsocketAndConnect",
    "LuteNetSuite.serveWebsocketAndConnect",
  ]),
};

// Prefix → reason. Whole files skipped: tests that can't run under rodeo.
const GAP: Record<string, string> = {};

// Prefix → reason. Needs a built-in the lute adapter can't provide.
const OUT_OF_SCOPE: Record<string, string> = {};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

// Runs the suites the test file registered with lute's runner, prints its
// report, then one line per failing case and a final marker for the harness.
const RUN_REGISTERED = `
local __rodeoResult = require("@std/test/runner").run(require("@std/test/env"))
require("@std/test/reporter").simple(__rodeoResult)
for _, __rodeoFailure in __rodeoResult.failures do
\tprint("RODEO_LUTE_FAILED " .. __rodeoFailure.test)
end
print("RODEO_LUTE_DONE")
`;

function classify(file: string): { state: "run" | "gap" | "out-of-scope" | "unclassified"; reason?: string } {
  const prefixed = (table: Record<string, string>) =>
    Object.entries(table).find(([prefix]) => file === prefix || file.startsWith(`${prefix}/`))?.[1];
  if (RUN.includes(file)) return { state: "run" };
  const gap = prefixed(GAP);
  if (gap) return { state: "gap", reason: gap };
  const outOfScope = prefixed(OUT_OF_SCOPE);
  if (outOfScope) return { state: "out-of-scope", reason: outOfScope };
  return { state: "unclassified" };
}

function testFiles(): string[] {
  const testsDir = join(SUITE_DIR, "tests");
  const out: string[] = [];
  for (const entry of readdirSync(testsDir, { withFileTypes: true, recursive: true })) {
    if (entry.isFile() && entry.name.endsWith(".test.luau")) {
      // Forward slashes on every platform: the manifest is written with them.
      out.push(relative(testsDir, join(entry.parentPath, entry.name)).replaceAll("\\", "/"));
    }
  }
  return out.sort();
}

const studio = cliStudioHandle(PORT);
const scratchDirs: string[] = [];

beforeAll(async () => {
  await studio.spawn();
}, 180_000);

afterAll(async () => {
  await studio.close();
  for (const d of scratchDirs) {
    try { rmSync(d, { recursive: true, force: true }); } catch {}
  }
});

describe("lute conformance", () => {
  const files = testFiles();

  it("manifest classifies every test file", () => {
    const unclassified = files.filter((f) => classify(f).state === "unclassified");
    expect(unclassified, "add these to the manifest (run/gap/out-of-scope)").toEqual([]);
  });

  for (const file of files) {
    const { state, reason } = classify(file);
    if (state === "unclassified") continue;
    if (state !== "run") {
      it.skip(`[${state}] ${file} — ${reason}`, () => {});
      continue;
    }
    it(
      file,
      () => {
        // A scratch copy of tests/ and examples/ laid out as in lute's repo, run from its
        // root, so files the tests write stay out of the repo. A test file
        // registers its suites and `lute test` runs them, so the copy gets
        // RUN_REGISTERED appended (harness provisioning, not a test edit: the
        // files in the repo stay verbatim).
        const cwd = mkdtempSync(join(tmpdir(), "rodeo-lute-conformance-"));
        scratchDirs.push(cwd);
        cpSync(join(SUITE_DIR, "tests"), join(cwd, "tests"), { recursive: true });
        cpSync(join(SUITE_DIR, "examples"), join(cwd, "examples"), { recursive: true });
        const script = join(cwd, "tests", file);
        appendFileSync(script, RUN_REGISTERED);
        // Its own temp dir too: @std/system.tmpdir() is the run client's,
        // and the fs tests leave fixed names there.
        const tmp = join(cwd, "tmp");
        mkdirSync(tmp);
        const proc = Bun.spawnSync(
          [RODEO, "run", script, "--adapter", "lute", "--port", String(PORT)],
          {
            cwd,
            env: { ...process.env, TMPDIR: tmp, TMP: tmp, TEMP: tmp },
            stdout: "pipe",
            stderr: "pipe",
            timeout: 110_000,
          },
        );
        const stdout = proc.stdout?.toString() ?? "";
        const stderr = proc.stderr?.toString() ?? "";
        const context = `${file}\n--- stdout:\n${stdout}\n--- stderr:\n${stderr}`;
        expect(proc.exitCode, context).toBe(0);
        expect(stdout, `the runner never finished\n${context}`).toContain("RODEO_LUTE_DONE");
        const failed = [...stdout.matchAll(/^RODEO_LUTE_FAILED (.+)$/gm)].map((m) => m[1].trim()).sort();
        const expected = Object.keys(CASE_GAPS[file] ?? {}).sort();
        expect(failed, `failing cases differ from CASE_GAPS["${file}"]\n${context}`).toEqual(expected);
      },
      120_000,
    );
  }
});

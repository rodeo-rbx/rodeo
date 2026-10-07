// Lune adapter conformance: run lune's own (verbatim) test scripts through
// `rodeo run` against a resident Studio, holding the @lune adapters to lune's
// ground truth. See tests/adapters/lune/README.md for provenance.
//
// Every .luau file under lune/ must be classified by the manifest below —
// an unclassified file fails the suite, so upstream additions force a
// conscious triage instead of silently not running.
//
//   run          — executes via `rodeo run`, must exit 0
//   gap          — in-scope module, unimplemented surface; skipped with reason
//   out-of-scope — no adapter for the module; skipped with reason
//   helper       — required by tests, not a test itself
import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { cliStudioHandle } from "../cli/helpers.js";

const PORT = 46280;
const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
const SUITE_DIR = join(import.meta.dir, "lune");

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

// Files that must pass. Paths relative to lune/.
const RUN: string[] = [
  "fs/copy.luau",
  "fs/dirs.luau",
  "fs/files.luau",
  "fs/metadata.luau",
  "fs/move.luau",
  "process/args.luau",
  "process/create/non_blocking.luau",
  "process/create/status.luau",
  "process/create/stream.luau",
  "process/cwd.luau",
  "process/exec/async.luau",
  "process/exec/basic.luau",
  "process/exec/cwd.luau",
  "process/exec/no_panic.luau",
  "process/exec/shell.luau",
  "process/exec/stdin.luau",
  "process/exec/stdio.luau",
  "process/exit.luau",
  "serde/json/decode.luau",
  "stdio/ewrite.luau",
  "stdio/write.luau",
  "task/cancel.luau",
  "task/defer.luau",
  "task/delay.luau",
  "task/spawn.luau",
  "task/wait.luau",
  "globals/coroutine.luau",
  "globals/error.luau",
  "globals/type.luau",
  "globals/warn.luau",
];

// Script args some tests assume their runner provides (lune's own runner
// passes args). Harness provisioning, not a test edit.
const ARGS: Record<string, string[]> = {
  "process/args.luau": ["Foo", "Bar"],
};

// Files some tests expect in their cwd (lune's runner starts at its repo
// root). Harness provisioning, not a test edit.
const CWD_FILES: Record<string, string[]> = {
  "process/exec/basic.luau": ["Cargo.toml", ".gitignore"],
  "process/exec/shell.luau": ["Cargo.toml", ".gitignore"],
};

// Prefix (directory or exact file) → reason. In-scope modules, missing surface.
const GAP: Record<string, string> = {
  "process/create/kill.luau": "races cat's startup against the kill; fails under lune 0.10.5 itself (20/20 on macOS). Covered by the process.create block below",
  "process/env.luau": "rodeo env is a read-only remote snapshot; lune env is assignable",
  "serde/compression": "adapter has no serde.compress/decompress",
  "serde/hashing": "adapter has no serde.hash/hmac",
  "serde/json/encode.luau": "lune asserts its exact encoder output; HttpService key order/pretty differ",
  "serde/jsonc": "serde shim is json-only (no jsonc)",
  "serde/toml": "serde shim is json-only (no toml)",
  "stdio/color.luau": "adapter has no stdio.color",
  "stdio/style.luau": "adapter has no stdio.style",
  "stdio/prompt.luau": "adapter has no stdio.prompt (no interactive stdin in a run)",
  "globals/_G.luau": "lune expects a pristine _G; rodeo publishes runtime state there by design",
  "globals/_VERSION.luau": "reports Studio's Luau version, not 'Lune x.y.z'",
};

// Prefix → reason. Whole modules with no adapter.
const OUT_OF_SCOPE: Record<string, string> = {
  "datetime": "no @lune/datetime adapter",
  "luau": "no @lune/luau adapter (loadstring is Studio-restricted)",
  "net": "no @lune/net adapter (HttpService semantics differ)",
  "regex": "no @lune/regex adapter",
  "require": "lune require semantics vs darklua bundling (issue #6 territory)",
  "roblox": "no @lune/roblox adapter (Studio natives differ from lune's reimpl)",
  "stdio/format.luau": "requires @lune/regex and @lune/roblox",
  "globals/pcall.luau": "requires @lune/net",
  "globals/typeof.luau": "requires @lune/roblox",
};

// Required by tests; not tests themselves.
const HELPERS: string[] = [
  "fs/utils.luau",
  "task/fcheck.luau",
  "serde/json/source.luau",
  "serde/jsonc/source.luau",
  "serde/toml/source.luau",
];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

function allLuauFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true, recursive: true })) {
    if (entry.isFile() && entry.name.endsWith(".luau")) {
      // Normalize to forward slashes: the manifests above are written with
      // them, but `relative` yields backslashes on Windows, which would leave
      // every nested file unclassified.
      out.push(relative(SUITE_DIR, join(entry.parentPath, entry.name)).replaceAll("\\", "/"));
    }
  }
  return out.sort();
}

function classify(file: string): { state: string; reason?: string } {
  if (RUN.includes(file)) return { state: "run" };
  if (HELPERS.includes(file)) return { state: "helper" };
  for (const [prefix, reason] of Object.entries(GAP)) {
    if (file === prefix || file.startsWith(prefix + "/")) return { state: "gap", reason };
  }
  for (const [prefix, reason] of Object.entries(OUT_OF_SCOPE)) {
    if (file === prefix || file.startsWith(prefix + "/")) return { state: "out-of-scope", reason };
  }
  return { state: "unclassified" };
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

describe("lune conformance", () => {
  const files = allLuauFiles(SUITE_DIR);

  it("manifest classifies every file", () => {
    const unclassified = files.filter((f) => classify(f).state === "unclassified");
    expect(unclassified, "add these to the manifest (run/gap/out-of-scope/helper)").toEqual([]);
  });

  for (const file of files) {
    const { state, reason } = classify(file);
    if (state === "helper" || state === "unclassified") continue;
    if (state !== "run") {
      it.skip(`[${state}] ${file} — ${reason}`, () => {});
      continue;
    }
    it(
      file,
      () => {
        // Fresh cwd per test: lune's fs tests write bin/-relative paths, and
        // rodeo's fs executes at the run client's cwd — a scratch dir keeps
        // that out of the repo (and out of rodeo's actual bin/).
        const cwd = mkdtempSync(join(tmpdir(), "rodeo-lune-conformance-"));
        scratchDirs.push(cwd);
        for (const name of CWD_FILES[file] ?? []) writeFileSync(join(cwd, name), "");
        const extraArgs = ARGS[file] ? ["--", ...ARGS[file]] : [];
        const proc = Bun.spawnSync(
          [RODEO, "run", join(SUITE_DIR, file), "--port", String(PORT), ...extraArgs],
          { cwd, stdout: "pipe", stderr: "pipe", timeout: 90_000 },
        );
        const stdout = proc.stdout?.toString() ?? "";
        const stderr = proc.stderr?.toString() ?? "";
        expect(proc.exitCode, `${file}\n--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
      },
      120_000,
    );
  }
});

// A member a shim does not provide must raise an error naming it, not index
// to nil and fail later as "attempt to call a nil value" (issue #37).
const UNSUPPORTED_SOURCE = `
local modules = {
	fs = require("@lune/fs"),
	process = require("@lune/process"),
	serde = require("@lune/serde"),
	stdio = require("@lune/stdio"),
	task = require("@lune/task"),
}
local function expectError(fn, expected)
	local ok, err = pcall(fn)
	assert(not ok and string.find(tostring(err), expected, 1, true), \`expected "{expected}", got: {err}\`)
end
for name, module in modules do
	expectError(function()
		return module.missingMember
	end, \`@lune/{name}.missingMember is not supported by rodeo's Lune adapter\`)
end
expectError(function()
	return modules.serde.hash("sha256", "abc")
end, "@lune/serde.hash is not supported by rodeo's Lune adapter")
expectError(function()
	return modules.serde.encode("toml", {})
end, '@lune/serde.encode format "toml" is not supported by rodeo\\'s Lune adapter (only "json" is)')
assert(modules.serde.decode("json", modules.serde.encode("json", { ok = true })).ok)
`;

// process.create behavior the upstream tests leave out. kill.luau is listed
// as a gap: it races cat's startup against the kill and fails under lune
// itself. A JS runtime stands in for cat (none on stock Windows): the one
// running these tests, so it is present at an absolute path.
const RT = globalThis.process.execPath.replace(/\\/g, "\\\\");
const CREATE_SOURCE = `
local process = require("@lune/process")
local task = require("@lune/task")
local CAT = { "-e", "process.stdin.pipe(process.stdout)" }

-- A read waiting for output must not hold up the run's other calls: the
-- write that produces the output, or a print.
local child = process.create("${RT}", CAT)
local got
task.spawn(function()
	got = child.stdout:read()
end)
task.wait(0.5)
print("a read is pending")
child.stdin:write("late")
for _ = 1, 100 do
	if got then
		break
	end
	task.wait(0.05)
end
assert(got == "late", \`read while a write was pending returned {got}\`)

-- Killed: lune reports code 9, and the child's stdin no longer takes writes.
child:kill()
local status = child:status()
assert(status.ok == false, "a killed child is not ok")
if process.os ~= "windows" then
	assert(status.code == 9, \`killed child code {status.code}, expected 9\`)
end
assert(child:status().code == status.code, "status can be read again")
assert(not pcall(function()
	child.stdin:write("after kill")
end), "writing to a killed child's stdin should fail")

-- A spawn that fails surfaces from the first method called.
local missing = process.create("rodeo-no-such-program")
local ok, err = pcall(function()
	return missing:status()
end)
assert(not ok and string.find(tostring(err), "create error", 1, true), \`expected a create error, got: {err}\`)
`;

describe("lune adapter process.create", () => {
  it(
    "reads while writing, kills, and reports a failed spawn",
    () => {
      const cwd = mkdtempSync(join(tmpdir(), "rodeo-lune-create-"));
      scratchDirs.push(cwd);
      const proc = Bun.spawnSync(
        [RODEO, "run", "--source", CREATE_SOURCE, "--port", String(PORT)],
        { cwd, stdout: "pipe", stderr: "pipe", timeout: 90_000 },
      );
      const stdout = proc.stdout?.toString() ?? "";
      const stderr = proc.stderr?.toString() ?? "";
      expect(proc.exitCode, `--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
      expect(stdout).toContain("a read is pending");
      expect(stderr).toContain("@lune/process.create failed");
    },
    120_000,
  );
});

describe("lune adapter unsupported members", () => {
  it(
    "raise an error naming the member",
    () => {
      const cwd = mkdtempSync(join(tmpdir(), "rodeo-lune-unsupported-"));
      scratchDirs.push(cwd);
      const proc = Bun.spawnSync(
        [RODEO, "run", "--source", UNSUPPORTED_SOURCE, "--port", String(PORT)],
        { cwd, stdout: "pipe", stderr: "pipe", timeout: 90_000 },
      );
      const stdout = proc.stdout?.toString() ?? "";
      const stderr = proc.stderr?.toString() ?? "";
      expect(proc.exitCode, `--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
    },
    120_000,
  );
});

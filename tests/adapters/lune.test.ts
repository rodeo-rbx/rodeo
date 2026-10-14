// Lune adapter conformance: run lune's own (verbatim) test scripts through
// `rodeo run --adapter lune` against a resident Studio, holding the adapter to
// lune's ground truth. See tests/adapters/lune/README.md for provenance.
//
// Every .luau file under lune/tests must be classified by the manifest below —
// an unclassified file fails the suite, so upstream additions force a
// conscious triage instead of silently not running.
//
//   run          — executes via `rodeo run`, must exit 0
//   gap          — in-scope module, a test that can't pass; skipped with reason
//   out-of-scope — no adapter for the module; skipped with reason
//   helper       — not one of lune's tests (required by them, or not run by lune)
import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { cliStudioHandle } from "../cli/helpers.js";

const PORT = 46280;
const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
// Laid out as lune's repo root: its tests/ and its .luaurc (the `@tests`
// aliases), since lune runs every test from there.
const LUNE_ROOT = join(import.meta.dir, "lune");
const SUITE_DIR = join(LUNE_ROOT, "tests");

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

// Files that must pass. Paths relative to lune/tests.
const RUN: string[] = [
  "datetime/formatUniversalTime.luau",
  "datetime/fromLocalTime.luau",
  "datetime/fromRfc2822.luau",
  "datetime/fromRfc3339.luau",
  "datetime/fromUniversalTime.luau",
  "datetime/fromUnixTimestamp.luau",
  "datetime/toLocalTime.luau",
  "datetime/toRfc2822.luau",
  "datetime/toRfc3339.luau",
  "datetime/toUniversalTime.luau",
  "fs/copy.luau",
  "fs/dirs.luau",
  "fs/files.luau",
  "fs/move.luau",
  "globals/coroutine.luau",
  "globals/error.luau",
  "globals/pcall.luau",
  "globals/type.luau",
  "globals/warn.luau",
  "luau/compile.luau",
  "luau/load.luau",
  "luau/options.luau",
  "luau/safeenv.luau",
  "net/request/codes.luau",
  "net/request/compression.luau",
  "net/request/https.luau",
  "net/request/methods.luau",
  "net/request/query.luau",
  "net/request/redirect.luau",
  "net/serve/addresses.luau",
  "net/serve/handles.luau",
  "net/serve/non_blocking.luau",
  "net/serve/requests.luau",
  "net/tcp/info.luau",
  "net/url/decode.luau",
  "net/url/encode.luau",
  "process/args.luau",
  "process/create/non_blocking.luau",
  "process/create/status.luau",
  "process/create/stream.luau",
  "process/cwd.luau",
  "process/env.luau",
  "process/exec/async.luau",
  "process/exec/basic.luau",
  "process/exec/cwd.luau",
  "process/exec/no_panic.luau",
  "process/exec/shell.luau",
  "process/exec/stdin.luau",
  "process/exec/stdio.luau",
  "process/exit.luau",
  "regex/general.luau",
  "regex/replace.luau",
  "require/tests/aliases.luau",
  "require/tests/async.luau",
  "require/tests/async_sequential.luau",
  "require/tests/children.luau",
  "require/tests/init_files.luau",
  "require/tests/invalid.luau",
  "require/tests/multi_ext.luau",
  "require/tests/nested.luau",
  "require/tests/parents.luau",
  "require/tests/siblings.luau",
  "require/tests/state.luau",
  "serde/compression/files.luau",
  "serde/compression/roundtrip.luau",
  "serde/hashing/hash.luau",
  "serde/hashing/hmac.luau",
  "serde/json/decode.luau",
  "serde/json/encode.luau",
  "serde/jsonc/decode.luau",
  "serde/jsonc/encode.luau",
  "serde/toml/decode.luau",
  "serde/toml/encode.luau",
  "stdio/color.luau",
  "stdio/ewrite.luau",
  "stdio/style.luau",
  "stdio/write.luau",
  "task/cancel.luau",
  "task/defer.luau",
  "task/delay.luau",
  "task/spawn.luau",
  "task/wait.luau",
];

// Lune runs each test from its repo root with these script args, and so does
// the harness. Files the exec tests expect in that directory are created.
const ARGS = ["Foo", "Bar"];
const CWD_FILES = ["Cargo.toml", ".gitignore"];

const TYPEOF =
  "asserts the typeof of a lune value; Studio's typeof reports a stand-in for one as userdata";
const UNRELEASED_FIX =
  "fails under lune 0.10.5 itself: socket:close() errors, fixed after the release (lune #400)";
const EXAMPLE_COM =
  "connects to example.com:80/443, which lune 0.10.5 itself is intermittently refused from IPv6-capable networks";

// Prefix (directory or exact file) → reason. In-scope modules, tests that can't pass.
const GAP: Record<string, string> = {
  "datetime/now.luau": TYPEOF,
  "fs/metadata.luau": TYPEOF,
  "regex/metamethods.luau": TYPEOF,
  "datetime/formatLocalTime.luau": "ends early with a bare top-level `return`; a rodeo script must return exactly one value",
  "net/serve/websockets.luau": UNRELEASED_FIX,
  "net/socket/basic.luau": UNRELEASED_FIX,
  "net/socket/wss.luau": UNRELEASED_FIX,
  "net/socket/wss_rw.luau": UNRELEASED_FIX,
  "net/tcp/basic.luau": EXAMPLE_COM,
  "net/tcp/tls.luau": EXAMPLE_COM,
  "require/tests/async_concurrent.luau": "two concurrent requires of a module that yields while loading load it twice in a bundle",
  "require/tests/builtins.luau": "requires `@lune` and `@lune/`, which a bundle can't leave unresolved",
  "globals/_G.luau": "lune expects a pristine _G; rodeo publishes runtime state there by design",
  "globals/_VERSION.luau": "reports Studio's Luau version, not 'Lune x.y.z'",
};

// Prefix → reason. Whole modules with no adapter.
const OUT_OF_SCOPE: Record<string, string> = {
  "roblox": "no @lune/roblox adapter: its instances and types are lune's own, not Studio's",
  "stdio/format.luau": "requires @lune/roblox",
  "globals/typeof.luau": "requires @lune/roblox",
};

// Not among the tests lune runs (crates/lune/src/tests.rs): modules its tests
// require, and files it leaves out.
const HELPERS: string[] = [
  "fs/utils.luau",
  "net/request/user_agent.luau",
  "net/request/util.luau",
  "process/create/kill.luau",
  "require/modules",
  "require/tests/module.luau",
  "require/tests/modules",
  "require/tests/multi.ext.file.luau",
  "require/tests/state_module.luau",
  "require/tests/state_second.luau",
  "serde/json/source.luau",
  "serde/jsonc/source.luau",
  "serde/toml/source.luau",
  "stdio/prompt.luau",
  "task/fcheck.luau",
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
  const prefixed = (prefix: string) => file === prefix || file.startsWith(prefix + "/");
  if (RUN.includes(file)) return { state: "run" };
  if (HELPERS.some(prefixed)) return { state: "helper" };
  for (const [prefix, reason] of Object.entries(GAP)) {
    if (prefixed(prefix)) return { state: "gap", reason };
  }
  for (const [prefix, reason] of Object.entries(OUT_OF_SCOPE)) {
    if (prefixed(prefix)) return { state: "out-of-scope", reason };
  }
  return { state: "unclassified" };
}

const studio = cliStudioHandle(PORT);
const scratchDirs: string[] = [];

// A scratch copy of lune's repo root to run from: lune's fs tests write
// paths relative to it, and that keeps them out of this repo.
function luneRoot(prefix: string): string {
  const cwd = mkdtempSync(join(tmpdir(), prefix));
  scratchDirs.push(cwd);
  cpSync(SUITE_DIR, join(cwd, "tests"), { recursive: true });
  cpSync(join(LUNE_ROOT, ".luaurc"), join(cwd, ".luaurc"));
  for (const name of CWD_FILES) writeFileSync(join(cwd, name), "");
  return cwd;
}

function runLune(args: string[], cwd: string, scriptArgs: string[] = []) {
  const command = [RODEO, "run", ...args, "--adapter", "lune", "--port", String(PORT), "--", ...scriptArgs];
  const proc = Bun.spawnSync(command, {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
    timeout: 90_000,
  });
  return {
    exitCode: proc.exitCode,
    stdout: proc.stdout?.toString() ?? "",
    stderr: proc.stderr?.toString() ?? "",
  };
}

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
        const cwd = luneRoot("rodeo-lune-conformance-");
        const { exitCode, stdout, stderr } = runLune([join(cwd, "tests", file)], cwd, ARGS);
        expect(exitCode, `${file}\n--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
      },
      120_000,
    );
  }
});

// What the adapter adds on top of lune: the script runs in Studio and lune's
// libraries in a lune process, so a call waits for lune while the script's
// other threads go on, and lune calls the script's functions back.
const BRIDGE_PORT = 46288;
const CONCURRENT_SOURCE = `
local net = require("@lune/net")
local task = require("@lune/task")

-- The handler (a function of this script's, which lune calls back) holds the
-- request until the script lets it go, while another thread's request waits
-- in lune.
local release = false
local handle = net.serve(${BRIDGE_PORT}, function()
	while not release do
		task.wait(0.05)
	end
	return "done"
end)
local got
task.spawn(function()
	got = net.request("http://127.0.0.1:${BRIDGE_PORT}/").body
end)
task.wait(0.5)
print("a request is pending")
release = true
for _ = 1, 100 do
	if got then
		break
	end
	task.wait(0.05)
end
handle.stop()
assert(got == "done", \`the request got {got}\`)
`;

// lune's create doesn't yield; the adapter's returns at once too, and a child
// that fails to start raises from the first member called.
const CREATE_SOURCE = `
local process = require("@lune/process")
local missing = process.create("rodeo-no-such-program")
local ok, err = pcall(function()
	return missing:status()
end)
assert(not ok, "status of a child that never started should fail")
print("status failed:", err)
`;

// A require of @lune/roblox (no adapter) fails only where it runs.
const ROBLOX_SOURCE = `
if false then
	require("@lune/roblox")
end
local ok, err = pcall(function()
	return require("@lune/roblox")
end)
assert(not ok and string.find(tostring(err), "@lune/roblox is not supported by rodeo's Lune adapter", 1, true), \`got: {err}\`)
`;

describe("lune adapter", () => {
  it(
    "keeps the script's other threads going while a call waits in lune",
    () => {
      const { exitCode, stdout, stderr } = runLune(["--source", CONCURRENT_SOURCE], luneRoot("rodeo-lune-bridge-"));
      expect(exitCode, `--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
      expect(stdout).toContain("a request is pending");
    },
    120_000,
  );

  it(
    "returns from process.create at once and reports a child that fails to start",
    () => {
      const { exitCode, stdout, stderr } = runLune(["--source", CREATE_SOURCE], luneRoot("rodeo-lune-create-"));
      expect(exitCode, `--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
      expect(stdout).toContain("status failed:");
      expect(stderr).toContain("@lune/process.create failed");
    },
    120_000,
  );

  it(
    "fails a require of @lune/roblox only where it runs",
    () => {
      const { exitCode, stdout, stderr } = runLune(["--source", ROBLOX_SOURCE], luneRoot("rodeo-lune-roblox-"));
      expect(exitCode, `--- stdout:\n${stdout}\n--- stderr:\n${stderr}`).toBe(0);
    },
    120_000,
  );
});

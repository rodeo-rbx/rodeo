// Repro: roblox.import reads the entire file through one StreamReadBytes
// response — the read-direction mirror of the export bug in
// largeRobloxExportDisconnectsBackend.test.ts. A sufficiently large .rbxm
// kills the run's transport instead of completing the import or returning an
// actionable, import-scoped error.
//
// The fixture exports its own input files first (export is chunked as of the
// issue #7 fix), so it does not depend on checked-in binary assets. The
// StringValues contain deterministic pseudo-random printable ASCII, which
// prevents Roblox's binary serializer from shrinking the model below the
// relay limit. The control import (~3.1 MiB) proves the import path works
// below the relay boundary; the large import is sized to exceed the
// per-envelope cap (16 MiB since the cap bump; originally 4 MiB) so the
// test keeps guarding the transport boundary — an unchunked read of this
// file fails BY CONSTRUCTION.
//
// Observed with rodeo v1.2.0-rc.8 (+ chunked export): the run client's
// oversized StreamReadBytesResponse kills its own run stream — the master
// logs `run client disconnected`, the run is torn down, and the CLI exits 1
// with NO stderr diagnostic. Unlike the export bug the backend survives; the
// blast radius is the single run, but the silent exit is its own bug.
//
// Expected behavior: roblox.import either streams/chunks the file and
// completes, or fails promptly with an actionable size/resource error without
// killing the transport.
import { test, expect, afterAll } from "bun:test";
import { rmSync } from "fs";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");
const PORT = 47344;
const CONTROL_FILE = "/tmp/rodeo-large-import-control.rbxm";
const LARGE_FILE = "/tmp/rodeo-large-import-repro.rbxm";

const procs: Bun.Subprocess[] = [];

function cleanup() {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
  killLaunchedStudios(PORT, STARTED);
  try { rmSync(CONTROL_FILE, { force: true }); } catch {}
  try { rmSync(LARGE_FILE, { force: true }); } catch {}
}

afterAll(cleanup);

test("large roblox.import does not kill the run transport", async () => {
  cleanup();

  const source = `
local roblox = require("@rodeo/roblox")

local CONTROL_COUNT = 1500
local VALUE_COUNT = 10500
local BYTES_PER_VALUE = 2048
local WORDS_PER_VALUE = BYTES_PER_VALUE / 4
local root = Instance.new("Folder")
root.Name = "LargeImportRepro"

-- xorshift32 gives deterministic, poorly-compressible data. Each generated
-- byte is mapped into printable ASCII so this only exercises normal Roblox
-- StringValue serialization, not invalid-UTF-8 handling.
local state = 0x6D2B79F5
local function nextWord()
	state = bit32.bxor(state, bit32.lshift(state, 13))
	state = bit32.bxor(state, bit32.rshift(state, 17))
	state = bit32.bxor(state, bit32.lshift(state, 5))

	local a = 32 + bit32.band(state, 63)
	local b = 32 + bit32.band(bit32.rshift(state, 6), 63)
	local c = 32 + bit32.band(bit32.rshift(state, 12), 63)
	local d = 32 + bit32.band(bit32.rshift(state, 18), 63)
	return bit32.bor(a, bit32.lshift(b, 8), bit32.lshift(c, 16), bit32.lshift(d, 24))
end

local function appendValues(targetCount)
	for index = #root:GetChildren() + 1, targetCount do
		local payload = buffer.create(BYTES_PER_VALUE)
		for wordIndex = 0, WORDS_PER_VALUE - 1 do
			buffer.writeu32(payload, wordIndex * 4, nextWord())
		end

		local value = Instance.new("StringValue")
		value.Name = string.format("Payload_%05d", index)
		value.Value = buffer.tostring(payload)
		value.Parent = root
	end
end

-- Control: export + re-import the same model below the 4 MiB relay limit.
appendValues(CONTROL_COUNT)
roblox.export("${CONTROL_FILE}", { root })
local controlImported = roblox.import("${CONTROL_FILE}")
assert(#controlImported == 1, "control import returned " .. #controlImported .. " roots")
assert(#controlImported[1]:GetChildren() == CONTROL_COUNT, "control import lost children")
print("control import completed")

-- Repro: the same round-trip at ~8 MiB kills the transport on the way back.
appendValues(VALUE_COUNT)
roblox.export("${LARGE_FILE}", { root })
print("large export completed")

local imported = roblox.import("${LARGE_FILE}")
assert(#imported == 1, "large import returned " .. #imported .. " roots")
assert(#imported[1]:GetChildren() == VALUE_COUNT, "large import lost children")
print("large import completed")
return true
`.trim();

  const proc = Bun.spawn(
    [
      RODEO,
      "run",
      "--port",
      String(PORT),
      "--place",
      "--source",
      source,
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  const stdoutPromise = new Response(proc.stdout as ReadableStream).text();
  const stderrPromise = new Response(proc.stderr as ReadableStream).text();
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(240_000).then(() => ({ exited: false, code: -1 })),
  ]);
  if (!result.exited) {
    proc.kill(9);
    await proc.exited.catch(() => {});
  }
  const [stdout, stderr] = await Promise.all([
    stdoutPromise.catch(() => ""),
    stderrPromise.catch(() => ""),
  ]);

  expect(result.exited).toBe(true);
  expect(stdout).toContain("control import completed");
  expect(stdout).toContain("large export completed");
  expect(stderr).not.toContain("disconnected");
  expect(result.code, `${stdout}\n${stderr}`).toBe(0);
  expect(stdout).toContain("large import completed");
}, 270_000);

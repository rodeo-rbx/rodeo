// Repro: an RPC response bigger than the connectrpc envelope cap used to kill
// the run stream instead of erroring the call. stream.read on a file handle
// dispatches to readAll — one response carrying the whole file — so reading a
// file larger than the cap (16 MiB since the cap bump; originally 4 MiB) tore
// down the run with exit 1 and no stderr diagnostic.
//
// The sender-side relay guard converts this to an actionable, run-scoped
// error: the oversized response is replaced with an rpc error (the plugin's
// rpc.call raises; stream.read returns nil per its contract), the run stream
// survives, and subsequent RPCs on the same run keep working — which this
// test proves by re-reading the same file through the chunked readBytes path
// afterward.
//
// The backend has the mirror-image guard for plugin→master messages; that
// direction can't currently be tripped from a run script (plugin-side
// chunking sits in front of it), so this test exercises the client-side one.
import { test, expect, afterAll } from "bun:test";
import { rmSync } from "fs";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");
const PORT = 47346;
const BIG_FILE = "/tmp/rodeo-oversized-readall.txt";

const procs: Bun.Subprocess[] = [];

function cleanup() {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
  killLaunchedStudios(PORT, STARTED);
  try { rmSync(BIG_FILE, { force: true }); } catch {}
}

afterAll(cleanup);

test("oversized rpc response errors the call instead of killing the run stream", async () => {
  cleanup();

  const source = `
local fs = require("@rodeo/fs")
local stream = require("@rodeo/stream")

-- 20 MiB of plain ASCII: over the 16 MiB envelope cap. Written through the
-- chunked write path (fine), then read back via stream.read, whose file
-- branch is still a single-envelope readAll.
local line = string.rep("abcdefghijklmnopqrstuvwxyz012345", 128) -- 4096 bytes
local payload = table.concat(table.create(5120, line)) -- 20,971,520 bytes

local wh = fs.open("${BIG_FILE}", "w")
stream.write(wh, payload)
stream.close(wh)

local rh = fs.open("${BIG_FILE}", "r")
local data = stream.read(rh) -- readAll: guard replaces the response with an error -> nil
stream.close(rh)
assert(data == nil, "expected oversized readAll to fail, got " .. tostring(data and #data) .. " bytes")
print("oversized readAll errored instead of dying")

-- The refusal must be call-scoped: the same run's transport keeps working.
local rh2 = fs.open("${BIG_FILE}", "r")
local bytes = stream.readBytes(rh2) -- chunked path handles any size
stream.close(rh2)
assert(buffer.len(bytes) == #payload, "post-guard read returned " .. buffer.len(bytes) .. " bytes")
print("transport alive after guard")
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
  expect(stderr).not.toContain("disconnected");
  expect(stdout).toContain("oversized readAll errored instead of dying");
  expect(stdout).toContain("transport alive after guard");
  expect(result.code, `${stdout}\n${stderr}`).toBe(0);
}, 270_000);

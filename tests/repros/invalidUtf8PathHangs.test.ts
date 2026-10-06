// Repro: an rpc argument that becomes a JSON *string* field (e.g. a filesystem
// `path` for fs.exists / roblox.export) containing invalid UTF-8 makes the
// plugin's MessageHandler.onRpc throw a bare "Can't convert to JSON" from
// HttpService:JSONEncode (MessageHandler.luau ~L127). The error escapes
// uncaught: the failing rpc never returns, so the calling script hangs forever
// (and the run is wedged). There is no indication of which rpc / field / value /
// byte is at fault.
//
// buffer/`bytes` fields are base64-encoded so they're always JSON-safe; only
// plain string fields hit this. Here a lone 0xE2 (orphan UTF-8 lead byte) is
// built inside Luau via string.char to avoid any shell/TS escaping.
//
// Expected behavior: fail fast with an actionable error (which field, which
// byte), or byte-safe-encode string fields — NOT hang.
import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { killLaunchedStudios } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const RODEO = join(ROOT, "bin", "rodeo");

const procs: Bun.Subprocess[] = [];

function cleanup(port: number) {
  Bun.spawnSync(["pkill", "-f", `__master --port ${port}`]);
  Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${port + 1}`]);
  killLaunchedStudios(port, STARTED);
}

afterAll(() => {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  cleanup(47338);
});

test("rpc string arg with invalid UTF-8 fails fast instead of hanging", async () => {
  const source = [
    'local fs = require("@rodeo/fs")',
    // lone 0xE2 -> not valid UTF-8
    'local badPath = "x_" .. string.char(0xE2) .. "_y.rbxm"',
    "fs.exists(badPath)",
    "return true",
  ].join("\n");

  const proc = Bun.spawn(
    [RODEO, "run", "--port", "47338", "--place", "-s", source],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  // Studio launch (~10-30s) + one rpc + teardown is plenty; anything past this
  // means the run hung on the un-returned rpc.
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(90_000).then(() => ({ exited: false, code: -1 })),
  ]);

  expect(result.exited).toBe(true); // FAILS today: fs.exists never returns, run hangs
  expect(result.code).not.toBe(0);
}, 120_000);

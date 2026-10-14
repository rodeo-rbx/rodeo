// Repro: `rodeo run --place <file>` hangs forever when the Studio launch
// fails (e.g. the place file isn't a valid binary place). The backend logs
// "Studio spawn failed: failed to parse binary place" but the run client
// never exits — observed alive 1h40m after the error.
//
// Expected behavior: the launch failure propagates and the run exits
// non-zero promptly.
import { test, expect, afterAll } from "bun:test";
import { join } from "path";
import { stopServe } from "../cli/helpers.js";
import { tempPath } from "../utils/paths.js";

const ROOT = join(import.meta.dir, "..", "..");
const RODEO = join(ROOT, "bin", "rodeo");
const PORT = 47310;

const procs: Bun.Subprocess[] = [];

afterAll(() => {
  for (const p of procs) {
    try { p.kill(9); } catch {}
  }
  // Reap serve children the killed client may leave behind (see
  // killedRunLeaksServe repro) so this repro never poisons later runs.
  stopServe(PORT);
});

test("run exits promptly when Studio spawn fails (invalid place file)", async () => {
  const badPlace = tempPath("rodeo-repro-bad-place.rbxl");
  await Bun.write(badPlace, "this is not a binary place file");

  const proc = Bun.spawn(
    [RODEO, "run", "--port", String(PORT), "--place", badPlace, "-s", "return 1"],
    { stdout: "pipe", stderr: "pipe" },
  );
  procs.push(proc);

  // 30s is generous: no Studio ever launches on this path — the failure is
  // detected during place prep, within a second of the serve coming up.
  const result = await Promise.race([
    proc.exited.then((code) => ({ exited: true, code })),
    Bun.sleep(30_000).then(() => ({ exited: false, code: -1 })),
  ]);

  expect(result.exited).toBe(true); // FAILS today: client hangs forever
  expect(result.code).not.toBe(0);
}, 45_000);

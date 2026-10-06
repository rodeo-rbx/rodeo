// Issue #3: Studio writes the dump holding a profiled run's last frames when
// that frame's 60-frame window closes, after the run has completed. The
// backend unregistered the run on RunCompleted, so that tail dump was
// skipped. And nothing ever removed Studio's auto-capture dumps from
// ProfilerCaptures, which a profiled Studio writes for as long as it runs, so
// the directory grew without bound.
import { describe, afterAll, it, expect } from "bun:test";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  killLaunchedStudios,
  removeKeptPluginFile,
  runRodeo,
  spawnBackground,
  waitForHealthy,
  waitUntil,
} from "../helpers.js";
import { extractMarker } from "../../utils/profiling.js";

const PORT = 47700;
const SWEEP_PORT = 47720;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const profileDir = ".rodeo/.temp/test-profile-tail";

// Only the run's final frame carries the marker, so finding it means the
// dump written after the run completed was delivered.
const TAIL_SCRIPT = `
local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local marker = HttpService:GenerateGUID(false)
print("MARKER:" .. marker)
for _ = 1, 90 do
  RunService.Heartbeat:Wait()
end
debug.profilebegin(marker)
debug.profileend()
return marker
`;

function profilerCapturesDir(): string {
  return process.platform === "win32"
    ? join(process.env.LOCALAPPDATA ?? "", "Roblox", "logs", "ProfilerCaptures")
    : join(homedir(), "Library", "Logs", "Roblox", "ProfilerCaptures");
}

describe("--profile tail dump and cleanup (CLI)", () => {
  // Stand-ins for Studio's output, from a capture start no Studio will reuse.
  const dir = profilerCapturesDir();
  const oldDump = join(dir, "AutoCapture_2000-01-01T000000.000Z_Frames-1-61.raw");
  const newDump = join(dir, "AutoCapture_2000-01-01T000000.000Z_Frames-62-122.raw");
  const savedDump = join(dir, "rodeo-test-saved-capture.raw");

  afterAll(() => {
    rmSync(profileDir, { recursive: true, force: true });
    for (const f of [oldDump, newDump, savedDump]) rmSync(f, { force: true });
    killLaunchedStudios(PORT, STARTED);
    removeKeptPluginFile(PORT);
    removeKeptPluginFile(SWEEP_PORT);
  });

  it("delivers the dump holding the run's last frame", () => {
    rmSync(profileDir, { recursive: true, force: true });

    const result = runRodeo(
      ["run", "--place", "--port", String(PORT), "--profile", profileDir, "--source", TAIL_SCRIPT],
      { timeout: 110_000 },
    );
    expect(result.ok).toBe(true);
    const marker = extractMarker(result.stdout + result.stderr);

    const delivered = readdirSync(profileDir).filter((n) => n.endsWith(".raw"));
    expect(delivered.length).toBeGreaterThan(0);
    const withLastFrame = delivered.filter((n) => readFileSync(join(profileDir, n), "utf8").includes(marker));
    expect(withLastFrame.length).toBeGreaterThan(0);
  }, 120_000);

  it("a serve deletes auto-capture dumps older than five minutes and nothing else", async () => {
    mkdirSync(dir, { recursive: true });
    for (const f of [oldDump, newDump, savedDump]) writeFileSync(f, "frames");
    const tenMinutesAgo = new Date(Date.now() - 10 * 60 * 1000);
    utimesSync(oldDump, tenMinutesAgo, tenMinutesAgo);
    utimesSync(savedDump, tenMinutesAgo, tenMinutesAgo);

    // A backend's profile scanner sweeps as it starts.
    const serve = spawnBackground(["serve", "--port", String(SWEEP_PORT)]);
    try {
      await waitForHealthy(SWEEP_PORT);
      await waitUntil(() => !existsSync(oldDump), 15_000, "the old auto-capture dump to be deleted");
      expect(existsSync(newDump)).toBe(true);
      expect(existsSync(savedDump)).toBe(true);
    } finally {
      serve.kill();
      await serve.exited;
    }
  }, 60_000);
});

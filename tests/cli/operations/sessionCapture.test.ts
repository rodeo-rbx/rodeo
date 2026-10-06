import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { randomUUID } from "node:crypto";
import {
  cliStudioHandle,
  makeCliRunFn,
  spawnBackground,
  waitForDom,
  type BackgroundProcess,
} from "../helpers.js";
import { decodePng, countPixels } from "../../utils/png.js";

// roblox.captureViewport inside a running session (issue #17). The engine
// refuses to promote a capture's temporary texture into an EditableImage in
// any play DOM, so the run client takes the frame from the file the engine
// writes for every capture. The two session kinds differ, and each gets a
// fresh Studio because the mode driver treats a live solo session as already
// satisfying --mode play:
//  - multiplayer (--mode play): the child client process renders a real frame;
//  - solo play-test (--mode test): Studio 0.739 on macOS returned this frame
//    all zero (a real frame by 0.741.19). A real frame is accepted; a black
//    one must be refused with a specific error and nothing written (v1.3.0
//    wrote the black PNG).

type Capture =
  | { ok: true; path: string; width: number; height: number; vpX: number; vpY: number }
  | { ok: false; err: string };

// A red 20-stud cube and a camera looking at it, then a capture into `out`.
// captureViewport is pcalled so the run itself succeeds either way and the
// assertions see the error text.
const source = (out: string) => `local roblox = require("@rodeo/roblox")
  local part = Instance.new("Part")
  part.Size = Vector3.new(20, 20, 20)
  part.Color = Color3.fromRGB(255, 0, 0)
  part.Anchored = true
  part.Position = Vector3.new(0, 10, 0)
  part.Parent = workspace
  local cam = workspace.CurrentCamera
  local vp = cam.ViewportSize
  local ok, path, info = pcall(roblox.captureViewport, "${out}",
    { cframe = CFrame.lookAt(Vector3.new(40, 30, 40), part.Position), settle = 1 })
  if not ok then return { ok = false, err = tostring(path) } end
  return { ok = true, path = path, width = info.width, height = info.height, vpX = vp.X, vpY = vp.Y }`;

function captureOut(name: string): string {
  return `.rodeo/.temp/captures/session-${name}-${randomUUID()}.png`;
}

function rmrf(path: string): void {
  try {
    rmSync(path, { force: true });
  } catch {}
}

// The written image is exactly the viewport, mostly lit (sky), and shows the
// red cube.
function expectRealFrame(r: Extract<Capture, { ok: true }>): void {
  expect(existsSync(r.path)).toBe(true);
  const img = decodePng(readFileSync(r.path));
  expect({ width: img.width, height: img.height }).toEqual({ width: Math.round(r.vpX), height: Math.round(r.vpY) });
  expect({ width: r.width, height: r.height }).toEqual({ width: img.width, height: img.height });
  const total = img.width * img.height;
  const lit = countPixels(img, (rr, g, b) => rr + g + b > 0);
  const red = countPixels(img, (rr, g, b) => rr > 150 && g < 90 && b < 90);
  expect(lit / total).toBeGreaterThan(0.9);
  expect(red / total).toBeGreaterThan(0.02);
}

describe("captureViewport in a multiplayer session (CLI)", () => {
  const PORT = 46330;
  let bg: BackgroundProcess;
  const run = makeCliRunFn(PORT);

  beforeAll(async () => {
    bg = spawnBackground(["run", "--port", String(PORT), "--place", "--mode", "play", "--context", "server"]);
    await waitForDom(PORT);
    // waitForDom returns on the edit DOM; a client run before the server DOM
    // exists would wait for a one-client session that never forms. A server
    // run blocks until the session is up (isolatedPlay does the same).
    const ready = await run({ mode: "play", context: "server", source: "return true" });
    if (!ready.ok) throw new Error(`play session did not start:\n${ready.output}`);
  });
  afterAll(async () => {
    bg.kill();
    await bg.exited;
  });

  it("play:client — writes the client's frame from the engine's capture file", async () => {
    const out = captureOut("play");
    try {
      const result = await run({ mode: "play", domKind: "client", showReturn: true, source: source(out) });
      if (!result.ok) throw new Error(`run failed (exit ${result.exitCode}):\n${result.output}`);
      const r = result.return as Capture;
      if (!r.ok) throw new Error(`captureViewport failed: ${r.err}`);
      expectRealFrame(r);
    } finally {
      rmrf(out);
    }
  });
});

describe("captureViewport in a solo play-test session (CLI)", () => {
  const cli = cliStudioHandle(46332);
  beforeAll(cli.spawn);
  afterAll(cli.close);

  it("test:client — a real frame or a specific error, never a silent black PNG", async () => {
    const out = captureOut("test");
    try {
      const result = await cli.runFn({ mode: "test", context: "client", showReturn: true, source: source(out) });
      if (!result.ok) throw new Error(`run failed (exit ${result.exitCode}):\n${result.output}`);
      const r = result.return as Capture;
      if (r.ok) {
        console.info("solo play-test capture: the engine rendered a real frame on this platform");
        expectRealFrame(r);
      } else {
        console.info("solo play-test capture: refused as black, nothing written");
        expect(r.err).toContain("entirely black");
        expect(existsSync(out)).toBe(false);
      }
    } finally {
      rmrf(out);
    }
  });
});

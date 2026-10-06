// Repro (issue #32): with Workspace.NextGenerationReplication enabled, a run on
// a play-test server DOM never started and the CLI waited forever. The plugin
// handed run parameters to its runner as string attributes, and under NGR a
// game server rejects string attributes over 50 characters (the log filter's
// JSON alone is ~100). The error escaped the plugin's run handler, so no
// ExecutionDone was sent. Scene import stored source material names the same
// way.
//
// Expected: runs work under NGR, long parameters included, and a run that
// fails to start ends with its error instead of hanging.
import { describe, beforeAll, afterAll, it, expect } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cliStudioHandle, killLaunchedStudios, runRodeo } from "../cli/helpers.js";

const ROOT = join(import.meta.dir, "..", "..");
const PORT = 47544;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const DIR = mkdtempSync(join(tmpdir(), "rodeo-ngr-"));
const PLACE = join(DIR, "ngr.rbxl");
// Starting a play test takes a while on top of the run.
const TEST_MODE_TIMEOUT = 120_000;

const cli = cliStudioHandle(PORT, { place: PLACE });

function run(args: string[], timeout?: number) {
  return runRodeo(["run", "--port", String(PORT), "--studio-id", cli.studio().studioId, ...args], { timeout });
}

// --show-return prints the value last; Studio may log other lines first.
function lastLine(stdout: string): string {
  return stdout.trim().split("\n").pop() ?? "";
}

// An empty place with NGR on, built the way the issue's repro builds it.
function buildNgrPlace(): void {
  const builder = join(DIR, "build.luau");
  writeFileSync(
    builder,
    [
      'local fs = require("@lune/fs")',
      'local roblox = require("@lune/roblox")',
      'local game = roblox.Instance.new("DataModel")',
      'game:GetService("Workspace").NextGenerationReplication = roblox.Enum.RolloutState.Enabled',
      `fs.writeFile(${JSON.stringify(PLACE)}, roblox.serializePlace(game))`,
    ].join("\n"),
  );
  const r = Bun.spawnSync(["lune", "run", builder]);
  if (r.exitCode !== 0 || !existsSync(PLACE)) {
    throw new Error(`building the NGR place failed: ${r.stdout}${r.stderr}`);
  }
}

describe("runs under NextGenerationReplication (issue #32)", () => {
  beforeAll(async () => {
    buildNgrPlace();
    await cli.spawn();
  });
  afterAll(async () => {
    await cli.close();
    Bun.spawnSync(["pkill", "-f", `__master --port ${PORT}`]);
    Bun.spawnSync(["pkill", "-f", `__studio-backend --port ${PORT + 1}`]);
    killLaunchedStudios(PORT, STARTED);
    rmSync(DIR, { recursive: true, force: true });
  });

  it("a run that fails to start ends with its error", () => {
    // A module with Archivable off can't be cloned, and an instance-path run
    // executes a clone: the plugin fails while setting the run up.
    const make = run([
      "--source",
      [
        'local m = Instance.new("ModuleScript")',
        'm.Name = "RodeoUnclonable"',
        'm.Source = "return 1"',
        "m.Archivable = false",
        'm.Parent = game:GetService("ServerStorage")',
        "return true",
      ].join("\n"),
    ]);
    expect(make.exitCode, make.stdout + make.stderr).toBe(0);

    const script = join(DIR, "unclonable.luau");
    const sourcemap = join(DIR, "sourcemap.json");
    writeFileSync(script, "return 2");
    writeFileSync(
      sourcemap,
      JSON.stringify({
        name: "Game",
        className: "DataModel",
        children: [
          {
            name: "ServerStorage",
            className: "ServerStorage",
            children: [{ name: "RodeoUnclonable", className: "ModuleScript", filePaths: ["unclonable.luau"] }],
          },
        ],
      }),
    );

    const r = run(["--sourcemap", sourcemap, script]);
    // FAILS before the fix: the run hangs until runRodeo's timeout kills it.
    expect(r.exitCode, r.stdout + r.stderr).toBe(1);
    expect(r.stderr).toContain("run failed to start");
    expect(r.stderr).toContain("Archivable");

    // The failed setup left the original module as it was.
    const check = run(["--show-return", "--source", 'return game:GetService("ServerStorage"):FindFirstChild("RodeoUnclonable") ~= nil']);
    expect(lastLine(check.stdout), check.stdout + check.stderr).toBe("true");
  }, 150_000);

  it("a server run works, with parameters longer than 50 characters", () => {
    const arg = "x".repeat(80);
    const returnFile = join(DIR, "a-return-file-whose-name-alone-is-longer-than-fifty-characters.json");
    const outputFile = join(DIR, "an-output-file-whose-name-alone-is-longer-than-fifty-characters.txt");
    const r = run(
      [
        "--mode", "test", "--context", "server",
        "--return", returnFile,
        "--output", outputFile,
        "--no-info",
        "--source", 'print("ngr-output") return require("@rodeo/process").args[1]',
        "--", arg,
      ],
      TEST_MODE_TIMEOUT,
    );
    // FAILS before the fix: the run never starts, and runRodeo's timeout kills it.
    expect(r.exitCode, r.stdout + r.stderr).toBe(0);
    expect(JSON.parse(readFileSync(returnFile, "utf8"))).toBe(arg);
    expect(readFileSync(outputFile, "utf8")).toContain("ngr-output");
  }, 150_000);

  it("the place enforces NGR's 50-character limit on string attributes", () => {
    // Guards the repro itself: without the limit, the test above proves nothing.
    const r = run(
      [
        "--mode", "test", "--context", "server", "--show-return",
        "--source", 'return (pcall(function() Instance.new("Folder"):SetAttribute("probe", string.rep("x", 51)) end))',
      ],
      TEST_MODE_TIMEOUT,
    );
    expect(r.exitCode, r.stdout + r.stderr).toBe(0);
    expect(lastLine(r.stdout), "a 51-character string attribute was accepted; is NGR on in this place?").toBe("false");
  }, 150_000);

  it("a scene with a material name longer than 50 characters round-trips on a server", () => {
    // The multi-byte character straddles the 50-byte piece boundary.
    const name = `${"m".repeat(49)}é${"n".repeat(20)}`;
    const doc = JSON.parse(readFileSync(join(ROOT, "tests/fixtures/pkg/scenes/structured.gltf"), "utf8"));
    // Untextured, so the material name lives in attributes on the part.
    delete doc.materials[0].pbrMetallicRoughness.baseColorTexture;
    delete doc.images;
    delete doc.textures;
    doc.materials[1].name = name;
    const input = join(DIR, "long-material.gltf");
    const output = join(DIR, "long-material-out.gltf");
    writeFileSync(input, JSON.stringify(doc));

    const r = run(
      [
        "--mode", "test", "--context", "server",
        "--source",
        [
          'local r = require("@rodeo/roblox")',
          `local scene = r.importEditableScene(${JSON.stringify(input)})`,
          `r.exportEditableScene(${JSON.stringify(output)}, scene.roots)`,
          "for _, root in scene.roots do root:Destroy() end",
          "for _, mesh in scene.meshes do mesh:Destroy() end",
          "return true",
        ].join("\n"),
      ],
      TEST_MODE_TIMEOUT,
    );
    expect(r.exitCode, r.stdout + r.stderr).toBe(0);
    const exported = JSON.parse(readFileSync(output, "utf8"));
    expect(exported.materials.map((m: { name: string }) => m.name)).toContain(name);
  }, 150_000);
});

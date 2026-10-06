// Repro (issue #7, item 8): `roblox.exportInstances` of a MeshPart built
// from an in-memory EditableMesh (`Content.fromObject`) succeeded silently,
// and the file reloaded with placeholder geometry. RBXM can't store the
// reference (SerializationService writes it empty), so the loss itself is
// the engine's; the export must at least say so.
//
// Expected: the export still writes the file and warns, naming each
// property whose content is lost; an export without such content is quiet.
import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { cliStudioHandle, killLaunchedStudios } from "../cli/helpers.js";

const PORT = 47532;
// Studios this file launches are reaped by session, never by pattern.
const STARTED = Date.now();
const files: string[] = [];

function tmpModel(): string {
  const path = join(tmpdir(), `rodeo-repro-export-${randomUUID()}.rbxm`);
  files.push(path);
  return path;
}

describe("exportInstances with in-memory editable content", () => {
  const cli = cliStudioHandle(PORT);
  beforeAll(cli.spawn, 120_000);
  afterAll(async () => {
    await cli.close();
    killLaunchedStudios(PORT, STARTED);
    for (const f of files) rmSync(f, { force: true });
  });

  it("warns, naming the MeshPart and SurfaceAppearance properties it loses", async () => {
    const out = tmpModel();
    const result = await cli.runFn({
      source: `
        local roblox = require("@rodeo/roblox")
        local AssetService = game:GetService("AssetService")
        local mesh = assert(AssetService:CreateEditableMesh())
        mesh:AddTriangle(mesh:AddVertex(Vector3.zero), mesh:AddVertex(Vector3.xAxis), mesh:AddVertex(Vector3.yAxis))
        local part = AssetService:CreateMeshPartAsync(Content.fromObject(mesh))
        part.Name = "rodeoIssue7Road"
        local image = assert(AssetService:CreateEditableImage({ Size = Vector2.one }))
        local surface = Instance.new("SurfaceAppearance")
        surface.ColorMapContent = Content.fromObject(image)
        surface.Parent = part
        local map = Instance.new("Model")
        map.Name = "rodeoIssue7Map"
        part.Parent = map
        roblox.exportInstances("${out}", { map })
        map:Destroy(); mesh:Destroy(); image:Destroy()
        return true
      `,
    });
    expect(result.ok, result.output).toBe(true);
    expect(existsSync(out)).toBe(true);
    expect(result.output).toContain(`[rodeo] roblox.exportInstances(${out})`);
    expect(result.output).toContain("rodeoIssue7Map.rodeoIssue7Road.MeshContent (EditableMesh)");
    expect(result.output).toContain("rodeoIssue7Map.rodeoIssue7Road.SurfaceAppearance.ColorMapContent (EditableImage)");
  });

  it("stays quiet for an export without in-memory content", async () => {
    const out = tmpModel();
    const result = await cli.runFn({
      source: `
        local roblox = require("@rodeo/roblox")
        local map = Instance.new("Model")
        Instance.new("Part").Parent = map
        Instance.new("MeshPart").Parent = map
        roblox.exportInstances("${out}", { map })
        map:Destroy()
        return true
      `,
    });
    expect(result.ok, result.output).toBe(true);
    expect(existsSync(out)).toBe(true);
    expect(result.output).not.toContain("roblox.exportInstances(");
  });
});

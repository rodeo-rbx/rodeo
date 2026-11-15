---
name: import-asset
description: Import a 3D model from an external modelling program (Blender, Maya, 3ds Max, Cinema 4D, Houdini, SketchUp, or any tool that exports glTF/GLB or FBX) into the open Studio place by exporting it, uploading it through Open Cloud with open_cloud_invoke, inserting it, and verifying that geometry, textures, colours, scale, orientation, anchoring, and placement survived. Use whenever the user asks to bring, move, import, send, or copy a model, scene, or mesh from another program or a local model file into Studio. MCP clients only.
---

# Importing a Model from an External Program

Four stages: **export → upload → insert → verify and repair**. The importer regularly changes scale, rotation, colours and sometimes textures on the way in, so **verifying is required**.

Read `rbx-open-cloud-usage` (how `open_cloud_invoke` works) and `rbx-permissions` (checking and asking for permissions) first. Never work around a permission error.

## Tool call notes

- `execute_luau` needs `datamodel_type: "Edit"` for everything in this skill.
- `screen_capture` needs a `capture_id`.
- The `skill` tool takes `skill_name`.
- End each `execute_luau` edit with `game:GetService("ChangeHistoryService"):SetWaypoint("<description>")`.

## 0. Settle the up-front questions

Uploads create **permanent assets**: Open Cloud can't archive or delete Model or Image assets; only the Creator Dashboard can.

**Permissions needed:** **Publish & manage assets → `asset:write` and `asset:read`**, plus an approved upload folder. Probe the scopes first:

```
open_cloud_invoke(method: "GET", endpoint: "/cloud/v2/universes/<game.GameId>", scopes: ["asset:read", "asset:write"])
```

Ask once, together, for whatever you don't already know:
- **Any missing scope**, named by section and toggle.
- **The upload folder** approved in MCP Permissions (no tool lists them). Write exports straight into it.
- **Consent to create assets**, saying whose account they'll go on.
- **A supplied file outside an approved folder:** ask the user to approve its folder or move the file there themselves. Don't copy or move it yourself.

Reuse anything the user already told you. Ask detail-reduction questions when step 2 finds the need. Upload only what's needed, and reuse asset IDs you already have.

## 1. Gather context

1. Call `list_roblox_studios`, then use the instance the user means. Studio instance IDs change when a place is reopened; if a call says the ID is not connected, list again.
2. Find the creator:
   ```lua
   return { creatorType = tostring(game.CreatorType), creatorId = game.CreatorId,
            userId = game:GetService("StudioService"):GetUserId() }
   ```
   For `Enum.CreatorType.Group`, upload with `creationContext.creator.groupId` = `creatorId`. Otherwise upload with `userId`.

## 2. Inspect and export from the source program

How you reach the source program depends on what you have:

- **The program has a tool or MCP integration you can drive:** inspect the scene and run the export yourself.
- **You can't drive the program:** ask the user to export the file with the settings below into an approved directory, and to tell you the path.
- **The user already has a file:** check its format and size, then read your verification targets from the file itself:
  - **glTF/GLB:** the JSON chunk lists nodes, meshes, accessor counts, and materials with `baseColorFactor` and images.
  - **FBX:** the header holds `UnitScaleFactor` and the axes, and objects list models, geometry, and materials.
  - Check triangle counts against the limit before uploading, as below.
  - If you can't read the file, say so and rely on the step 5 comparison, plus anything the user tells you.

### Respect the user's live session

You are working in someone's open, possibly unsaved, file.
- Don't change their objects. Make any changes the export needs (applying transforms, decimating, packing images) on **temporary copies**, or with exporter options, and remove the copies afterwards.
- If the file has unsaved changes, tell the user before doing anything heavy, such as decimating, exporting very dense meshes, or baking. A crash would lose their work.
- Export only the objects the user means. If the file has several scenes or collections, limit the export to the right one and check what the exported file contains.

### Record what you expect, before uploading

These are your verification targets for step 5:
- **Meshes:** the number of mesh objects, and the **triangle count of each one**, after modifiers.
- **Materials:** for each material, its image textures (base colour, normal, roughness, metalness) **and** its plain base-colour value if it has no colour texture.
- **Size:** overall size in metres, after the source unit scale.
  - Measure the world-space vertex extents. Per-object bounding boxes overstate the size of rotated objects.
  - Expected size in studs ≈ metres ÷ 0.28.
- **One asymmetric feature**, for checking orientation in step 5e. For example: "the gate is on the −Y side", or "part X sits at +X of the centre".

Then check these before exporting:
- **Triangle limit.** Roblox rejects meshes over its per-mesh triangle limit. Look up the current limit with `http_get` on `https://create.roblox.com/docs/art/modeling/specifications.md`. Check before uploading, because a failed import still leaves a permanent asset.
  - If a mesh is over the limit, ask the user whether to **split** it into pieces (keeps full detail) or **decimate** it (simpler, but loses detail).
  - Then do it on a copy.
  - **To split:** assign faces to spatial regions (for example octants of the bounding box) so that each piece is under the limit. Build each piece as a new mesh with the original's material slots and world transform. Name the pieces `<Name>_Part<N>`.
  - **To decimate:** add the modifier to a copy, then export with modifiers applied.
  - Delete the temporary objects after exporting.
- **Textures that won't travel.** Some images exist only in memory: generated, or unsaved with no file path. Pack or save them first, or use a format that embeds them.
- **Textures that are blank.** If a texture is a single flat colour, or entirely black or empty, tell the user. Importing it will look wrong even though nothing was lost.

### Format

Use a model format that `open_cloud_invoke` accepts. Its tool description lists the current set (at the time of writing: `glb`, `gltf`, `fbx`, `rbxm`, `rbxmx`) and the per-file size limit.

- **Prefer GLB** when the program can write it. It is a single file with textures embedded.
- **Use FBX with embedded textures** as the alternative.
- A `.gltf` with separate `.bin` and image files only works if every file is uploaded together.
- If the file is too big, reduce texture resolution or split the scene into several models.

### Export settings (whatever the program calls them)

- Export **meshes only**: no cameras, lights, rigs, or helpers unless the user wants them.
- Apply modifiers through the exporter option rather than by changing the user's objects.
- Embed or pack textures.
- Keep real-world units. Studio's conversion varies by format, so step 5c checks it.
- Give every mesh and material a meaningful, unique name. Repairs match parts to source materials by name.

## 3. Upload through Open Cloud

```
open_cloud_invoke(
  method: "POST",
  endpoint: "/assets/v1/assets",
  body: '{"request":{"assetType":"Model","displayName":"<name>","description":"<description>","creationContext":{"creator":{"userId":"<id>"}}}}',
  files: [{ name: "fileContent", path: "<absolute path in an approved directory>" }],
  scopes: ["asset:read", "asset:write"])
```

The response is an operation. Poll `GET /assets/v1/operations/<operationId>` (scope `asset:read`) every few seconds until `done` is `true`. Then read `response.assetId` and `response.moderationResult.moderationState`:
- **`Approved`:** continue.
- **`Rejected`:** for the **Model**, stop and tell the user, because there is nothing to insert. Don't re-upload the same file.
- **`Reviewing`, or any other pending state:** the asset exists and will usually be approved shortly.
  - Poll `GET /assets/v1/assets/<assetId>` (scope `asset:read`) every 10–20 seconds for a few minutes, reading `moderationResult.moderationState`.
  - If the **Model** is still pending after that, tell the user its asset ID is waiting for moderation, and stop.
  - **Never upload again to get around a pending state**, because that only creates duplicate permanent assets.

For the **Image** uploads in 5b, a rejected or pending image **doesn't stop the import**: leave that texture unrepaired, note it for the report, and carry on.

## 4. Insert

```
insert_asset(assetId: "<assetId>", assetType: "Model", assetName: "<name>", parentPath: "game.Workspace")
```

Note these in the result:
- **`tag`:** use it, or `insertedInstances[1].fullPath`, to find your model from now on. Other models in the place may share its name.
- **`isPackage: true`:** the instance is linked to the uploaded asset. Your repairs change only this copy (see step 6).

Expect the model to arrive at an **arbitrary position**, often near the camera's focus, and **rotated by an arbitrary angle around the vertical axis**. Step 5e fixes both.

## 5. Verify and repair

Compare everything with what you recorded in step 2. Meshes may be nested inside `<Name>_Node` or `<Name>` models, and named after the mesh data (for example `<Name>_mesh`), so always walk `GetDescendants()` and match names loosely.

### 5a. Geometry

Count the `MeshPart` descendants and compare with the source mesh count. If any are missing, check them against the triangle limit and the source, fix the source (on copies), and re-export.

### 5b. Textures and colours

A part is textured if **either** its `SurfaceAppearance` has a non-empty `ColorMap` **or** its `MeshPart.TextureID` is non-empty. Materials with only a colour map often import as `TextureID` with no `SurfaceAppearance`.

```lua
local model = -- find by the insert tag or fullPath
local report = {}
for _, d in model:GetDescendants() do
    if d:IsA("MeshPart") then
        local sa = d:FindFirstChildWhichIsA("SurfaceAppearance")
        table.insert(report, {
            name = d.Name,
            colorMap = sa and sa.ColorMap or "",
            normalMap = sa and sa.NormalMap or "",
            textureId = d.TextureID,
            color = tostring(d.Color),
        })
    end
end
return report
```

For each part, compare with its source material:

- **The source had a colour texture, but the part has neither `ColorMap` nor `TextureID`:** the texture was dropped. Repair it in this order:
  1. If another part with the same source material did get a texture, clone its `SurfaceAppearance`, or copy its `TextureID`. No upload is needed.
  2. Otherwise, write that material's maps as PNG or JPG into an approved directory and upload each one with `assetType: "Image"`, using the same create-and-poll flow.
     - Upload only maps with real detail. **Skip single-flat-colour maps**: moderation tends to reject them, and they add nothing.
     - Then create or replace the part's `SurfaceAppearance` with `ColorMap`/`NormalMap = "rbxassetid://<id>"`.
- **The source had a plain base colour and no texture:** check `MeshPart.Color`. The importer often drops it and leaves default grey (about 0.64, 0.64, 0.65). If so, set `Color` from the source value.
  - glTF `baseColorFactor` and Blender's Principled BSDF Base Color are **linear**; convert them to sRGB first: `c <= 0.0031308 ? 12.92c : 1.055·c^(1/2.4) − 0.055`.
  - Other programs' colour fields, such as FBX `DiffuseColor` from some exporters, may already be sRGB. If you can't tell, compare the result against a screenshot of the source and pick the version that matches.
  - Don't invent transparency or materials the source didn't specify.
- **Several materials share one image ID:** that is fine. The importer de-duplicates identical images.

If you can't match a part to a source material, list it for the user rather than guessing. Finish with a `screen_capture` close enough to tell textured, coloured, and default-grey surfaces apart.

### 5c. Scale

**Expect the scale to be wrong.** The importer's unit conversion depends on the format and the exporter. Commonly:
- a glTF/GLB in metres arrives at 1 m = 1 stud, which is **3.57× too small**;
- an FBX often arrives at 1 cm = 1 stud, which is **28× too big**. This happens whether the source was authored in centimetres or in metres, because many exporters write FBX in centimetres.

Don't match against a list; compute the fix. Measure the **world-space** height from the parts themselves. `GetBoundingBox` measures in the model's pivot frame, and that frame may not be upright.

```lua
local minY, maxY = math.huge, -math.huge
for _, p in model:GetDescendants() do
    if p:IsA("BasePart") then
        local h = p.Size / 2
        for _, c in { Vector3.new(h.X, h.Y, h.Z), Vector3.new(-h.X, h.Y, h.Z), Vector3.new(h.X, -h.Y, h.Z), Vector3.new(h.X, h.Y, -h.Z),
                      Vector3.new(-h.X, -h.Y, h.Z), Vector3.new(-h.X, h.Y, -h.Z), Vector3.new(h.X, -h.Y, -h.Z), Vector3.new(-h.X, -h.Y, -h.Z) } do
            local y = p.CFrame:PointToWorldSpace(c).Y
            minY, maxY = math.min(minY, y), math.max(maxY, y)
        end
    end
end
assert(maxY > minY, "model has no parts to measure")
local factor = expectedHeightStuds / (maxY - minY)   -- expected = source metres / 0.28
model:ScaleTo(model:GetScale() * factor)
```

If the model is lying on its side (5e), stand it upright first, then measure. The world-space height is unaffected by any turn around the vertical axis, so you don't need to fix that before scaling.

### 5d. Anchoring

Imported parts arrive **unanchored** and will fall or come apart at playtest. Anchor every `BasePart` unless the user said the model should be physically simulated.

### 5e. Orientation and placement

1. **Rotation.**
   - Check orientation against the **asymmetric feature** you recorded in step 2, not against the pivot. The pivot frame the importer creates doesn't always line up with the source axes. So "reset the pivot to identity" can leave the model turned, and the yaw it arrived with is sometimes already correct.
   - Rotate until the feature is on the expected side. A common mapping for glTF from a Z-up program is source (x, y, z) → Roblox (x, z, −y). Confirm it with the feature rather than assuming it.
   - If the model is lying on its side, the up axis was wrong; rotate it, or re-export with a different up axis.
   - Check for mirroring with a second asymmetric feature. For example: feature A is on +X **and** feature B is on −Z. A mirror flips one of them.
2. **Position.** Put the model where the user asked. "Next to X" means just outside X's actual geometry, with a small gap. If they didn't say, search outward from the origin for the nearest clear spot. In either case:
   - **Find the ground** under the target spot with a downward raycast that excludes the model. Don't assume Y = 0.
   - **Check for overlaps against real geometry.** `GetPartBoundsInBox` only tests bounding boxes, so a large or ring-shaped mesh (terrain backdrops, cliffs) can make every spot look taken. Instead:
     1. Place a temporary probe `Part` the size of the model's footprint, raised slightly above the ground so it doesn't touch it, with `Anchored = true`, `CanCollide = false`, and `Transparency = 1`.
     2. Call `workspace:GetPartsInPart(probe, params)`, excluding the model and the ground part.
     3. Destroy the probe.
   - **Never block a `SpawnLocation`.** Keep at least a few studs between the model and the spawn's footprint.
   - Rest the model's bottom on the ground. Position by its **bounding-box** centre: centre Y = ground Y + half the height. The pivot can be offset from that centre, so move by `targetCentre − boxCentre` rather than setting the pivot directly.
   - If the nearest clear spot is far from where the user would expect (for example, more than about 50 studs away), say so in your report, or ask where they want it.
3. Tell the user where the model ended up.

### 5f. Final look

Take a `screen_capture` with `camera_position` and `look_at_position` framing the model, and look at it. Confirm that geometry, textures, colours, and orientation match the source before you report.

## 6. Package caveat

If the model was inserted as a package, every repair from step 5 lives only on this copy. The uploaded asset is still the unrepaired version, so a re-insert or a package update brings the problems back. Tell the user, and offer one of these:
- publish the repaired copy as a new asset version or a new asset, or
- keep it as an unlinked local copy, if they don't need package updates.

Do neither without the user's go-ahead.

## 7. Report

Tell the user:
- the asset IDs you created and how many uploads you made in total;
- any image IDs the importer created on its own (from `TextureID` or `ColorMap`), so they know those exist too;
- where the model is in the place, and its size in studs;
- what step 5 found and fixed: missing textures or colours, scale, rotation, anchoring, placement;
- anything you couldn't fix, such as rejected images, unmatched parts, or textures that were blank in the source;
- the package caveat, if it applies.

## Extending to other formats

Steps 3–7 don't depend on the format. For a new one: check `open_cloud_invoke` accepts it, and add notes under step 2 on how its textures and colours travel. Step 5 catches whatever its importer drops.

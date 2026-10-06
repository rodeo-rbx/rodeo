---
title: roblox
---

```luau
local roblox = require("@rodeo/roblox")
```
:::caution
These APIs are not finalized and may change in backwards incompatible ways.
:::

## Summary

| Entry | Description |
| :--- | :--- |
| [CaptureInfo](#captureinfo) | Size of the image `captureViewport` wrote, in pixels. The capture's logical |
| [CaptureOptions](#captureoptions) | Camera and device options for `captureViewport`. All fields optional. |
| [EditableScene](#editablescene) | Caller-owned unparented roots and live editable resources. Destroy roots, |
| [EditableSceneExport](#editablesceneexport) |  |
| [ExportEditableSceneOptions](#exporteditablesceneoptions) | Procedural scenes can supply portable motion along with their roots. |
| [ImportEditableSceneOptions](#importeditablesceneoptions) |  |
| [SceneAnimation](#sceneanimation) | Channels are authoritative for export. The native CurveAnimation has a |
| [SceneAnimationChannel](#sceneanimationchannel) | Exact glTF node-local channel. Values are flat scalars (XYZ or XYZW); |
| [SceneJoint](#scenejoint) | An original glTF joint can bind multiple mesh instances. |
| [SceneMorph](#scenemorph) |  |
| [SceneMorphTarget](#scenemorphtarget) | Position/tangent deltas use stable EditableMesh vertex IDs; normal deltas |
| [SceneMorphWeights](#scenemorphweights) |  |
| [ScenePrimitive](#sceneprimitive) | One imported primitive and its original glTF indices (zero-based). |
| [SceneSourceMap](#scenesourcemap) | Source indices are zero-based glTF indices, not Luau array positions. |
| [bake](#robloxbake) | Writes `value` to `path` as a Luau module (`return <value>`), so the data |
| [capture](#robloxcapture) | Deprecated alias of `roblox.captureViewport`. |
| [captureViewport](#robloxcaptureviewport) | Captures a Studio screenshot and writes it to a stable path, returning |
| [export](#robloxexport) | Deprecated alias of `roblox.exportInstances`. |
| [exportEditableImage](#robloxexporteditableimage) | Writes an `EditableImage`'s pixels to `path` as a PNG. Only `.png` is |
| [exportEditableMesh](#robloxexporteditablemesh) | Writes an `EditableMesh` to `path` as glTF 2.0 (`.glb` binary, or `.gltf` |
| [exportEditableScene](#robloxexporteditablescene) | Exports supported instance roots to self-contained glTF/GLB, including |
| [exportInstances](#robloxexportinstances) | Exports `instances` as a `.rbxm` or `.rbxmx` model file at `path`. |
| [import](#robloximport) | Deprecated alias of `roblox.importInstances`. |
| [importEditableImage](#robloximporteditableimage) | Loads the PNG or JPEG at `path` into a new `EditableImage` (RGBA8) and |
| [importEditableMesh](#robloximporteditablemesh) | Loads the `.glb`, `.gltf` or `.obj` at `path` into a new `EditableMesh` and |
| [importEditableScene](#robloximporteditablescene) | Imports the default (or first) glTF/GLB scene into one named Model, |
| [importInstances](#robloximportinstances) | Imports a `.rbxm` or `.rbxmx` model file at `path` as Instances. |

---

## Types

### CaptureInfo

Size of the image `captureViewport` wrote, in pixels. The capture's logical

size: the window's viewport, the `viewportSize`, or a preset's resolution

(rotated in portrait; a phone or tablet preset renders its full screen, so

this is larger than its inset `Camera.ViewportSize`). Always the capture's

`Camera.ViewportSize`: the engine's frame is larger on high-DPI displays and

is resampled down to it, so a capture is the same size on every machine and

offset-based UI maps 1:1 onto pixels.

```luau
type CaptureInfo = {
	width: number,
	height: number,
}
```

---

### CaptureOptions

Camera and device options for `captureViewport`. All fields optional.

`cframe` — scripted camera position for the shot (restored after).

`fov` — field of view. `focus` — camera focus CFrame (defaults to 100 studs

along `cframe`'s look vector when `cframe` is set). `settle` — seconds to

wait before capturing.

`device` — a Studio device-simulator preset id to capture as (for example

`"iphone_13"` or `"hd_1080"`); layout, insets and orientation come from the

preset. `viewportSize` — the `Camera.ViewportSize` to capture at, as a

custom desktop device (at most 7680 by 4320); with `device`, overrides the

preset's resolution. Either drives Studio's device simulator for the shot

and restores it afterward, the way the camera fields are restored.

`resample` — `true` (the default) resamples the engine's frame to exactly

the viewport, so UI offsets map 1:1 onto pixels; `false` writes the frame

at its rendered size, the viewport times the display scale (2x on Retina),

or whatever scale `scalingMode`/`pixelDensity` produced.

Simulator overrides, applied for the shot and restored after like the

camera fields; each needs `device` or `viewportSize`: `scalingMode` —

`"ActualResolution"` (default, the display's scale) or `"ScaleToPhysicalSize"`

(host DPI over `pixelDensity`); `"FitToWindow"` renders at window size and is

refused, there is no frame to capture. `pixelDensity` — DPI,

72 to 10000, the scale knob in ScaleToPhysicalSize mode (density 72 on a

2x display renders 3.06x: 7680x4320 becomes 23466x13200 with `resample =

false`). `orientation` — `"Portrait"`, `"LandscapeLeft"` or

`"LandscapeRight"`, phone and tablet forms only. `deviceForm` — the form of

the custom `viewportSize` device, `"Desktop"` (default), `"Phone"`,

`"Tablet"`, `"Console"` or `"VR"`; non-desktop forms add their chrome.

Presets on Studio 0.739 (`StudioDeviceSimulatorService:GetDeviceListAsync()`

has the live list): consoles `xbox`, `ps4`, `ps5`, `android_tv_1080`; desktops `average_laptop`, `hd_720`, `hd_1080`, `vga`; handhelds `generic_handheld_720`, `generic_handheld_1080`; VR `meta_quest_2`, `meta_quest_3`; phones `iphone_6_Plus`, `iphone_7`, `iphone_XR`, `iphone_11`, `iphone_13`, `iphone_13_pro`, `iphone_13_pro_max`, `iphone_14`, `iphone_16`, `iphone_16_pro`, `iphone_16_pro_max`, `iphone_17_pro`, `samsung_galaxy_a06`, `samsung_galaxy_a16`, `samsung_galaxy_s22_ultra`, `samsung_galaxy_s25_ultra`; tablets `ipad_6th_generation`, `ipad_8th_generation`, `ipad_9th_generation`, `ipad_10th_generation`, `ipad_a16`, `ipad_air_5th_generation`, `ipad_pro_M4_11in`, `ipad_pro_M5_13in`, `xiaomi_redmi_pad_se`, `amazon_fire_hd10_2023`, `samsung_galaxy_tab_a8`, `samsung_galaxy_tab_a9`, `samsung_galaxy_tab_a9+`, `samsung_galaxy_tab_S11`.

```luau
type CaptureOptions = {
	cframe: CFrame?,
	fov: number?,
	focus: CFrame?,
	settle: number?,
	device: string?,
	viewportSize: Vector2?,
	resample: boolean?,
	scalingMode: string?,
	pixelDensity: number?,
	orientation: string?,
	deviceForm: string?,
}
```

---

### EditableScene

Caller-owned unparented roots and live editable resources. Destroy roots,

meshes, and images when finished. Destroying a root does not destroy the

editable resources it references; meshes/images can be shared by parts.

```luau
type EditableScene = {
    roots: { Model }, -- one container for the selected glTF scene
    meshes: { EditableMesh },
    images: { EditableImage },
    sourceMap: SceneSourceMap,
    warnings: { string },
    animations: { SceneAnimation },
    morphs: { SceneMorph },
    morphWeights: { SceneMorphWeights },
    animationRig: Model?,
    animator: Animator?,
}
```

---

### EditableSceneExport

```luau
type EditableSceneExport = {
    roots: { Instance },
    animations: { SceneAnimation }?,
    morphs: { SceneMorph }?,
    morphWeights: { SceneMorphWeights }?,
}
```

---

### ExportEditableSceneOptions

Procedural scenes can supply portable motion along with their roots.

The full import result is also accepted and retains source coordinate frames.

```luau
type ExportEditableSceneOptions = {
    strict: boolean?, -- abort before writing on any warning
}
```

---

### ImportEditableSceneOptions

```luau
type ImportEditableSceneOptions = {
    strict: boolean?, -- abort on any approximation/unsupported-feature warning
}
```

---

### SceneAnimation

Channels are authoritative for export. The native CurveAnimation has a

Studio-only registered Animation ID. Quaternion cubic/composed curves may

require an adaptive approximation; native key times use Studio's clock.

Any approximation or timestamp quantization is reported in warnings.

```luau
type SceneAnimation = {
    name: string,
    sourceIndex: number?,
    channels: { SceneAnimationChannel },
    clip: CurveAnimation?,
    animation: Animation?,
}
```

---

### SceneAnimationChannel

Exact glTF node-local channel. Values are flat scalars (XYZ or XYZW);

weights have one scalar per morph target. CUBICSPLINE stores incoming

tangent, value, outgoing tangent for each key; tangents are per second.

```luau
type SceneAnimationChannel = {
    node: Instance,
    path: "translation" | "rotation" | "scale" | "weights",
    interpolation: "LINEAR" | "STEP" | "CUBICSPLINE",
    times: { number },
    values: { number },
}
```

---

### SceneJoint

An original glTF joint can bind multiple mesh instances.

```luau
type SceneJoint = {
    mesh: EditableMesh,
    boneId: number,
    bone: Bone,
    part: MeshPart,
}
```

---

### SceneMorph

```luau
type SceneMorph = {
    mesh: EditableMesh,
    targets: { SceneMorphTarget },
    tangents: { [number]: { number } }?, -- base tangent XYZW, keyed by vertex ID
}
```

---

### SceneMorphTarget

Position/tangent deltas use stable EditableMesh vertex IDs; normal deltas

use normal IDs. An omitted attribute means zero deltas. Include every

referenced ID when supplying an attribute, including explicit zero deltas.

```luau
type SceneMorphTarget = {
    name: string?,
    positions: { [number]: Vector3 }?,
    normals: { [number]: Vector3 }?,
    tangents: { [number]: Vector3 }?,
}
```

---

### SceneMorphWeights

```luau
type SceneMorphWeights = {
    node: Instance, -- source node; multiple primitive parts can share it
    part: MeshPart,
    weights: { number },
}
```

---

### ScenePrimitive

One imported primitive and its original glTF indices (zero-based).

```luau
type ScenePrimitive = {
    nodeIndex: number,
    meshIndex: number,
    primitiveIndex: number,
    skinIndex: number?,
    materialIndex: number?,
    mesh: EditableMesh,
    part: MeshPart,
}
```

---

### SceneSourceMap

Source indices are zero-based glTF indices, not Luau array positions.

Images/materials/joints can map to several objects after channel splitting

or mesh instancing. `primitives` is an ordinary one-based binding array.

```luau
type SceneSourceMap = {
    nodes: { [number]: Instance },
    primitives: { ScenePrimitive },
    materials: { [number]: { SurfaceAppearance | MeshPart } },
    images: { [number]: { EditableImage } },
    joints: { [number]: { SceneJoint } },
}
```

---

## Functions and Properties

### roblox.bake

Writes `value` to `path` as a Luau module (`return <value>`), so the data

can be required straight back into Studio. Roblox types round-trip through

their constructors (vectors, CFrames, colors, enums, …); values with no

source representation (Instances, functions) become their `tostring`.

Parent directories are created as needed. This is the same path

`--return <file>.luau` uses.

```luau
(path: string, value: any) -> ()
```

---

### roblox.capture

Deprecated alias of `roblox.captureViewport`.

:::caution[Deprecated]

Use `roblox.captureViewport`. This alias warns once per run and will be

removed in 2.0.

:::

```luau
(output: string?, options: CaptureOptions?) -> (string, CaptureInfo)
```

---

### roblox.captureViewport

Captures a Studio screenshot and writes it to a stable path, returning

that absolute path and the image size. `output` ending in `.png` is the

exact file path; any other value is a directory the auto-named `.png` lands

in; omitted defaults to the `.rodeo/.temp/captures` directory. Relative

paths resolve against the run client's cwd. The image is always exactly

the capture's `Camera.ViewportSize` (the window's, or `viewportSize` /

the `device` preset's). Camera and device-simulator state are restored

after the capture, on error too. Requires a viewport: plugin context, or

the client DOM of a running session. The frame is taken from the file the

engine writes for every capture, so the simulator's full 7680 by 4320

works on every display; a frame that comes back entirely black is refused

with an error and nothing is written. Limits are Studio's own and surface

as errors: the device simulator accepts at most 7680 by 4320. The capture

then waits for the engine with no deadline, since a large frame or a slow

GPU can take well over 10s (the largest frame takes about 7s). One case

never completes and waits until the run is killed: a minimized Studio on

Windows (background launches are minimized there; launch focused or

restore the window).

```luau
(output: string?, options: CaptureOptions?) -> (string, CaptureInfo)
```

---

### roblox.export

Deprecated alias of `roblox.exportInstances`.

:::caution[Deprecated]

Use `roblox.exportInstances`. This alias warns once per run and will be

removed in 2.0.

:::

```luau
(path: string, instances: { Instance }) -> ()
```

---

### roblox.exportEditableImage

Writes an `EditableImage`'s pixels to `path` as a PNG. Only `.png` is

supported. Parent directories are created as needed. EditableImage has no

file representation of its own (`export` writes an Object-backed image

content as an empty reference), so this is how generated textures reach

the source tree.

```luau
(path: string, image: EditableImage) -> ()
```

---

### roblox.exportEditableMesh

Writes an `EditableMesh` to `path` as glTF 2.0 (`.glb` binary, or `.gltf`

with an embedded buffer) or Wavefront OBJ (`.obj`). Positions, per-corner

normals, UVs and colors, triangles, and skinning (bones with bind poses and

up to four influences per vertex) are written to glTF; FACS poses are not.

Faces must be triangles (`mesh:Triangulate()` first). glTF's conventions

are Roblox's, so nothing is converted: studs, Y-up, right-handed, UV origin

top-left. OBJ has no units or handedness, so the same frame is written

(Blender's default), with one conversion: `vt` V is flipped, since OBJ's UV

origin is bottom-left. OBJ faces index positions, UVs and normals per

corner, so seams and hard edges survive; no MTL is written. OBJ cannot

carry vertex colors or skinning. Returns the list of features the format

dropped, empty for glTF.

```luau
(path: string, mesh: EditableMesh) -> { string }
```

---

### roblox.exportEditableScene

Exports supported instance roots to self-contained glTF/GLB, including

live editable or readable asset-backed meshes and images. Works with

procedurally created roots; no import result or source map is required.

Models/Folders, MeshParts, block Parts, Attachments and skins are supported.

Other instance behavior is reported in warnings; unsupported part geometry

and unreadable assets error. Native materials use documented scalar PBR

approximations; emissive/transmission behavior warns. Mesh centering, size, hierarchy and current

Bone poses are preserved. Publishing and RBXM serialization are separate.

Returns warnings for unsupported features. The destination is replaced

only after successful scene encoding; caller-owned objects are untouched.

Pass the full EditableScene to preserve edited animation channels, morph

data and original animation coordinate frames. Mesh edits modify base

geometry directly; no remembered morph offsets are subtracted. Generated

playback helpers are omitted. Editing CurveAnimations does not edit portable

channels. Source scale metadata is stored in RodeoScene* attributes and

survives Instance:Clone(); callers must retarget channels/morph bindings to

cloned instances. Scene containers become glTF scenes, not extra nodes.

Roots-only export captures the current static pose and warns when imported

motion data is omitted. Removed animation targets and incomplete morph maps

error instead of silently corrupting motion. No animations are published.

```luau
(path: string, sceneOrRoots: EditableSceneExport | { Instance }, options: ExportEditableSceneOptions?) -> { string }
```

---

### roblox.exportInstances

Exports `instances` as a `.rbxm` or `.rbxmx` model file at `path`.

Neither format stores an in-memory `EditableMesh` or `EditableImage`: a

`Content.fromObject` reference (for example a `MeshPart` made with

`AssetService:CreateMeshPartAsync(Content.fromObject(mesh))`) exports

empty. The export still succeeds and warns, naming each such property;

export those objects with `roblox.exportEditableMesh` or

`roblox.exportEditableImage`.

```luau
(path: string, instances: { Instance }) -> ()
```

---

### roblox.import

Deprecated alias of `roblox.importInstances`.

:::caution[Deprecated]

Use `roblox.importInstances`. This alias warns once per run and will be

removed in 2.0.

:::

```luau
(path: string) -> { Instance }
```

---

### roblox.importEditableImage

Loads the PNG or JPEG at `path` into a new `EditableImage` (RGBA8) and

returns it. Relative paths resolve against the run client's cwd. Studio

bounds EditableImage dimensions; an image it refuses errors with its size.

```luau
(path: string) -> EditableImage
```

---

### roblox.importEditableMesh

Loads the `.glb`, `.gltf` or `.obj` at `path` into a new `EditableMesh` and

returns it. glTF: node transforms are baked into the geometry, all

primitives merge into one mesh, and a skin becomes bones plus vertex

weights. OBJ: positions, per-corner UVs (V flipped from OBJ's bottom-left

origin) and normals; faces as `v`, `v/vt`, `v//vn` or `v/vt/vn` with

negative indices allowed; planar polygons (including concave faces) are

triangulated with their winding preserved; `o`/`g` groups

merge into the one mesh; materials are ignored. In both formats an

attribute (normals, UVs, colors) is kept only when every primitive, or

every OBJ corner, carries it.

OBJ numeric components must be finite; invalid values and polygons that

cannot be triangulated report the source line.

Relative paths resolve against the run client's cwd. Turn the result into a

part with `AssetService:CreateMeshPartAsync(Content.fromObject(mesh), opts)`.

The mesh is built with the per-element EditableMesh calls, so only the

engine's own per-mesh limits apply (60000 vertices and 20000 triangles on

Studio 0.739); past one, the engine's error surfaces with the count reached,

and the file needs splitting into smaller primitives.

```luau
(path: string) -> EditableMesh
```

---

### roblox.importEditableScene

Imports the default (or first) glTF/GLB scene into one named Model,

preserving hierarchy, node poses, separate primitives, shared

meshes, UVs/colors/normals, PBR base color/normal/metallic/roughness maps,

and supported skins with Bone instances and their initial poses.

PNG/JPEG textures can be embedded or external. Resources remain live and

editable. Source indices map to objects without depending on their names.

Coordinates follow the mesh APIs: Y-up, one file unit per stud.

Sheared world transforms and scaled skins error; reflected static meshes

are supported. Animation curves (STEP/LINEAR/CUBICSPLINE), morph position/

normal/tangent deltas, names, and per-instance weights are preserved as

editable data. Meshes contain the unposed base geometry and remain shared

across instances with different morph weights. Initial weights are data;

Roblox playback does not apply generic vertex morphs. Update delta maps

after topology edits that create vertex/normal IDs.

Translation/rotation clips create a rig inside the scene Model, with an

anchored root, driven unanchored parts, Motor6Ds, Animator and CurveAnimations.

Compatible STEP/LINEAR and translation CUBICSPLINE keys remain exact.

Quaternion cubic/composed transforms use adaptive linear curves and warn.

Studio may quantize key times; collisions omit the native clip and warn.

Parent the scene Model to workspace before Animator:LoadAnimation.

Scale/weight channels remain portable data and warn. Unrepresentable poses

and Attachment-targeted animation warn and omit the native clip.

Cameras, material extensions, emissive/

occlusion maps and sampler differences warn. Untextured materials have no

SurfaceAppearance/images: factors are preserved as attributes, with an

explicitly reported native-material preview approximation. Native material

identity is restored only from Rodeo metadata, never guessed from factors.

strict=true fails on warnings and destroys all temporary objects.

Required extensions and unreadable geometry/images error. On failure all

partially created objects are destroyed. No assets are uploaded.

```luau
(path: string, options: ImportEditableSceneOptions?) -> EditableScene
```

---

### roblox.importInstances

Imports a `.rbxm` or `.rbxmx` model file at `path` as Instances.

```luau
(path: string) -> { Instance }
```

---

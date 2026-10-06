---
name: rodeo
description: CLI tool for Roblox Studio that lets you create studio instances, and run code in any studio environment. Includes commands, flags, DOM targeting, directives, return values, and @rodeo APIs. Use when writing rodeo commands, scripts, or working with Roblox Studio.
metadata:
  version: 1.5.0-rc.11
---

# rodeo

This skill describes rodeo **1.5.0-rc.11**. Projects pin their own rodeo version, so
check `rodeo --version` in the project: if it differs, this copy of the skill may
document flags or APIs that binary does not have (or lack ones it does).

CLI that executes Luau code inside Roblox Studio. Studio is the runtime. rodeo connects to it over a WebSocket, sends scripts, and streams output back. Use it like a conventional language runtime.

## Quick start

```bash
rodeo run --place --source "return 1"   # open a fresh place, run, close it
rodeo run script.luau                   # file execution (against a running serve)
rodeo run myscript                      # shorthand for .rodeo/myscript.luau
rodeo run - < script.luau               # stdin
rodeo serve                             # optional: persistent server (no Studio launch)
rodeo state                             # canonical state: studios, DOMs, runs
rodeo kill <id>                         # kill a run OR close a studio by id (from rodeo state)
rodeo save <studio-id>                  # save a studio's place back to its source file
```

`rodeo run --place` spawns a Studio with an empty place. That Studio belongs to the run process and closes when the run ends. Add `--detach` to keep the Studio alive after the run exits.

## Use your own studio

Launch your own Studio. Do not route work into one that is already connected.

A run with no `--place`, `--studio-id`, or `--dom-id` matches **any** connected
DOM. So the easy mistake is to run `rodeo state`, see a Studio, and use it.
That Studio usually belongs to someone else, with unsaved edits and a specific
place open. Your script mutates its DOM, `--save` writes to their file, and
`rodeo kill <studio-id>` closes it on them.

Use a Studio you did not launch only when:

- you opened it earlier and know what is in it,
- the task is explicitly about that session ("the place I have open"), or
- you were told which Studio to use.

Otherwise start your own with `rodeo run --place`. Add `--detach` to keep it
alive across several runs, then `rodeo kill <studio-id>` when you finish. Pin
follow-up runs with `--studio-id` so they cannot drift onto another Studio.

On a shared machine, other agents and people run their own serves and Studios
at the same time:

- **Never run `pkill RobloxStudio`, and never kill a Studio you did not
  launch.** Find yours by the temp place path in its arguments:
  `ps -eo pid,command | grep "RobloxStudio -task"` prints
  `-localPlaceFile .../.rodeo/.temp/rodeo-<uuid>.rbxl`. Match the uuid to your
  own launch.
- `rodeo kill <studio-id>` closes Studios that the current serve launched. For
  a leftover Studio from an earlier serve, kill its pid instead. Studio ignores
  SIGTERM, so send SIGKILL.
- **Give each harness its own port** (`rodeo serve --port <n>`, or `RODEO_PORT`
  in the project's `.mise.toml`/`.env`). Two agents on one port route runs into
  each other's sessions. Serves on different ports are fully independent —
  each installs its own `rodeo-<build>-<port>.rbxm` plugin file — so different
  rodeo versions can run side by side.
- A Studio the user opened by hand connects to every running serve and shows
  up in each `rodeo state`; Studios a serve launched belong to that serve only.
- When a serve restarts, Studios from earlier runs reconnect to it. If
  `rodeo state` lists more than one Studio, pin every run with `--studio-id`.

## Commands

### `rodeo serve`

Start a persistent server. Does NOT launch Studio — use `run --place` for that.

- `--port <n>` — master port (resolution: the flag, then `RODEO_PORT`, then 44872)
- `--master` / `--studio` / `--master-host` / `--master-port` — process-split internals; rarely needed directly
- `--ppid <pid>` — exit when this process dies

### `rodeo run`

Run a script in Studio.

- `<script>` — path to script, or `-` for stdin. Any name without a `.` resolves to `.rodeo/<name>.luau` when that file exists — nested paths included (`rodeo run tests/smoke` → `.rodeo/tests/smoke.luau`)
- `-s` / `--source <code>` — execute inline source code
- `--mode edit|run|test|play` — Studio mode (auto-transitions; the only flag that does). Defaults to edit; never inferred from --context/--dom, so a server/client run must pass --mode
- `--context plugin|server|client|elevated|cmdbar` — the identity level to run at: plugin, server-runtime identity, client-runtime identity, command bar via StudioMCP (elevated), or command bar via the launch bootstrap's bridge (cmdbar; edit DOM of a rodeo-launched Studio only). Not a script class
- `--dom edit|server|client` — which DOM (usually inferred); `edit` targets the edit DOM even while a test/play session runs
- `--studio-id <id>` — scope routing to one studio (id from `rodeo state`; unique prefix ok)
- `--dom-id <id>` — pin the run to one DOM (id from `rodeo state`; unique prefix ok). Only `--context` may accompany it
- `--show-return` — print return value to stdout (any size)
- `--return <path>` — write return value to file: `.luau`/`.lua` emits Luau source, anything else JSON. Size-unbounded.
- `--output <path>` — write execution output (prints/logs) to file
- `--reload-requires` — re-evaluate instance requires instead of using the live cached modules (see below)
- `--place [<value>]` — launch Studio: empty (no value), a place ID (number), or a file path (`.rbxl`/`.rbxlx`). Guarantees a fresh place even if a serve already has one open; the run is pinned to it and it closes after the run (unless `--detach`).
- `--place.universe <id>` — universe ID (auto-resolved from place ID if omitted)
- `--detach` — keep Studio running after rodeo exits
- `--focus` — bring Studio to the front on launch (default: background). Studio takes keyboard focus only if it opens on the display you're working on; on another display it's raised there and your focus stays where it is
- Without `--focus`, Studio stays in the background on every display: it opens without activating, and when it activates itself (Studio does this as a test session starts or ends) rodeo hands focus straight back to the app you were in — unless you switched to Studio yourself (click or ⌘-Tab)
- `--show-widgets <spec>` — allow-list of Studio dock widgets to keep; everything else (panels, ribbon, command bar) is hidden. `none` hides all; a comma list keeps those (aliases: output, explorer, properties, editor, toolbox, assistant, ribbon, commandbar, rodeo — this serve's own panel; or a raw panel ID). Restored on exit
- `--save [path]` — save the place after the run; a missed save is a nonzero exit, never silent. Bare `--save` opens the source file directly and saves into it; `--save <path>` saves to that path. With `--detach`, saves at run end and leaves Studio open
- `--profile [dir]` — enable microprofiler auto-capture and collect dumps (optional output directory)
- `--sourcemap <path>` — path to sourcemap.json for instance resolution
- `--host <host>` / `--port <port>` — server address (port resolution: the flag, then `RODEO_PORT`, then 44872)
- `--no-output` — suppress all output
- `--no-print` / `--no-warn` / `--no-error` / `--no-info` — suppress specific log levels
- `--fflag.override <KEY=VALUE>` (repeatable) / `--fflag.file <path>` — FFlag overrides at launch
- `--ppid <pid>` — exit when this process dies
- `-- arg1 arg2` — script arguments (access via `require("@rodeo/process").args`)

### `rodeo state` / `rodeo kill <id>` / `rodeo save [id]`

`rodeo state` prints the canonical state as flat tables joined by the short
(8-char) studio id. Studios are split by origin — LOCAL (file launches, with
the source file the launch asked for and the working file Studio actually has
open) and UPLOADED (place-id launches) — followed by DOMS (one row per DOM,
with the player for client DOMs) and RUNS (each run joined to the DOM/studio
it executes on). Empty sections keep their header with a `-` placeholder row:

```
LOCAL
 ID        MODE  SOURCE_PATH      WORKING_PATH                    STATUS
 68298c7b  edit  ./MyGame.rbxl    .rodeo/.temp/rodeo-<uuid>.rbxl  connected

UPLOADED
 ID        MODE  PLACE           STATUS
 9aec44bb  test  Place1 (12345)  connected

DOMS
 ID        KIND    STUDIO    USER
 2a32ef67  edit    9aec44bb  -
 f37d718d  server  9aec44bb  -
 b8f11a11  client  9aec44bb  revvy02 (902015375)

RUNS
 ID            STATE    MODE  KIND    CONTEXT  DOM       STUDIO
 b0ec4d9a103b  running  test  client  client   b8f11a11  9aec44bb
```

`--json` emits the full snapshot (including `sourcePath`/`workingPath`).

`rodeo kill <id>` takes a run id or a Studio id. Unique prefixes work. A run
id kills the run. A Studio id closes that Studio, including detached ones, and
its active runs fail as disconnected. Ids change on every launch, so read them
from `rodeo state` instead of storing them.

`rodeo save [studio-id] [--out <path>]` saves a Studio's place. Omit the id to
target the only connected Studio; with several connected it errors and lists
them. It then writes the saved place to `--out`, or back to the launch's
SOURCE_PATH by default — so a plain `rodeo save` persists the live place into
the file you opened. A blank-place Studio with no `--out` keeps its working
file. You cannot save a manually-opened Studio this way, because it has no
rodeo session.

## Directives

A single-line comment that pre-fills `rodeo run` flags. It accepts every CLI flag, so a script declares its own runtime configuration:

```luau
-- @rodeo run --place ./game.rbxl --mode test --context client --save -- --user frank

local process = require("@rodeo/process")
print(process.args)  --> { "--user", "frank" }
```

Then run `rodeo run my-script.luau` with no flags. Everything after `--` becomes `process.args`.

The directive is the base configuration. The CLI overrides it per layer. A flag on the CLI replaces the directive's copy of that flag, except repeatable flags like `--fflag.override`, which accumulate. A `--` tail on the CLI replaces the directive's script args entirely.

## DOM Targeting: --mode / --context / --dom

Three independent flags. All are optional and have defaults. `--mode` picks the
Studio mode, `--dom` picks the DataModel to run on, and `--context` picks the
identity to run at.

- **mode** — `edit`, `run`, `test`, `play`
- **dom** — `edit`, `server`, `client`. Which DataModel. The edit DOM exists in
  every mode, so `--dom edit` reaches it even while a test or play session runs,
  and does not disturb that session. The DOM is the **communication boundary**.
  Code on the same DOM shares instances through BindableEvents. Code on
  different DOMs communicates through RemoteEvents.
- **context** — the **identity level**, not a script class:
  - `plugin` — plugin identity
  - `server` — the identity server-side code runs at when the game is running
  - `client` — the identity client-side code runs at when running (LocalScripts / `RunContext = Client`)
  - `elevated` — command-bar identity (via StudioMCP), for privileged APIs; works on any DOM
  - `cmdbar` — command-bar identity (via the BindableFunction bridge the launch bootstrap installs), no StudioMCP; edit DOM only, and only in a Studio rodeo launched

  Each context is an **independent Luau VM** on the DOM with its own global
  state. Contexts cannot read each other's Luau values, so they coordinate
  through DOM instances. A ModuleScript has no fixed context. It runs at
  whatever context requires it.

`mode` defaults to **edit**, and rodeo **never infers** it from `--context` or
`--dom`. So a server or client run must pass `--mode`: `--context server` alone
resolves to edit+server, which errors. `context` alone implies its DOM. `mode`
alone selects that mode's primary DOM at its native context.

### Common combinations

Read each row as (studio mode, which DOM, at which identity):

| Flags | Runs |
|-------|------|
| *(none)* | edit DOM, plugin identity (default) |
| `--context elevated` | edit DOM, command-bar identity (via StudioMCP) |
| `--context cmdbar` | edit DOM, command-bar identity (via the launch bootstrap; no StudioMCP) |
| `--mode run --context server` | run mode, server DOM, server identity |
| `--mode test --context server` | play test, server DOM, server identity |
| `--mode test --context client` | play test, client DOM, client identity |
| `--mode test --context plugin` | play test, server DOM, plugin identity |
| `--mode test --dom client --context plugin` | play test, client DOM, plugin identity |
| `--dom edit` | edit DOM, plugin identity — even while a test/play session runs (session preserved) |
| `--mode play --context server` | multiplayer test, server DOM, server identity |
| `--mode play --dom client` | multiplayer test, client DOM, client identity (starts one client; appends one more against a running session) |

Any combination that isn't a valid (mode, dom, context) triple errors at
submit — including a server/client `--context`/`--dom` with no `--mode` (mode
defaults to edit, and edit has only an edit DOM).

### Studio modes

| Mode | DOMs |
|------|-----|
| Edit | Edit DOM only |
| Run | Edit + server (F8) |
| Test | Edit + server + client (F5) |
| Play | Edit + server + N clients via `StudioTestService:ExecuteMultiplayerTestAsync` (one Studio; the engine caps multiplayer-test clients at 8) |

## `--reload-requires`

Instance requires (`require(game.ReplicatedStorage.Foo)`) use the require cache by default. This matches Roblox's own semantics: the require resolves to the **live module the running game already uses**. Mutate state in one run and the next run sees it. Inspect a running game's modules and you get its real state.

`--reload-requires` re-evaluates them instead, so the run gets its own fresh copies. Use it for test isolation, or to pick up edits you made to a DOM ModuleScript after it was first required. It temporarily adds and renames instances in the open place while the run is in flight.

Filesystem requires are fresh on every run either way, because the bundler inlines them.

Two consequences decide which mode you need:

- **To mutate live game state, use the default.** A script that drives the
  running game through its own modules — inserting into a jobs table, setting
  ECS components the real systems react to — must reach the live instances.
  Under `--reload-requires` those writes land in a fresh copy that the game
  never reads.
- **`--reload-requires` re-runs module top-level code, and modules with
  load-time side effects can hang or error under it** even though they load
  fine in the real game: `WaitForChild` on instances a server script creates at
  boot, HTTP calls in the require path, load-order assumptions. The failure
  reads as `Requested module experienced an error while loading` and names only
  the outermost require. Bisect it with `pcall(require, ...)` down the
  dependency chain, one level at a time, until you isolate the module.

## `@rodeo` API

Run `rodeo setup` once per project to generate types and `.luaurc`.

```lua
local rodeo = require("@rodeo")         -- full API
local fs = require("@rodeo/fs")         -- individual modules
local process = require("@rodeo/process")
local io = require("@rodeo/io")
local stream = require("@rodeo/stream")
local roblox = require("@rodeo/roblox")
```

### `@rodeo/fs` — file system (host-side, run-client cwd)

```lua
fs.exists(path) -> boolean
fs.stat(path) -> FileMetadata
fs.type(path) -> string
fs.open(path, mode?) -> StreamHandle
fs.remove(path)
fs.mkdir(path)
fs.rmdir(path)
fs.copy(src, dest)
fs.listdir(path) -> { DirectoryEntry }
```

### `@rodeo/process` — system processes

```lua
process.args        -- script arguments (from -- arg1 arg2)
process.env         -- environment variables
process.cwd()       -- current working directory
process.homedir()   -- home directory
process.execpath()  -- path to rodeo executable
process.platform()  -- "macos" | "windows" | ...
process.exit(code)

-- Blocking execution; ProcessResult is { ok, exitcode, out, err }
process.run(args, options?) -> ProcessResult
process.system(command, options?) -> ProcessResult
-- options: cwd, env (added to the inherited environment), input (string|buffer written to stdin)

-- Async execution with stdio piping
process.create(args, options?) -> ProcessHandle
process.kill(handle)
```

### `@rodeo/io` — stdin/stdout/stderr

```lua
io.stdin   -- StreamHandle
io.stdout  -- StreamHandle
io.stderr  -- StreamHandle
io.read()  -- read line from stdin
```

### `@rodeo/stream` — stream operations

```lua
stream.read(handle) -> string?        -- text; one-shot (~16MiB cap on files — use readBytes for big files)
stream.write(handle, data)            -- any size is safe
stream.readBytes(handle) -> buffer    -- any size is safe
stream.writeBytes(handle, data: buffer) -- any size is safe
stream.close(handle)
```

### `@rodeo/roblox` — models, data files, screenshots

```lua
roblox.importInstances(path) -> { Instance }   -- load .rbxm/.rbxmx as Instances
roblox.exportInstances(path, { instances })    -- write Instances to .rbxm/.rbxmx/.rbxl/.rbxlx
roblox.bake(path, value)                 -- write a table/value as a Luau module
roblox.captureViewport(output?, options?) -> (string, { width, height })  -- screenshot the viewport
roblox.exportEditableImage(path, image)   -- write an EditableImage as .png
roblox.importEditableImage(path) -> EditableImage  -- load a .png/.jpg as an EditableImage
roblox.exportEditableMesh(path, mesh) -> { string }  -- write an EditableMesh as .glb/.gltf/.obj; returns what the format dropped
roblox.importEditableMesh(path) -> EditableMesh    -- load a .glb/.gltf/.obj as an EditableMesh
roblox.importEditableScene(path, options?) -> EditableScene -- .glb/.gltf hierarchy, live resources, curves/morphs and optional playback rig
roblox.exportEditableScene(path, sceneOrRoots, options?) -> { string } -- portable motion or native clip/rig pairs
```

`bake` emits `return <value>` and writes Roblox types as constructors
(vectors, CFrames, colors, enums), so you can require the file straight back
into Studio. Instances and functions become their `tostring`. `bake` creates
parent directories as needed.

`importInstances` and `exportInstances` handle arbitrarily large models. The file extension
selects XML or binary output.

`exportEditableImage` and `importEditableImage` move pixels between PNG files
and `EditableImage` objects, which `exportInstances` cannot serialize (an Object-backed
image content is written as an empty reference). Export supports `.png` only;
import reads PNG and JPEG. Studio bounds EditableImage dimensions and the
import errors with the size if it refuses one.

`exportEditableMesh` and `importEditableMesh` do the same for `EditableMesh`
with glTF 2.0 (`.glb` binary, or `.gltf` with an embedded buffer) or Wavefront
OBJ (`.obj`). Faces must be triangles (`mesh:Triangulate()`). glTF: geometry,
per-corner normals/UVs/colors and skinning (bones with bind poses, up to four
influences per vertex) round-trip; FACS poses do not. Import bakes node
transforms into the geometry and merges all primitives into one mesh. glTF's
conventions are Roblox's, so nothing is converted. OBJ: positions, per-corner
UVs and normals; `vt` V is flipped (OBJ's UV origin is bottom-left), planar
polygons (including concave faces) are triangulated with their winding preserved,
`o`/`g` groups merge into one mesh, materials are
ignored. OBJ cannot carry vertex colors or skinning; `exportEditableMesh`
returns the list of what the format dropped (empty for glTF). Make a part with
`AssetService:CreateMeshPartAsync(Content.fromObject(mesh), opts)`.
OBJ numeric components must be finite; invalid values and polygons that cannot
be triangulated report the source line.

`importEditableScene` preserves the default glTF scene's hierarchy, separate
material primitives, compatible shared meshes, PBR textures and supported skins.
Returns `{ roots, meshes, images, sourceMap, warnings, animations, morphs, morphWeights, animationRig?, animator? }`. `roots` always contains one unparented Model named after the selected glTF scene;
all returned objects belong to the caller. Destroying roots does not destroy the
editable resources. `sourceMap.nodes`, `.images`, `.materials` and `.joints`
use original **zero-based glTF indices**; `.primitives` is a one-based binding
array with `nodeIndex`, `meshIndex`, `primitiveIndex`, `skinIndex`, `materialIndex`,
`mesh` and `part`. Joint bindings expose the numeric EditableMesh `boneId` and
renderable `Bone`, so names need not be unique in the source.

`exportEditableScene(path, sceneOrRoots, { strict = true }?)` accepts imported or procedural instance trees:
Models/Folders, MeshParts, block/ball/cylinder Parts, WedgeParts,
CornerWedgeParts, primitive legacy meshes, Attachments, and supported Bone rigs.
Curved primitives use 24 segments (balls: 16 latitude intervals). Native
ball/cylinder sizing and legacy Scale/Offset are respected. Legacy
FileMesh/Head/Torso meshes and reflected legacy scales require baking.
Neon emission and Glass/ForceField transmission use glTF material factors and
report approximation warnings. Scalar factors survive import/export; emissive
textures remain unsupported. Unposed skins without Bone instances can export
with MeshPart sizing; scaled live Bone poses still require baking.

Native Roblox clips can be exported with an explicit rig pairing:

```luau
roblox.exportEditableScene("walk.glb", {rig}, {
    animationClips = {{rig = rig, clip = keyframeSequenceOrCurveAnimation}},
    animationSampleRate = 60, -- optional; Hz, maximum 1000
})
```

An isolated clone evaluates each clip through Roblox's Animator. Motor6Ds,
kinematic AnimationConstraints and Bones become glTF joint hierarchies; welds
carry rigid attachments. This bakes **full rig poses**, including unkeyed joints,
so exported clips are standalone poses rather than additive/partial layers.
Source key times are retained, with uniform samples between them; interpolation
between samples approximates native easing and always reports a warning.
`strict = true` therefore rejects native baking. Use portable `scene.animations`
for exact glTF curve round trips; it cannot be combined with `animationClips`.
Rigs/clips must be Archivable. Non-kinematic constraints, scaled rig frames,
cycles, multiple joints driving one part, incomplete live skins, and facial
NumberPose tracks error.
Zero-duration clips export a static pose; empty clips error. The source rig,
its current pose, and its Animator tracks are not modified. Clip loop, priority,
named keyframes and markers survive in `SceneAnimation.metadata` and glTF
animation `extras.rodeo`. Markers are portable metadata, not restored native
event tracks. Clip registration is local to Studio; no upload is performed.

`imageSources` in export options maps an `EditableImage` (or a URI `Content`)
to an original host image path. This replaces preview pixels during export and
avoids EditableImage's 1024-pixel limit; paths are relative to the caller's cwd.
For example: `{ imageSources = { [previewImage] = "textures/original.png" } }`.
Exports current poses, sizes, meshes and readable textures, embedding resources
in both file formats. Shear, scaled posed skins, unreadable resources and unsupported
part shapes error. Cameras, other material extensions, occlusion/emissive textures
and sampler differences are unsupported and reported;
unsupported required glTF extensions error. Native Roblox materials use scalar PBR baselines;
emissive/transmission behavior warns. Untextured imports have no SurfaceAppearance
or images: source factors survive as attributes, and approximate native previews
are reported. `strict = true` on import/export aborts on warnings, cleans temporary
objects, and preserves the destination. Repeated warnings include a count.
Attachments carry explicit glTF metadata; arbitrary empty nodes remain Models.
Source scale attributes survive cloning; animation/morph references still need
retargeting to cloned instances. No publishing is performed.

```luau
local scene = roblox.importEditableScene("vehicle.glb")
for _, root in scene.roots do root.Parent = workspace end
-- Edit scene.meshes, scene.images, or the instance hierarchy.
local warnings = roblox.exportEditableScene("edited.glb", scene)
for _, root in scene.roots do root:Destroy() end
for _, mesh in scene.meshes do mesh:Destroy() end
for _, image in scene.images do image:Destroy() end
```

Scene motion is portable data in the returned result. Pass the **full scene** to
export to preserve it; roots-only export captures the static pose and warns when
motion data is omitted. `animations` contains named clips with a zero-based
`sourceIndex` and channels (`node: Instance`, `path`, `interpolation`, `times`,
`values`). Paths are translation, rotation, scale, weights; interpolation is
STEP, LINEAR, CUBICSPLINE. Values are flat XYZ/XYZW/scalars; cubic keys contain
incoming tangent, value, outgoing tangent, with tangents per second. Curves are
not resampled for glTF export. Edit these channels for portable changes; editing
a generated CurveAnimation does not change them.

`morphs` contains `{ mesh, targets, tangents? }`. Each target has an optional
name and optional position/normal/tangent delta maps. Positions and tangents
are keyed by stable EditableMesh vertex IDs; normals use normal IDs. An omitted
attribute means all-zero deltas; a supplied map must cover every referenced ID.
Base tangents are XYZW arrays keyed by vertex ID. `morphWeights` contains
`{ node, part, weights }` per primitive instance. Meshes remain the shared,
unposed base even when instances have different initial weights. Mesh edits
modify that base directly; export does not subtract hidden pose offsets.
Update delta maps after topology edits; absent mappings error. Generic morph
weights remain editable data and are not automatically rendered by Animator.

Every import has the same scene Model container. Translation/rotation clips
build a rig inside it with Motor6Ds/Bones, an Animator, and `clip: CurveAnimation`
plus `animation: Animation` on convertible clips. IDs are temporary Studio IDs;
publishing remains separate. Parent `scene.roots[1]` to workspace, then load
`clip.animation` through `scene.animator`. Wait for the track's Length before
seeking. STEP/LINEAR keys and cubic translation tangents are preserved directly
where the joint-frame conversion permits it. Quaternion cubic/composed motion
uses adaptive linear curves and reports that approximation; portable channels
stay exact. Native timestamps use Studio's clock and quantization warns;
colliding keys and Attachment-targeted animation omit the native clip and warn.
Independent clips omit unrelated joints. Scale and generic morph
channels remain portable data and warn. `animationRig` and `animationSampleRate`
options were removed. Generated helpers and scene containers are omitted as
nodes on export, so repeated round trips do not accumulate wrappers.

`captureViewport` treats `output` as an exact file path when it ends in `.png`.
Otherwise it treats it as a directory for the auto-named file, and defaults to
`.rodeo/.temp/captures/`. All `options` are optional: `cframe` (scripted camera
for the shot, restored afterward), `fov`, `focus`, `settle` (seconds to wait
before capturing), `device` (a Studio device-simulator preset id such as
`"iphone_13"` or `"hd_1080"`; layout, insets and orientation come from the
preset), and `viewportSize` (a `Vector2`, the `Camera.ViewportSize` to capture
at, at most 7680 by 4320; with `device` it overrides the preset's resolution),
and `resample` (default `true`: the image is exactly the viewport so UI offsets
map 1:1 onto pixels; `false` writes the engine's frame at its rendered size, the
viewport times the display scale, or whatever a script-driven device simulator
set; that is how to get the engine's largest frame, 23466x13200 on a 2x display).
Simulator overrides, applied for the shot and restored after like the camera
fields, each needing `device` or `viewportSize`: `scalingMode` (`"ActualResolution"`
default, `"ScaleToPhysicalSize"` = host DPI over `pixelDensity`; `"FitToWindow"` is
refused, it renders at window size), a phone/tablet preset renders its full
resolution so the image is larger than its inset viewport,
`pixelDensity` (DPI 72 to 10000; 72 on a 2x display is 3.06x), `orientation`
(`"Portrait"`, `"LandscapeLeft"`, `"LandscapeRight"`; phone/tablet forms only) and
`deviceForm` (`"Desktop"` default, `"Phone"`, `"Tablet"`, `"Console"`, `"VR"` for the
custom viewportSize device). Bad names error naming the option; the engine's own
limits propagate. Presets: consoles `xbox`, `ps4`, `ps5`, `android_tv_1080`; desktops `average_laptop`, `hd_720`, `hd_1080`, `vga`; handhelds `generic_handheld_720`, `generic_handheld_1080`; VR `meta_quest_2`, `meta_quest_3`; phones `iphone_6_Plus`, `iphone_7`, `iphone_XR`, `iphone_11`, `iphone_13`, `iphone_13_pro`, `iphone_13_pro_max`, `iphone_14`, `iphone_16`, `iphone_16_pro`, `iphone_16_pro_max`, `iphone_17_pro`, `samsung_galaxy_a06`, `samsung_galaxy_a16`, `samsung_galaxy_s22_ultra`, `samsung_galaxy_s25_ultra`; tablets `ipad_6th_generation`, `ipad_8th_generation`, `ipad_9th_generation`, `ipad_10th_generation`, `ipad_a16`, `ipad_air_5th_generation`, `ipad_pro_M4_11in`, `ipad_pro_M5_13in`, `xiaomi_redmi_pad_se`, `amazon_fire_hd10_2023`, `samsung_galaxy_tab_a8`, `samsung_galaxy_tab_a9`, `samsung_galaxy_tab_a9+`, `samsung_galaxy_tab_S11`.

The written image is always exactly the capture's `Camera.ViewportSize`, so
UI offsets map 1:1 onto pixels and the size is the same on every machine;
the engine's larger high-DPI frame is resampled down. For a sharper or larger
image, raise `viewportSize`. `device`/`viewportSize` drive Studio's device
simulator for the shot and restore it afterward, like the camera fields; the
second return value is `{ width, height }`. A frame captured before the new
viewport rendered is reported as an error (raise `settle`), never retried.

`captureViewport` needs a viewport, so use plugin context, or client context in a
running session. Server context errors. The frame is taken from the file the
engine writes for every capture, snapshotting the directory first and erroring
on ambiguity rather than guessing, so the simulator's full 7680 by 4320 works
on every display. A frame the engine did not render is refused: a solo
play-test session (`--mode test`) captures black on macOS (issue #17), so
capture from a multiplayer session (`--mode play`) or in edit mode. Limits are
Studio's own and surface as errors: the device simulator accepts at most 7680
by 4320. The capture then waits for the engine with no deadline (a large frame
or a slow GPU can take well over 10s; the largest frame takes about 7s). One
case never completes and waits until the run is killed: a minimized Studio on
Windows (background launches are minimized there; launch with `--focus` or
restore the window first).

### `@lune` adapters — run lune-flavored code unchanged

rodeo ships adapters that map lune's std lib onto its own runtime, so scripts
(and dependencies) written for lune run inside Studio with rodeo's host doing
the I/O:

```lua
require("@lune/fs")       -- readFile/writeFile/isFile/isDir/readDir/remove*/writeDir/copy/move/metadata
require("@lune/process")  -- args, env, cwd, exit, os, exec (no create)
require("@lune/serde")    -- encode/decode
require("@lune/stdio")    -- write/ewrite
require("@lune/task")     -- Roblox task, wait/delay clamped to lune's out-of-range handling
```

This also solves the wally/roblox-target package wall, where instance-path
requires cannot bundle (issue #6): **pesde packages published with a `lune`
target work under rodeo bundling as-is**, because their `@lune/*` imports
resolve through these adapters. Prefer lune-target dependencies for run
scripts.

`@lute/*` adapters exist too, for lute-flavored code:

```lua
require("@lute/fs")       -- handle-based open/read/write/close, readIntoBuffer, stat (Duration timestamps), recursive copy/move/rmdir; link/symlink/watch error
require("@lute/io")       -- write(...)/read
require("@lute/process")  -- args, env, cwd, homedir, execPath, run, system, exit; pid/onSignal error
require("@lute/task")     -- Roblox task + resume/deferSelf; wait/delay accept @lute/time Durations
require("@lute/time")     -- Instant/Duration with full arithmetic and comparisons (pure Luau)
```

## Common patterns

```bash
# One-shot in a fresh place
rodeo run --place --show-return --source "return game.Workspace:GetChildren()"

# Against a published place
rodeo run --place 1234567890 --source "print(game.PlaceId)"

# Big data out of Studio (size-unbounded)
rodeo run --return dump.luau --source "return game.Workspace:GetDescendants()"
rodeo run --return data.json --source "return bigTable"

# Script arguments
rodeo run script.luau -- arg1 arg2
# In script: local args = require("@rodeo/process").args

# Multiplayer test: start the session with a client, then run on the server.
# Append more clients to a running session with the same --mode play --dom client.
rodeo run --place --mode play --dom client --show-return --source "return game.Players.LocalPlayer.UserId"
rodeo run --mode play --context server --source "print(#game.Players:GetPlayers())"

# Profiling
rodeo run --place --profile ./profiles --mode play --context server perf-script.luau

# Suppress logs
rodeo run --no-output script.luau    # suppress all
rodeo run --no-print script.luau     # suppress print() only

# .rodeo/ shorthand
rodeo run myscript                   # runs .rodeo/myscript.luau

# Work against a local place, then persist it
rodeo run --place MyGame.rbxl --detach --source "..."  # edits land in a temp copy
rodeo save <studio-id>                                 # commit the copy back to MyGame.rbxl
rodeo kill <studio-id>                                 # close the Studio
```

## Verifying game behavior

Patterns that pay off when you use rodeo to reproduce a bug and prove a fix:

- **Capture the bug first, then the fix.** Build the place from the unfixed
  source, drive the repro, screenshot. Rebuild with the fix, run the same
  driver script, screenshot again. Two captures with identical steps are much
  stronger evidence than a passing assertion, and the first one proves your
  repro actually reproduces the bug.
- **Rebuild before every run.** The place file is a snapshot. Run your build
  task after any source edit, before launching. Testing a stale build silently
  verifies old code.
- **The server drives, the client observes.** `roblox.captureViewport` needs a
  viewport, so a `--context server` run cannot screenshot. Split the work: one
  `--context server` run mutates game state, a parallel `--context client` run
  waits and captures. Coordinate the stages through workspace attributes. The
  server calls `workspace:SetAttribute("TEST_STAGE", n)` and the client polls
  it, since attributes replicate from server to client immediately.
- **Drive state through the game's own write paths.** To simulate a gameplay
  event, find the exact `world:set` or module call the real flow performs and
  call that, rather than approximating the effect. Replication and downstream
  systems then behave as they do in production.
- **Pump batched networking yourself.** Libraries with a manual event loop
  (such as zap's `manual_event_loop`) are flushed by a game script that only
  services the main VM's copy of the module. A run gets its own VM, so its
  `Fire` calls sit in a buffer forever. Call the library's send function
  yourself afterward. The same applies to anything a game-script loop flushes.
- **Filter engine noise.** Local sessions print `Failed to load sound/asset ...
  not authorized` for team-owned assets. Pipe run output through a filter, or
  write it with `--output` and grep the file. Do not mistake it for the bug.

## Gotchas

- `--mode run|test|play` when the studio isn't already in that mode → auto-transitions
- `--context elevated` requires Studio's AI assistant / StudioMCP to be available — rodeo bridges to elevated identity through it
- **`--context elevated` can hang ~10s and fail if the Studio never connected to StudioMCP** — the Assistant plugin only opens its MCP socket when the Assistant panel is opened, and `mcp-server.enabled=true` alone doesn't guarantee it (github.com/revvy02/rodeo issue #4). Recovery: open the AI Assistant panel in that Studio, or use `--context cmdbar` for edit-DOM work
- `--context cmdbar` needs a Studio that rodeo launched (`--place`, or a serve-launched Studio): the launch bootstrap installs its bridge. A hand-opened Studio fails immediately with a message saying so
- Luau hotcomments work, including via `--source` — put `--!native` at the top of a script for a large speedup on numeric code
- `rodeo setup` must be run once per project for `@rodeo/*` imports
- Return values >2MiB without a `--return` file fail the run by design — pass a file path for big payloads
- `--place` always opens its own fresh place, even when another place is already open on the serve — runs never silently land in a resident place
- **`--place file.rbxl` without `--save` opens a temp COPY** (the WORKING_PATH in `rodeo state`) — in-Studio edits and manual Cmd+S land in the copy, which is deleted when the Studio closes. Persist with `rodeo save <studio-id>` (commits back to SOURCE_PATH) before closing, or launch with `--save` to open the source directly
- `stream.read` on a file handle is a single read and fails on very large files (~16MiB); the run survives — use `fs.open` + `stream.readBytes`, which handles any size
- A killed/disconnected run exits 2 with `rodeo: run disconnected: <reason>` on stderr; an explicit `rodeo kill` exits 1

`roblox.import`, `roblox.export` and `roblox.capture` are deprecated aliases of
`importInstances`, `exportInstances` and `captureViewport`. They still work,
warn once per run, and will be removed in 2.0.

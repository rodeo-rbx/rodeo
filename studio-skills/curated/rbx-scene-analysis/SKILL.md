---
name: rbx-scene-analysis
description: "Analyze a Roblox scene with SceneAnalysisService: rendering cost, memory, instance composition and unparented instances. Use when investigating performance, memory or leaks."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

`SceneAnalysisService` only works at **runtime**, so run every query against a play DM: `rodeo run --mode test --context client` for rendering/triangle queries (view-dependent), `--mode test --context server` for memory/unparented/leak queries (server-side leaks are common). `--mode test` starts the playtest itself; over the Studio MCP, call `start_stop_play` first. `commands/` holds the four analysis routines; `QuerySpecs.md` documents the return shapes.

# Scene Analysis

Uses `SceneAnalysisService` to analyze and optimize Roblox game scenes. All commands follow the same cycle:

1. Start play mode (required — `SceneAnalysisService` only works at runtime)
2. Run queries via `SceneAnalysisService`
3. Stop play mode
4. Analyze results and present findings or make edits

## When to Use

- User asks to optimize a scene, check performance, trace memory, or find leaks
- Keywords: `performance`, `memory`, `draw calls`, `triangles`, `scene health`, `unparented`, `leak`, `optimize`, `SceneAnalysisService`

## Commands

| Command | What it does |
|---------|-------------|
| `/scene-health` | Full overview — runs all 6 queries, summarizes rendering, memory, instances, and unparented objects |
| `/optimize-rendering` | Deep rendering analysis — auto-finds geometry hotspots, sweeps camera at player height, measures triangles and draw calls per view, places hotspot markers in the scene |
| `/optimize-memory` | Memory analysis — script VM memory, animation clip memory, audio asset memory with reference counts |
| `/fix-leaks` | Traces unparented instances to host scripts, reads the source, identifies cleanup patterns, offers to apply fixes |

## SceneAnalysisService queries

| Query | Returns |
|-------|---------|
| `GetInstanceCompositionAsync()` | Instance counts by category and class |
| `GetTriangleCompositionAsync()` | Triangle and draw call counts by render pass (view-dependent) |
| `GetScriptMemoryAsync()` | Per-script Luau VM heap memory |
| `GetUnparentedInstancesAsync()` | Instances held in memory but not in the DataModel, traced to host scripts |
| `GetAnimationMemoryAsync()` | Loaded animation clip memory with owner Animators |
| `GetAudioMemoryAsync()` | Loaded audio asset memory with owner Sound/AudioPlayer instances |

See `QuerySpecs.md` in this skill for full data shape documentation.

## Key facts

- **Play mode is required.** `SceneAnalysisService` queries runtime data (GPU render passes, Luau VM heap, loaded assets). It doesn't work in edit mode.
- **Triangle composition is view-dependent.** A single measurement only represents one viewpoint — `/optimize-rendering` handles this with an automatic hotspot sweep.
- **Set `CameraType = Scriptable`** before programmatically moving the camera, or the PlayerModule will override it each frame.
- **Shadows should be excluded** from triangle/draw call budgets — engine-driven, not creator-controllable.
- **Animation and audio assets are reference-counted.** Memory can only be freed by removing ALL references.
- **Server mode** — running memory and unparented instance queries from a server session can surface server-side issues (server-side animation leaks are especially common).

## Tone

Frame findings as opportunities, not problems. Be helpful and collaborative. Avoid language like "CRITICAL", "WARNING", "bloat", "problem", "offender", "over budget", "failing". Don't assign grades or severity labels. Present data clearly and let the creator decide what to act on.

## Reference points (rendering)

~800k triangles and ~600 draw calls is roughly the range where most devices can maintain 60fps. Present these as context, not pass/fail criteria — some creators target higher fidelity and accept lower framerates. Ask about framerate/device target before making strong recommendations.

Internally, each draw call costs roughly the equivalent of 20,000 triangles in GPU overhead — use this to prioritize draw call reduction in recommendations, but don't show the ratio to the creator.

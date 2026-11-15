# studio-skills

A skill-creation workspace: generate the **Roblox Studio Assistant's built-in skills**
from the shipped `Assistant.rbxm`, then repackage them for agents that drive Studio
through rodeo or the Studio MCP.

- `raw/`      — generated, pristine extraction (don't edit)
- `overlays/` — rodeo's own notes, one file per skill (this is what you edit)
- `curated/`  — generated from the two above (what you actually ship; don't edit)

The skills ship as `StringValue`s inside the Assistant plugin, zstd-compressed in the
rbxm — **not** fetched from any endpoint. The MCP `skill` tool only surfaces a
server-chosen subset under `rbx-` names (13 as of Studio 0.747; its entry in
`STUDIO_MCP_TOOLS.md` lists them); `extract.luau` dumps **all** of them.

## Workflow

1. **Extract** the pristine reference:
   ```bash
   lune run extract.luau          # Assistant.rbxm -> raw/   (all 23, verbatim)
   ```
   Needs Lune 0.10.5 or newer: older Lune's rbxm reader rejects the `Tags` property
   as current Studio serializes it (`mise install` on this repo gets the latest).
2. **Build** the curated skills:
   ```bash
   lune run build.luau            # raw/ + overlays/ -> curated/
   ```
   Each curated skill is its raw skill with:
   - an `rbx-` prefix on its name, unless it has one already (`scene-analysis` becomes
     `rbx-scene-analysis`), so the names match the ones Roblox's skills use for each other,
   - `overlays/_header.md` above Roblox's text: the tool-name mapping every skill needs,
   - the description and notes from `overlays/<name>.md`, if the skill has one,
   - the `{ToolNames.X}` placeholders, which the Assistant fills in as it loads a skill,
     replaced with the Studio MCP tool names,
   - `SKILL-combined.md` left out (just `SKILL.md` + commands/specs concatenated).
3. **Change a skill** by writing `overlays/<name>.md` (`<name>` is the curated name,
   such as `rbx-debug`) and building again.

   Open the file with a frontmatter block to replace Roblox's description:
   ```markdown
   ---
   description: One or two plain sentences, written out on this one line.
   ---
   ```
   Roblox writes its descriptions for the Assistant, and they run up to 925 characters.
   An agent's skill listing has a size budget, and Claude Code drops descriptions to
   stay inside it, so keep these short: the build points out any over 200 characters.
   Write the text as it should read. The build quotes it for YAML.

   The rest of the file is notes. Use them to:
   - **correct anything that assumes the in-Studio Assistant's context** — the base
     skills assume the Assistant (already elevated, privileged debugger channel), so
     claims like "plugin-level security, usable by plugins" mislead when driving via
     rodeo / the Studio MCP,
   - say how the skill runs under rodeo, and fold in anything verified by testing.

   The notes go under the header. To put a note inside Roblox's text instead, start it
   with an anchor line:
   ```markdown
   <!-- after: ### OnStopped Late-Set Behavior -->
   > **External caveat:** ...
   ```
   The paragraphs after the anchor go after the line of the raw `SKILL.md` that starts
   with that text. It has to match exactly one line.
4. **Deploy** a curated skill where an agent will load it:
   ```bash
   cp -r curated/rbx-debug ../../../.claude/skills/rbx-debug   # e.g. the game repo
   ```

After a Studio update, run steps 1 and 2 again. Roblox's changes flow into `curated/`
on their own, and your notes stay in `overlays/`. The build stops, leaving the last
`curated/` in place, when an anchor line is gone or an overlay's skill no longer
exists: fix the overlay and build again.

## The 23 skills

| Skill | What it teaches | Engine API |
|-------|-----------------|------------|
| `rbx-debug` | breakpoints, `OnStopped`, thread/stack/variable inspection | `ScriptDebuggerService` |
| `scene-analysis` | triangles/draw calls, VM memory, unparented/leak tracing | `SceneAnalysisService` |
| `virtual-input` | click/type/key/scroll/camera on a live game | `VirtualInputManager` |
| `device-simulator` | test UI across device form factors | `StudioDeviceSimulatorService` |
| `docs-search` | look up create.roblox.com API docs | `http_get` |
| `open-cloud-usage` | Open Cloud calls through `open_cloud_invoke` + endpoint discovery | Open Cloud |
| `convert-to-streaming` | convert a place to streaming, fix anti-patterns | streaming |
| `luau-heap-profiling` | heap snapshots, allocation diffs, leak hunting | `HeapProfilerService` |
| `perf-profiling` | MicroProfiler CPU/GPU frame analysis via the LibMP Luau module | MicroProfiler / LibMP |
| `perf-profiling-ref` | reference variant of `perf-profiling` (same description, different body) | MicroProfiler / LibMP |
| `unit-test` | write, run, debug Luau unit tests for ModuleScripts | test runner |
| `unit-test-testservice` | `unit-test` variant that runs tests through TestService | `TestService` |
| `instrument-analytics` | audit and add Economy / Funnel / Custom events | `AnalyticsService` |
| `configs-experimentation` | live-tunable values and experiments instead of constants or DataStores | `ConfigService` |
| `process-receipt-misuse` | audit Developer Product receipt handling for ProcessReceipt misuse | `MarketplaceService` |
| `create-skill` | author or modify a custom Assistant skill | none |
| `input-action-system` | set up input on the Input Action System, or migrate to it from ContextActionService / UserInputService | `InputAction` / `InputBinding` / `InputContext` |
| `server-authority` | audit, convert, and write Server Authority scripts for client-side prediction | `RunService:BindToSimulation` |
| `network-simulator` | simulate latency, jitter, and packet loss in a playtest; measure ping and responsiveness | `settings().Network` |
| `import-asset` | bring a model from Blender, Maya, etc. into the place through an Open Cloud upload | `open_cloud_invoke` |
| `permissions` | work out, probe, and ask for the MCP permissions a task needs | MCP Permissions |
| `rbx-search-asset` | how to call the `search_asset` tool | `search_asset` |
| `rbx-generate-procedural-model` | how to call the `generate_procedural_model` tool | `generate_procedural_model` |

`raw/` and `curated/` content is **Roblox's**, extracted from the local install.
`extract.luau`, `build.luau` and `overlays/` are original.

`curated/` holds 20 of the 23. `build.luau` leaves out `create-skill` (it authors skills
in the Assistant's own skill store) and the two variants, `perf-profiling-ref` and
`unit-test-testservice`, whose descriptions all but repeat their siblings'.

Eighteen have a short description from their overlay. `rbx-convert-to-streaming` and
`rbx-server-authority` keep Roblox's, which are under 100 characters already.

Six have notes of their own on running them with rodeo: `rbx-debug`, `rbx-scene-analysis`,
`rbx-virtual-input`, `rbx-device-simulator`, `rbx-docs-search` and `rbx-open-cloud-usage`
(whose current `open_cloud_invoke` flow is untested). `rbx-import-asset`, `rbx-search-asset`
and `rbx-generate-procedural-model` only say that they need the Studio MCP. The other
eleven carry the shared header and no notes: they haven't been run with rodeo yet.

`server-authority`'s `code_samples/` are ModuleScripts, and the shipped rbxm has no
readable source for any script. `extract.luau` takes their text from that skill's
`SKILL-combined.md`, which quotes each one.

## STUDIO_MCP_TOOLS.md

Reference for the live `mcp__Roblox_Studio__*` tool surface (params + what each does),
generated from the StudioMCP proxy's `tools-cache.json`.

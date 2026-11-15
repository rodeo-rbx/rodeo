---
name: rbx-server-authority
description: Assists in debugging and designing Server Authority compliant scripts for Client Side Prediction.
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

# Server Authority Skill
This skill understands the Server Authority API and how to build scripts that work with client side prediction. It can review scripts for compliance with the core pillars of the Server Authority API, refactor non-compliant scripts, and author new server-authoritative gameplay.

## When to Use
- User mentions they want to convert their game to use Server Authority
- User mentions they are using Server Authority
- User asks for help debugging mispredictions or rollbacks

## How to Use This Skill
Pick the command that matches what the user wants, and read the listed files BEFORE acting. If none clearly applies, ask the user which one they want.

| User wants… | Run command | Read first |
|---|---|---|
| To find problems in an existing game | `server_authority_audit` | `api_reference/3_core_rules.md`, `api_reference/4_anti_patterns.md` |
| To fix / migrate a game to Server Authority | `server_authority_convert` | `api_reference/3_core_rules.md`, `api_reference/4_anti_patterns.md` |
| To build a new gameplay mechanic | `server_authority_create` | `api_reference/3_core_rules.md`, `code_samples/1_SimulationTemplate.luau` |
| To understand a concept or ask a question | `server_authority_explain` | `api_reference/1_concepts.md` |

## Glossary
- **Simulation**: the core gameplay logic that runs on BOTH client and server, bound via `RunService:BindToSimulation()`.
- **Predicted instance**: an instance the client simulates ahead of the server. Its SimulationAccess properties and attributes are rolled back on a misprediction. Whether a given instance is predicted is often runtime- and proximity-based (Automatic prediction) and cannot be fully determined from source — confirm at runtime with `RunService:GetPredictionStatus()`.
- **SimulationAccess property**: an engine property (for example `BasePart.CFrame`) that the prediction system synchronizes. Only these properties/methods may be read or written inside the simulation.
- **Attribute-backed state**: custom gameplay data stored in an Attribute on a predicted instance so it survives rollback. Raw Luau variables do NOT survive rollback.
- **Misprediction / rollback / resimulation**: when the server's state differs from the client's prediction, the client resets to the server state and re-runs the simulation.
- **InputAction**: the only client-authoritative input. It is replayed during resimulation. Use it instead of `UserInputService` / `ContextActionService` inside the simulation.

## Core Rules (summary)
Full detail in `api_reference/3_core_rules.md`; violation → fix table in `api_reference/4_anti_patterns.md`.
1. **Shared simulation** — core gameplay logic lives in a ModuleScript run on client AND server, updated only via `RunService:BindToSimulation()` (or `BasePart.Touched` / `InputAction.StateChanged`).
2. **Instance prediction** — only predicted instances are simulated. Skip any instance whose `RunService:GetPredictionStatus()` is `Enum.PredictionStatus.None`. Force prediction with `RunService:SetPredictionMode()`, called inside `task.defer()`.
3. **Synchronized data** — back all dynamic simulation state with Attributes on predicted instances. Never rely on raw Luau variables for state that affects the simulation.
4. **Input** — read player input only through an `InputAction` parented under a Player. Never use `UserInputService` / `ContextActionService` inside the simulation.

## Recommended Scripting Architecture
Recommended scripting architecture is to have your core game loop implemented in a single Module Script in a method called Simulate() that is passed into `RunService:BindToSimulation()` from a Start() method that is called on both Client and Server.
> See code_samples/1_SimulationTemplate.luau

Loading the Module script should be done with two child scripts below the Module Script. One with RunContext = Client, one with RunContext = Server. Each should simply be:

> ReplicatedStorage.Simulation.Client (Client Script)
```lua
require(script.Parent).Start()
```

> ReplicatedStorage.Simulation.Server (Server Script)
```lua
require(script.Parent).Start()
```

## Reference Material
Detailed API documentation lives in `api_reference/`:
- `api_reference/1_concepts.md` — core concepts of server authority and client side prediction
- `api_reference/2_spec.md` — the Server Authority API specification
- `api_reference/3_core_rules.md` — the core rules scripts must follow to be compliant
- `api_reference/4_anti_patterns.md` — common anti patterns and their fixes

Worked examples live in `code_samples/`, starting with `code_samples/1_SimulationTemplate.luau`.

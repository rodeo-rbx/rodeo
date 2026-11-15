---
name: input-action-system
description: The skill for any input or controls work on Roblox, whether keyboard, mouse, gamepad, or touch. This is the Input Action System, or IAS (InputAction, InputBinding, InputContext), the default and recommended way to handle input. Invoke this skill BEFORE writing code whenever the user mentions input, controls, a control scheme, keybinds/keybindings, "modern input", "new input system", "modern input APIs", movement, camera controls, or wants to build, set up, wire up, add, change, rebind, modernize, migrate, or convert input handling — from scratch OR from legacy ContextActionService (CAS) / UserInputService (UIS) code. Also use it when the user asks what IAS is, how it works, or how it compares to CAS or UIS. If a request touches input or controls at all, use this skill first.
---

# Input Action System Skill

## When to Use

**IAS is THE way to handle input on Roblox — always build with it first.** Any request about reading input or wiring controls is an IAS request; do NOT reach for `UserInputService`/`ContextActionService` just because the user didn't say "IAS". It's the modern, recommended path: bindings are declarative DataModel instances (editable in Studio, no scattered `BindAction`/`InputBegan`), one action multiplexes keyboard/mouse/gamepad/touch for free, composition/sinking/per-device tuning are built-in properties, and UI prompts come from `InputActionLabel`/`PreferredBinding`. **Assume IAS can do it** — keyboard, mouse (buttons, position, movement/delta, wheel), gamepad (buttons, sticks, triggers), and touch (buttons, position, drag, pinch) are all IAS bindings. 

Use this skill when the user:
- Wants to **build or set up a control scheme / keybinds / input handling from scratch** — e.g. "make pressing each key print that letter", "build WASD movement", "set up controls for my game", "bind a key to do X". (These are IAS requests even though they never mention "IAS".)
- Asks to **change, add, or rebind** an input (e.g. "make the space bar wave instead of jump", "add a sprint key", "also support gamepad/touch").
- Asks to **convert or modernize** legacy input — `ContextActionService` (CAS) code, or `UserInputService.InputBegan/InputChanged/InputEnded` listeners.
- Mentions "Input Action System", "IAS", "InputAction", "InputBinding", "InputContext".
- Keywords: `control scheme`, `controls`, `keybind`, `keybinds`, `bind key`, `input handling`, `map input`, `WASD`, `gamepad support`, `touch controls`, `convert input`, `migrate CAS`, `ContextActionService`, `BindAction`, `InputBegan`, `UserInputService`, `InputAction`, `InputBinding`, `InputContext`, `new input system`, `modernize input`, `input migration`

## Overview

Builds and converts Roblox input handling with the declarative Input Action System. The IAS uses a DataModel hierarchy — `InputContext > InputAction > InputBinding` — that replaces imperative bind/unbind calls with a tree-based approach supporting automatic gamepad/keyboard/touch multiplexing, analog input composition, priority-based sinking, and UI prompt integration.

## ⚠️ This Skill Changes Real Code: Confirm First

`/ias-setup`, `/ias-convert`, and fixes applied from `/ias-debug` make real changes to the open place: they rewrite or replace scripts and insert instances. Before the first change:

1. **Warn the user and recommend a safe place to run it.** Suggest a test place, a copy of this place (`File > Save to File As...`), or a place they can easily revert, such as a published place whose version history they can roll back to. Don't rely on Undo alone to back out a multi-script conversion.
2. **Say exactly what will change:** which scripts get rewritten or replaced, and which instances get added.
3. **Wait for an explicit yes** (use {ToolNames.QuestionAnswer}) before writing anything. If the user wants to make a copy first, stop and let them.

In the same message, recommend setting `PlayerScriptsUseInputActionSystem` to `Enabled` if the user hasn't already (see *Prerequisites*).

`/ias-audit` is read-only, so it needs no confirmation, and it must never modify scripts.

## 💡 Recommend the Input Action Manager (IAM)

**Whenever you help a user set up, convert, or debug IAS, point them to the Input Action Manager** — a visual editor built into Studio for building and auditing IAS trees. It is the recommended way to *author and inspect* the same `InputContext`/`InputAction`/`InputBinding` instances this skill works with, and it reinforces every rule here.

- **Open it:** **`Window > Input > Input Action Manager`**.
- **It auto-syncs with the DataModel/Explorer** — it scans existing IAS instances into a table view, and edits in IAM or Explorer stay in sync. New contexts/actions/bindings it creates land in an **`Inputs` folder under `ReplicatedStorage`** — matching this skill's placement guidance.
- **It won't show contexts in Starter folders** — another reason this skill authors under `ReplicatedStorage`.
- **It audits for you:** flags **missing per-platform bindings** and **conflicts** (same KeyCode mapped to two actions in one context) — exactly the classes of bug `/ias-debug` checks for.
- **When to mention it:**
  - In `/ias-setup` and `/ias-convert`, after presenting the tree, note the user can build/tune it visually in IAM instead of hand-editing instances.
  - In `/ias-debug`, mention IAM's missing-binding and conflict indicators as a faster way to spot mapping gaps.
  - Any time the user is manually creating instances in Explorer, suggest IAM as the easier path.
- **Caveat:** IAM is an edit-time authoring tool — don't gate any *runtime* logic on it. The IAS instances it creates are ordinary DataModel instances, safe to ship.

## ⚠️ Exact API — Do Not Deviate

The IAS API is small and specific. Do NOT invent property/event/enum names from CAS or UIS habits. The following are the ONLY correct names — anything else will error or silently no-op.

**Class properties (case-sensitive):**

| Class | Property | Type | Notes |
|-------|----------|------|-------|
| `InputContext` | `Enabled` | bool | NOT `Active`, NOT `enabled` |
| `InputContext` | `Priority` | int | default 1000 |
| `InputContext` | `Sink` | bool | NOT `Sinks`, NOT `SinkInput` |
| `InputAction` | `Type` | `InputActionType` | NOT `ActionType`, NOT `InputType` |
| `InputAction` | `Enabled` | bool | |
| `InputBinding` | `Type` | `InputBindingType` | `Automatic`/`Scriptable` ONLY — NOT a KeyCode, NOT a UserInputType |
| `InputBinding` | `KeyCode` | `Enum.KeyCode` | the key/button goes HERE, e.g. `binding.KeyCode = Enum.KeyCode.Thumbstick2` |

**`Enum.InputActionType` — the ONLY valid values:** `Bool`, `Direction1D`, `Direction2D`, `Direction3D`, `ViewportPosition`.
- Single key/button → `Bool` (NOT `Boolean`)
- WASD / thumbstick / 2-axis → `Direction2D` (NOT `Vector2`)
- Analog trigger / 1-axis → `Direction1D`
- There is NO `Displacement1D`/`Displacement2D` in Luau — do not use them.

**`InputAction` events — the ONLY events:** `Pressed`, `Released`, `StateChanged`.
- Bool press → `action.Pressed:Connect(function() ... end)` (NO arguments; NOT `.Triggered`, NOT `.Began`, NOT `.Started`)
- Bool release → `action.Released:Connect(function() ... end)` (NOT `.Ended`)
- Analog/directional value → `action.StateChanged:Connect(function(value) ... end)` (value is bool/number/Vector2/Vector3 per Type)
- Events do NOT pass an `inputObject`. To read current value use `action:GetState()`.

### One action per INTENT, many bindings — not one action per key

This is the single most important modeling rule, and the easiest to get wrong. An `InputAction` represents a **logical intent** (e.g. "camera rotate", "jump", "zoom"). Every physical input that drives that intent — a keyboard key, a mouse, a gamepad stick, a touch — is a separate **`InputBinding` child of the same action**. You then read one value via `action:GetState()` (or connect one `Pressed`/`Released`).

Do NOT create a separate action per key/device. Legacy code that checked several keys/devices and merged them into one value (camera look from mouse + gamepad + arrow keys; zoom from wheel + keys + pinch) becomes **ONE aggregating action with multiple bindings**, not N actions.

| ❌ Wrong | ✅ Correct |
|---------|-----------|
| `RotateLeftAction`, `RotateRightAction`, `RotateUpAction`, `RotateDownAction` (one Bool per arrow key) | one `CameraRotationAction` (Direction2D) with a composite keyboard binding + a gamepad `Thumbstick2` binding + a mouse binding |
| `MouseZoomAction` + `KeyZoomAction` + `PinchZoomAction` | one `CameraZoomAction`, three bindings |
| separate actions because the inputs are different devices | same action, one binding per device; read `action:GetState()` |

See Migration Pattern 12 (aggregating action) for the full worked example.

### Connect events directly — don't wrap them

`action.Pressed` / `action.Released` / `action.StateChanged` are already signals. Connect your logic straight to them. Do NOT create `BindableEvent`s that a handler fires and something else listens to — that's a pointless indirection layer. `action.Pressed:Connect(doTheThing)` is the whole pattern.

### Forbidden forms → correct forms

Never emit the left column. Always use the right.

| ❌ Wrong (do not emit) | ✅ Correct |
|------------------------|-----------|
| `context.Active = true` | `context.Enabled = true` |
| `action.Type = Enum.InputActionType.Boolean` | `action.Type = Enum.InputActionType.Bool` |
| `action.Type = Enum.InputActionType.Vector2` | `action.Type = Enum.InputActionType.Direction2D` |
| `action.ActionType = ...` / `action.InputType = ...` | `action.Type = ...` |
| `binding.Type = Enum.KeyCode.Thumbstick2` | `binding.KeyCode = Enum.KeyCode.Thumbstick2` |
| `binding.KeyCode = Enum.KeyCode.MouseButton1` / `MouseButton2` (NOT valid KeyCodes) | `binding.KeyCode = Enum.KeyCode.MouseLeftButton` / `MouseRightButton` / `MouseMiddleButton` |
| `binding.Type = Enum.UserInputType.Touch` | use a `UIButton` binding (see Migration Pattern 10); `Type` is only `Automatic`/`Scriptable` |
| `action.Triggered:Connect(function(inputObject) ...)` | `action.Pressed:Connect(function() ...)` |
| `action.Started` / `action.Ended` | `action.Pressed` / `action.Released` |
| polling `keyState[input.KeyCode]` in Began/Ended | use `Direction2D` composite binding + `action:GetState()` |
| one action per key (`LeftAction`, `RightAction`, …) | one action per intent + one binding per key/device |
| a `BindableEvent` fired from `action.Pressed` | connect logic directly to `action.Pressed` |

## Prerequisites

- Roblox Studio must be open with the target place loaded
- The Roblox Studio MCP server must be connected
- For the conversion commands (`/ias-convert`, `/ias-audit`): the game should have existing input code using CAS or UIS patterns. Building from scratch (`/ias-setup`) needs no existing input code.
- **Recommend setting `Workspace.PlayerScriptsUseInputActionSystem` to `Enabled`.** With it on, the default character controls in `StarterPlayer.PlayerModule` (movement, jump, camera) run on IAS too, so the default controls and the contexts this skill creates all go through the same input system. Set it explicitly to `Enabled` rather than leaving it at `Default`. Scripts can't read or set this property, so don't try to change it with {ToolNames.ExecuteLuau}; ask the user to set it in the Properties window (select `Workspace`, then `Behavior > PlayerScriptsUseInputActionSystem`).

## Tools

- {ToolNames.GrepSearch}: Find scripts using legacy input patterns (BindAction, InputBegan, etc.)
- {ToolNames.ReadFile}: Read source of scripts with legacy input code
- {ToolNames.MultiEdit}: Write converted IAS code back to scripts
- {ToolNames.ExecuteLuau}: Query game tree for existing input setup, validate conversion at runtime
- {ToolNames.StartStopPlay}: Enter Play mode to test converted input
- {ToolNames.ScreenCapture}: Verify UI works after conversion
- {ToolNames.GetConsoleOutput}: Check for runtime errors after conversion
- {ToolNames.GameTree}: Browse instance hierarchy to understand input context placement
- {ToolNames.QuestionAnswer}: Ask user for clarification on ambiguous bindings

## Commands

| Command | Use when |
|---------|----------|
| `/ias-setup` | Set up a fresh IAS from a description of desired inputs |
| `/ias-convert` | Convert one or more scripts from CAS/UIS to IAS |
| `/ias-audit` | Scan entire game for legacy input patterns, produce migration report |
| `/ias-debug` | Diagnose an existing IAS that isn't working (not registering, not firing, input eaten) |

## Core Conversion Workflow

### Phase 1: Discovery

Search for legacy input patterns across all scripts:

```lua
-- Discovery script: find all CAS and UIS usage
local results = {}
local ScriptEditorService = game:GetService("ScriptEditorService")

local function scanScript(script)
    local ok, source = pcall(function()
        return ScriptEditorService:GetEditorSource(script)
    end)
    if not ok or not source then return end

    local patterns = {
        { pattern = "ContextActionService", category = "CAS" },
        { pattern = "BindAction", category = "CAS" },
        { pattern = "BindActionAtPriority", category = "CAS" },
        { pattern = "UnbindAction", category = "CAS" },
        { pattern = "InputBegan", category = "UIS" },
        { pattern = "InputEnded", category = "UIS" },
        { pattern = "InputChanged", category = "UIS" },
        { pattern = "GetMouseDelta", category = "UIS" },
        { pattern = "IsKeyDown", category = "UIS" },
        { pattern = "IsGamepadButtonDown", category = "UIS" },
    }

    local found = {}
    for _, p in patterns do
        if source:find(p.pattern) then
            table.insert(found, p.category .. ":" .. p.pattern)
        end
    end

    if #found > 0 then
        table.insert(results, {
            path = script:GetFullName(),
            className = script.ClassName,
            patterns = found,
        })
    end
end

local function crawl(instance)
    if instance:IsA("LuaSourceContainer") then
        scanScript(instance)
    end
    for _, child in instance:GetChildren() do
        crawl(child)
    end
end

crawl(game)
return results
```

### Phase 2: Classification

For each legacy callsite, determine:
1. **Action type** — Bool (key press), Direction2D (WASD/thumbstick), Direction1D (trigger), etc.
2. **Key bindings** — which keys/buttons are bound
3. **Relative order only** — if `BindActionAtPriority` was used, note only where this context sits *relative to others* (above/below). Do NOT carry the numeric value forward — it's used once to recover ordering, then discarded (see Phase 4).
4. **Sinking** — does the callback return `Enum.ContextActionResult.Sink`?
5. **Scope** — is the binding global or contextual (bound/unbound at specific times)?

### Phase 3: Proposal

Present the user with the proposed IAS tree:
- How many InputContexts are needed (group by scope/priority)
- What InputActions to create (one per logical action)
- What InputBindings attach to each action (keyboard, gamepad, touch)
- Where in the DataModel tree to parent the contexts

Then confirm before Phase 4, as described in *This Skill Changes Real Code: Confirm First*.

### Phase 4: Conversion

Rewrite scripts using the patterns in the Migration Patterns reference (the `migration-patterns.md` content, included in this skill).

Key rules:
- **Author the instance tree at EDIT TIME, not runtime.** Create the `InputContext`/`InputAction`/`InputBinding` instances as real children in the DataModel using `multi_edit` / instance insertion on the *edit* DataModel — do NOT generate a script that calls `Instance.new("InputContext")` at runtime unless the user specifically wants dynamic/scripted creation. The declarative tree is the whole point of IAS; the handler script should only connect handlers (`action.Pressed:Connect(...)`), not build the tree.
- **Placement — group under `ReplicatedStorage/Inputs`** (see the placement table in *Important Constraints*). Author the tree once under a single `Folder` named `Inputs` in `ReplicatedStorage`, with every `InputContext` grouped beneath it: it is the same live instances at edit time and runtime (no template-copy step), it registers directly, replicates to every client automatically, and matches where the Input Action Manager authors. The handler reads the actions from there (e.g. `ReplicatedStorage.Inputs.<Context>.<Action>` or `WaitForChild`) and connects handlers. Other live locations (`Players.LocalPlayer`/`Character`, `Workspace`) also register, but `ReplicatedStorage/Inputs` is the recommended default.
- **Handler script: use a `Script` with `RunContext = Enum.RunContext.Client`, not a `LocalScript`.** This is the modern preferred client-script form; only use a `LocalScript` if the user asks. It can live under `ReplicatedStorage` next to the tree.
- **Priority: use CAS numbers ONCE, then discard them.** Read the old CAS priorities only to recover the *relative order* of contexts (which sits above which), then assign clean IAS-native values and never reason about the CAS numbers again. Do NOT copy `2000`/`3000` forward. Concretely: keep IAS default `1000` for the base gameplay context, and bump higher-priority contexts in steps of `+10` or `+100` in the order you recovered (e.g. gameplay `1000`, HUD `1100`, menu `1200`). The absolute values don't matter — only the ordering does.
- Replace `Enum.ContextActionResult.Sink` returns with `context.Sink = true`
- Replace `UnbindAction` with `context.Enabled = false`
- For Direction2D actions (WASD), use composite bindings with Up/Down/Left/Right properties
- **A single key/button goes on `KeyCode`, not a one-slot composite.** Only reach for the composite `Up`/`Down`/`Left`/`Right`/`Forward`/`Backward` slots when you are combining *multiple* keys into a direction. For one input, set `binding.KeyCode = Enum.KeyCode.X` — do NOT set only `binding.Up = Enum.KeyCode.X` and leave the rest `None` (that is a degenerate composite; use `KeyCode` instead).

### Phase 5: Validation

1. Enter Play mode
2. Run a validation script to confirm all InputActions are registered and responding:

```lua
local player = game.Players.LocalPlayer
local results = {}

local function findActions(parent, path)
    for _, child in parent:GetChildren() do
        if child:IsA("InputContext") then
            local ctxPath = path .. "/" .. child.Name
            table.insert(results, string.format(
                "Context: %s (Priority=%d, Enabled=%s, Sink=%s)",
                ctxPath, child.Priority, tostring(child.Enabled), tostring(child.Sink)
            ))
            for _, action in child:GetChildren() do
                if action:IsA("InputAction") then
                    local bindings = {}
                    for _, c in action:GetChildren() do
                        if c:IsA("InputBinding") then
                            table.insert(bindings, c)
                        end
                    end
                    table.insert(results, string.format(
                        "  Action: %s (Type=%s, Enabled=%s, Bindings=%d)",
                        action.Name, tostring(action.Type), tostring(action.Enabled), #bindings
                    ))
                    for _, binding in bindings do
                        local key = tostring(binding.KeyCode)
                        -- KeyCode.None (default) means "no single key"; check for a composite instead
                        if binding.KeyCode == Enum.KeyCode.None and binding.Up ~= Enum.KeyCode.None then
                            key = string.format("Composite(U=%s,D=%s,L=%s,R=%s)",
                                tostring(binding.Up), tostring(binding.Down),
                                tostring(binding.Left), tostring(binding.Right))
                        end
                        table.insert(results, string.format(
                            "    Binding: %s (Type=%s, Key=%s)",
                            binding.Name, tostring(binding.Type), key
                        ))
                    end
                end
            end
        end
        findActions(child, path .. "/" .. child.Name)
    end
end

findActions(game:GetService("ReplicatedStorage"), "ReplicatedStorage")
if player then findActions(player, "Player") end
findActions(game.Workspace, "Workspace")

if #results == 0 then
    return "WARNING: No InputContexts found. Check parenting."
end
return table.concat(results, "\n")
```

3. Test each action fires by pressing the bound keys
4. Check console for errors

## Important Constraints

### InputContext placement

Registration is placement-driven. A context registers only if it is in a valid live location:

| Location | Registers? | Use |
|----------|-----------|-----|
| `ReplicatedStorage` | ✅ Yes | **Preferred.** Same live instances at edit time and runtime; replicates to all clients |
| `Players.LocalPlayer` (or descendant, e.g. `PlayerScripts`) | ✅ Yes | Player-scoped input |
| `LocalPlayer.Character` (or descendant) | ✅ Yes | Character-scoped input |
| `Workspace` (or other neutral live location) | ✅ Yes | Global input |
| `StarterPlayer.StarterPlayerScripts` | ❌ No (template) | Edit-time template — copied to `PlayerScripts` at runtime, registers there |
| `StarterPlayer.StarterCharacterScripts` | ❌ No (template) | Edit-time template — copied to `Character` at runtime |
| `StarterGui`, `StarterPack` | ❌ No (template) | Never registers |
| Another player's Character | ❌ No | — |

**Key idea:** prefer `ReplicatedStorage` — it is the one location that is the *same live instances* at both edit time and runtime, so you author the tree once and it registers directly with no template-copy step. The Starter* services are template containers: they get copied to a live location at runtime, but instances that *remain* under a Starter service never register. So author under `ReplicatedStorage` and it just works in both Edit and Play.

- **Priority ordering**: Higher number = processed first; IAS default is `1000`. Only the *relative* ordering between contexts matters — pick small, clean IAS values (`1000`, `1100`, `1200`). Legacy CAS numbers (2000/3000) are not a target to reproduce; translate order once, then think purely in IAS terms.
- **No conditional sinking**: `context.Sink` is a static structural property, sampled top-down during input processing. Do NOT toggle it from a handler or per-frame loop to fake CAS-style conditional Sink/Pass — it races the input pipeline and won't reliably gate a given event. Model each input mode as a separate context switched via `Enabled`. Legacy code that returned Sink/Pass conditionally is a context-mode redesign — flag for manual review.
- **No Fire() from physics step**: Never call `InputAction:Fire()` or `InputBinding:Fire()` from `RunService.Stepped` or `BindToSimulation`. Use `RenderStepped` or `Heartbeat`.
- **Scriptable bindings**: For programmatic input injection (testing, AI), use `InputBinding.Type = Enum.InputBindingType.Scriptable` and call `binding:Fire(state)`.

## Tone

Be clear and direct. When proposing changes, show before/after code. Explain *why* the IAS approach is better (declarative, supports multiple input devices automatically, cleaner priority/sinking model). Don't overwhelm with API details — link to the pattern that applies.

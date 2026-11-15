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

---

## Input Action System API Reference

### Class Hierarchy

```
InputContext (Instance)
└── InputAction (Instance)
    └── InputBinding (Instance)
```

All three are standard Roblox Instances — create with `Instance.new()`, parent into the DataModel tree.

---

### InputContext

Container for related input actions. Controls priority and input sinking.

#### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Enabled` | bool | `true` | When false, all child actions stop processing input |
| `Priority` | int | `1000` | Higher = processed first. Only relative order matters; use small IAS values, not legacy CAS numbers |
| `Sink` | bool | `false` | When true, consumed inputs don't propagate to lower-priority contexts |

Enumerate child actions with `InputContext:GetChildren()` (the child `InputAction` instances).

#### Valid Parents

| Parent | Registers? | Scope |
|--------|-----------|-------|
| `ReplicatedStorage` | ✅ | **Preferred** — same live instances in Edit and Play; replicates to all clients |
| `Players.LocalPlayer` (or descendant, e.g. `PlayerScripts`) | ✅ | Player-scoped |
| `LocalPlayer.Character` (or descendant) | ✅ | Character-scoped |
| `Workspace` (or other neutral live location) | ✅ | Global |

**Preferred placement:** author the tree under `ReplicatedStorage`. Unlike the Starter services (which are edit-time templates copied to a live location at runtime), `ReplicatedStorage` holds the *same live instances* at edit time and runtime, so the context registers directly with no copy step.

**Never registers** (template copies, not live): StarterPlayerScripts, StarterCharacterScripts, StarterGui, StarterPack, and any instance that *remains* under a Starter service at runtime. Also invalid: another player's Character. A context outside a DataModel does not register.

---

### InputAction

Typed state machine representing a gameplay action.

#### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Type` | `Enum.InputActionType` | `Bool` | Determines state value type and available events |
| `Enabled` | bool | `true` | When false, action ignores all input |
| `PreferredBinding` | `InputBinding?` | nil | **Read-only.** The child `InputBinding` best matching the player's current input device (per `UserInputService.PreferredInput`). Use it to drive device-appropriate prompts (see migration Pattern 2 for an example). Re-resolves on device change; may be nil before first resolve. |

#### Enum.InputActionType

| Value | State Type | Use Case |
|-------|-----------|----------|
| `Bool` | boolean | Key press, button tap, toggle |
| `Direction1D` | number | Single analog axis. An analog trigger KeyCode reads 0–1; a composite (`Up`=+1 / `Down`=−1) reads −1–1. A plain digital key is transformed to 0 or 1 only (no intermediate values). |
| `Direction2D` | Vector2 | Movement (WASD/thumbstick), aiming |
| `Direction3D` | Vector3 | 6-DOF movement (forward/back + left/right + up/down) |
| `ViewportPosition` | Vector2 | Mouse/pointer screen position |

These five are the only script-usable values. Do not use `Displacement1D`, `Displacement2D`, `ViewportPositionSet`, `UIButton`, or `None` as an action `Type` — they are not available to Luau.

#### Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `GetState()` | none | `bool \| number \| Vector2 \| Vector3 \| nil` | Current action state |
| `Fire(state)` | varies by Type | none | **Deprecated** — use `InputBinding:Fire()` instead |

Enumerate the action's bindings with `InputAction:GetChildren()` (the child `InputBinding` instances).

#### Events

| Event | Parameters | Fires When |
|-------|-----------|------------|
| `Pressed` | none | Bool action: false → true |
| `Released` | none | Bool action: true → false |
| `StateChanged` | `value` (type matches action Type) | Any state change (except same-to-same) |

#### Placement

Must be parented to an `InputContext`.

---

### InputBinding

Maps physical hardware input to action state changes.

#### Core Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Type` | `Enum.InputBindingType` | `Automatic` | `Automatic` responds to hardware; `Scriptable` only responds to `Fire()` |
| `KeyCode` | `Enum.KeyCode` | `None` | Primary key/button. `None` = no single-key binding (`Unknown` is a deprecated alias for the same value) |
| `UIButton` | `Instance?` | nil | GuiButton click as primary trigger |

#### Modifier Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `PrimaryModifier` | `Enum.KeyCode` | `None` | Required modifier key (e.g., LeftCtrl). `None` = no modifier |
| `SecondaryModifier` | `Enum.KeyCode` | `None` | Second required modifier |
| `UIModifier` | `Instance?` | nil | Required GuiButton that must be held |

#### Directional Composition (for Direction1D/2D/3D actions)

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Up` | `Enum.KeyCode` | `None` | Positive Y key (Direction1D uses this as its **+1** axis) |
| `Down` | `Enum.KeyCode` | `None` | Negative Y key (Direction1D uses this as its **−1** axis) |
| `Left` | `Enum.KeyCode` | `None` | Negative X key |
| `Right` | `Enum.KeyCode` | `None` | Positive X key |
| `Forward` | `Enum.KeyCode` | `None` | Negative Z (Direction3D only) |
| `Backward` | `Enum.KeyCode` | `None` | Positive Z (Direction3D only) |
| `ClampMagnitudeToOne` | bool | `true` | Clamp result to unit length (Direction2D/3D) / −1..1 (Direction1D) |

**Direction1D composites use `Up`/`Down`** (the Y axis), not `Left`/`Right`. For a single-axis control like steering left/right, map `Up` to the "increase" key and `Down` to the "decrease" key — e.g. Steer: `Up = D`, `Down = A`.

#### Analog Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `PressedThreshold` | number | `0.5` | Analog value above which Bool becomes true |
| `ReleasedThreshold` | number | `0.2` | Analog value below which Bool becomes false |
| `ResponseCurve` | number | `1.0` | Gamepad sensitivity curve (1–10) |
| `Scale` | number | `1.0` | Uniform output multiplier |
| `Vector2Scale` | Vector2 | `(1, 1)` | Per-axis scaling for Direction2D |
| `Vector3Scale` | Vector3 | `(1, 1, 1)` | Per-axis scaling for Direction3D |

#### Display Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `DisplayName` | string | `""` | Custom display text for UI prompts |
| `DisplayImage` | Content | empty | Custom icon for UI prompts |

#### Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `Fire(state)` | varies | none | Programmatically fire (only works on Scriptable bindings) |

#### Placement

Must be parented to an `InputAction`.

---

### InputActionLabel

A `GuiObject` that displays a device-appropriate prompt (button glyph or key text) for an `InputAction`. Set its `InputAction` and it auto-resolves and updates when the player's input device changes — no scripting needed. Use this instead of hand-building prompt UI from `action.PreferredBinding`.

#### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `InputAction` | `InputAction?` | nil | The action to display a prompt for. Resolves against the action's `PreferredBinding` for the current device. |
| `ResolvedText` | string | `""` | **Read-only.** The resolved key/button text for the current device (e.g. `"E"`). Empty when a glyph image is shown instead. |
| `ResolvedImageContent` | Content | empty | **Read-only.** The resolved glyph image for the current device (e.g. a gamepad button icon). |

It renders either the resolved text or the resolved image, so it also exposes standard Gui styling: text (`TextColor3`, `FontFace`, `TextSize`, `TextTransparency`, `TextXAlignment`, `TextYAlignment`, `TextWrapped`) and image (`ImageColor3`, `ImageTransparency`), plus everything inherited from `GuiObject` (`Size`, `Position`, `AnchorPoint`, …).

#### Placement

Anywhere a `GuiObject` is valid (under a `ScreenGui`/`SurfaceGui`). It reads the `InputAction` you assign; it does not need to be parented near the IAS tree.

---

### Common KeyCode Values

#### Keyboard
`Enum.KeyCode.W`, `S`, `A`, `D`, `Space`, `LeftShift`, `LeftControl`, `E`, `F`, `R`, `Q`, `Tab`, `Escape`

#### Gamepad
`Enum.KeyCode.ButtonA`, `ButtonB`, `ButtonX`, `ButtonY`, `ButtonL1`, `ButtonR1`, `ButtonL2`, `ButtonR2`, `ButtonL3`, `ButtonR3`, `ButtonStart`, `ButtonSelect`, `Thumbstick1`, `Thumbstick2`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`

#### Mouse
Mouse buttons bind by `KeyCode` using the **`Mouse*Button`** names — NOT `MouseButton1/2/3` (those are `UserInputType` values and are **not** valid `Enum.KeyCode` members; using them throws `MouseButton1 is not a valid member of "Enum.KeyCode"`):

`Enum.KeyCode.MouseLeftButton`, `MouseRightButton`, `MouseMiddleButton`

For pointer position on a `ViewportPosition` action, bind `Enum.KeyCode.MousePosition`.

⚠️ Legacy UIS/CAS code refers to `Enum.UserInputType.MouseButton1/2/3`. When converting, translate those to the `KeyCode.Mouse*Button` names above — do not carry the `MouseButton1` spelling onto a `binding.KeyCode`.

Pointer motion binds by `KeyCode` onto directional actions:
- `Enum.KeyCode.MouseDelta` — mouse movement delta; bind on a `Direction2D` action (mouse-look camera).
- `Enum.KeyCode.MouseWheel` — scroll wheel; bind on a `Direction1D` action (e.g. zoom).
- `Enum.KeyCode.TrackpadPan` — two-finger trackpad pan; bind on a `Direction2D` action.
- `Enum.KeyCode.TrackpadPinch` — trackpad pinch; bind on a `Direction1D` action.

#### Touch
- `Enum.KeyCode.TouchPosition` — finger screen position; bind on a `ViewportPosition` action (or a `Bool` action to detect any touch).
- `Enum.KeyCode.TouchDelta` — per-frame drag delta; bind on a `Direction2D` action.
- `Enum.KeyCode.TouchPinch` — two-finger pinch; bind on a `Direction1D` action.

For most on-screen mobile controls, prefer a `UIButton` binding or a Scriptable binding driven by a `GuiButton` rather than raw touch keycodes — see the touch/pointer note in the migration patterns.

---

### Priority

`InputContext.Priority` defaults to `1000`. Higher number = processed first. **Only the relative order between your own contexts matters** — choose small, readable IAS values and space them so you can insert later:

| Context | Suggested Priority |
|---------|-------------------|
| Base gameplay | `1000` (default) |
| HUD / overlay | `1100` |
| Modal / menu | `1200` |

When migrating from CAS, read the old priority numbers **once** to recover which context sat above which, then assign values from this scheme and discard the CAS numbers. Do not reproduce `2000`/`3000` — those were CAS/Core internals, not a target.

---

---

## Migration Patterns: CAS/UIS → IAS

> The examples below show **runtime creation** (parenting to `Players.LocalPlayer`) for compactness — `LocalPlayer` is a valid live parent, so they work as-is. **The preferred production shape is Pattern 0**: author the tree once at edit time under `ReplicatedStorage/Inputs`, and let a `Script` with `RunContext = Client` only connect handlers. Convert the runtime snippets to that shape when possible. API names (`Enabled`, `Type`, `KeyCode`, `Sink`, `Bool`, `Direction2D`, `Pressed`, `Released`, `StateChanged`) are exact — see the "⚠️ Exact API" section of the main skill.

### Pattern 0: The Preferred Shape (edit-time tree + handler script)

This is the target for most conversions. The `InputContext`/`InputAction`/`InputBinding` instances are **authored in the DataModel at edit time** — grouped under a single `Inputs` folder in `ReplicatedStorage` — so they exist as data; no code builds them. A `Script` with `RunContext = Client` only reads the actions and connects behavior.

**Authored instance tree (create these in the Explorer / via instance insertion, not with `Instance.new`):**

```
ReplicatedStorage
└── Inputs                         Folder
    └── GameplayContext            InputContext   { Priority = 1000 }
        ├── Jump                   InputAction    { Type = Bool }
        │   ├── Keyboard           InputBinding   { KeyCode = Space }
        │   └── Gamepad            InputBinding   { KeyCode = ButtonA }
        └── Move                   InputAction    { Type = Direction2D }
            ├── WASD               InputBinding   { Up=W, Down=S, Left=A, Right=D }
            └── Thumbstick         InputBinding   { KeyCode = Thumbstick1 }
```

**Handler script** (a `Script` with `RunContext = Enum.RunContext.Client`, e.g. under `ReplicatedStorage`) — connects only:

```lua
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")

local inputs = ReplicatedStorage:WaitForChild("Inputs")
local context = inputs:WaitForChild("GameplayContext")
local jumpAction = context:WaitForChild("Jump")
local moveAction = context:WaitForChild("Move")

local player = game.Players.LocalPlayer

jumpAction.Pressed:Connect(function()
    local humanoid = player.Character and player.Character:FindFirstChildOfClass("Humanoid")
    if humanoid then
        humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
    end
end)

-- Continuous input: poll every frame (see Pattern 3 / Pattern 7)
RunService.RenderStepped:Connect(function()
    local humanoid = player.Character and player.Character:FindFirstChildOfClass("Humanoid")
    if not humanoid then return end
    local dir = moveAction:GetState() -- Vector2 or nil
    humanoid:Move(dir and Vector3.new(dir.X, 0, -dir.Y) or Vector3.zero)
end)
```

#### Notes
- The tree is **data**, versioned with the place and editable in the Explorer — designers can retune bindings without touching code.
- Authoring under `ReplicatedStorage` means the *same instances* exist in Edit and Play (no template-copy step) and replicate to all clients.
- The remaining patterns show runtime `Instance.new` for brevity; the binding/action/handler logic is identical — only *where the tree comes from* differs.

### Pattern 1: Simple Key Press (Bool Action)

#### Before (CAS)

```lua
local CAS = game:GetService("ContextActionService")

CAS:BindAction("Interact", function(actionName, state, inputObject)
    if state == Enum.UserInputState.Begin then
        interactWithNearestObject()
    end
end, false, Enum.KeyCode.E)

-- Later:
CAS:UnbindAction("Interact")
```

#### After (IAS)

```lua
local player = game.Players.LocalPlayer

local inputContext = Instance.new("InputContext")
inputContext.Name = "GameplayContext"
inputContext.Parent = player

local interactAction = Instance.new("InputAction")
interactAction.Name = "Interact"
interactAction.Type = Enum.InputActionType.Bool
interactAction.Parent = inputContext

local keyBinding = Instance.new("InputBinding")
keyBinding.Name = "KeyboardBinding"
keyBinding.KeyCode = Enum.KeyCode.E
keyBinding.Parent = interactAction

interactAction.Pressed:Connect(function()
    interactWithNearestObject()
end)

-- To disable (equivalent to UnbindAction):
inputContext.Enabled = false
```

#### Notes
- `Pressed` fires on the false→true transition (equivalent to `UserInputState.Begin`)
- `Released` fires on true→false (equivalent to `UserInputState.End`)
- If you also need the End state, connect to `Released`

---

### Pattern 2: Multiple Keys for Same Action

#### Before (CAS)

```lua
CAS:BindAction("Jump", function(_, state)
    if state == Enum.UserInputState.Begin then
        humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
    end
end, false, Enum.KeyCode.Space, Enum.KeyCode.ButtonA)
```

#### After (IAS)

```lua
local jumpAction = Instance.new("InputAction")
jumpAction.Name = "Jump"
jumpAction.Type = Enum.InputActionType.Bool
jumpAction.Parent = inputContext

-- One binding per input device
local keyboardBinding = Instance.new("InputBinding")
keyboardBinding.Name = "Keyboard"
keyboardBinding.KeyCode = Enum.KeyCode.Space
keyboardBinding.Parent = jumpAction

local gamepadBinding = Instance.new("InputBinding")
gamepadBinding.Name = "Gamepad"
gamepadBinding.KeyCode = Enum.KeyCode.ButtonA
gamepadBinding.Parent = jumpAction

jumpAction.Pressed:Connect(function()
    humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
end)
```

#### Notes
- Each physical input gets its own InputBinding child
- The action fires regardless of which binding triggered it
- **Device-appropriate prompts.** Two options:
  - *Declarative (preferred):* parent an `InputActionLabel` GuiObject and set its `InputAction` — it auto-resolves the device glyph (`ResolvedImageContent` / `ResolvedText`) and updates on device change with no code.
  - *Custom UI:* read `action.PreferredBinding` — the child binding matching the player's current device — and drive your own prompt from its `DisplayName` / `DisplayImage` / `KeyCode`. It re-resolves when the device changes, so listen for that:

```lua
local UIS = game:GetService("UserInputService")

local function refreshPrompt()
    local binding = jumpAction.PreferredBinding  -- may be nil before first resolve
    if binding then
        promptLabel.Text = binding.DisplayName ~= "" and binding.DisplayName
            or tostring(binding.KeyCode)
    end
end

refreshPrompt()
UIS:GetPropertyChangedSignal("PreferredInput"):Connect(refreshPrompt)  -- keyboard ↔ gamepad ↔ touch
```

---

### Pattern 3: WASD Movement (Direction2D Action)

#### Before (UIS)

```lua
local UIS = game:GetService("UserInputService")
local RunService = game:GetService("RunService")

local keysDown = {}

UIS.InputBegan:Connect(function(input, gameProcessed)
    if gameProcessed then return end
    keysDown[input.KeyCode] = true
end)

UIS.InputEnded:Connect(function(input)
    keysDown[input.KeyCode] = nil
end)

RunService.RenderStepped:Connect(function()
    local moveDir = Vector3.zero
    if keysDown[Enum.KeyCode.W] then moveDir += Vector3.new(0, 0, -1) end
    if keysDown[Enum.KeyCode.S] then moveDir += Vector3.new(0, 0, 1) end
    if keysDown[Enum.KeyCode.A] then moveDir += Vector3.new(-1, 0, 0) end
    if keysDown[Enum.KeyCode.D] then moveDir += Vector3.new(1, 0, 0) end
    if moveDir.Magnitude > 0 then
        moveDir = moveDir.Unit
    end
    humanoid:Move(moveDir)
end)
```

#### After (IAS)

```lua
local player = game.Players.LocalPlayer
local RunService = game:GetService("RunService")

local inputContext = Instance.new("InputContext")
inputContext.Name = "MovementContext"
inputContext.Parent = player

local moveAction = Instance.new("InputAction")
moveAction.Name = "Move"
moveAction.Type = Enum.InputActionType.Direction2D
moveAction.Parent = inputContext

-- Keyboard: composite directional binding
local wasdBinding = Instance.new("InputBinding")
wasdBinding.Name = "WASD"
wasdBinding.Up = Enum.KeyCode.W
wasdBinding.Down = Enum.KeyCode.S
wasdBinding.Left = Enum.KeyCode.A
wasdBinding.Right = Enum.KeyCode.D
wasdBinding.ClampMagnitudeToOne = true
wasdBinding.Parent = moveAction

-- Gamepad: thumbstick (automatically provides Direction2D)
local thumbstickBinding = Instance.new("InputBinding")
thumbstickBinding.Name = "Thumbstick"
thumbstickBinding.KeyCode = Enum.KeyCode.Thumbstick1
thumbstickBinding.Parent = moveAction

RunService.RenderStepped:Connect(function()
    local direction = moveAction:GetState() -- Vector2 or nil
    if direction and direction.Magnitude > 0 then
        humanoid:Move(Vector3.new(direction.X, 0, -direction.Y))
    else
        humanoid:Move(Vector3.zero)
    end
end)
```

#### Notes
- **Poll `GetState()` every frame** for continuous movement — do NOT drive movement from `StateChanged`, which fires only on change and would stop applying while a key/stick is held steady (see Pattern 7).
- `ClampMagnitudeToOne = true` (default) normalizes diagonal W+D to unit length
- Thumbstick automatically provides Vector2 when bound to a Direction2D action
- `GetState()` returns the composed value across all active bindings
- No manual key tracking needed — IAS handles state internally

---

### Pattern 4: Priority and Sinking (Context Stacking)

#### Before (CAS)

```lua
-- Menu steals input from gameplay
CAS:BindActionAtPriority("MenuSelect", function(_, state)
    if state == Enum.UserInputState.Begin then
        selectMenuItem()
    end
    return Enum.ContextActionResult.Sink  -- Block lower priority
end, 3000, false, Enum.KeyCode.E)

-- Gameplay at default priority
CAS:BindAction("Interact", function(_, state)
    if state == Enum.UserInputState.Begin then
        interact()
    end
end, false, Enum.KeyCode.E)
```

#### After (IAS)

The old CAS used priority `3000` and a `Sink` return; in IAS we recover only the *order* (menu above gameplay) and assign a clean IAS value (`1100`), then toggle `Enabled` to switch modes.

```lua
-- Menu context: higher priority, sinks input
local menuContext = Instance.new("InputContext")
menuContext.Name = "MenuContext"
menuContext.Priority = 1100  -- just above gameplay's 1000; the old 3000 is irrelevant
menuContext.Sink = true  -- Blocks lower-priority contexts (structural, evaluated top-down each event)
menuContext.Enabled = false  -- Disabled until menu opens
menuContext.Parent = player

local menuSelectAction = Instance.new("InputAction")
menuSelectAction.Name = "MenuSelect"
menuSelectAction.Type = Enum.InputActionType.Bool
menuSelectAction.Parent = menuContext

local menuBinding = Instance.new("InputBinding")
menuBinding.KeyCode = Enum.KeyCode.E
menuBinding.Parent = menuSelectAction

menuSelectAction.Pressed:Connect(function()
    selectMenuItem()
end)

-- Gameplay context: lower priority
local gameplayContext = Instance.new("InputContext")
gameplayContext.Name = "GameplayContext"
gameplayContext.Priority = 1000
gameplayContext.Parent = player

local interactAction = Instance.new("InputAction")
interactAction.Name = "Interact"
interactAction.Type = Enum.InputActionType.Bool
interactAction.Parent = gameplayContext

local interactBinding = Instance.new("InputBinding")
interactBinding.KeyCode = Enum.KeyCode.E
interactBinding.Parent = interactAction

interactAction.Pressed:Connect(function()
    interact()
end)

-- Toggle contexts when menu opens/closes:
menuContext.Enabled = true   -- Menu takes over E key
menuContext.Enabled = false  -- Gameplay gets E key back
```

#### Notes
- `Sink = true` on the higher-priority context blocks input from reaching lower ones
- Enable/disable contexts to switch between input modes (replaces bind/unbind dance)
- Priority is only relative ordering — `1100` over `1000`. Don't reproduce CAS's `3000`.

#### ⚠️ IAS does NOT support conditional sinking

`Sink` is a **static structural property** of a context. During each input event the engine walks contexts in priority order and reads `context.Sink` as-is — there is no per-event "sink this one, pass that one" decision like CAS's callback that returned `Sink` sometimes and `Pass` other times.

- **Do NOT** flip `context.Sink = true/false` from inside an input handler or a per-frame loop to emulate conditional sinking. The value is sampled during processing; toggling it reactively races the input pipeline and will not reliably block or pass a specific event.
- **Instead, model each input mode as its own context** and switch modes with `Enabled` (as above). "While the menu is open, E selects and is blocked from gameplay" = a menu context with `Sink = true` that you `Enabled` when the menu opens — not a single context whose `Sink` you toggle mid-stream.
- If the legacy code returned `Sink`/`Pass` conditionally based on game state, that is a **context-mode redesign**, not a property toggle — flag it for manual review.

---

### Pattern 5: Modifier Keys (Ctrl+E, Ctrl+Shift+S)

#### Before (UIS)

```lua
UIS.InputBegan:Connect(function(input, gameProcessed)
    if gameProcessed then return end
    if input.KeyCode == Enum.KeyCode.E and UIS:IsKeyDown(Enum.KeyCode.LeftControl) then
        openEditor()
    elseif input.KeyCode == Enum.KeyCode.S
        and UIS:IsKeyDown(Enum.KeyCode.LeftControl)
        and UIS:IsKeyDown(Enum.KeyCode.LeftShift) then
        saveAs()
    end
end)
```

#### After (IAS)

```lua
-- Ctrl+E
local openEditorAction = Instance.new("InputAction")
openEditorAction.Name = "OpenEditor"
openEditorAction.Type = Enum.InputActionType.Bool
openEditorAction.Parent = inputContext

local openBinding = Instance.new("InputBinding")
openBinding.KeyCode = Enum.KeyCode.E
openBinding.PrimaryModifier = Enum.KeyCode.LeftControl
openBinding.Parent = openEditorAction

openEditorAction.Pressed:Connect(openEditor)

-- Ctrl+Shift+S (two modifiers)
local saveAsAction = Instance.new("InputAction")
saveAsAction.Name = "SaveAs"
saveAsAction.Type = Enum.InputActionType.Bool
saveAsAction.Parent = inputContext

local saveBinding = Instance.new("InputBinding")
saveBinding.KeyCode = Enum.KeyCode.S
saveBinding.PrimaryModifier = Enum.KeyCode.LeftControl
saveBinding.SecondaryModifier = Enum.KeyCode.LeftShift
saveBinding.Parent = saveAsAction

saveAsAction.Pressed:Connect(saveAs)
```

#### Notes
- Supports up to two modifiers: `PrimaryModifier` and `SecondaryModifier`
- Modifier keys must be held when the primary key is pressed
- `KeyCode.None` (the default) means no modifier required (`Unknown` is a deprecated alias for the same value)

---

### Pattern 6: Gamepad Trigger (Direction1D / Analog)

#### Before (UIS)

```lua
UIS.InputChanged:Connect(function(input, gameProcessed)
    if input.KeyCode == Enum.KeyCode.ButtonR2 then
        local triggerValue = input.Position.Z  -- 0 to 1
        setThrottle(triggerValue)
    end
end)
```

#### After (IAS)

```lua
local throttleAction = Instance.new("InputAction")
throttleAction.Name = "Throttle"
throttleAction.Type = Enum.InputActionType.Direction1D
throttleAction.Parent = inputContext

local triggerBinding = Instance.new("InputBinding")
triggerBinding.KeyCode = Enum.KeyCode.ButtonR2
triggerBinding.Parent = throttleAction

throttleAction.StateChanged:Connect(function(value)
    setThrottle(value)  -- 0 to 1
end)
```

#### Notes
- Direction1D from a single analog trigger reads 0–1. From a two-key composite (`Up`=+1 / `Down`=−1) it reads −1..1 — Direction1D composites use the `Up`/`Down` slots, not `Left`/`Right`.
- For a Bool-from-trigger (pressed/released), use `InputActionType.Bool` with threshold:
  - `binding.PressedThreshold = 0.5` (trigger "pressed" at 50%)
  - `binding.ReleasedThreshold = 0.2` (trigger "released" below 20%)

---

### Pattern 7: Camera Look (Thumbstick with Response Curve)

#### Before (UIS)

```lua
UIS.InputChanged:Connect(function(input, gameProcessed)
    if input.KeyCode == Enum.KeyCode.Thumbstick2 then
        local delta = input.Position
        camera.CFrame = camera.CFrame * CFrame.Angles(
            -delta.Y * sensitivity,
            -delta.X * sensitivity,
            0
        )
    end
end)
```

#### After (IAS)

Camera look is **continuous**: the camera keeps rotating for as long as the stick is deflected. `StateChanged` fires only when the value *changes*, so a held-steady stick would emit one event and stop. **Poll `GetState()` every frame instead** (same as the WASD movement pattern):

```lua
local RunService = game:GetService("RunService")

local lookAction = Instance.new("InputAction")
lookAction.Name = "CameraLook"
lookAction.Type = Enum.InputActionType.Direction2D
lookAction.Parent = inputContext

local thumbstickBinding = Instance.new("InputBinding")
thumbstickBinding.KeyCode = Enum.KeyCode.Thumbstick2
thumbstickBinding.ResponseCurve = 3.0  -- Non-linear sensitivity
thumbstickBinding.Vector2Scale = Vector2.new(sensitivity, sensitivity)
thumbstickBinding.Parent = lookAction

RunService.RenderStepped:Connect(function()
    local look = lookAction:GetState() -- Vector2 or nil
    if look then
        camera.CFrame = camera.CFrame * CFrame.Angles(-look.Y * 0.01, -look.X * 0.01, 0)
    end
end)
```

#### Notes
- **`StateChanged` vs `GetState()`**: use `StateChanged` for *discrete reactions to a change* (e.g. "value crossed into a new bucket"); use per-frame `GetState()` polling for *continuous* effects (camera, movement) where a held-steady input must keep applying. A steady thumbstick fires no `StateChanged` events.
- `ResponseCurve` applies a power curve to thumbstick input (1 = linear, higher = more deadzone in center)
- `Vector2Scale` scales each axis independently
- Response curve is applied before scale

---

### Pattern 8: Toggle (Press to enable, press again to disable)

#### Before (CAS)

```lua
local sprintActive = false

CAS:BindAction("ToggleSprint", function(_, state)
    if state == Enum.UserInputState.Begin then
        sprintActive = not sprintActive
        humanoid.WalkSpeed = sprintActive and 32 or 16
    end
end, false, Enum.KeyCode.LeftShift)
```

#### After (IAS)

```lua
local sprintActive = false

local sprintAction = Instance.new("InputAction")
sprintAction.Name = "ToggleSprint"
sprintAction.Type = Enum.InputActionType.Bool
sprintAction.Parent = inputContext

local shiftBinding = Instance.new("InputBinding")
shiftBinding.KeyCode = Enum.KeyCode.LeftShift
shiftBinding.Parent = sprintAction

sprintAction.Pressed:Connect(function()
    sprintActive = not sprintActive
    humanoid.WalkSpeed = sprintActive and 32 or 16
end)
```

#### Notes
- Toggle logic stays in user code — IAS doesn't have a built-in toggle mode
- For hold-to-sprint (non-toggle), use both `Pressed` and `Released`

---

### Pattern 9: Hold-to-Sprint

#### Before (CAS)

```lua
CAS:BindAction("Sprint", function(_, state)
    if state == Enum.UserInputState.Begin then
        humanoid.WalkSpeed = 32
    elseif state == Enum.UserInputState.End then
        humanoid.WalkSpeed = 16
    end
end, false, Enum.KeyCode.LeftShift)
```

#### After (IAS)

```lua
local sprintAction = Instance.new("InputAction")
sprintAction.Name = "Sprint"
sprintAction.Type = Enum.InputActionType.Bool
sprintAction.Parent = inputContext

local shiftBinding = Instance.new("InputBinding")
shiftBinding.KeyCode = Enum.KeyCode.LeftShift
shiftBinding.Parent = sprintAction

sprintAction.Pressed:Connect(function()
    humanoid.WalkSpeed = 32
end)

sprintAction.Released:Connect(function()
    humanoid.WalkSpeed = 16
end)
```

---

### Pattern 10: Touch / Mobile

Touch in IAS today centers on **`UIButton` bindings** — an on-screen `GuiButton` drives the same `InputAction` as keyboard/gamepad. This is the reliable, always-available touch path and covers the overwhelming majority of mobile controls (buttons, on-screen d-pads via four buttons, action bars). Start here.

> **Scope note.** Beyond `UIButton`, IAS also handles touch gestures directly: finger position (`TouchPosition` → `ViewportPosition` action), drag delta (`TouchDelta` → `Direction2D`), and pinch (`TouchPinch` → `Direction1D`), with `PointerIndex` selecting which finger for multi-touch. `UIButton` is still the simplest path for on-screen buttons; use the gesture keycodes when you need raw touch motion. See the touch subsection below.

#### 10a. On-screen button → action (the common case)

##### Before (manual GuiButton)

```lua
local button = script.Parent.JumpButton
button.Activated:Connect(function()
    humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
end)
```

##### After (IAS with UIButton binding)

```lua
local jumpAction = Instance.new("InputAction")
jumpAction.Name = "Jump"
jumpAction.Type = Enum.InputActionType.Bool
jumpAction.Parent = inputContext

-- Keyboard binding
local spaceBinding = Instance.new("InputBinding")
spaceBinding.KeyCode = Enum.KeyCode.Space
spaceBinding.Parent = jumpAction

-- Gamepad binding
local padBinding = Instance.new("InputBinding")
padBinding.KeyCode = Enum.KeyCode.ButtonA
padBinding.Parent = jumpAction

-- Touch button binding (same action, different input device).
-- Match the GuiButton container to WHEN you set UIButton:
--   * authoring at edit time -> reference the button in StarterGui
--   * assigning at runtime     -> reference the live button in PlayerGui
local touchBinding = Instance.new("InputBinding")
touchBinding.UIButton = game.StarterGui.MobileButtons.JumpButton  -- edit-time
touchBinding.Parent = jumpAction

-- ONE handler serves keyboard, gamepad, and touch
jumpAction.Pressed:Connect(function()
    humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
end)
```

- `UIButton` accepts any `GuiButton` instance (`TextButton`/`ImageButton`).
- **Match the container to when you bind:**
  - **Edit time** (preferred, declarative): reference a `GuiButton` in **`StarterGui`**. The reference re-resolves to that button's live copy in the player's `PlayerGui` at runtime — no runtime wiring needed.
  - **Runtime** (from a script): reference the live `GuiButton` in the player's **`PlayerGui`** (e.g. `player.PlayerGui.MobileButtons.JumpButton`). A `StarterGui` reference set from a runtime script points at the inert template, not what the player sees.
- Holding the button holds the Bool `true` → `Pressed`/`Released` bracket a press-and-hold, so hold-to-sprint (Pattern 9) works on touch too.
- The GuiButton is yours to create and position; IAS does not generate it.

#### 10b. On-screen movement (four buttons → Direction2D)

An on-screen d-pad reuses the composite Direction2D binding — one `UIButton` per direction is NOT how composites work (composite slots take `KeyCode`s), so drive movement with four **Bool** actions wired to buttons, or use a thumbstick UI that reports a Vector2 and feed it through a **Scriptable** binding:

```lua
-- Thumbstick widget reports a unit Vector2; push it into the Move action
local moveAction = -- Direction2D action (see Pattern 3), also has a WASD + Thumbstick1 binding
local touchStick = Instance.new("InputBinding")
touchStick.Name = "TouchStick"
touchStick.Type = Enum.InputBindingType.Scriptable  -- fed by script, not hardware
touchStick.Parent = moveAction

-- In your on-screen-joystick controller, each frame:
--   touchStick:Fire(Vector2.new(x, y))   -- x,y in [-1, 1]
```

#### 10c. Advanced touch (gesture keycodes)

| Capability | Action Type | Binding |
|-----------|-------------|---------|
| Finger screen position | `ViewportPosition` | `KeyCode = TouchPosition`. Poll `GetState()` — a held finger doesn't re-fire `StateChanged`. |
| Drag delta | `Direction2D` | `KeyCode = TouchDelta` |
| Pinch / two-finger | `Direction1D` | `KeyCode = TouchPinch` |
| Specific finger (multi-touch) | (any of the above) | set `InputBinding.PointerIndex` (0–10) to pick the finger |

Bind these touch KeyCodes the same way as any other input — one action per intent, one binding per device. To show the correct on-screen icon per device, parent an `InputActionLabel` GuiObject and set its `InputAction` — it auto-resolves the device-appropriate glyph — or read `action.PreferredBinding` for custom prompt UI (see Pattern 2).

---

### Pattern 11: Dynamic Binding Changes

#### Before (CAS)

```lua
-- Rebind action to different key
CAS:UnbindAction("Interact")
CAS:BindAction("Interact", interactCallback, false, Enum.KeyCode.F)
```

#### After (IAS)

```lua
-- Simply change the binding property
interactBinding.KeyCode = Enum.KeyCode.F
```

#### Notes
- Binding properties are live — changing `KeyCode` immediately takes effect
- No need to destroy/recreate the binding
- Use this for settings menus where players remap controls

---

### Pattern 12: Aggregating Action (many inputs → one intent)

**This is the most commonly mis-modeled conversion.** When legacy code reads several inputs and merges them into a single value or behavior — camera look from mouse + gamepad stick + arrow keys, zoom from wheel + keys — the result is **ONE action with MANY bindings**, not one action per input. Read the merged value once via `action:GetState()`.

A camera control scheme is the canonical example: a single `CameraRotationAction` (Direction2D) carries a gamepad binding, a mouse binding, and a keyboard binding; a single `CameraZoomAction` carries the rest.

#### Before (UIS — merges several devices by hand)

```lua
local rotateDelta = Vector2.zero

-- gamepad stick
UIS.InputChanged:Connect(function(input)
    if input.KeyCode == Enum.KeyCode.Thumbstick2 then
        rotateDelta += Vector2.new(input.Position.X, input.Position.Y)
    end
end)

-- mouse
UIS.InputChanged:Connect(function(input)
    if input.UserInputType == Enum.UserInputType.MouseMovement then
        rotateDelta += Vector2.new(input.Delta.X, input.Delta.Y)
    end
end)

-- arrow keys, tracked manually
local keys = {}
UIS.InputBegan:Connect(function(i) keys[i.KeyCode] = true end)
UIS.InputEnded:Connect(function(i) keys[i.KeyCode] = nil end)
-- ...per frame, fold arrow keys into rotateDelta...
```

#### After (IAS — one action, one binding per device)

Authored tree (edit time, under `ReplicatedStorage/Inputs`):

```
Inputs                                  Folder
└── CameraContext                       InputContext
    └── CameraRotationAction            InputAction { Type = Direction2D }
        ├── MouseBinding                InputBinding { KeyCode = MouseDelta }
        ├── GamepadBinding              InputBinding { KeyCode = Thumbstick2 }
        └── KeyboardBinding             InputBinding { Up=Up, Down=Down, Left=Left, Right=Right }
```

Mouse-look is just another binding: `MouseDelta` on the same `Direction2D` action. All three devices feed one `GetState()` — no per-device merging, and no `UserInputService` needed. Give each binding its own `Scale`/`Vector2Scale` for per-device sensitivity and Y-invert.

Handler script:

```lua
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")

local rotationAction = ReplicatedStorage.Inputs.CameraContext.CameraRotationAction

RunService.RenderStepped:Connect(function(dt)
    -- ONE read merges every bound device
    local rotate = rotationAction:GetState() -- Vector2
    if rotate and rotate.Magnitude > 0 then
        applyCameraRotation(rotate * dt)
    end
end)
```

#### Notes
- **One action per intent.** "Rotate the camera" is one intent → `CameraRotationAction`. Mouse, stick, and arrows are three *bindings* on it, not three actions.
- **Per-device tuning lives on the binding**, not in separate actions: `binding.Scale` (per-device sensitivity), `binding.Vector2Scale` (e.g. Y-invert), `binding.ResponseCurve` (stick feel). Set `GamepadBinding.Scale` and `MouseBinding.Scale` independently on the one action.
- **`GetState()` returns the combined result** across all active bindings — you never merge by hand.
- Continuous (camera/movement): poll `GetState()` each frame (Pattern 3/7). Discrete (a button): connect `Pressed`/`Released` directly — do not route through a `BindableEvent`.

---

### Anti-Patterns to Avoid

| Don't | Do Instead |
|-------|-----------|
| Parent InputContext to StarterGui / StarterPack / StarterPlayerScripts | Author under `ReplicatedStorage` (preferred) — same live instances in Edit and Play |
| `Instance.new("InputContext").Parent = StarterPlayerScripts` **at runtime** | Parent to a live location (`ReplicatedStorage`, `LocalPlayer`); Starter services are edit-time templates only |
| `context.Active = true` | `context.Enabled = true` |
| `action.Type = Enum.InputActionType.Boolean` / `.Vector2` | `.Bool` / `.Direction2D` |
| `action.ActionType` / `action.InputType` | `action.Type` |
| `binding.Type = Enum.KeyCode.X` or `= Enum.UserInputType.Touch` | `binding.KeyCode = Enum.KeyCode.X`; `binding.Type` is only `Automatic`/`Scriptable` |
| `binding.KeyCode = Enum.KeyCode.MouseButton1/2/3` (not valid KeyCodes) | `Enum.KeyCode.MouseLeftButton` / `MouseRightButton` / `MouseMiddleButton` |
| `action.Triggered` / `.Started` / `.Ended` (with `inputObject`) | `action.Pressed` / `action.Released` (no args); read value via `action:GetState()` |
| One action per key (`LeftAction`, `RightAction`, `UpAction`, …) | One action per **intent**; each key/device is a binding on it (Pattern 12) |
| Separate actions for mouse vs gamepad vs keyboard of the same intent | One aggregating action, one binding per device; read `GetState()` once |
| Wrap `action.Pressed`/`Released` in a `BindableEvent` | Connect logic directly to the action's event |
| Generate a runtime script that builds the tree | Author the declarative tree at edit time; script only connects handlers |
| Create one context per action | Group related actions in one context |
| Call `action:Fire()` from Stepped | Use `RenderStepped` or `Heartbeat` |
| Nest InputContexts inside each other | Keep contexts flat under `ReplicatedStorage` (or Player/Workspace) |
| Copy CAS priority numbers (2000/3000) forward | Recover *order* once; assign clean IAS values (`1000`, `1100`, `1200`) |
| Toggle `context.Sink` per-event/per-frame for conditional sinking | Separate contexts per input mode, switched via `Enabled`; no conditional sink exists |
| Destroy and recreate contexts to toggle | Use `context.Enabled = false/true` |

---

## Command Workflows

The sections below describe example workflows the user may invoke as slash commands (e.g. `/ias-setup`, `/ias-convert`, `/ias-audit`, `/ias-debug`). Each section is a reference for one specific use case of this skill — read the matching section when the user invokes that command, otherwise treat them as background context. They are not mandatory steps for every interaction with this skill.

### `/ias-audit` — Audit Legacy Input Patterns

_Scan the entire game for legacy ContextActionService and UserInputService input patterns and produce a migration report._

Scans all scripts in the game for CAS/UIS input patterns and produces a structured migration report showing what can be auto-converted, what needs manual review, and the recommended IAS structure.

### What to do

1. **Verify Studio is connected** — ensure the MCP server is active and a place is loaded.

2. **Scan all scripts for legacy patterns** — run via `execute_luau`:

```lua
local ScriptEditorService = game:GetService("ScriptEditorService")
local results = {}

local patterns = {
    -- CAS patterns
    { pat = ":BindAction%(", cat = "CAS", desc = "BindAction call", auto = true },
    { pat = ":BindActionAtPriority%(", cat = "CAS", desc = "BindActionAtPriority call", auto = true },
    { pat = ":UnbindAction%(", cat = "CAS", desc = "UnbindAction call", auto = true },
    { pat = ":SetTitle%(", cat = "CAS", desc = "SetTitle (mobile button)", auto = true },
    { pat = ":SetDescription%(", cat = "CAS", desc = "SetDescription (mobile button)", auto = true },
    { pat = ":SetImage%(", cat = "CAS", desc = "SetImage (mobile button)", auto = true },
    { pat = ":GetButton%(", cat = "CAS", desc = "GetButton (mobile button ref)", auto = false },
    -- UIS patterns
    { pat = "%.InputBegan:Connect", cat = "UIS", desc = "InputBegan listener", auto = true },
    { pat = "%.InputEnded:Connect", cat = "UIS", desc = "InputEnded listener", auto = true },
    { pat = "%.InputChanged:Connect", cat = "UIS", desc = "InputChanged listener (may be analog)", auto = true },
    { pat = ":IsKeyDown%(", cat = "UIS", desc = "Polling key state", auto = false },
    { pat = ":IsGamepadButtonDown%(", cat = "UIS", desc = "Polling gamepad state", auto = false },
    { pat = ":GetMouseDelta%(", cat = "UIS", desc = "Mouse delta polling", auto = false },
    { pat = ":GetGamepadState%(", cat = "UIS", desc = "Full gamepad state polling", auto = false },
}

local function scanScript(script)
    local ok, source = pcall(function()
        return ScriptEditorService:GetEditorSource(script)
    end)
    if not ok or not source then return end

    local hits = {}
    for _, p in patterns do
        local count = 0
        for _ in source:gmatch(p.pat) do
            count += 1
        end
        if count > 0 then
            table.insert(hits, {
                category = p.cat,
                description = p.desc,
                count = count,
                autoConvertible = p.auto,
            })
        end
    end

    if #hits > 0 then
        table.insert(results, {
            path = script:GetFullName(),
            className = script.ClassName,
            hits = hits,
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

-- Format output
local output = {}
table.insert(output, "=== LEGACY INPUT AUDIT ===")
table.insert(output, string.format("Scripts with legacy input: %d", #results))
table.insert(output, "")

local autoCount, manualCount = 0, 0
for _, r in results do
    table.insert(output, "--- " .. r.path .. " (" .. r.className .. ") ---")
    for _, h in r.hits do
        local tag = h.autoConvertible and "[AUTO]" or "[MANUAL]"
        table.insert(output, string.format("  %s %s (x%d) - %s", tag, h.category, h.count, h.description))
        if h.autoConvertible then
            autoCount += h.count
        else
            manualCount += h.count
        end
    end
    table.insert(output, "")
end

table.insert(output, "=== SUMMARY ===")
table.insert(output, string.format("Auto-convertible callsites: %d", autoCount))
table.insert(output, string.format("Needs manual review: %d", manualCount))

return table.concat(output, "\n")
```

3. **For each script with hits**, use `script_read` to read the full source and identify:
   - What actions are being defined (the string names in BindAction calls)
   - What keys are bound
   - Whether callbacks use Begin/End/Changed states
   - Whether Sink is returned — and whether it is returned **conditionally** (different result on different events); conditional sinking has no IAS equivalent and needs a context-mode redesign (flag for manual review)
   - The *relative order* of any explicit priorities (which context sits above which) — not the numeric values

4. **Produce the migration report** — present to the user as a structured table:

```
## Migration Report

### Auto-Convertible (X callsites)

| Script | Action | Keys | Type | Priority | Sinks |
|--------|--------|------|------|----------|-------|
| ... | ... | ... | Bool | default | No |

### Needs Manual Review (Y callsites)

| Script | Pattern | Reason |
|--------|---------|--------|
| ... | IsKeyDown polling | Must redesign as event-driven |
| ... | GetButton reference | Custom mobile button layout |

### Recommended IAS Structure

| Context | Priority | Actions | Bindings |
|---------|----------|---------|----------|
| GameplayContext | 1000 | Move, Jump, Interact | WASD, Space, E + gamepad |
| MenuContext | 1100 | Select, Back | E, Escape + gamepad |
```

(Priorities are clean IAS values chosen to preserve order — `1000` base, `1100` above it — not carried over from CAS.)

5. **Ask the user** how they want to proceed:
   - Convert all auto-convertible scripts at once
   - Convert one script at a time for review
   - Generate the IAS structure only (without modifying existing scripts)

### Notes

- This is a **read-only** audit — no scripts are modified
- If the game is large (50+ scripts), summarize by category rather than listing every callsite
- Flag scripts that mix CAS and UIS patterns in the same file (common source of bugs)
- Highlight any `ContextActionService:GetButton()` usage — this requires UI migration too

### `/ias-convert` — Convert Scripts to IAS

_Convert one or more scripts from ContextActionService/UserInputService to the Input Action System._

Converts legacy CAS/UIS input code in specified scripts to the declarative Input Action System.

> **Read the "⚠️ Exact API — Do Not Deviate" section of the main skill before writing any code.** Property names (`Enabled`, `Type`, `KeyCode`, `Sink`), enum values (`Bool`, `Direction2D`, …), and events (`Pressed`, `Released`, `StateChanged`) are exact — do not carry over CAS/UIS names like `.Active`, `.Boolean`, `.Vector2`, `.ActionType`, `.Triggered`, `.Started`/`.Ended`.

> **Model by INTENT, not by key.** The most common conversion mistake is emitting one action per key or per device (`LeftAction`/`RightAction`, or `MouseLookAction` + `GamepadLookAction`). Collapse all inputs that drive the *same logical intent* into ONE action with one binding per key/device, read via `action:GetState()`. See "One action per INTENT" in the main skill and Migration Pattern 12 (aggregating action). Also: connect logic directly to `action.Pressed`/`Released` — never wrap them in a `BindableEvent`.

> **Prefer authoring the tree at edit time under `ReplicatedStorage`.** When the source scripts create input at load, the cleanest conversion moves the *bindings* into a declarative instance tree under `ReplicatedStorage` — the same live instances at edit time and runtime, registers directly, replicates to all clients — and leaves only handler connections (`action.Pressed:Connect(...)`) in the script. Keep runtime `Instance.new("InputContext")` only when the original code was itself dynamic (conditional binds/unbinds).

> **Recommend the Input Action Manager.** Once the tree exists, tell the user they can review and tune the bindings visually in the **Input Action Manager** (see the IAM section in the main skill). It's especially useful post-conversion for spotting missing per-platform bindings and KeyCode conflicts across the migrated actions.

### What to do

1. **Identify target scripts** — if the user specifies scripts, use those. Otherwise, run the audit scan from `/ias-audit` to find candidates and ask the user which to convert.

2. **Read the target script(s)** — use `script_read` to get the full source.

3. **Analyze each legacy callsite** — for each `BindAction`, `InputBegan`, etc., determine:

   - **Intent, not key**: group callsites by the *logical action* they serve. If mouse-move, a thumbstick, and arrow keys all feed one "rotate camera" value, that is ONE Direction2D action with three bindings — not three actions. If wheel + keys + pinch all zoom, that is ONE action. Merge first, then name.
   - **Action name**: the string identifier for the intent (e.g., "Jump", "CameraRotation")
   - **Action type**: map from key usage to InputActionType:
     - Single key press/release → `Bool`
     - WASD or arrow keys checked together → `Direction2D`
     - Thumbstick → `Direction2D`
     - Trigger (ButtonR2/L2 with Position.Z) → `Direction1D`
     - Mouse position tracking → `ViewportPosition`
   - **Key bindings**: all Enum.KeyCode values passed to BindAction or checked in InputBegan
   - **Relative order**: from `BindActionAtPriority`, note only where this sits relative to other contexts — do NOT copy the numeric CAS value; you'll assign clean IAS priorities (1000/1100/1200) preserving order only
   - **Sinking**: does the callback `return Enum.ContextActionResult.Sink`?
   - **Scope**: is it bound/unbound dynamically? (determines if we need Enabled toggling)
   - **Modifiers**: is `IsKeyDown` used to check for Ctrl/Shift/Alt?

4. **Group actions into contexts** — heuristics:
   - Actions bound/unbound together → same context
   - Actions at same priority → same context
   - Actions that sink together → same context with `Sink = true`
   - Separate gameplay, UI/menu, and tool contexts

5. **Propose the conversion** to the user — show:
   - The IAS tree structure (contexts → actions → bindings)
   - A before/after code snippet for the most complex conversion
   - Any callsites that need manual attention (GetButton, polling patterns)

6. **Wait for user approval** — use `ask_questions` if the user hasn't confirmed. This conversion rewrites real scripts, so include the warning from *This Skill Changes Real Code: Confirm First* in the main skill: recommend a test place, a copy, or a place they can easily revert, and don't write until they say yes.

7. **Write the converted code** — use `multi_edit` to rewrite the script(s).

   **Preferred (declarative): author the tree at edit time under `ReplicatedStorage/Inputs`, connect handlers in the script.**
   Insert the `InputContext`/`InputAction`/`InputBinding` instances under a single `Folder` named `Inputs` in `ReplicatedStorage` (group every context beneath it) in the edit DataModel; a client `Script` (`RunContext = Client`) then only connects handlers:

   ```lua
   -- InputController (Script with RunContext = Enum.RunContext.Client, e.g. under ReplicatedStorage)
   local ReplicatedStorage = game:GetService("ReplicatedStorage")
   local inputs = ReplicatedStorage:WaitForChild("Inputs")
   local context = inputs:WaitForChild("GameplayContext")
   local interactAction = context:WaitForChild("Interact")

   interactAction.Pressed:Connect(function()
       -- converted logic here
   end)
   ```

   **Runtime creation — ONLY if the original code was dynamic (conditional bind/unbind).**
   Parent to a live location such as `ReplicatedStorage`, never to a Starter service:

   ```lua
   local ReplicatedStorage = game:GetService("ReplicatedStorage")

   local inputs = ReplicatedStorage:FindFirstChild("Inputs") or Instance.new("Folder")
   inputs.Name = "Inputs"
   inputs.Parent = ReplicatedStorage

   local inputContext = Instance.new("InputContext")
   inputContext.Name = "ContextName"
   inputContext.Priority = 1000
   inputContext.Parent = inputs

   local actionName = Instance.new("InputAction")
   actionName.Name = "ActionName"
   actionName.Type = Enum.InputActionType.Bool   -- Bool / Direction1D / Direction2D / Direction3D / ViewportPosition
   actionName.Parent = inputContext

   local binding = Instance.new("InputBinding")
   binding.KeyCode = Enum.KeyCode.E              -- the key goes on .KeyCode, NOT .Type
   binding.Parent = actionName

   actionName.Pressed:Connect(function()
       -- converted logic here
   end)
   ```

   **Conversion rules:**
   - Remove `local CAS = game:GetService("ContextActionService")` if no other CAS usage remains
   - Remove `BindAction`/`UnbindAction` calls
   - Remove `InputBegan`/`InputEnded`/`InputChanged` connections that are fully replaced
   - Replace `Enum.UserInputState.Begin` checks with `Pressed` event
   - Replace `Enum.UserInputState.End` checks with `Released` event
   - Replace `Enum.ContextActionResult.Sink` with `context.Sink = true`
   - For WASD patterns, collapse 4 key checks into one Direction2D binding
   - Add gamepad bindings alongside keyboard ones (free multi-device support)

8. **Validate the conversion** — enter Play mode with `start_stop_play` and run:

```lua
local player = game.Players.LocalPlayer
local results = {}

-- Enumerate children of a given class (creator scripts read the tree with GetChildren).
local function childrenOfClass(instance, className)
    local out = {}
    for _, child in instance:GetChildren() do
        if child:IsA(className) then
            table.insert(out, child)
        end
    end
    return out
end

-- Check contexts registered
local function findContexts(parent, path)
    for _, child in parent:GetChildren() do
        if child:IsA("InputContext") then
            local actions = childrenOfClass(child, "InputAction")
            table.insert(results, string.format(
                "OK: %s/%s (Priority=%d, Actions=%d, Enabled=%s)",
                path, child.Name, child.Priority, #actions, tostring(child.Enabled)
            ))
            for _, action in actions do
                local bindings = childrenOfClass(action, "InputBinding")
                if #bindings == 0 then
                    table.insert(results, string.format(
                        "WARN: %s has no bindings", action:GetFullName()
                    ))
                end
            end
        end
        findContexts(child, path .. "/" .. child.Name)
    end
end

findContexts(game:GetService("ReplicatedStorage"), "ReplicatedStorage")
if player then findContexts(player, "Player") end
findContexts(game.Workspace, "Workspace")

if #results == 0 then
    return "ERROR: No InputContexts found after conversion. Check script execution and parenting."
end
return table.concat(results, "\n")
```

9. **Check console** for errors with `get_console_output`.

10. **Report results** — tell the user what was converted, any remaining manual items, and how to test each action.

### Handling Edge Cases

#### gameProcessedEvent check
```lua
-- Before: UIS.InputBegan:Connect(function(input, gameProcessed)
--     if gameProcessed then return end
-- This is handled by IAS priority/sinking automatically.
-- Lower-priority contexts won't fire if a higher one sinks.
-- Remove the gameProcessed check.
```

#### Dynamic bind/unbind
```lua
-- Before: CAS:BindAction("X", ...) / CAS:UnbindAction("X")
-- After: Create the context/action once, toggle with:
inputContext.Enabled = true   -- "bind"
inputContext.Enabled = false  -- "unbind"
```

#### CAS mobile button (createTouchButton = true)
```lua
-- Before: CAS:BindAction("Action", callback, true, KeyCode.E)
-- The 'true' parameter creates a mobile touch button.
--
-- After: the user must author the GuiButton themselves (IAS doesn't generate
-- it). Match the UIButton container to WHEN the binding is set:
--
-- PREFERRED — author the binding + button at edit time, reference StarterGui:
--   (binding lives in the ReplicatedStorage tree; button in StarterGui;
--    the ref resolves to the live PlayerGui copy at runtime — no runtime wiring)
--     binding.UIButton = game.StarterGui.MobileButtons.ActionButton
--
-- RUNTIME — if this binding is created by a script at runtime, reference the
-- live button in PlayerGui (a StarterGui ref from runtime points at the inert template):
local binding = Instance.new("InputBinding")
binding.UIButton = player.PlayerGui.MobileButtons.ActionButton
binding.Parent = action
-- Flag for manual review if no existing button exists.
```

#### Conditional Sink/Pass from callback (NO direct IAS equivalent)
```lua
-- Before: callback returns Enum.ContextActionResult.Sink sometimes,
--         Pass other times, based on game state.
--
-- IAS has NO conditional sinking. context.Sink is a static structural
-- property sampled top-down during input processing — you cannot toggle
-- it per-event to reproduce this.
--
-- Correct approach: model the two states as two contexts and switch with
-- Enabled. e.g. "sink E while a prompt is open" becomes a prompt context
-- (Sink = true) that you set Enabled = true when the prompt opens and
-- Enabled = false when it closes — NOT a single context whose Sink you flip.
--
-- If the Sink/Pass decision is genuinely per-input-event (not per-mode),
-- flag for manual review — it needs a design change, not a mechanical port.
```

Do NOT emit code that assigns `context.Sink` inside an input handler, `RenderStepped`, or any loop. Setting `Sink` mid-stream to emulate conditional sinking does not work reliably.

### Notes

- Never remove code the user didn't ask to convert
- If a script has both CAS and non-input code, only modify the input portions
- Preserve comments that explain game logic (remove comments about CAS mechanics)
- Add a brief comment at the top: `-- Input: using Input Action System`

### `/ias-debug` — Debug IAS

_Diagnose why an existing Input Action System setup isn't working — contexts not registering, actions not firing, input being swallowed, or wrong values._

Inspects a live Input Action System tree and reports the likely cause when input "doesn't work". This is the companion to `/ias-audit`: audit answers *"what legacy code do I need to convert?"*, debug answers *"my IAS is set up but not behaving — why?"*

> This command is for an existing IAS setup. If the game still uses ContextActionService/UserInputService, run `/ias-audit` first. Read the "⚠️ Exact API" and placement sections of the main skill before diagnosing.

### When to use

- "My InputAction doesn't fire / `Pressed` never runs"
- "The context isn't registering"
- "One action is eating input meant for another"
- "Movement stops when I hold the key" / "camera only moves once"
- "It works in Studio but not in a live game" (or vice-versa)

### What to do

1. **Enter Play mode** — IAS only registers contexts in a live DataModel. Use `start_stop_play(is_start: true)` and wait for the game to initialize. Most "not working" reports are really "inspected in Edit mode where nothing is live yet."

2. **Dump the live IAS tree** — run this diagnostic via `execute_luau`. It walks every valid live location, reports each context/action/binding, and flags the common failure modes:

```lua
local Players = game:GetService("Players")
local player = Players.LocalPlayer
local out = {}
local issues = {}

-- Enumerate children of a given class (creator scripts read the tree with GetChildren).
local function childrenOfClass(instance, className)
    local result = {}
    for _, child in instance:GetChildren() do
        if child:IsA(className) then
            table.insert(result, child)
        end
    end
    return result
end

local function inspectContext(ctx, path)
    local actions = childrenOfClass(ctx, "InputAction")
    table.insert(out, string.format(
        "Context %s  Enabled=%s  Priority=%d  Sink=%s  Actions=%d",
        path, tostring(ctx.Enabled), ctx.Priority, tostring(ctx.Sink), #actions
    ))

    if not ctx.Enabled then
        table.insert(issues, path .. ": context is DISABLED (Enabled=false) — no child action will fire")
    end
    if #actions == 0 then
        table.insert(issues, path .. ": context has NO actions")
    end

    for _, action in actions do
        local bindings = childrenOfClass(action, "InputBinding")
        local t = tostring(action.Type):gsub("Enum.InputActionType.", "")
        table.insert(out, string.format(
            "  Action %s  Type=%s  Enabled=%s  Bindings=%d",
            action.Name, t, tostring(action.Enabled), #bindings
        ))
        if not action.Enabled then
            table.insert(issues, action:GetFullName() .. ": action is DISABLED")
        end
        if #bindings == 0 then
            table.insert(issues, action:GetFullName() .. ": action has NO bindings — nothing can trigger it")
        end
        for _, b in bindings do
            -- KeyCode.None (default) means "no single key" (Unknown is a deprecated alias)
            local hasKey = b.KeyCode ~= Enum.KeyCode.None
            local key = tostring(b.KeyCode):gsub("Enum.KeyCode.", "")
            local composite = b.Up ~= Enum.KeyCode.None or b.Down ~= Enum.KeyCode.None
                or b.Left ~= Enum.KeyCode.None or b.Right ~= Enum.KeyCode.None
            local desc = composite and "composite(WASD-style)" or ("KeyCode=" .. key)
            table.insert(out, string.format("    Binding %s  %s  Type=%s", b.Name, desc, tostring(b.Type):gsub("Enum.InputBindingType.", "")))
            -- Bool action with only a composite binding, or directional action with no input at all
            if t == "Bool" and composite then
                table.insert(issues, b:GetFullName() .. ": Bool action with composite (Up/Down/Left/Right) binding — composites feed Direction2D, not Bool")
            end
            if (t == "Direction2D" or t == "Direction3D") and not composite and not hasKey then
                table.insert(issues, b:GetFullName() .. ": directional action but binding has no KeyCode and no composite directions set")
            end
        end
    end
end

-- Walk the valid live locations
local function walk(root, rootName)
    if not root then return end
    for _, d in root:GetDescendants() do
        if d:IsA("InputContext") then
            inspectContext(d, rootName .. "/" .. d:GetFullName():gsub("^" .. rootName .. "%.", ""))
        end
    end
end

walk(game:GetService("ReplicatedStorage"), "ReplicatedStorage")
walk(player, "Players.LocalPlayer")
walk(workspace, "Workspace")

-- Detect contexts sitting in never-registering template locations
for _, svcName in {"StarterGui", "StarterPack"} do
    local svc = game:FindFirstChild(svcName)
    if svc then
        for _, d in svc:GetDescendants() do
            if d:IsA("InputContext") then
                table.insert(issues, d:GetFullName() .. ": InputContext under " .. svcName .. " NEVER registers (template copy)")
            end
        end
    end
end
local sps = game:GetService("StarterPlayer"):FindFirstChild("StarterPlayerScripts")
if sps then
    for _, d in sps:GetDescendants() do
        if d:IsA("InputContext") then
            table.insert(issues, d:GetFullName() .. ": InputContext left under StarterPlayerScripts. It is copied to PlayerScripts at runtime; the copy under Players.LocalPlayer.PlayerScripts is the live one. If you don't see it there, the copy step hasn't run.")
        end
    end
end

local report = "=== LIVE IAS TREE ===\n" .. (#out > 0 and table.concat(out, "\n") or "(no InputContexts found in any live location)")
report = report .. "\n\n=== LIKELY ISSUES ===\n" .. (#issues > 0 and table.concat(issues, "\n") or "(none detected structurally — see checklist)")
return report
```

3. **Work through the diagnostic checklist** using the dump plus the symptom:

   | Symptom | Likely cause | Check |
   |---------|-------------|-------|
   | Context absent from the dump entirely | Placement — it's in a template (Starter*) or outside a live tree | Dump flags Starter* contexts; confirm a live copy exists under `ReplicatedStorage`/`PlayerScripts` |
   | Action's `Pressed`/`StateChanged` never fires | Context or action `Enabled = false`; or no bindings | Dump flags both |
   | Nothing fires only in a live game (works in Studio) | Authored under `StarterPlayerScripts` and reading from the wrong location, or server-authority replication | Verify the handler reads the live copy; check SA if using `Fire()` |
   | Input "eaten" — lower context stops responding | A higher-`Priority` context has `Sink = true` | Dump shows Priority + Sink; a sinking context above blocks lower ones |
   | Movement/camera stops while input is held steady | Driving continuous input from `StateChanged` instead of polling | Handler should poll `action:GetState()` each frame (`RenderStepped`) |
   | Diagonal movement too fast | `ClampMagnitudeToOne = false` on a Direction2D composite | Inspect the binding |
   | Trigger/analog reads as on/off only | Action `Type = Bool` with thresholds instead of `Direction1D` | Check the action Type |
   | Bool action never triggers from WASD keys | Composite (`Up/Down/Left/Right`) binding on a `Bool` action | Composites only feed Direction2D/3D |

4. **For "input eaten" reports, map the sink chain** — list enabled contexts by descending priority and show which one sinks:

```lua
local Players = game:GetService("Players")
local contexts = {}
local function collect(root)
    if not root then return end
    for _, d in root:GetDescendants() do
        if d:IsA("InputContext") and d.Enabled then
            table.insert(contexts, d)
        end
    end
end
collect(game:GetService("ReplicatedStorage"))
collect(Players.LocalPlayer)
collect(workspace)
table.sort(contexts, function(a, b) return a.Priority > b.Priority end)

local lines = {"Enabled contexts, highest priority first:"}
local sunkBy = nil
for _, c in contexts do
    local marker = ""
    if c.Sink then
        marker = "  <== Sink=true (blocks everything below it for keys it binds)"
        sunkBy = sunkBy or c
    end
    table.insert(lines, string.format("  [%d] %s%s", c.Priority, c.Name, marker))
end
if sunkBy then
    table.insert(lines, "\nIf a lower context isn't responding, '" .. sunkBy.Name .. "' is the likely culprit. Reminder: Sink is a static structural property — do NOT toggle it per-event; switch input modes with Enabled instead.")
end
return table.concat(lines, "\n")
```

5. **Check the console** with `get_console_output` — a handler erroring inside `Pressed`/`StateChanged` can look like "input not working." Also watch for warnings about contexts failing to register.

6. **Reproduce and confirm** — if the fix is behavioral (e.g. polling vs `StateChanged`), use `execute_luau` to read `action:GetState()` live while the user holds the input, or drive a scriptable binding to confirm the handler path fires:

```lua
-- Confirm the handler path works independent of hardware:
-- (only if the binding Type is Scriptable)
local action = -- resolve the action instance
for _, b in action:GetChildren() do
    if b:IsA("InputBinding") and b.Type == Enum.InputBindingType.Scriptable then
        b:Fire(true)   -- should fire action.Pressed
        task.wait(0.1)
        b:Fire(false)  -- should fire action.Released
    end
end
```

7. **Report** — state the symptom, the confirmed cause, the exact instance/property to change, and (if useful) a before/after. Prefer a minimal fix (flip `Enabled`, fix `Type`, move the context to a live location, switch a handler to polling) over rebuilding the tree. If you apply the fix yourself, confirm first, per *This Skill Changes Real Code: Confirm First* in the main skill.

### Notes

- Most reports are placement (Edit vs live tree) or an `Enabled`/`Sink`/`Type` mistake — check those first.
- Never recommend toggling `context.Sink` to fix an "input eaten" issue by flipping it off mid-play; that's conditional sinking, which IAS does not support. The fix is priority/`Enabled` mode switching.
- If the tree looks correct but nothing fires and there are no console errors, suspect server-authority replication (`Fire()` under a Player ancestor) — flag for deeper review.
- **Point the user to the Input Action Manager** (`Window > Input > Input Action Manager`) for visual auditing — it flags **missing per-platform bindings** and **KeyCode conflicts** (same key on two actions in one context) directly in a table view, catching the same mapping gaps this command checks for. Note IAM is an edit-time tool and does not show contexts in Starter folders.

### `/ias-setup` — Set Up IAS

_Set up a fresh Input Action System from a description of desired inputs (e.g., "WASD movement + sprint + jump + interact")._

Generates a complete Input Action System hierarchy from a natural-language description of desired inputs. Creates the InputContext/InputAction/InputBinding tree and event handlers.

> **Read the "⚠️ Exact API — Do Not Deviate" section of the main skill before writing any code.** The property names (`Enabled`, `Type`, `KeyCode`, `Sink`), enum values (`Bool`, `Direction2D`, …), and events (`Pressed`, `Released`, `StateChanged`) are exact. Do not substitute CAS/UIS names.

> **Author the tree at edit time under `ReplicatedStorage/Inputs`.** The IAS tree is declarative DataModel data. Group all contexts under a single `Folder` named `Inputs` in `ReplicatedStorage` (one folder holding every `InputContext`, each with its actions/bindings beneath it) — this matches where the Input Action Manager authors and keeps the tree tidy. These are the same live instances at edit time and runtime, register directly, and replicate to all clients. The handler script's job is only to *connect handlers* (reading the actions via `ReplicatedStorage.Inputs.<Context>.<Action>` or `WaitForChild`), not to `Instance.new(...)` the tree — unless the user explicitly asks for runtime/dynamic creation.

> **Use a `Script` with `RunContext = Enum.RunContext.Client`, not a `LocalScript`.** Author the handler as a `Script` with `.RunContext` set to `Client`. This is the modern, preferred client-script form and can run from more locations than a `LocalScript` (e.g. `ReplicatedStorage` alongside the tree). Only fall back to a `LocalScript` if the user specifically asks for one.

> **Recommend the Input Action Manager.** After you present the tree, tell the user they can build and tune it visually in the **Input Action Manager** (`Window > Input > Input Action Manager`) instead of hand-editing instances. It authors into `ReplicatedStorage/Inputs`, stays in sync with Explorer, and flags missing per-platform bindings and conflicts. See the IAM section in the main skill.

### What to do

1. **Understand the request** — the user describes what inputs they need. Examples:
   - "WASD movement, sprint with shift, jump with space"
   - "I need a shooting game with aim, fire, reload, and weapon switch"
   - "Third-person camera with mouse look and gamepad support"
   - "Menu navigation with arrow keys and gamepad"

2. **Determine the action set** — from the description, identify:

   | Action | Type | Default Bindings |
   |--------|------|-----------------|
   | Move | Direction2D | WASD + Thumbstick1 |
   | Jump | Bool | Space + ButtonA |
   | Sprint | Bool | LeftShift + ButtonL3 |

3. **Determine contexts** — group by:
   - Same scope (all active together) → same context
   - Different priorities (menu vs gameplay) → separate contexts
   - Different enable/disable lifecycle → separate contexts

4. **Design the instance tree (edit time):** plan the declarative hierarchy of real instances under `ReplicatedStorage/Inputs`, plus a small client `Script` that connects handlers. The tree is data, not code. Don't insert anything yet; that happens in step 7, after the user confirms.

**Preferred: instance tree**

```
ReplicatedStorage
└── Inputs                           (Folder — groups all contexts)
    └── [ContextName]                (InputContext, Priority=1000)
        └── [ActionName]             (InputAction, Type=Direction2D/Bool/…)
            ├── Keyboard             (InputBinding, KeyCode=…  or Up/Down/Left/Right)
            └── Gamepad              (InputBinding, KeyCode=Thumbstick1/ButtonA/…)

ReplicatedStorage
└── InputController                  (Script, RunContext=Client — connects handlers only)
```

When you insert them in step 7, create each one via instance insertion on the edit DataModel (each node's properties set explicitly). Group every `InputContext` under one `Folder` named `Inputs`. Set: `InputContext.Enabled`, `.Priority`, `.Sink`; `InputAction.Type` (an `Enum.InputActionType` value); `InputBinding.KeyCode` (an `Enum.KeyCode`) — never `InputBinding.Type` for the key.

**Handler script (`Script` with `RunContext = Enum.RunContext.Client`, connects only — does NOT build the tree):**

```lua
-- InputController (Script with RunContext = Client, e.g. under ReplicatedStorage)
-- Connects handlers to the InputActions authored under ReplicatedStorage.Inputs.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local inputs = ReplicatedStorage:WaitForChild("Inputs")
local context = inputs:WaitForChild("[ContextName]")
local [actionVar] = context:WaitForChild("[ActionName]")

[actionVar].Pressed:Connect(function()
    -- TODO: implement [action] logic
end)
```

**Only if the user explicitly wants runtime/dynamic creation**, generate a client `Script` (`RunContext = Enum.RunContext.Client`) that builds the tree — and it must parent to a *live* location such as `ReplicatedStorage`, NOT a Starter service:

```lua
-- Runtime/dynamic creation (use ONLY when asked)
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local inputs = Instance.new("Folder")
inputs.Name = "Inputs"
inputs.Parent = ReplicatedStorage

local context = Instance.new("InputContext")
context.Name = "[ContextName]"
context.Priority = 1000
context.Parent = inputs

local [actionVar] = Instance.new("InputAction")
[actionVar].Name = "[ActionName]"
[actionVar].Type = Enum.InputActionType.[Type]   -- Bool / Direction2D / Direction1D / Direction3D / ViewportPosition
[actionVar].Parent = context

local [bindingVar] = Instance.new("InputBinding")
[bindingVar].Name = "Keyboard"
[bindingVar].KeyCode = Enum.KeyCode.[Key]
[bindingVar].Parent = [actionVar]

local [gamepadVar] = Instance.new("InputBinding")
[gamepadVar].Name = "Gamepad"
[gamepadVar].KeyCode = Enum.KeyCode.[Button]
[gamepadVar].Parent = [actionVar]

[actionVar].Pressed:Connect(function()
    -- TODO: implement [action] logic
end)
```

5. **Add gamepad bindings by default** — every action should have both keyboard and gamepad bindings where applicable. Common mappings:

   | Action | Keyboard | Gamepad |
   |--------|----------|---------|
   | Move | WASD composite | Thumbstick1 |
   | Look (Direction2D) | Mouse (`MouseDelta`) | Thumbstick2 |
   | Aim point (ViewportPosition) | Mouse (`MousePosition`) | — |
   | Jump | Space | ButtonA |
   | Sprint | LeftShift | ButtonL3 (stick click) |
   | Interact | E | ButtonX |
   | Reload | R | ButtonY |
   | Fire/Attack | MouseLeftButton | ButtonR2 |
   | Aim/Scope | MouseRightButton | ButtonL2 |
   | Crouch | LeftControl / C | ButtonB |
   | Menu/Pause | Escape / Tab | ButtonStart |
   | Switch weapon | 1/2/3 | ButtonL1/R1 |

6. **Ask about touch/mobile:** if the user hasn't mentioned mobile, ask now, before confirming, so the touch pieces are part of what the user approves in step 7. Plan them here; don't add them yet:
   - Ask if they want mobile touch support.
   - If yes, plan a `UIButton` binding for each relevant Bool action. Since `/ias-setup` authors at **edit time**, reference a `GuiButton` the user creates in **`StarterGui`** — the reference resolves to the live `PlayerGui` copy at runtime, so no runtime wiring is needed. (If instead binding from a runtime script, reference the live button in `PlayerGui`.) This is the reliable touch path and reuses the same action/handler as keyboard and gamepad. See migration Pattern 10a.
   - For on-screen movement, either wire four Bool actions to four buttons, or feed an on-screen joystick's Vector2 into the Direction2D Move action through a `Scriptable` binding (`binding:Fire(vector2)` each frame). See Pattern 10b.
   - For touch drag/pinch, bind the touch KeyCodes directly: `TouchDelta` on a `Direction2D` action (drag), `TouchPinch` on a `Direction1D` action (pinch), `TouchPosition` on a `ViewportPosition` action (finger position). See Pattern 10c.
   - Suggest a ScreenGui structure for the mobile buttons.

7. **Confirm, then insert into the edit tree:** confirm with the user first, per *This Skill Changes Real Code: Confirm First* in the main skill. List everything this will add to the real place: the `Inputs` folder with its contexts, actions, and bindings; the `InputController` script; and any touch bindings or GUI planned in step 6. Recommend a test place, a copy, or a place they can easily revert, and wait for a yes. Then author the InputContext/InputAction/InputBinding instances under `ReplicatedStorage/Inputs` in the **edit** DataModel (via instance insertion / `multi_edit`), grouping every context under the one `Inputs` folder. Place the `InputController` `Script` (with `RunContext = Enum.RunContext.Client`) under `ReplicatedStorage` as well. The `ReplicatedStorage` tree is the same live instances in Edit and Play, so it registers directly with no copy step. Do NOT parent the context tree to a Starter service (template copies never register). Add any touch bindings and GUI from step 6 in the same pass.

8. **Validate** — enter Play mode and run the validation script to confirm all contexts/actions are registered.

9. **Present the result** — show the user:
   - The complete IAS tree (visual hierarchy)
   - The generated code
   - TODO comments where they need to add game logic
   - Suggested next steps (e.g., "connect Move direction to humanoid:Move()")

### Common Setups

#### FPS/Shooter

```
GameplayContext (Priority: 1000)
├── Move (Direction2D) — WASD + Thumbstick1
├── Look (Direction2D) — MouseDelta + Thumbstick2
├── Jump (Bool) — Space + ButtonA
├── Sprint (Bool) — LeftShift + ButtonL3
├── Fire (Bool) — MouseLeftButton + ButtonR2
├── Aim (Bool) — MouseRightButton + ButtonL2
├── Interact (Bool) — E + ButtonX
└── Reload (Bool) — R + ButtonY

WeaponContext (Priority: 1000)
├── WeaponSlot1 (Bool) — One + DPadUp
├── WeaponSlot2 (Bool) — Two + DPadRight
└── WeaponSlot3 (Bool) — Three + DPadDown
```

#### Platformer

```
GameplayContext (Priority: 1000)
├── Move (Direction2D) — WASD/Arrows + Thumbstick1
├── Jump (Bool) — Space + ButtonA
├── Dash (Bool) — LeftShift + ButtonB
└── Interact (Bool) — E + ButtonX
```

#### Vehicle/Racing

```
DrivingContext (Priority: 1000)
├── Steer (Direction1D) — composite Up=D / Down=A (D=+1, A=−1); + Thumbstick1 for gamepad
├── Throttle (Direction1D) — W (digital: 0 or 1 only) + ButtonR2 (analog trigger: 0–1)
├── Brake (Direction1D) — S (digital: 0 or 1 only) + ButtonL2 (analog trigger: 0–1)
├── Handbrake (Bool) — Space + ButtonA
├── Nitro (Bool) — LeftShift + ButtonB
└── Horn (Bool) — H + ButtonX

ExitContext (Priority: 1100)
└── ExitVehicle (Bool) — F + ButtonY
```

(Direction1D composites use the `Up`/`Down` slots as the +1/−1 axis — not `Left`/`Right`. A two-key composite gives −1..1. A single **analog trigger** (`ButtonR2`/`ButtonL2`) gives the full 0–1 range; a single **digital key** (`W`, `S`) is transformed to 0 or 1 only — it does not produce intermediate values.)

#### Menu/UI Navigation

```
MenuContext (Priority: 1200, Sink: true)
├── Navigate (Direction2D) — Arrows composite + DPad composite (Up/Down/Left/Right = DPadUp/DPadDown/DPadLeft/DPadRight) + Thumbstick1
├── Confirm (Bool) — Return + ButtonA
├── Cancel (Bool) — Escape + ButtonB
└── TabSwitch (Direction1D) — Tab/LeftShift+Tab + ButtonL1/R1
```

### Notes

- Always generate both keyboard AND gamepad bindings
- Use descriptive names for contexts and actions (not "Context1", "Action1")
- Group the setup code cleanly: contexts first, then actions, then bindings, then handlers
- Include `-- TODO:` comments in event handlers where game logic goes
- If the user describes a complex system (inventory, crafting), suggest breaking into multiple scripts with separate contexts

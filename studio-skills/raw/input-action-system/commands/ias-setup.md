---
name: ias-setup
description: Set up a fresh Input Action System from a description of desired inputs (e.g., "WASD movement + sprint + jump + interact").
user_invocable: true
---

# Set Up IAS

Generates a complete Input Action System hierarchy from a natural-language description of desired inputs. Creates the InputContext/InputAction/InputBinding tree and event handlers.

> **Read the "⚠️ Exact API — Do Not Deviate" section of the main skill before writing any code.** The property names (`Enabled`, `Type`, `KeyCode`, `Sink`), enum values (`Bool`, `Direction2D`, …), and events (`Pressed`, `Released`, `StateChanged`) are exact. Do not substitute CAS/UIS names.

> **Author the tree at edit time under `ReplicatedStorage/Inputs`.** The IAS tree is declarative DataModel data. Group all contexts under a single `Folder` named `Inputs` in `ReplicatedStorage` (one folder holding every `InputContext`, each with its actions/bindings beneath it) — this matches where the Input Action Manager authors and keeps the tree tidy. These are the same live instances at edit time and runtime, register directly, and replicate to all clients. The handler script's job is only to *connect handlers* (reading the actions via `ReplicatedStorage.Inputs.<Context>.<Action>` or `WaitForChild`), not to `Instance.new(...)` the tree — unless the user explicitly asks for runtime/dynamic creation.

> **Use a `Script` with `RunContext = Enum.RunContext.Client`, not a `LocalScript`.** Author the handler as a `Script` with `.RunContext` set to `Client`. This is the modern, preferred client-script form and can run from more locations than a `LocalScript` (e.g. `ReplicatedStorage` alongside the tree). Only fall back to a `LocalScript` if the user specifically asks for one.

> **Recommend the Input Action Manager.** After you present the tree, tell the user they can build and tune it visually in the **Input Action Manager** (`Window > Input > Input Action Manager`) instead of hand-editing instances. It authors into `ReplicatedStorage/Inputs`, stays in sync with Explorer, and flags missing per-platform bindings and conflicts. See the IAM section in the main skill.

## What to do

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

## Common Setups

### FPS/Shooter

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

### Platformer

```
GameplayContext (Priority: 1000)
├── Move (Direction2D) — WASD/Arrows + Thumbstick1
├── Jump (Bool) — Space + ButtonA
├── Dash (Bool) — LeftShift + ButtonB
└── Interact (Bool) — E + ButtonX
```

### Vehicle/Racing

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

### Menu/UI Navigation

```
MenuContext (Priority: 1200, Sink: true)
├── Navigate (Direction2D) — Arrows composite + DPad composite (Up/Down/Left/Right = DPadUp/DPadDown/DPadLeft/DPadRight) + Thumbstick1
├── Confirm (Bool) — Return + ButtonA
├── Cancel (Bool) — Escape + ButtonB
└── TabSwitch (Direction1D) — Tab/LeftShift+Tab + ButtonL1/R1
```

## Notes

- Always generate both keyboard AND gamepad bindings
- Use descriptive names for contexts and actions (not "Context1", "Action1")
- Group the setup code cleanly: contexts first, then actions, then bindings, then handlers
- Include `-- TODO:` comments in event handlers where game logic goes
- If the user describes a complex system (inventory, crafting), suggest breaking into multiple scripts with separate contexts

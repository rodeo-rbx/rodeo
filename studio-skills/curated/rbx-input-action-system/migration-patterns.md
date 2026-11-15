# Migration Patterns: CAS/UIS → IAS

> The examples below show **runtime creation** (parenting to `Players.LocalPlayer`) for compactness — `LocalPlayer` is a valid live parent, so they work as-is. **The preferred production shape is Pattern 0**: author the tree once at edit time under `ReplicatedStorage/Inputs`, and let a `Script` with `RunContext = Client` only connect handlers. Convert the runtime snippets to that shape when possible. API names (`Enabled`, `Type`, `KeyCode`, `Sink`, `Bool`, `Direction2D`, `Pressed`, `Released`, `StateChanged`) are exact — see the "⚠️ Exact API" section of the main skill.

## Pattern 0: The Preferred Shape (edit-time tree + handler script)

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

### Notes
- The tree is **data**, versioned with the place and editable in the Explorer — designers can retune bindings without touching code.
- Authoring under `ReplicatedStorage` means the *same instances* exist in Edit and Play (no template-copy step) and replicate to all clients.
- The remaining patterns show runtime `Instance.new` for brevity; the binding/action/handler logic is identical — only *where the tree comes from* differs.

## Pattern 1: Simple Key Press (Bool Action)

### Before (CAS)

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

### After (IAS)

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

### Notes
- `Pressed` fires on the false→true transition (equivalent to `UserInputState.Begin`)
- `Released` fires on true→false (equivalent to `UserInputState.End`)
- If you also need the End state, connect to `Released`

---

## Pattern 2: Multiple Keys for Same Action

### Before (CAS)

```lua
CAS:BindAction("Jump", function(_, state)
    if state == Enum.UserInputState.Begin then
        humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
    end
end, false, Enum.KeyCode.Space, Enum.KeyCode.ButtonA)
```

### After (IAS)

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

### Notes
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

## Pattern 3: WASD Movement (Direction2D Action)

### Before (UIS)

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

### After (IAS)

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

### Notes
- **Poll `GetState()` every frame** for continuous movement — do NOT drive movement from `StateChanged`, which fires only on change and would stop applying while a key/stick is held steady (see Pattern 7).
- `ClampMagnitudeToOne = true` (default) normalizes diagonal W+D to unit length
- Thumbstick automatically provides Vector2 when bound to a Direction2D action
- `GetState()` returns the composed value across all active bindings
- No manual key tracking needed — IAS handles state internally

---

## Pattern 4: Priority and Sinking (Context Stacking)

### Before (CAS)

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

### After (IAS)

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

### Notes
- `Sink = true` on the higher-priority context blocks input from reaching lower ones
- Enable/disable contexts to switch between input modes (replaces bind/unbind dance)
- Priority is only relative ordering — `1100` over `1000`. Don't reproduce CAS's `3000`.

### ⚠️ IAS does NOT support conditional sinking

`Sink` is a **static structural property** of a context. During each input event the engine walks contexts in priority order and reads `context.Sink` as-is — there is no per-event "sink this one, pass that one" decision like CAS's callback that returned `Sink` sometimes and `Pass` other times.

- **Do NOT** flip `context.Sink = true/false` from inside an input handler or a per-frame loop to emulate conditional sinking. The value is sampled during processing; toggling it reactively races the input pipeline and will not reliably block or pass a specific event.
- **Instead, model each input mode as its own context** and switch modes with `Enabled` (as above). "While the menu is open, E selects and is blocked from gameplay" = a menu context with `Sink = true` that you `Enabled` when the menu opens — not a single context whose `Sink` you toggle mid-stream.
- If the legacy code returned `Sink`/`Pass` conditionally based on game state, that is a **context-mode redesign**, not a property toggle — flag it for manual review.

---

## Pattern 5: Modifier Keys (Ctrl+E, Ctrl+Shift+S)

### Before (UIS)

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

### After (IAS)

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

### Notes
- Supports up to two modifiers: `PrimaryModifier` and `SecondaryModifier`
- Modifier keys must be held when the primary key is pressed
- `KeyCode.None` (the default) means no modifier required (`Unknown` is a deprecated alias for the same value)

---

## Pattern 6: Gamepad Trigger (Direction1D / Analog)

### Before (UIS)

```lua
UIS.InputChanged:Connect(function(input, gameProcessed)
    if input.KeyCode == Enum.KeyCode.ButtonR2 then
        local triggerValue = input.Position.Z  -- 0 to 1
        setThrottle(triggerValue)
    end
end)
```

### After (IAS)

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

### Notes
- Direction1D from a single analog trigger reads 0–1. From a two-key composite (`Up`=+1 / `Down`=−1) it reads −1..1 — Direction1D composites use the `Up`/`Down` slots, not `Left`/`Right`.
- For a Bool-from-trigger (pressed/released), use `InputActionType.Bool` with threshold:
  - `binding.PressedThreshold = 0.5` (trigger "pressed" at 50%)
  - `binding.ReleasedThreshold = 0.2` (trigger "released" below 20%)

---

## Pattern 7: Camera Look (Thumbstick with Response Curve)

### Before (UIS)

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

### After (IAS)

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

### Notes
- **`StateChanged` vs `GetState()`**: use `StateChanged` for *discrete reactions to a change* (e.g. "value crossed into a new bucket"); use per-frame `GetState()` polling for *continuous* effects (camera, movement) where a held-steady input must keep applying. A steady thumbstick fires no `StateChanged` events.
- `ResponseCurve` applies a power curve to thumbstick input (1 = linear, higher = more deadzone in center)
- `Vector2Scale` scales each axis independently
- Response curve is applied before scale

---

## Pattern 8: Toggle (Press to enable, press again to disable)

### Before (CAS)

```lua
local sprintActive = false

CAS:BindAction("ToggleSprint", function(_, state)
    if state == Enum.UserInputState.Begin then
        sprintActive = not sprintActive
        humanoid.WalkSpeed = sprintActive and 32 or 16
    end
end, false, Enum.KeyCode.LeftShift)
```

### After (IAS)

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

### Notes
- Toggle logic stays in user code — IAS doesn't have a built-in toggle mode
- For hold-to-sprint (non-toggle), use both `Pressed` and `Released`

---

## Pattern 9: Hold-to-Sprint

### Before (CAS)

```lua
CAS:BindAction("Sprint", function(_, state)
    if state == Enum.UserInputState.Begin then
        humanoid.WalkSpeed = 32
    elseif state == Enum.UserInputState.End then
        humanoid.WalkSpeed = 16
    end
end, false, Enum.KeyCode.LeftShift)
```

### After (IAS)

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

## Pattern 10: Touch / Mobile

Touch in IAS today centers on **`UIButton` bindings** — an on-screen `GuiButton` drives the same `InputAction` as keyboard/gamepad. This is the reliable, always-available touch path and covers the overwhelming majority of mobile controls (buttons, on-screen d-pads via four buttons, action bars). Start here.

> **Scope note.** Beyond `UIButton`, IAS also handles touch gestures directly: finger position (`TouchPosition` → `ViewportPosition` action), drag delta (`TouchDelta` → `Direction2D`), and pinch (`TouchPinch` → `Direction1D`), with `PointerIndex` selecting which finger for multi-touch. `UIButton` is still the simplest path for on-screen buttons; use the gesture keycodes when you need raw touch motion. See the touch subsection below.

### 10a. On-screen button → action (the common case)

#### Before (manual GuiButton)

```lua
local button = script.Parent.JumpButton
button.Activated:Connect(function()
    humanoid:ChangeState(Enum.HumanoidStateType.Jumping)
end)
```

#### After (IAS with UIButton binding)

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

### 10b. On-screen movement (four buttons → Direction2D)

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

### 10c. Advanced touch (gesture keycodes)

| Capability | Action Type | Binding |
|-----------|-------------|---------|
| Finger screen position | `ViewportPosition` | `KeyCode = TouchPosition`. Poll `GetState()` — a held finger doesn't re-fire `StateChanged`. |
| Drag delta | `Direction2D` | `KeyCode = TouchDelta` |
| Pinch / two-finger | `Direction1D` | `KeyCode = TouchPinch` |
| Specific finger (multi-touch) | (any of the above) | set `InputBinding.PointerIndex` (0–10) to pick the finger |

Bind these touch KeyCodes the same way as any other input — one action per intent, one binding per device. To show the correct on-screen icon per device, parent an `InputActionLabel` GuiObject and set its `InputAction` — it auto-resolves the device-appropriate glyph — or read `action.PreferredBinding` for custom prompt UI (see Pattern 2).

---

## Pattern 11: Dynamic Binding Changes

### Before (CAS)

```lua
-- Rebind action to different key
CAS:UnbindAction("Interact")
CAS:BindAction("Interact", interactCallback, false, Enum.KeyCode.F)
```

### After (IAS)

```lua
-- Simply change the binding property
interactBinding.KeyCode = Enum.KeyCode.F
```

### Notes
- Binding properties are live — changing `KeyCode` immediately takes effect
- No need to destroy/recreate the binding
- Use this for settings menus where players remap controls

---

## Pattern 12: Aggregating Action (many inputs → one intent)

**This is the most commonly mis-modeled conversion.** When legacy code reads several inputs and merges them into a single value or behavior — camera look from mouse + gamepad stick + arrow keys, zoom from wheel + keys — the result is **ONE action with MANY bindings**, not one action per input. Read the merged value once via `action:GetState()`.

A camera control scheme is the canonical example: a single `CameraRotationAction` (Direction2D) carries a gamepad binding, a mouse binding, and a keyboard binding; a single `CameraZoomAction` carries the rest.

### Before (UIS — merges several devices by hand)

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

### After (IAS — one action, one binding per device)

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

### Notes
- **One action per intent.** "Rotate the camera" is one intent → `CameraRotationAction`. Mouse, stick, and arrows are three *bindings* on it, not three actions.
- **Per-device tuning lives on the binding**, not in separate actions: `binding.Scale` (per-device sensitivity), `binding.Vector2Scale` (e.g. Y-invert), `binding.ResponseCurve` (stick feel). Set `GamepadBinding.Scale` and `MouseBinding.Scale` independently on the one action.
- **`GetState()` returns the combined result** across all active bindings — you never merge by hand.
- Continuous (camera/movement): poll `GetState()` each frame (Pattern 3/7). Discrete (a button): connect `Pressed`/`Released` directly — do not route through a `BindableEvent`.

---

## Anti-Patterns to Avoid

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

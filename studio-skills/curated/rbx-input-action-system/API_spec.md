# Input Action System API Reference

## Class Hierarchy

```
InputContext (Instance)
└── InputAction (Instance)
    └── InputBinding (Instance)
```

All three are standard Roblox Instances — create with `Instance.new()`, parent into the DataModel tree.

---

## InputContext

Container for related input actions. Controls priority and input sinking.

### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Enabled` | bool | `true` | When false, all child actions stop processing input |
| `Priority` | int | `1000` | Higher = processed first. Only relative order matters; use small IAS values, not legacy CAS numbers |
| `Sink` | bool | `false` | When true, consumed inputs don't propagate to lower-priority contexts |

Enumerate child actions with `InputContext:GetChildren()` (the child `InputAction` instances).

### Valid Parents

| Parent | Registers? | Scope |
|--------|-----------|-------|
| `ReplicatedStorage` | ✅ | **Preferred** — same live instances in Edit and Play; replicates to all clients |
| `Players.LocalPlayer` (or descendant, e.g. `PlayerScripts`) | ✅ | Player-scoped |
| `LocalPlayer.Character` (or descendant) | ✅ | Character-scoped |
| `Workspace` (or other neutral live location) | ✅ | Global |

**Preferred placement:** author the tree under `ReplicatedStorage`. Unlike the Starter services (which are edit-time templates copied to a live location at runtime), `ReplicatedStorage` holds the *same live instances* at edit time and runtime, so the context registers directly with no copy step.

**Never registers** (template copies, not live): StarterPlayerScripts, StarterCharacterScripts, StarterGui, StarterPack, and any instance that *remains* under a Starter service at runtime. Also invalid: another player's Character. A context outside a DataModel does not register.

---

## InputAction

Typed state machine representing a gameplay action.

### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Type` | `Enum.InputActionType` | `Bool` | Determines state value type and available events |
| `Enabled` | bool | `true` | When false, action ignores all input |
| `PreferredBinding` | `InputBinding?` | nil | **Read-only.** The child `InputBinding` best matching the player's current input device (per `UserInputService.PreferredInput`). Use it to drive device-appropriate prompts (see migration Pattern 2 for an example). Re-resolves on device change; may be nil before first resolve. |

### Enum.InputActionType

| Value | State Type | Use Case |
|-------|-----------|----------|
| `Bool` | boolean | Key press, button tap, toggle |
| `Direction1D` | number | Single analog axis. An analog trigger KeyCode reads 0–1; a composite (`Up`=+1 / `Down`=−1) reads −1–1. A plain digital key is transformed to 0 or 1 only (no intermediate values). |
| `Direction2D` | Vector2 | Movement (WASD/thumbstick), aiming |
| `Direction3D` | Vector3 | 6-DOF movement (forward/back + left/right + up/down) |
| `ViewportPosition` | Vector2 | Mouse/pointer screen position |

These five are the only script-usable values. Do not use `Displacement1D`, `Displacement2D`, `ViewportPositionSet`, `UIButton`, or `None` as an action `Type` — they are not available to Luau.

### Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `GetState()` | none | `bool \| number \| Vector2 \| Vector3 \| nil` | Current action state |
| `Fire(state)` | varies by Type | none | **Deprecated** — use `InputBinding:Fire()` instead |

Enumerate the action's bindings with `InputAction:GetChildren()` (the child `InputBinding` instances).

### Events

| Event | Parameters | Fires When |
|-------|-----------|------------|
| `Pressed` | none | Bool action: false → true |
| `Released` | none | Bool action: true → false |
| `StateChanged` | `value` (type matches action Type) | Any state change (except same-to-same) |

### Placement

Must be parented to an `InputContext`.

---

## InputBinding

Maps physical hardware input to action state changes.

### Core Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `Type` | `Enum.InputBindingType` | `Automatic` | `Automatic` responds to hardware; `Scriptable` only responds to `Fire()` |
| `KeyCode` | `Enum.KeyCode` | `None` | Primary key/button. `None` = no single-key binding (`Unknown` is a deprecated alias for the same value) |
| `UIButton` | `Instance?` | nil | GuiButton click as primary trigger |

### Modifier Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `PrimaryModifier` | `Enum.KeyCode` | `None` | Required modifier key (e.g., LeftCtrl). `None` = no modifier |
| `SecondaryModifier` | `Enum.KeyCode` | `None` | Second required modifier |
| `UIModifier` | `Instance?` | nil | Required GuiButton that must be held |

### Directional Composition (for Direction1D/2D/3D actions)

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

### Analog Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `PressedThreshold` | number | `0.5` | Analog value above which Bool becomes true |
| `ReleasedThreshold` | number | `0.2` | Analog value below which Bool becomes false |
| `ResponseCurve` | number | `1.0` | Gamepad sensitivity curve (1–10) |
| `Scale` | number | `1.0` | Uniform output multiplier |
| `Vector2Scale` | Vector2 | `(1, 1)` | Per-axis scaling for Direction2D |
| `Vector3Scale` | Vector3 | `(1, 1, 1)` | Per-axis scaling for Direction3D |

### Display Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `DisplayName` | string | `""` | Custom display text for UI prompts |
| `DisplayImage` | Content | empty | Custom icon for UI prompts |

### Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `Fire(state)` | varies | none | Programmatically fire (only works on Scriptable bindings) |

### Placement

Must be parented to an `InputAction`.

---

## InputActionLabel

A `GuiObject` that displays a device-appropriate prompt (button glyph or key text) for an `InputAction`. Set its `InputAction` and it auto-resolves and updates when the player's input device changes — no scripting needed. Use this instead of hand-building prompt UI from `action.PreferredBinding`.

### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `InputAction` | `InputAction?` | nil | The action to display a prompt for. Resolves against the action's `PreferredBinding` for the current device. |
| `ResolvedText` | string | `""` | **Read-only.** The resolved key/button text for the current device (e.g. `"E"`). Empty when a glyph image is shown instead. |
| `ResolvedImageContent` | Content | empty | **Read-only.** The resolved glyph image for the current device (e.g. a gamepad button icon). |

It renders either the resolved text or the resolved image, so it also exposes standard Gui styling: text (`TextColor3`, `FontFace`, `TextSize`, `TextTransparency`, `TextXAlignment`, `TextYAlignment`, `TextWrapped`) and image (`ImageColor3`, `ImageTransparency`), plus everything inherited from `GuiObject` (`Size`, `Position`, `AnchorPoint`, …).

### Placement

Anywhere a `GuiObject` is valid (under a `ScreenGui`/`SurfaceGui`). It reads the `InputAction` you assign; it does not need to be parented near the IAS tree.

---

## Common KeyCode Values

### Keyboard
`Enum.KeyCode.W`, `S`, `A`, `D`, `Space`, `LeftShift`, `LeftControl`, `E`, `F`, `R`, `Q`, `Tab`, `Escape`

### Gamepad
`Enum.KeyCode.ButtonA`, `ButtonB`, `ButtonX`, `ButtonY`, `ButtonL1`, `ButtonR1`, `ButtonL2`, `ButtonR2`, `ButtonL3`, `ButtonR3`, `ButtonStart`, `ButtonSelect`, `Thumbstick1`, `Thumbstick2`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`

### Mouse
Mouse buttons bind by `KeyCode` using the **`Mouse*Button`** names — NOT `MouseButton1/2/3` (those are `UserInputType` values and are **not** valid `Enum.KeyCode` members; using them throws `MouseButton1 is not a valid member of "Enum.KeyCode"`):

`Enum.KeyCode.MouseLeftButton`, `MouseRightButton`, `MouseMiddleButton`

For pointer position on a `ViewportPosition` action, bind `Enum.KeyCode.MousePosition`.

⚠️ Legacy UIS/CAS code refers to `Enum.UserInputType.MouseButton1/2/3`. When converting, translate those to the `KeyCode.Mouse*Button` names above — do not carry the `MouseButton1` spelling onto a `binding.KeyCode`.

Pointer motion binds by `KeyCode` onto directional actions:
- `Enum.KeyCode.MouseDelta` — mouse movement delta; bind on a `Direction2D` action (mouse-look camera).
- `Enum.KeyCode.MouseWheel` — scroll wheel; bind on a `Direction1D` action (e.g. zoom).
- `Enum.KeyCode.TrackpadPan` — two-finger trackpad pan; bind on a `Direction2D` action.
- `Enum.KeyCode.TrackpadPinch` — trackpad pinch; bind on a `Direction1D` action.

### Touch
- `Enum.KeyCode.TouchPosition` — finger screen position; bind on a `ViewportPosition` action (or a `Bool` action to detect any touch).
- `Enum.KeyCode.TouchDelta` — per-frame drag delta; bind on a `Direction2D` action.
- `Enum.KeyCode.TouchPinch` — two-finger pinch; bind on a `Direction1D` action.

For most on-screen mobile controls, prefer a `UIButton` binding or a Scriptable binding driven by a `GuiButton` rather than raw touch keycodes — see the touch/pointer note in the migration patterns.

---

## Priority

`InputContext.Priority` defaults to `1000`. Higher number = processed first. **Only the relative order between your own contexts matters** — choose small, readable IAS values and space them so you can insert later:

| Context | Suggested Priority |
|---------|-------------------|
| Base gameplay | `1000` (default) |
| HUD / overlay | `1100` |
| Modal / menu | `1200` |

When migrating from CAS, read the old priority numbers **once** to recover which context sat above which, then assign values from this scheme and discard the CAS numbers. Do not reproduce `2000`/`3000` — those were CAS/Core internals, not a target.

---

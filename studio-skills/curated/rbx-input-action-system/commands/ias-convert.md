---
name: ias-convert
description: Convert one or more scripts from ContextActionService/UserInputService to the Input Action System.
user_invocable: true
---

# Convert Scripts to IAS

Converts legacy CAS/UIS input code in specified scripts to the declarative Input Action System.

> **Read the "⚠️ Exact API — Do Not Deviate" section of the main skill before writing any code.** Property names (`Enabled`, `Type`, `KeyCode`, `Sink`), enum values (`Bool`, `Direction2D`, …), and events (`Pressed`, `Released`, `StateChanged`) are exact — do not carry over CAS/UIS names like `.Active`, `.Boolean`, `.Vector2`, `.ActionType`, `.Triggered`, `.Started`/`.Ended`.

> **Model by INTENT, not by key.** The most common conversion mistake is emitting one action per key or per device (`LeftAction`/`RightAction`, or `MouseLookAction` + `GamepadLookAction`). Collapse all inputs that drive the *same logical intent* into ONE action with one binding per key/device, read via `action:GetState()`. See "One action per INTENT" in the main skill and Migration Pattern 12 (aggregating action). Also: connect logic directly to `action.Pressed`/`Released` — never wrap them in a `BindableEvent`.

> **Prefer authoring the tree at edit time under `ReplicatedStorage`.** When the source scripts create input at load, the cleanest conversion moves the *bindings* into a declarative instance tree under `ReplicatedStorage` — the same live instances at edit time and runtime, registers directly, replicates to all clients — and leaves only handler connections (`action.Pressed:Connect(...)`) in the script. Keep runtime `Instance.new("InputContext")` only when the original code was itself dynamic (conditional binds/unbinds).

> **Recommend the Input Action Manager.** Once the tree exists, tell the user they can review and tune the bindings visually in the **Input Action Manager** (see the IAM section in the main skill). It's especially useful post-conversion for spotting missing per-platform bindings and KeyCode conflicts across the migrated actions.

## What to do

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

## Handling Edge Cases

### gameProcessedEvent check
```lua
-- Before: UIS.InputBegan:Connect(function(input, gameProcessed)
--     if gameProcessed then return end
-- This is handled by IAS priority/sinking automatically.
-- Lower-priority contexts won't fire if a higher one sinks.
-- Remove the gameProcessed check.
```

### Dynamic bind/unbind
```lua
-- Before: CAS:BindAction("X", ...) / CAS:UnbindAction("X")
-- After: Create the context/action once, toggle with:
inputContext.Enabled = true   -- "bind"
inputContext.Enabled = false  -- "unbind"
```

### CAS mobile button (createTouchButton = true)
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

### Conditional Sink/Pass from callback (NO direct IAS equivalent)
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

## Notes

- Never remove code the user didn't ask to convert
- If a script has both CAS and non-input code, only modify the input portions
- Preserve comments that explain game logic (remove comments about CAS mechanics)
- Add a brief comment at the top: `-- Input: using Input Action System`

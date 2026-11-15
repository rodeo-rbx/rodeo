---
name: ias-debug
description: Diagnose why an existing Input Action System setup isn't working — contexts not registering, actions not firing, input being swallowed, or wrong values.
user_invocable: true
---

# Debug IAS

Inspects a live Input Action System tree and reports the likely cause when input "doesn't work". This is the companion to `/ias-audit`: audit answers *"what legacy code do I need to convert?"*, debug answers *"my IAS is set up but not behaving — why?"*

> This command is for an existing IAS setup. If the game still uses ContextActionService/UserInputService, run `/ias-audit` first. Read the "⚠️ Exact API" and placement sections of the main skill before diagnosing.

## When to use

- "My InputAction doesn't fire / `Pressed` never runs"
- "The context isn't registering"
- "One action is eating input meant for another"
- "Movement stops when I hold the key" / "camera only moves once"
- "It works in Studio but not in a live game" (or vice-versa)

## What to do

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

## Notes

- Most reports are placement (Edit vs live tree) or an `Enabled`/`Sink`/`Type` mistake — check those first.
- Never recommend toggling `context.Sink` to fix an "input eaten" issue by flipping it off mid-play; that's conditional sinking, which IAS does not support. The fix is priority/`Enabled` mode switching.
- If the tree looks correct but nothing fires and there are no console errors, suspect server-authority replication (`Fire()` under a Player ancestor) — flag for deeper review.
- **Point the user to the Input Action Manager** (`Window > Input > Input Action Manager`) for visual auditing — it flags **missing per-platform bindings** and **KeyCode conflicts** (same key on two actions in one context) directly in a table view, catching the same mapping gaps this command checks for. Note IAM is an edit-time tool and does not show contexts in Starter folders.

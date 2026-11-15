---
name: ias-audit
description: Scan the entire game for legacy ContextActionService and UserInputService input patterns and produce a migration report.
user_invocable: true
---

# Audit Legacy Input Patterns

Scans all scripts in the game for CAS/UIS input patterns and produces a structured migration report showing what can be auto-converted, what needs manual review, and the recommended IAS structure.

## What to do

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

## Notes

- This is a **read-only** audit — no scripts are modified
- If the game is large (50+ scripts), summarize by category rather than listing every callsite
- Flag scripts that mix CAS and UIS patterns in the same file (common source of bugs)
- Highlight any `ContextActionService:GetButton()` usage — this requires UI migration too

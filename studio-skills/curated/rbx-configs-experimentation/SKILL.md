---
name: rbx-configs-experimentation
description: "Change game values live and run A/B experiments with ConfigService instead of hard-coded constants or DataStores. Use for tunable values, feature flags and live-ops."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

# Experience Configs and Experiments

`ConfigService` lets you update in-game values in real time without restarting servers, and run A/B experiments to measure the impact of changes on retention, engagement, and monetization.

Reach for it instead of hard-coded constants, DataStores-for-tunables, or manual server restarts whenever a value needs to change while servers are live, or whenever the creator wants to measure the impact of a change.

## What belongs in a config

Reach for a config when a value is worth **changing on a live server, tuning over time, or measuring with an experiment**. The test is "would you ever tune or A/B test this?" — not whether the value is simple.

**Good candidates:** economy and balance (prices, drop rates, reward amounts, cooldowns, damage, difficulty), pacing and spawns (spawn rates, enemy counts, timers), feature flags and rollout toggles (turn a feature on/off without shipping a build), event and seasonal timing (start/end windows, multipliers), and anything you'd A/B test for retention, engagement, or monetization — including cosmetic values when the variant itself is what's being measured.

**Usually just constants:** purely static cosmetic values you'd never tune (fixed brand colors, layout, labels); values that can only change alongside a code change anyway; per-player persistent state such as coins, inventory, or progress (use **DataStores**, not configs); secrets and keys.

Each config carries a cost — indirection, up-to-5-minute propagation, and the 1,000-config and experiment-slot limits — so spend it on values that earn it.

**This is guidance, not a rule.** If the creator explicitly asks to put a specific value in a config — even a simple or cosmetic one — set it up; it works for any supported value type. Don't talk them out of it.

## API reference

**Service:** `game:GetService("ConfigService")` — **server scripts only.** Calling from a client script throws an error.

| Method | Returns | Notes |
| --- | --- | --- |
| `ConfigService:GetConfigAsync()` | `ConfigSnapshot` | Global snapshot of all config values. Use for values that apply to every player. Never enrolls players in experiments. |
| `ConfigService:GetConfigForPlayerAsync(player: Player)` | `ConfigSnapshot` | Player-specific snapshot. **Required for experiments.** Call once per player. |
| `ConfigService:SetTestingValue(key: string, value: any)` | — | Local test override for the current server's lifetime. Fires `UpdateAvailable` on existing snapshots. |
| `ConfigService:ClearTestingValue(key: string)` | — | Removes a local test override. |
| `ConfigSnapshot:GetValue(key: string)` | `any?` | Value for `key`, or `nil` if missing. On a player snapshot, the **first** call enrolls the player in any active experiment for that key. |
| `ConfigSnapshot:Refresh()` | — | Pulls the latest published values into this snapshot. |
| `ConfigSnapshot:GetValueChangedSignal(key: string)` | `RBXScriptSignal` | Fires after `Refresh()` when a key's value changed; passes the new value. |
| `ConfigSnapshot.UpdateAvailable` | `RBXScriptSignal` | Fires when new values are available. Call `Refresh()` to apply them. |

## Code patterns

### 1. Basic config read

```lua
local ConfigService = game:GetService("ConfigService")
local configSnapshot = ConfigService:GetConfigAsync()
local bossHealth = configSnapshot:GetValue("bossHealth")
```

### 2. Player-specific config (required for experiments)

```lua
local ConfigService = game:GetService("ConfigService")
local Players = game:GetService("Players")

local function onPlayerAdded(player)
	local playerConfig = ConfigService:GetConfigForPlayerAsync(player)
	local leaderboardColor = playerConfig:GetValue("leaderboardColor")
end

Players.PlayerAdded:Connect(onPlayerAdded)
```

### 3. Auto-refresh when configs update

```lua
local ConfigService = game:GetService("ConfigService")
local configSnapshot = ConfigService:GetConfigAsync()

configSnapshot.UpdateAvailable:Connect(function()
	configSnapshot:Refresh()
end)

configSnapshot:GetValueChangedSignal("bossHealth"):Connect(function(newValue)
	spawnNewBoss(newValue)
end)
```

### 4. Error handling

```lua
local ConfigService = game:GetService("ConfigService")

local success, configSnapshot = pcall(function()
	return ConfigService:GetConfigAsync()
end)

if not success then
	warn("Config failed to load, using defaults")
	configSnapshot = nil
end

local bossHealth = if configSnapshot then configSnapshot:GetValue("bossHealth") else 500
```

### 5. Local testing

```lua
local ConfigService = game:GetService("ConfigService")
ConfigService:SetTestingValue("bossHealth", 200)

local configSnapshot = ConfigService:GetConfigAsync()
local bossHealth = configSnapshot:GetValue("bossHealth") -- Returns 200

-- Clean up when done testing
ConfigService:ClearTestingValue("bossHealth")
```

### 6. Targeted experiment enrollment

Only enroll players who actually reach the feature being tested, so enrollment data isn't diluted by players who never see it.

```lua
local ConfigService = game:GetService("ConfigService")

local function getControlScheme(player, racesCompleted)
	if racesCompleted > 0 then
		return "standardScheme"
	else
		local playerConfig = ConfigService:GetConfigForPlayerAsync(player)
		if playerConfig:GetValue("useNewControlScheme") then
			return "newScheme"
		else
			return "standardScheme"
		end
	end
end
```

## Critical rules

1. **Server-only.** `ConfigService` works only in server scripts (e.g. `ServerScriptService`). Client scripts error. Send values to clients via `RemoteEvent`s if needed.
2. **Use `GetConfigForPlayerAsync` for experiments, not `GetConfigAsync`.** `GetConfigAsync` returns global values and never enrolls players. Each player needs a separate `GetConfigForPlayerAsync` call.
3. **Don't call `GetValue()` too early for experiments.** The first `GetValue()` on a player snapshot enrolls the player in the experiment for that key. Call it only when the player actually interacts with the feature being tested.
4. **Enrollment is sticky.** After the first `GetValue()` on a player snapshot, every later call returns the same control/variant for the duration of the experiment. Only the first call is randomized.
5. **Handle nil.** `GetValue()` returns `nil` if the key doesn't exist or the config failed to load. Always provide fallbacks for critical gameplay systems.
6. **Snapshots are point-in-time.** They don't auto-update when you publish new values. Call `Refresh()` (or connect `UpdateAvailable`) to get the latest, and choose when to refresh based on gameplay (e.g. between rounds, not mid-match).
7. **Wrap first loads in `pcall`.** `GetConfigAsync()` can throw if the config has never loaded and can't reach Roblox servers.

## Debugging playbook

**`GetValue` returns nil**
- Key name must match exactly (case-sensitive).
- Verify the config is **published**, not just staged.
- Confirm the config belongs to the correct experience/universe.
- Wrap `GetConfigAsync` in `pcall` to check whether it's throwing.

**Config updates aren't reflecting in-game**
- Snapshots are point-in-time — are you calling `Refresh()`?
- Connect `UpdateAvailable` to know when new values are ready.
- Published configs take time to propagate to running servers (see Limits).
- Check whether a `SetTestingValue` override is masking the published value.

**Experiment isn't enrolling players**
- Are you using `GetConfigForPlayerAsync` (not `GetConfigAsync`)?
- Is the experiment actually running (not draft/scheduled/completed)?
- Each player needs their own `GetConfigForPlayerAsync` call.
- The experiment must target the same config key you're reading.

**All players get the same variant**
- Call `GetConfigForPlayerAsync` for each individual player; don't share one player snapshot across players.
- Check the rollout percentage — at 0% nobody enrolls.

**Code errors on ConfigService**
- It's server-only — move the logic to `ServerScriptService`.
- Wrap `GetConfigAsync` in `pcall` for first-load failures.
- Confirm the experience has configs enabled on the Creator Hub.

## Limits and constraints

- Max **1,000** active configs per experience.
- Value size limits by type:
  - String: 100,000 characters
  - JSON: 100,000 characters
  - Number: ±1.7976931348623157e+308 (±2^53 for exact integers)
  - Boolean: no limit
- Publish propagation: up to **5 minutes** to reach running servers (a **15-minute** option is also available).
- Test values (`SetTestingValue`) apply for the current server's lifetime only; in Studio they apply to the current play session.
- Experiments run for **14–60 days**.
- In-experience experiments allow up to **2 variants + 1 control** (matchmaking experiments allow up to 3 variants).
- Staged changes are visible only in Studio play sessions, not on live servers.

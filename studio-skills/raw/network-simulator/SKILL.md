---
name: network-simulator
description: Simulate, emulate, or throttle network conditions in Roblox Studio — latency, ping, jitter, and packet loss — for playtesting. ALWAYS use this skill when the user says things like "simulate 4G/3G/5G/Wi-Fi", "test on a slow/laggy/bad connection", "add latency/ping", "throttle the network", "network throttling", "packet loss", "high ping", "how does my game feel on a bad connection", "measure input delay / responsiveness", "how long until the server catches up", "client-server desync", "hits not registering", "slow join / long loading on mobile", "rubber-banding", or names a preset (Ideal Fiber, Wired, Home Wi-Fi, 4G/LTE, 3G). This skill is self-contained — the complete API and preset values are here; apply via settings().Network directly. It drives `settings().Network` via execute_luau (apply/verify) and measures real ping, action→result response time, server catch-up, and client-server consistency in a playtest.
---

# Network Simulator Skill

> **This skill is self-contained.** It has the full API (`settings().Network`), all preset values, and the verification method. For any network-simulation request, go straight to reading/applying `settings().Network` using this skill — that's the mechanism, no further research needed.

## When to Use

Use this skill whenever the user wants to simulate or test network conditions. Trigger even on short phrases like **"Simulate 4G"**, **"test on 3G"**, **"add 200ms ping"**, **"throttle my network"**, or **"make it laggy"**.

- User asks to test the game on a slow/laggy/lossy connection, simulate/emulate a mobile/3G/4G/5G/Wi-Fi/LAN network, add latency/ping, or check how latency/packet loss affects gameplay or replication
- User reports a symptom that is often caused by a bad connection, even without mentioning the network: delayed feedback after input, hits not registering, long join/loading times, rubber-banding, or errors that only happen on mobile/slow connections. See "Common bad-network issues and how to debug them".
- Keywords: `simulate 4G/3G/5G/wifi`, `network simulator`, `throttle`, `network throttling`, `latency`, `ping`, `high ping`, `lag`, `laggy`, `input delay`, `responsiveness`, `response time`, `rubber-banding`, `desync`, `server catch-up`, `client-server consistency`, `hit registration`, `hits not registering`, `join time`, `slow loading`, `jitter`, `packet loss`, `bad connection`, `slow connection`, `NetworkSettings`, `settings().Network`, and preset names (`Ideal Fiber`, `Wired Broadband`, `Home Wi-Fi`, `Standard Mobile (4G/LTE)`, `Bad Connection (3G)`)

**First action every time:** read the current `settings().Network` values (they're always available); this skill is the mechanism.

## The Device Simulator toolbar

The Device Simulator toolbar shows the current network profile (the LAN/Wifi/4G/3G pill) and a **Network simulation** popover where the user can adjust, apply, and save presets. It uses the same `settings().Network` values, so whatever you apply shows up there. The skill doesn't need the toolbar. The user only needs it for the saved-preset fallback (select the preset in the popover and click Apply).

Start commands by **reading the current `settings().Network` values** (to see any existing shaping — see the ⚠️ note under "How It Works").

## Commands

| Command | Use when |
|---------|----------|
| `/network-test` | Apply one preset (or custom values) and verify it took effect |
| `/network-preset` | List the built-in presets and apply one by name |
| `/network-compare` | Playtest under multiple presets and compare ping/throughput/loss |
| `/network-responsiveness` | Measure action→result latency, server catch-up, and client-server consistency under a preset |
| `/network-diagnose` | Debug a reported symptom (delayed feedback, hits not registering, long joins, rubber-banding, bad-network-only errors): reproduce, measure, find the code, suggest a fix |
| `/network-reset` | Clear all shaping back to 0 (no simulation), immediately |

---

## How It Works

The agent uses these MCP tools:

1. **`execute_luau`** — read/write the six `settings().Network` knobs (apply conditions, read them back) and run the measurements. Every call takes a **`datamodel_type`**:
   - **Not playing:** `"Edit"`. Use it to apply/read/restore knobs and to install/remove the temporary probe objects (`__NetProbe`, `__NetJoinProbe`, marked with the `__NetSimProbe` attribute).
   - **During a playtest:** only `"Client"` or `"Server"`. **The Edit datamodel is not available while playing.**
     - **`"Client"`** — measured ping (`NetworkPing`), the round-trip/movement/game-interaction measurements, and reading the join probe. **Default to Client for anything measured.**
     - **`"Server"`** — not needed by the measurements (the probe's server side runs as a temporary Script). Use it only for ad-hoc server-side inspection.
     - `settings().Network` is global, so reading/writing the knobs works from either.
2. **`start_stop_play`** — enter/exit a playtest so shaping actually takes effect and stats become available.
3. **`get_console_output`** — read console warnings/errors during a playtest (e.g. `Infinite yield` on late assets/remotes, RemoteEvent timeouts under lag).

**Multi-Studio:** If multiple instances are open, call `list_roblox_studios` + `set_active_studio` once at the start.

**First-call timeouts:** the very first `execute_luau` after connecting can time out if Studio is briefly busy. Retry 2–3 times before concluding Studio is unresponsive.

**⚠️ Always check the current emulation state before a playtest — never assume it's zero.** Settings persist across restarts and the user (or a previous session) may have set shaping via the toolbar. Before starting any playtest, profiling, or drawing any conclusion about lag, read `settings().Network` and state the actual current values. This is critical when another skill is involved: e.g. before profiling to answer "is my lag from the network or the scene?", confirm the knobs are actually zero — if latency is still applied, a "clean" profile isn't clean. Read first, report what's set, then proceed.

### The six knobs

All live on `settings().Network` (C++ `NetworkSettings`), `Plugin` security — readable and writable from `execute_luau`:

| Property | Range | Meaning |
|----------|-------|---------|
| `InboundNetworkMinDelayMs` | 0–cap ms (500 or 1000) | Base latency, server→client |
| `OutboundNetworkMinDelayMs` | 0–cap ms (500 or 1000) | Base latency, client→server |
| `InboundNetworkJitterMs` | 0–cap ms (500 or 1000) | Latency variance, server→client |
| `OutboundNetworkJitterMs` | 0–cap ms (500 or 1000) | Latency variance, client→server |
| `InboundNetworkLossPercent` | 0–0.5 | Packet loss %, server→client |
| `OutboundNetworkLossPercent` | 0–0.5 | Packet loss %, client→server |

**The delay/jitter cap depends on the Studio build.** It's **500ms** when both `FFlagNetworkSimulationParameterizedRanges` and `FFlagNetworkSimulationDelayJitterCap500ms` are on, and **1000ms** otherwise. Check once per session, before validating any request, with any `datamodel_type`. Pass the flag name without the `FFlag` prefix. `GetFFlag` throws on builds that don't define the flag, so the `pcall` treats a missing flag as off:

```lua
local function flagOn(name)
    local ok, value = pcall(function() return settings():GetFFlag(name) end)
    return ok and value == true
end
local cap = if flagOn("NetworkSimulationParameterizedRanges") and flagOn("NetworkSimulationDelayJitterCap500ms") then 500 else 1000
return string.format("delay/jitter cap: %dms", cap)
```

Use that cap everywhere below where it says "the cap". All built-in presets are under 500ms, so they're valid either way.

**Critical unit facts (see API_spec.md for source):**
- **Loss is a percent, and `0.5` means 0.5%, NOT 50%.** The engine divides the value by 100 before applying. **Max simulated loss is 0.5%.** Never tell the user `0.5` = "half the packets" — it's half of one percent.
- **Warn on out-of-range requests instead of silently clamping.** The engine silently clamps (loss to 0–0.5, delay/jitter to 0–the cap), which hides mistakes. Before applying, check the request against the ranges and **tell the user if it's out of range** — don't just apply the clamped value. The most common case: a user asks for **"30% packet loss"** — the simulator caps loss at **0.5%**, so 30% isn't achievable. Say so explicitly (e.g. "This simulator caps packet loss at 0.5%; 30% isn't supported. Want me to apply the 0.5% max?") and let them decide, rather than quietly setting 0.5. Same for latency/jitter above the cap (500ms or 1000ms; check the flags above, and state the cap that applies).
- Effects are **live during a Studio playtest** — a local playtest (**Play / Play Here**, F5) or a **Team Test** session (applied per-packet). Changing a knob mid-play takes effect immediately.
- Shaping only applies to an actual **playtest connection**. It has **no effect while just editing**, including in a Team Create *editing* session (there's no playtest connection to shape). Team **Test** does get shaped.
- Settings **persist across Studio restarts and across places**. The `NetworkSettings` class is `PERSISTENT_LOCAL_INTERNAL` — saved to local Studio settings per machine/user, not to the place file. So a place may **already have shaping applied from a previous session** — check current values at the start, and at the end of a test ask whether to restore the starting values or clear to 0 (see "Ending a test") so you don't leave a user unknowingly throttled or wipe their own setup.
- The knobs are **32-bit floats**, so fractional loss values carry float imprecision — `0.3` reads back as `0.30000001192092896`. This is expected, not a bug. **Always format loss for display** (e.g. `%.2f`), and when checking whether a read-back matches a preset, compare with a small tolerance (e.g. `math.abs(a-b) < 1e-4`) rather than `==`.
- **No bandwidth / throughput limit.** The only levers are latency, jitter, and loss — there is no "cap my bandwidth to N Mbps" knob. If a user asks to throttle bandwidth, say that isn't supported and offer latency/jitter/loss instead.

---

## Presets

A "preset" is just a named set of the six values. The skill knows the built-ins below and accepts ad-hoc presets the user describes; it **cannot** read the user's own **saved presets** (see "User-saved presets"). Treat any user-named preset (e.g. "UserPreset") like this, in order: (1) if the user already gave its values in the conversation, use them; (2) otherwise ask the user to either paste the values or select the preset in the Network popover and hit **Apply**, after which you can read the live values from `settings().Network`. Don't attempt `plugin:GetSetting` — it always fails here.

### Built-in presets (mirror the shipped toolbar UI)

Values are the raw numbers written to `settings().Network`. Loss values are the "percent" figure (0.5 = 0.5%).

| Preset | Pill | In Latency | In Jitter | In Loss | Out Latency | Out Jitter | Out Loss |
|--------|------|-----------:|----------:|--------:|------------:|-----------:|---------:|
| Current Device | — | 0 | 0 | 0 | 0 | 0 | 0 |
| Ideal Fiber | LAN | 8 | 0 | 0 | 8 | 0 | 0 |
| Wired Broadband | Wired | 25 | 3 | 0 | 25 | 3 | 0 |
| Home Wi-Fi | Wifi | 30 | 12 | 0.2 | 30 | 15 | 0.3 |
| Standard Mobile (4G/LTE) | 4G | 45 | 20 | 0.4 | 55 | 30 | 0.5 |
| Bad Connection (3G) | 3G | 150 | 70 | 0.5 | 180 | 90 | 0.5 |

**"Current Device"** is the off state — all six knobs at 0, meaning no artificial shaping (traffic uses the machine's real connection). Applying it is identical to `/network-reset`. The engine only surfaces it in the toolbar dropdown when the `DeviceSimulatorNetworkOffState` flag is on, but the skill writes zeros directly so it always works. When all six values read 0, that state is "Current Device"; any other non-preset combination reads as "Custom".

### Fuzzy term → preset

| User says | Preset |
|-----------|--------|
| "off", "none", "disable", "real network", "current device", "stop simulating" | Current Device (= all zeros; same as `/network-reset`) |
| "LAN", "ideal", "perfect", "fiber", "no lag" | Ideal Fiber |
| "wired", "ethernet", "broadband", "cable" | Wired Broadband |
| "Wi-Fi", "home", "wifi", "typical" | Home Wi-Fi |
| "4G", "LTE", "mobile", "phone", "cellular" | Standard Mobile (4G/LTE) |
| "3G", "bad", "slow", "worst", "laggy", "poor" | Bad Connection (3G) |

If the user names a preset that isn't built-in and hasn't been defined, ask for its values before applying — don't guess.

### User-saved presets (e.g. "UserPreset") — you CANNOT read them; use a fallback

The toolbar's **Save** button stores named presets in plugin settings under `DeviceSimulatorSavedPresets` (the shared built-in-plugin settings store).

**The skill cannot read this store.** `plugin:GetSetting("DeviceSimulatorSavedPresets")` requires the `plugin` global, and **`plugin` is `nil` in the Assistant's `execute_luau` context**. Studio Luau has no filesystem access to the store, and `PluginManager():CreatePlugin()` only yields a local plugin, which can't see the built-in keys. So **do not promise to read a user's saved preset by name** — go straight to a fallback:

1. **Ask the user to apply it in the toolbar, then read the live values.** Have them open the Network popover, select their preset, click **Apply**, then read the currently-applied values (this always works — `settings().Network` is global):
   ```lua
   local net = settings().Network
   return {
       net.InboundNetworkMinDelayMs,  net.InboundNetworkJitterMs,  net.InboundNetworkLossPercent,
       net.OutboundNetworkMinDelayMs, net.OutboundNetworkJitterMs, net.OutboundNetworkLossPercent,
   }
   ```
   Record those numbers as that preset for the rest of the session.
2. **Or ask the user to paste the values** (latency / jitter / loss per direction) and apply them directly.

State the limitation plainly and offer both options — don't spend turns trying `plugin:GetSetting`, it will always fail here.

**For reference**, the store's `values` are keyed by engine control ID (not property names), which map to the knobs as:

| Control ID | Property |
|------------|----------|
| `inbound-latency` | `InboundNetworkMinDelayMs` |
| `outbound-latency` | `OutboundNetworkMinDelayMs` |
| `inbound-jitter` | `InboundNetworkJitterMs` |
| `outbound-jitter` | `OutboundNetworkJitterMs` |
| `inbound-packet-loss` | `InboundNetworkLossPercent` |
| `outbound-packet-loss` | `OutboundNetworkLossPercent` |

### Custom values

The user can also give raw values directly ("300ms inbound latency, 0.3% loss both ways"). Apply exactly what they ask **as long as it's in range** — if a value exceeds its range (loss > 0.5%, delay/jitter > the cap: 500ms or 1000ms), tell the user it's out of range and what the max is, rather than silently clamping (see the out-of-range rule above). If they name a direction (e.g. "300ms inbound"), set only that direction. If they give a value **without** a direction (e.g. "add 300ms latency"), apply it to both inbound and outbound.

---

## Applying conditions

First **validate the request against the ranges and warn the user about anything out of range** (see the out-of-range rule under "Critical unit facts") — don't silently apply a clamped value. Then wrap the write in `pcall`; the `clamp` calls below are a safety net, not a substitute for telling the user.

```lua
local function flagOn(name)
    local ok, value = pcall(function() return settings():GetFFlag(name) end)
    return ok and value == true
end
local DELAY_CAP = if flagOn("NetworkSimulationParameterizedRanges") and flagOn("NetworkSimulationDelayJitterCap500ms") then 500 else 1000

local ok, err = pcall(function()
    local net = settings().Network
    local function clampDelay(v) return math.clamp(v, 0, DELAY_CAP) end
    local function clampLoss(v)  return math.clamp(v, 0, 0.5)  end

    net.InboundNetworkMinDelayMs  = clampDelay(45)
    net.OutboundNetworkMinDelayMs = clampDelay(55)
    net.InboundNetworkJitterMs    = clampDelay(20)
    net.OutboundNetworkJitterMs   = clampDelay(30)
    net.InboundNetworkLossPercent  = clampLoss(0.4)
    net.OutboundNetworkLossPercent = clampLoss(0.5)
end)
if not ok then warn("Failed to apply network settings:", err) end
```

## Reading back (primary verification — works in Edit mode)

`settings()` is a global app singleton, so this works from any datamodel: `"Edit"` when not playing, `"Client"` or `"Server"` during a playtest. This is the deterministic way to confirm the knobs are set:

```lua
local net = settings().Network
return string.format(
    "IN  delay=%dms jitter=%dms loss=%.2f%%\nOUT delay=%dms jitter=%dms loss=%.2f%%",
    net.InboundNetworkMinDelayMs,  net.InboundNetworkJitterMs,  net.InboundNetworkLossPercent,
    net.OutboundNetworkMinDelayMs, net.OutboundNetworkJitterMs, net.OutboundNetworkLossPercent
)
```

## Resetting

```lua
local net = settings().Network
for _, p in {
    "InboundNetworkMinDelayMs", "OutboundNetworkMinDelayMs",
    "InboundNetworkJitterMs", "OutboundNetworkJitterMs",
    "InboundNetworkLossPercent", "OutboundNetworkLossPercent",
} do
    net[p] = 0
end
```

### Ending a test: ask, then restore or clear

Don't clear the knobs to 0 automatically when a test finishes. The user may have had their own shaping applied (a saved toolbar preset, custom values) before you started, and wiping it silently loses their setup. Follow this:

1. **Record the starting values** at the start of every command: the six values exactly as read from `settings().Network`, before you change anything. Keep them for the session, and name them if they match a preset (within float tolerance), otherwise call them "Custom". All zeros means "Current Device" (off).
2. **When a test command finishes** (`/network-compare`, `/network-responsiveness`, `/network-diagnose`, or any multi-preset playtesting), stop the playtest, then **ask** before changing the knobs:
   - **Starting values were all 0:** "Clear the simulation back to 0 (off), or keep `<last preset>` applied?"
   - **Starting values were not 0:** "Restore your starting values (`<name or Custom>`: IN `<lat>/<jit>/<loss>%` · OUT `<lat>/<jit>/<loss>%`), clear to 0 (off), or keep `<last preset>` applied?" Recommend restoring: it puts the user back where they were.
3. **Until the user answers, leave the knobs as they are**, and state what's currently applied. Settings persist across Studio restarts, so make it clear that the current preset stays in effect.
4. **Do what they choose**, then read the six values back to confirm (with float tolerance):
   - **Restore** → write the recorded starting values (snippet below).
   - **Clear** → the reset snippet above (all six → 0).
   - **Keep** → change nothing; remind them it persists and that `/network-reset` clears it.

```lua
-- Restore the starting values recorded at the beginning of the test.
local startingValues = {
    InboundNetworkMinDelayMs = 30, InboundNetworkJitterMs = 12, InboundNetworkLossPercent = 0.35,
    OutboundNetworkMinDelayMs = 30, OutboundNetworkJitterMs = 15, OutboundNetworkLossPercent = 0.41,
}
local net = settings().Network
local ok, err = pcall(function()
    for property, value in startingValues do
        net[property] = value
    end
end)
if not ok then return "restore failed: " .. tostring(err) end
local out = {}
for property, value in startingValues do
    local actual = net[property]
    table.insert(out, string.format("%s=%.2f%s", property, actual, if math.abs(actual - value) < 1e-4 then "" else " (MISMATCH)"))
end
return table.concat(out, "\n")
```

`/network-test` and `/network-preset` exist to *apply* a preset, so the preset stays applied when they finish. If the starting values were different, mention that you can restore them ("say 'restore' to put back your `<name or Custom>` values").

An **explicit `/network-reset`** (or "reset", "turn off simulation", "clear it") always clears to 0 right away, without asking. "Restore", "put it back", or "undo the simulation" means restore the recorded starting values, not clear to 0. If no starting values were recorded this session, say so and ask for the values.

---

## Verifying the real effect in Play mode

Read-back (above) confirms the knobs are *set*. To confirm the shaping is actually *affecting traffic* — and to get a **real measured round-trip latency** — read the **client's network ping** during a playtest. This is the same number shown as **`NetworkPing`** in the in-game Performance Stats bar (the top-right `Mem · CPU · GPU · Sent · Recv · NetworkPing` strip), so what you report matches what the user sees on screen.

- **Source:** `Stats.PerformanceStats.Ping` in the **client** datamodel (the CoreScript stats bar reads exactly this item). The client replicator fills it from the transport's round-trip ping to the server. `NetworkSent` / `NetworkReceived` in the same folder are the bar's `Sent` / `Recv` tiles (KB/s).
- **Read it from the client datamodel via `execute_luau` with `datamodel_type = "Client"`.** The Edit and Server datamodels don't have a meaningful value — the stat is client-only. `StatsItem:GetValue()` is `Plugin` security, so `execute_luau` can read it but a **LocalScript gets `-1`**.

### Read the client ping via execute_luau during play

Start a playtest (`start_stop_play`), wait a few seconds for the connection to settle, then run this with **`datamodel_type = "Client"`**. It takes 5 samples ~1s apart, because the value is the latest sample and moves with the configured jitter:

```lua
-- Must run in the play client (datamodel_type = "Client"). Other datamodels don't carry the client's ping.
local Players = game:GetService("Players")
if not Players.LocalPlayer then
    return "WRONG DATAMODEL: no LocalPlayer, so this isn't the play client. Re-run with datamodel_type = \"Client\"."
end
local perf = game:GetService("Stats"):FindFirstChild("PerformanceStats")
if not perf then return "no Stats.PerformanceStats yet; wait a few seconds and retry" end
local function val(name)
    local item = perf:FindFirstChild(name)
    return if item then item:GetValue() else -1
end

local SAMPLES = 5
local pings, sum, min, max = {}, 0, math.huge, -math.huge
for i = 1, SAMPLES do
    local ping = val("Ping")
    table.insert(pings, string.format("%.1f", ping))
    sum += ping
    min = math.min(min, ping)
    max = math.max(max, ping)
    if i < SAMPLES then task.wait(1) end
end
return string.format("NetworkPing avg %.1fms (min %.1f / max %.1f; raw %s)  Sent=%.2f KB/s  Recv=%.2f KB/s",
    sum / SAMPLES, min, max, table.concat(pings, ", "), val("NetworkSent"), val("NetworkReceived"))
```

Report the average plus min/max.

**If the ping reads `0` or `-1`, you aren't reading the play client's stat yet.** Check that the call used `datamodel_type = "Client"` and the playtest is running, wait a few seconds, and retry. If it still fails, tell the user you couldn't read the client ping. Don't fall back to the server's `Stats.Network` or substitute another number.

The value is in **milliseconds** and matches the on-screen bar. **Values well under 1ms (e.g. `0.168`) are normal when no latency is configured**: a local playtest runs client and server in the same Studio process. Report them as-is (e.g. "0.2ms"); don't reinterpret them as seconds.

**Reporting latency honestly — important.** Configured inbound + outbound latency is a *theoretical* round-trip; **do not present it as measured.** Only call a number "measured" when it comes from the live client `NetworkPing` stat — which is close to but not exactly the configured sum (real traffic adds overhead). Label clearly, e.g. "configured round-trip <X>ms; measured NetworkPing <Y>ms".

**Notes:**
- `Stats.PerformanceStats` only has live network values once the client is connected in a playtest.
- Loss caps at 0.5%, so it barely shows up in any stat — latency/jitter are the observable signals. Don't over-index on loss.
- Shaping only applies during an actual playtest (Play or Team Test) — not while editing. Verify during a playtest.

---

## Measuring interaction responsiveness (Play mode)

Ping says how long a packet takes. These probes measure what the **player experiences**: how long an action takes to produce a result, how long the server takes to catch up to client input, and how far the client and server disagree about where things are. All three run during a playtest via `execute_luau` — the `datamodel_type` for each step is called out. Run them with shaping applied, and once under **Ideal Fiber** as the baseline so the deltas are meaningful.

Common rules:
- **The probe is added in Edit, before the playtest, and removed right after.** Objects created from `execute_luau` in the *play server* don't reliably replicate to the play client, so don't create the probe there. Install it in the Edit datamodel (below), so the playtest copies it like any other part of the place. Always remove it when the testing is done, and before finishing. If the user saves while it's in place, it's saved into the place.
- **Time sources.** `os.clock()` differences within one datamodel are exact (use them for round trips). Cross-datamodel timing (client → server) uses `workspace:GetServerTimeNow()`, which the client keeps synced to the server clock — accurate to a few ms, so label one-way splits and catch-up times as **estimates**.
- **Frame quantization.** Replicated changes are processed once per frame, so results have ~1 frame (~16ms at 60 FPS) of resolution. Don't report sub-frame differences as meaningful.
- Keep each `execute_luau` call short (the scripts below take ≤ ~10s each).

### Install the probe (Edit datamodel, before the playtest)

The probe is two temporary objects, both named `__NetProbe`: a `RemoteEvent` in `ReplicatedStorage`, and a server `Script` in `ServerScriptService` that handles it. Run this with **`datamodel_type = "Edit"`** *before* `start_stop_play`; the playtest copies both into the play server and client. Tell the user you're adding them, and ask them not to save the place until they're removed.

Every object the skill creates carries the attribute `__NetSimProbe = true`, and the skill only ever deletes objects with that marker. If the place already has an unmarked object with a probe name, it may belong to the game: the install stops with `STOPPED: ... already exists` and changes nothing. Tell the user which object collided and don't rename, move, or delete it yourself. Skip the probe-based measurements unless the user resolves the conflict.

```lua
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local ServerScriptService = game:GetService("ServerScriptService")
local MARKER = "__NetSimProbe" -- set on every object this skill creates
local parents = { ReplicatedStorage, ServerScriptService }

-- A same-named object without the marker may belong to the game: stop without changing anything.
for _, parent in parents do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetProbe" and child:GetAttribute(MARKER) ~= true then
            return "STOPPED: " .. child:GetFullName() .. " already exists and wasn't created by this skill. Nothing was changed."
        end
    end
end
-- Replace a probe left over from an earlier install (marked, so it's ours).
for _, parent in parents do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetProbe" and child:GetAttribute(MARKER) == true then child:Destroy() end
    end
end

local remote = Instance.new("RemoteEvent")
remote.Name = "__NetProbe"
remote:SetAttribute(MARKER, true)
remote.Parent = ReplicatedStorage

local handler = Instance.new("Script")
handler.Name = "__NetProbe"
handler:SetAttribute(MARKER, true)
handler.Source = [[
-- Temporary network probe handler, added by the network-simulator skill. Remove after testing.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local probe = ReplicatedStorage:WaitForChild("__NetProbe")

probe.OnServerEvent:Connect(function(player, kind, arg)
    if kind == "ping" then
        -- A server-side state change that replicates back, like a real server-validated action.
        probe:SetAttribute("AckRecvAt", workspace:GetServerTimeNow())
        probe:SetAttribute("Ack", arg) -- set last; the client listens on Ack
    elseif kind == "trace" then
        -- Record this player's character position as the server sees it, then send it back.
        task.spawn(function()
            local samples = {}
            local stopAt = workspace:GetServerTimeNow() + math.min(arg, 10)
            while workspace:GetServerTimeNow() < stopAt do
                local character = player.Character
                local root = character and character:FindFirstChild("HumanoidRootPart")
                if root then
                    local p = root.Position
                    table.insert(samples, { workspace:GetServerTimeNow(), p.X, p.Z })
                end
                RunService.Heartbeat:Wait()
            end
            probe:FireClient(player, "trace", samples)
        end)
    end
end)
]]
handler.Parent = ServerScriptService
return "probe installed — start the playtest now; remove it after stopping"
```

Then start the playtest and run the measurements below. For several conditions in a row, leave the probe in place between playtests (stop → apply the next preset → start) and remove it after the last one.

**Remove the probe** as soon as the last playtest stops, with `datamodel_type = "Edit"`. This also removes the join probe, if it was used. It deletes only objects carrying the `__NetSimProbe` marker and lists any same-named objects it left alone:

```lua
local removed, kept = 0, {}
for _, parent in { game:GetService("ReplicatedStorage"), game:GetService("ServerScriptService"), game:GetService("ReplicatedFirst") } do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetProbe" or child.Name == "__NetJoinProbe" then
            if child:GetAttribute("__NetSimProbe") == true then
                child:Destroy()
                removed += 1
            else
                table.insert(kept, child:GetFullName())
            end
        end
    end
end
local result = string.format("removed %d probe object(s)", removed)
if #kept > 0 then
    result ..= "; left alone (not created by this skill): " .. table.concat(kept, ", ")
end
return result
```

### Measure the action → server → result round trip (Client datamodel)

Models any server-validated interaction (buy, open door, hit confirm): the client fires a remote, the server changes replicated state, the client sees the change. Run with **`datamodel_type = "Client"`**:

```lua
if not game:GetService("Players").LocalPlayer then
    return "WRONG DATAMODEL: not the play client. Re-run with datamodel_type = \"Client\"."
end
local probe = game:GetService("ReplicatedStorage"):WaitForChild("__NetProbe", 10)
if not probe then return "NetProbe missing — install the probe in Edit before starting the playtest" end

local TRIALS, GAP = 8, 0.2
local rtts, ups, downs, lost = {}, {}, {}, 0
for i = 1, TRIALS do
    local seq = i + os.clock() -- unique per trial
    local ackedAt
    local conn = probe:GetAttributeChangedSignal("Ack"):Connect(function()
        if probe:GetAttribute("Ack") == seq then ackedAt = os.clock() end
    end)
    local sentServerTime = workspace:GetServerTimeNow()
    local sentAt = os.clock()
    probe:FireServer("ping", seq)
    while not ackedAt and os.clock() - sentAt < 5 do task.wait() end
    conn:Disconnect()
    if ackedAt then
        local rtt = (ackedAt - sentAt) * 1000
        local up = (probe:GetAttribute("AckRecvAt") - sentServerTime) * 1000
        table.insert(rtts, rtt)
        table.insert(ups, up)
        table.insert(downs, rtt - up)
    else
        lost += 1
    end
    task.wait(GAP)
end

local function summarize(list)
    if #list == 0 then return "n/a" end
    table.sort(list)
    local sum = 0
    for _, v in list do sum += v end
    return string.format("median %.0f · mean %.0f · min %.0f · max %.0f ms",
        list[math.ceil(#list / 2)], sum / #list, list[1], list[#list])
end
return table.concat({
    string.format("Action→result round trip (%d/%d ok): %s", #rtts, TRIALS, summarize(rtts)),
    "  client→server (est.): " .. summarize(ups),
    "  server→client (est.): " .. summarize(downs),
    "  timed out: " .. lost,
}, "\n")
```

The round trip is the **minimum** delay any server-validated action can have; real handlers add their own work on top. It should track NetworkPing plus ~1–2 frames. The up/down split shows which direction the shaping hits hardest.

### Measure movement: server catch-up and client-server consistency (Client datamodel)

Holds **W** via VirtualInput while the client and server each record the character's position on the shared server clock, then compares the two traces. Run with **`datamodel_type = "Client"`** (takes ~7s):

```lua
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local UserInputService = game:GetService("UserInputService")
if not game:GetService("Players").LocalPlayer then
    return "WRONG DATAMODEL: not the play client. Re-run with datamodel_type = \"Client\"."
end
local probe = game:GetService("ReplicatedStorage"):WaitForChild("__NetProbe", 10)
if not probe then return "NetProbe missing — install the probe in Edit before starting the playtest" end

local character = Players.LocalPlayer.Character or Players.LocalPlayer.CharacterAdded:Wait()
local root = character:WaitForChild("HumanoidRootPart")
local humanoid = character:WaitForChild("Humanoid")

local IDLE, HOLD, SETTLE = 1.0, 1.5, 2.5 -- seconds; IDLE covers the trace request's trip to the server
local THRESHOLD = 0.1 -- studs moved before counting as "moving"

local serverSamples
local traceConn = probe.OnClientEvent:Connect(function(kind, samples)
    if kind == "trace" then serverSamples = samples end
end)
probe:FireServer("trace", IDLE + HOLD + SETTLE)

local clientSamples = {}
local sampleConn = RunService.Heartbeat:Connect(function()
    local p = root.Position
    table.insert(clientSamples, { workspace:GetServerTimeNow(), p.X, p.Z })
end)

task.wait(IDLE)
local vi = UserInputService:CreateVirtualInput()
local pressAt = workspace:GetServerTimeNow()
vi:SendKey(true, Enum.KeyCode.W)
task.wait(HOLD)
local releaseAt = workspace:GetServerTimeNow()
vi:SendKey(false, Enum.KeyCode.W)

local waitStart = os.clock()
while not serverSamples and os.clock() - waitStart < SETTLE + 5 do task.wait() end
sampleConn:Disconnect()
traceConn:Disconnect()
if not serverSamples or #serverSamples < 2 then return "no server trace returned — is the __NetProbe Script in ServerScriptService? Install it in Edit before Play" end

local function distXZ(s, x, z)
    return math.sqrt((s[2] - x) ^ 2 + (s[3] - z) ^ 2)
end
-- Time after t until the trace moves THRESHOLD away from where it was at t.
local function startedAfter(samples, t)
    local base
    for _, s in samples do
        if s[1] <= t then
            base = s
        elseif base and distXZ(s, base[2], base[3]) > THRESHOLD then
            return s[1] - t
        end
    end
    return nil
end
-- Time after t until the trace reaches (within THRESHOLD) its final resting position.
local function stoppedAfter(samples, t)
    local final = samples[#samples]
    for _, s in samples do
        if s[1] >= t and distXZ(s, final[2], final[3]) < THRESHOLD then return s[1] - t end
    end
    return nil
end
-- Linear interpolation of a time-sorted trace.
local function positionAt(samples, t)
    local lo, hi = 1, #samples
    if t <= samples[lo][1] then return samples[lo][2], samples[lo][3] end
    if t >= samples[hi][1] then return samples[hi][2], samples[hi][3] end
    while hi - lo > 1 do
        local mid = (lo + hi) // 2
        if samples[mid][1] <= t then lo = mid else hi = mid end
    end
    local a, b = samples[lo], samples[hi]
    if b[1] == a[1] then return a[2], a[3] end
    local f = (t - a[1]) / (b[1] - a[1])
    return a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f
end

local clientStart, serverStart = startedAfter(clientSamples, pressAt), startedAfter(serverSamples, pressAt)
local clientStop, serverStop = stoppedAfter(clientSamples, releaseAt), stoppedAfter(serverSamples, releaseAt)
if not clientStart then
    return "character did not move on W — the game may not use default controls; use the MoveTo fallback"
end

-- Mean/max distance between the server's position at t and the client's at t - lag, while moving.
local windowStart, windowEnd = pressAt, releaseAt + (serverStop or 1)
local function divergence(lag)
    local sum, max, n = 0, 0, 0
    for _, s in serverSamples do
        if s[1] >= windowStart and s[1] <= windowEnd then
            local d = distXZ(s, positionAt(clientSamples, s[1] - lag))
            sum += d
            n += 1
            if d > max then max = d end
        end
    end
    return if n > 0 then sum / n else math.huge, max
end
local bestLag, bestErr = 0, math.huge
for lagMs = 0, 1500, 5 do
    local err = divergence(lagMs / 1000)
    if err < bestErr then bestLag, bestErr = lagMs, err end
end
local sameMean, sameMax = divergence(0)

-- Client-side position snaps (corrections / rubber-banding): steps far larger than walking allows.
local snaps = 0
for i = 2, #clientSamples do
    local a, b = clientSamples[i - 1], clientSamples[i]
    if distXZ(b, a[2], a[3]) > humanoid.WalkSpeed * (b[1] - a[1]) * 2 + 0.5 then snaps += 1 end
end
local lastClient, lastServer = clientSamples[#clientSamples], serverSamples[#serverSamples]

local function ms(v) return if v then string.format("%.0fms", v * 1000) else "n/a" end
return table.concat({
    "Input → client moves (local response): " .. ms(clientStart),
    "Input → server sees movement (server catch-up, est.): " .. ms(serverStart),
    "Release → client stops: " .. ms(clientStop) .. " · Release → server stops (est.): " .. ms(serverStop),
    string.format("Server trails client by ~%dms (best fit, residual %.2f studs)", bestLag, bestErr),
    string.format("Same-instant client/server divergence while moving: mean %.2f · max %.2f studs (WalkSpeed %d)",
        sameMean, sameMax, humanoid.WalkSpeed),
    string.format("Divergence at rest: %.2f studs", distXZ(lastServer, lastClient[2], lastClient[3])),
    "Client position snaps: " .. snaps,
    string.format("samples: client %d · server %d", #clientSamples, #serverSamples),
}, "\n")
```

**MoveTo fallback** (game doesn't move on W, e.g. custom controls): replace the two `vi:SendKey` lines with `humanoid:MoveTo(root.Position + root.CFrame.LookVector * humanoid.WalkSpeed * HOLD)` at press and `humanoid:MoveTo(root.Position)` at release. Make sure the character has room to walk forward (not facing a wall), or the start time never registers.

How to read it:
- **Local response** should stay ~1–3 frames under every preset — the client predicts its own character. If it grows with the shaping, movement is waiting on the server (server-authoritative movement), and every step will feel delayed.
- **Server catch-up** ≈ client→server one-way latency + the server's physics-replication buffer. It's what the server "knows" of the player's input: server-side hit detection, triggers, and anti-cheat checks all act this late.
- **Server trails client by ~X ms** is the steady-state lag of the server's view. **Same-instant divergence** is that lag in studs (≈ WalkSpeed × lag; e.g. 16 studs/s × 0.3s ≈ 5 studs). Server-validated hits/touches miss by about this much unless the game compensates for lag.
- **Divergence at rest** should return to ~0. A lasting offset or **snaps > 0** means corrections (rubber-banding) — the server overrode the client's position.

### Time a real game interaction (Client datamodel)

The round-trip and movement measurements are generic. For "how does *this* button/ability feel?", time the game's own action → visible result. First read the game's scripts to pick the action (the input that triggers it) and the result (a value the server changes and replicates: a leaderstat, an attribute, a part property, GUI text set from a remote). Fill in `ACTION` and `readResult`, then run with **`datamodel_type = "Client"`**:

```lua
local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local UserInputService = game:GetService("UserInputService")
if not game:GetService("Players").LocalPlayer then
    return "WRONG DATAMODEL: not the play client. Re-run with datamodel_type = \"Client\"."
end
local vi = UserInputService:CreateVirtualInput()
local player = Players.LocalPlayer

local function ACTION() -- the player's input; e.g. press E at the shop
    vi:SendKey(true, Enum.KeyCode.E)
    vi:SendKey(false, Enum.KeyCode.E)
end
local function readResult() -- a value that changes once the server's result arrives
    return player.leaderstats.Coins.Value
end

local TRIALS, TIMEOUT = 5, 5
local results = {}
for _ = 1, TRIALS do
    local before = readResult()
    local startedAt = os.clock()
    ACTION()
    local seenAt
    while os.clock() - startedAt < TIMEOUT do
        RunService.RenderStepped:Wait() -- check once per rendered frame: when the player could see it
        if readResult() ~= before then
            seenAt = os.clock()
            break
        end
    end
    table.insert(results, if seenAt then string.format("%.0fms", (seenAt - startedAt) * 1000) else "timeout")
    task.wait(0.5)
end
return "Action → visible result: " .. table.concat(results, ", ")
```

If the game shows instant local feedback (a predicted animation/sound/UI change) *and* a server-confirmed result, measure both — the gap between them is what the player perceives as lag. For UI buttons, use the `virtual-input` skill's click pattern as `ACTION`. Keep `TRIALS × TIMEOUT` modest so the call stays short, and make sure each trial can actually repeat (enough currency, cooldowns elapsed).

---

## Common bad-network issues and how to debug them

These are the problems players actually report on bad connections. Each one can be **reproduced** with this skill (apply a preset), **measured** (the probes above), and **traced to code** (read the game's scripts). When a user describes one of these symptoms — even without mentioning the network ("my sword doesn't always hit", "the shop button feels slow", "the game takes forever to load on mobile") — treat it as something this skill can debug. Use `/network-diagnose`.

Workflow for any symptom: measure under **Ideal Fiber** (baseline) → measure under the preset that matches the report (default **Bad Connection (3G)**) → compare → read the relevant scripts → explain the cause using the measured numbers → suggest a fix → optionally apply it and re-measure to show the improvement.

What the simulator can and can't reproduce: it adds latency, jitter and loss to the playtest's **client↔server connection** (inbound = server→client, outbound = client→server). It does **not** slow asset/CDN downloads or HTTP requests, and there's **no bandwidth cap**. So it reproduces anything bound by round trips or replication delay, but not "my 200 MB place downloads slowly". Say so when it matters.

### Delayed feedback after input

- **Symptom:** "I press the button and nothing happens for a moment", players double-click, duplicate purchases/actions.
- **Why:** the client waits for the server before showing *any* feedback. Every server-validated action costs at least one round trip (client→server + server work + server→client replication).
- **Measure:** the round-trip measurement gives the floor. Timing the actual action ("Time a real game interaction") gives the real delay. Measure both the local feedback (if any) and the server-confirmed result; a large gap or no local feedback means it feels laggy.
- **Look for in code:** client handlers that only update UI/animation/sound inside `OnClientEvent`, after `InvokeServer` returns, or on a replicated value change. Also no debounce or pending state, so a second click sends a second request.
- **Fixes to suggest:** immediate local feedback (press animation, sound, "pending" UI state), then reconcile with the server's answer. Debounce or disable the control while a request is in flight. Use `RemoteEvent` instead of yielding `RemoteFunction` where the client doesn't need the answer to continue.

### Hits not registering

- **Symptom:** "I clearly hit them but it didn't count", hits register on Wi-Fi but not on mobile, damage lands late.
- **Why:** the attacker aims at where *their client* sees the target. The server validates against *its own* view, which differs by the measured divergence. Remote players and NPCs are shown on each client delayed by server→client latency plus interpolation. The attacker's own position and input reach the server late (server catch-up). Strict server-side checks reject hits that were valid on screen, and server-side `Touched` hitboxes on fast-moving parts fire late or miss.
- **Measure:** the movement measurement's **same-instant divergence** (studs) and **server trails client by** (ms) give the size of the disagreement. Timing a real game interaction with the attack as `ACTION` and the target's `Humanoid.Health` (or a hit counter) as the result gives the delay *and* the **hit rate**: count trials that time out as "not registered" and compare against baseline. Use a stationary target first, then a moving one.
- **Look for in code:** the server's hit handler (`OnServerEvent` for the attack remote, server raycasts, `Touched` hitboxes, `GetPartBoundsInBox`). Check for distance and sanity checks (e.g. `(a - b).Magnitude < 5`) whose tolerance is smaller than the measured divergence. Also look for raycasts done on the server from the server's (late) positions.
- **Fixes to suggest:** detect hits on the client and validate on the server with a tolerance ≥ the measured divergence under the worst supported preset. Or apply lag compensation: rewind targets by the attacker's ping when validating. Avoid server `Touched` for fast melee/projectiles. Quote the numbers, e.g. "your check allows 4 studs; under 3G client and server disagree by ~5 studs, so ~X% of hits get rejected".
- **Player-vs-player** needs two clients: the movement measurement covers one player's own lag; a real PvP hit stacks the attacker's and the target's lag. If multi-client testing is available (Team Test / multiple playtest clients), time the attack there.

### Longer join times

- **Symptom:** "loading takes forever on mobile", long loading screens, players leave before spawning.
- **Why (what the simulator reproduces):** joining involves many round-trip-bound steps: connection handshake, initial replication, then the game's own startup. Startup code that waits for the server *sequentially* multiplies the round trip: 10 chained `InvokeServer` calls × 400ms RTT = 4s of waiting. Startup also slows from `WaitForChild` chains on instances that arrive late, the server loading data (DataStores) before sending anything to the client, and streaming requests. (Asset download time isn't simulated — see above.)
- **Measure:** the join probe below, which gives a milestone breakdown from connected to game ready. Compare against baseline: the **increase** is what the network costs. Read the console for `Infinite yield possible` warnings during startup.
- **Look for in code:** `ReplicatedFirst` loading screens and startup `LocalScript`s/modules that run `InvokeServer` calls one after another, `WaitForChild` chains, the client requesting data piece by piece, and loading screens that wait on things the player doesn't need yet.
- **Fixes to suggest:** batch startup data into one request, or have the server push it proactively (attributes or replicated values on the player). Run independent requests in parallel (`task.spawn`). Let the player in while non-critical content loads in the background. Remove unnecessary `WaitForChild` chains. Re-run the join probe to show the saving.

### Rubber-banding / character snapping back

- **Symptom:** character jerks back after moving, vehicles stutter, positions "teleport".
- **Why:** the server overrides a position the client predicted: anti-cheat or "safe zone" scripts setting `HumanoidRootPart.CFrame` or calling `PivotTo` on characters, server-side movement, or network ownership of vehicles/physics parts changing hands (`SetNetworkOwner`, auto-ownership flipping).
- **Measure:** the movement measurement's **snaps > 0** or a lasting **divergence at rest**.
- **Look for in code:** server scripts writing to player character CFrames/positions, speed/teleport checks with tight thresholds (which fire falsely when latency bunches up movement updates), and `SetNetworkOwner` calls.
- **Fixes to suggest:** loosen anti-cheat thresholds to tolerate the measured lag, correct only on real violations, and give vehicles/physics objects stable client ownership.

### Errors that only appear on bad connections

- **Symptom:** "works in Studio, breaks on mobile", intermittent `nil` errors, things missing on first spawn.
- **Why:** code assumes timing that only holds on a fast connection: the client indexes instances that haven't replicated yet, or a request sent at startup races the server's setup.
- **Measure:** run the playtest under a high-latency/jitter preset and read `get_console_output`: `attempt to index nil`, `Infinite yield possible on 'WaitForChild(...)'`, remote queue exhausted warnings.
- **Look for in code:** direct indexing (`workspace.Map.Spawn`) in client scripts where the instance arrives via replication, and assumptions about the order between remotes and property changes.
- **Fixes to suggest:** `WaitForChild` (with a timeout) for replicated instances, handle data that arrives late, and use one source of truth for "ready" instead of timing.

### Join probe (measuring join time)

Join happens before you can run code in the play client, so this probe places two **temporary scripts** in the place *before* starting the playtest. The Edit datamodel can't be changed while playing, so they're removed as soon as the playtest stops. Tell the user you're adding temporary probe scripts named `__NetJoinProbe`, that they're removed when the playtest stops, and **not to save the place until then**. Times are wall-clock (`DateTime.now()`), which is shared by the play client and server in the same Studio process.

1. **Install** (Edit datamodel, just before `start_stop_play`):

Like the responsiveness probe, it marks what it creates with `__NetSimProbe` and stops on an unmarked object with the same name (see "Install the probe").

```lua
local ServerScriptService = game:GetService("ServerScriptService")
local ReplicatedFirst = game:GetService("ReplicatedFirst")
local MARKER = "__NetSimProbe" -- set on every object this skill creates
local parents = { ServerScriptService, ReplicatedFirst }

-- A same-named object without the marker may belong to the game: stop without changing anything.
for _, parent in parents do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetJoinProbe" and child:GetAttribute(MARKER) ~= true then
            return "STOPPED: " .. child:GetFullName() .. " already exists and wasn't created by this skill. Nothing was changed."
        end
    end
end
-- Replace a join probe left over from an earlier install (marked, so it's ours).
for _, parent in parents do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetJoinProbe" and child:GetAttribute(MARKER) == true then child:Destroy() end
    end
end

local serverProbe = Instance.new("Script")
serverProbe.Name = "__NetJoinProbe"
serverProbe:SetAttribute(MARKER, true)
serverProbe.Source = [[
-- Temporary network join probe: wall-clock time each player finished connecting.
local Players = game:GetService("Players")
local function stamp(player)
    if player:GetAttribute("__JoinConnectedAt") == nil then
        player:SetAttribute("__JoinConnectedAt", DateTime.now().UnixTimestampMillis)
    end
end
Players.PlayerAdded:Connect(stamp)
for _, player in Players:GetPlayers() do stamp(player) end
]]
serverProbe.Parent = ServerScriptService

local clientProbe = Instance.new("LocalScript")
clientProbe.Name = "__NetJoinProbe"
clientProbe:SetAttribute(MARKER, true)
clientProbe.Source = [[
-- Temporary network join probe: wall-clock milestones of this client's join.
local Players = game:GetService("Players")
local function mark(name)
    script:SetAttribute(name, DateTime.now().UnixTimestampMillis)
end
mark("ReplicatedFirstRan")
if not game:IsLoaded() then game.Loaded:Wait() end
mark("GameLoaded")
local player = Players.LocalPlayer
if not player.Character then player.CharacterAdded:Wait() end
mark("CharacterSpawned")
-- Optional game-specific "ready" milestone, e.g. the main HUD appearing:
-- player:WaitForChild("PlayerGui"):WaitForChild("MainHUD")
-- mark("GameReady")
]]
clientProbe.Parent = ReplicatedFirst
return "join probe installed — start the playtest now; remove it after stopping"
```

If the game has a clear "ready" moment (loading screen destroyed, main HUD shown, a `ready` attribute set), uncomment and adapt the `GameReady` lines. That's the number players feel.

2. **Start the playtest** (`start_stop_play`).

3. **Read** with `datamodel_type = "Client"` once the character has spawned:

```lua
if not game:GetService("Players").LocalPlayer then
    return "WRONG DATAMODEL: not the play client. Re-run with datamodel_type = \"Client\"."
end
local player = game:GetService("Players").LocalPlayer
local probe = game:GetService("ReplicatedFirst"):FindFirstChild("__NetJoinProbe")
local connectedAt = player:GetAttribute("__JoinConnectedAt")
if not probe or not connectedAt then return "join probe data missing — was it installed before the playtest started?" end

local out = {}
for _, name in { "ReplicatedFirstRan", "GameLoaded", "CharacterSpawned", "GameReady" } do
    local at = probe:GetAttribute(name)
    if at then table.insert(out, string.format("connected → %s: %dms", name, at - connectedAt)) end
end
return table.concat(out, "\n")
```

4. **Stop the playtest, then immediately remove** the scripts with `datamodel_type = "Edit"`:

```lua
local removed = 0
for _, parent in { game:GetService("ServerScriptService"), game:GetService("ReplicatedFirst") } do
    for _, child in parent:GetChildren() do
        if child.Name == "__NetJoinProbe" and child:GetAttribute("__NetSimProbe") == true then
            child:Destroy()
            removed += 1
        end
    end
end
return string.format("removed %d join probe object(s)", removed)
```

To measure several conditions, you can leave the scripts in place between playtests and remove them after the last one. Always remove them before finishing.

Measure the baseline (Ideal Fiber) and the shaped preset with the same steps. Studio's own playtest startup happens before "connected", so it isn't counted, but it's still noisy run to run. Run each condition twice if the difference is small. Report the per-milestone increase and point at the step that grew most (e.g. "GameLoaded → GameReady grew from 0.3s to 3.9s: the loading screen makes 9 sequential `InvokeServer` calls").

Before finishing, confirm (with `datamodel_type = "Edit"`, after the playtest has stopped) that no marked `__NetJoinProbe` or `__NetProbe` remains. The "Remove the probe" snippet removes both, and running it again is harmless. It never deletes unmarked objects with those names; those belong to the game.

---

## Evaluating gameplay under a degraded network

The skill can *apply and measure* conditions; to judge how the game *feels*, combine signals — the Assistant works through measurements + console (+ screenshots for visual state), not by watching live motion. Layer these:

0. **First, read the current `settings().Network` and state it — never assume it's zero.** Because settings persist, the place may already be shaped from the toolbar or a prior session. This matters most when isolating causes: to answer "is my lag from the network or the scene?" you must confirm the knobs are actually `0` before calling a profile "clean." If shaping is still applied (e.g. 500ms), tell the user and ask to clear it before measuring (record it so you can offer to restore it afterwards), or your conclusion is wrong.
1. **Latency numbers (what you can measure directly).** The client `NetworkPing` (above, `Stats.PerformanceStats.Ping` read with `datamodel_type = "Client"`) is the round-trip time from the client to the server and back (ms) — the same value the in-game Performance Stats bar shows. It most directly reflects the applied inbound+outbound latency. Report it and its delta vs the Ideal Fiber baseline. This is the most reliable signal.
2. **Console signals.** Read `get_console_output` during the playtest for network-stress symptoms: `Infinite yield possible on 'WaitForChild(...)'` (assets/remotes arriving late), RemoteEvent/RemoteFunction timeouts or "exhausted" errors, `HttpService`/replication warnings, and any script errors that only appear under lag. These often reveal code that assumes a fast connection.
3. **Measure responsiveness (numbers, not impressions).** Run the measurements in "Measuring interaction responsiveness" above: the action → server → result round trip, movement (input → server catch-up, server lag, client/server divergence, snaps), and timing of the game's own key interaction. These turn "it feels laggy" into measured milliseconds and studs.
4. **Screenshot visual state (optional).** `screen_capture` shows things the probes can't read: a loading screen that hangs, UI that never updates after a press, error dialogs. The **`device-simulator`** skill's `commands/device-playtest.md` is a good template for the overall flow (set conditions before Play → screenshot → interact → check console → finish per "Ending a test").
5. **Separate "slow network" from "slow frame".** If the game hitches, use the **`perf-profiling`** skill (`require("@rbx/LibMP")` works in `execute_luau`) to check whether time is spent in the `Network` group / waiting on replication versus CPU/GPU/render. This distinguishes "laggy because of the simulated network" from "laggy because the scene is heavy."

Suggested combined flow for "how does my game hold up on 3G?": apply the preset → install the probe (Edit) → `start_stop_play` → read the client `NetworkPing` → measure the round trip and movement (and time the game's key action) → read console (screenshot if something visual looks wrong) → optionally profile with LibMP → stop and remove the probe → ask whether to restore the starting values or clear to 0 ("Ending a test"). Report measured ping and responsiveness deltas vs baseline, plus any console warnings (e.g. "3 client position snaps and a 4-stud offset at rest: the server is correcting the character under the simulated ping").

Be honest about limits: you can *measure* ping and *observe* screenshots/console, but you can't perceive smoothness frame-by-frame. Frame it as evidence (measured ping, a 340ms action→result round trip, the server trailing by 300ms, a WaitForChild infinite-yield warning) rather than a subjective "it feels laggy."

---

## Verification output format

**Formatting rules for chat output:**
- **Never output Markdown tables** — the Assistant's renderer can't render them (`No renderer found for node: TABLE`) and the content breaks.
- **Prefer bullet lists** for presets, results, and comparisons — they render as normal text with line breaks preserved.
- **Avoid fenced code blocks (```` ``` ````) for regular output** — the Assistant renders them as muted **gray monospace**, which reads as de-emphasized. Only use a code block for actual Luau snippets. Present preset lists, verification results, and comparisons as bullets or short labeled lines, not inside a code fence.
- The reference tables elsewhere in this skill are for your own context, not for copying into a reply.

Single test (`/network-test`) — as bullets (no code block). Use the actual applied values and the ping you actually measured; `<measured>` is a placeholder:

**Network Test: Standard Mobile (4G/LTE)**
- Applied (read back from `settings().Network`): IN 45ms / 20ms / 0.40% · OUT 55ms / 30ms / 0.50%
- Measured in playtest (client NetworkPing, as shown in the Performance Stats bar): `<measured>`ms (avg of a few samples; min `<x>` / max `<y>`)
- Packet loss barely visible (capped at 0.5% → few drops)
- Configured round-trip = 100ms (45+55); the measured ping is the real number.
- Shaping is live only during a playtest (Play or Team Test), and persists across Studio restarts. It stays applied; say "restore" to put back your previous values or `/network-reset` to clear to 0.

Comparison (`/network-compare`) — one bullet per preset, filling in the pings you measured:

**Network Comparison (measured ping)**
- Ideal Fiber — IN 8/0/0.00% · OUT 8/0/0.00% · NetworkPing `<measured>`ms · sent/recv `<x>`/`<y>` KB/s
- Bad Connection (3G) — IN 150/70/0.50% · OUT 180/90/0.50% · NetworkPing `<measured>`ms · sent/recv `<x>`/`<y>` KB/s

Findings:
- Report the measured NetworkPing jump from the baseline preset to the worst preset, and that server-validated actions get proportionally more input delay.
- Throughput barely changes (loss capped at 0.5%) — this sim stresses latency/jitter, not bandwidth.

Responsiveness (`/network-responsiveness`) — bullets, measured values only, with the baseline alongside:

**Responsiveness: Bad Connection (3G)** (baseline Ideal Fiber in parentheses)
- Action → result round trip: median `<x>`ms (`<b>`ms) · client→server ~`<u>`ms · server→client ~`<d>`ms
- Input → client moves: `<x>`ms (`<b>`ms) — local prediction, unaffected by the network
- Input → server sees movement: ~`<x>`ms (`<b>`ms) · server trails client by ~`<x>`ms
- Client/server divergence while moving: mean `<x>` · max `<y>` studs; at rest `<z>` studs · snaps `<n>`
- Game action (`<what>`) → visible result: `<x>`ms (`<b>`ms)

Findings: name what the numbers mean for this game (e.g. "server-validated purchases take ~350ms to show; melee hits validated on the server see the target ~5 studs behind where the attacker sees it").

(Values shown as `latency/jitter/loss%` per direction.)

### Tone

In chat, describe each measurement by what it does ("measuring the round trip for a server-validated action", "checking how far the server's view of your character lags behind"), not by section names or labels.

Frame findings as opportunities. Say "Under 3G the measured NetworkPing is `<measured>`ms — server-validated actions will feel noticeably behind; client-side prediction would smooth this," not "Your game lags." And never present the configured latency sum as if you measured it — say "configured" vs "measured".

---

## API Reference

Full details — property semantics, clamps, unit source, persistence, live-during-play mechanics, security, and the Stats verification tree — are in **`API_spec.md`** (same directory).

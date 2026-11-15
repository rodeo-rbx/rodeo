# Network Simulator — API Specification

Full reference for the Studio Network Simulator as driven from Luau. For behavior/workflow instructions, see `SKILL.md`. Facts below are grounded in the engine source (`game-engine/`) and the Studio plugin source (`StudioPlugins/Standalone/DeviceSimulatorToolbar/`).

## What network simulation is

The Network Simulator shapes the traffic of a Studio **playtest** to imitate a slower or lossier connection — adding latency, jitter, and packet loss in each direction. It does not change your real internet connection; it delays/drops packets on the playtest's UDP socket so you can see how your game feels on, say, a 3G phone.

The six knobs live on `settings().Network` and are **always available in Studio** — they are engine settings. The Device Simulator toolbar (a network pill LAN/Wired/Wifi/4G/3G + "Network simulation" popover) is a UI over the same values.

---

## Access path

```lua
local net = settings().Network   -- C++ RBX::NetworkSettings, a synthetic settings service
```

`settings()` returns the global application settings singleton, available from any `execute_luau` call (it is not tied to the Edit vs Play DataModel). `NetworkSettings` delegates to the `CoreNetworkSettings` runtime singleton — the same singleton the toolbar UI writes to. So the toolbar and `settings().Network` are two front-ends over one set of values.

### The six knobs

All are `float`, `Plugin` read/write security (writable + readable from plugin/`execute_luau` context):

| Property | Range (clamp) | Direction | Meaning |
|----------|---------------|-----------|---------|
| `InboundNetworkMinDelayMs` | 0–cap (500 or 1000) | server→client | Base latency (ms) |
| `OutboundNetworkMinDelayMs` | 0–cap (500 or 1000) | client→server | Base latency (ms) |
| `InboundNetworkJitterMs` | 0–cap (500 or 1000) | server→client | Latency variance (ms) |
| `OutboundNetworkJitterMs` | 0–cap (500 or 1000) | client→server | Latency variance (ms) |
| `InboundNetworkLossPercent` | 0–0.5 | server→client | Packet loss (percent — see below) |
| `OutboundNetworkLossPercent` | 0–0.5 | client→server | Packet loss (percent — see below) |

---

## Units and clamping

### Loss is a PERCENT, and `0.5` means 0.5% — NOT 50%

This is the single most counterintuitive fact. Per the engine registrar comment and the consumer:

- `CoreNetworkSettings` stores loss as a percent; `NetworkEmulationUpdater` divides it by 100 before handing it to the transport (`sendPacketLossRatio = getOutboundNetworkLossPercent() / 100.0`).
- The `0.5` clamp ceiling therefore means **0.5% loss**, not 50%. A value of `0.5` drops ~1 packet in 200.

So the **maximum simulated packet loss is 0.5%.** Never describe `0.5` as "half the packets" — it's half of one percent. The toolbar displays the raw value with a `%` suffix (e.g. it shows `0.50%` for a stored `0.5`).

### Delay and jitter are milliseconds

`0`–cap ms, applied directly (no division). Jitter is added variance on top of the min delay. The cap is **500ms** when both `FFlagNetworkSimulationParameterizedRanges` and `FFlagNetworkSimulationDelayJitterCap500ms` are on, and **1000ms** otherwise. Read them with `settings():GetFFlag("<name without the FFlag prefix>")` inside a `pcall`: it throws on builds that don't define the flag, which counts as off.

### Clamping is silent

Setters clamp to range and do **not** error on overshoot:

- `InboundNetworkLossPercent = 200` → stored as `0.5`
- `InboundNetworkMinDelayMs = 5000` → stored as the cap (`500` or `1000`)

Because the clamp is silent, **warn the user when a request is out of range** (per `SKILL.md`) rather than quietly applying the clamped value — e.g. "30% loss" exceeds the 0.5% max. Still clamp in your own code as a safety net, so the value you report matches what actually took effect:

```lua
local function clampDelay(v) return math.clamp(v, 0, DELAY_CAP) end -- DELAY_CAP: 500 or 1000, from the flags above
local function clampLoss(v)  return math.clamp(v, 0, 0.5)  end
```

---

## Persistence — persists across restarts (local, global to the machine)

**The values survive a Studio restart.** The `NetworkSettings` class is declared `[[Functionality("PERSISTENT_LOCAL_INTERNAL")]]` and inherits `GlobalAdvancedSettingsNs::Item`, so it's persisted to **local Studio settings** (per machine/user), not to the place file. The `[[NoStorage]]` tag on each property only means the value isn't stored on the Instance / serialized into the `.rbxl` — persistence happens at the settings layer, and the values are **global across places** on that machine.

Implications:
- A place can **already have shaping applied from a previous session** — read the current values at the start of any command so you don't misreport.
- `/network-reset` (all six → 0) or restoring the starting values are the only things that change it; a restart does **not**. At the end of a test, ask whether to restore the starting values or clear to 0 (SKILL.md "Ending a test").

---

## Live-during-play — yes, on playtests only

Changing a knob takes effect **immediately during a Studio playtest** — a local playtest (Play/Play Here) or a Team Test — applied per-packet on the live UDP socket:

- `NetworkEmulationUpdater` subscribes to `CoreNetworkSettings.propertyChangedSignal`; on change it rebuilds the emulation config and pushes it to the active connection, which reads drop-rate/latency/jitter per packet.
- This means you can apply a preset *before* Play or *while* playing — both work.

**Constraints:**
1. Only on **playtest** connections — the engine gates on `isLocalTest() || isCloudTest()`, where `isLocalTest()` = a local playtest (Play/Play Here) and `isCloudTest()` = a **Team Test** client (`isCloudTest = isRunningTeamTestClient()`). Modern `RBX_USE_RBXTRANSPORT` path.
2. **No effect while just editing** — including a Team Create *editing* session (no playtest connection to shape). Team *Test* is shaped; Team *Create* editing is not.
3. A `NetworkTestControlProvider` test override, if set, takes precedence over these settings.
4. No effect in pure Edit mode — there's no network connection to shape. Edit mode is only useful for *reading back* the configured values.

---

## Reading back (verification in Edit mode)

`settings().Network` is readable at any time from `execute_luau`. This is the deterministic way to confirm the knobs are set (independent of whether a playtest is running):

```lua
local net = settings().Network
return string.format(
    "IN  delay=%dms jitter=%dms loss=%.2f%%\nOUT delay=%dms jitter=%dms loss=%.2f%%",
    net.InboundNetworkMinDelayMs,  net.InboundNetworkJitterMs,  net.InboundNetworkLossPercent,
    net.OutboundNetworkMinDelayMs, net.OutboundNetworkJitterMs, net.OutboundNetworkLossPercent
)
```

---

## Verifying real effect — network stats (Play mode)

To confirm shaping actually affects traffic (not just that the knobs are set), read the live **client network ping** during a playtest — the same value the in-game Performance Stats bar shows as **`NetworkPing`** (top-right `Mem · CPU · GPU · Sent · Recv · NetworkPing` strip). It lives under the `Stats` service in the **client** datamodel.

| Path (client datamodel) | Security | Notes |
|------|----------|-------|
| `Stats.PerformanceStats.Ping` | Plugin | **NetworkPing (ms)** — the bar's `NetworkPing` tile. Round-trip time from the client to the server and back, set every received-packet tick by `ClientReplicator` from the transport connection's ping. Most directly reflects the applied latency. Read via `:GetValue()`. |
| `Stats.PerformanceStats.NetworkSent` / `NetworkReceived` | Plugin | The bar's `Sent` / `Recv` tiles (KB/s) |
| `Stats.PerformanceStats.MaxPing` | Plugin | Bar graph target line (fixed), **not** a measured max |

The CoreScript stats bar (`CoreScripts/Modules/Stats/StatsAggregator.lua`) reads exactly `Stats.PerformanceStats.<name>:GetValue()`. The value is the latest sample (not averaged), so it moves with the configured jitter; sample a few times and average for reporting.

**Context matters:**
- Read it with `execute_luau` and **`datamodel_type = "Client"`** during a playtest. The stat is client-side; the Edit and Server datamodels don't carry the client's ping.
- `StatsItem:GetValue()` is `Plugin` security — `execute_luau` can read it, but a **LocalScript reads `-1`**.

See `SKILL.md` "Verifying the real effect in Play mode" for the snippet.

**Report latency honestly.** Configured inbound + outbound latency is a *theoretical* round-trip — not a measurement. The measured value is the live client `NetworkPing` (`Stats.PerformanceStats.Ping`); expect it to be close to but not exactly the configured inbound+outbound sum (real traffic adds overhead). Because loss is capped at 0.5%, loss barely shows up — latency/jitter are the observable signals.

---

## Presets

Canonical preset **names** are defined in the plugin (`networkLocalization.lua`). The numeric values live in the engine registrar (not in Lua); the values below are read from the shipped toolbar UI and are what to write to `settings().Network` to reproduce each preset. Loss values are the raw 0–0.5 number (= that many percent; `0.5` = 0.5%).

| Preset | Pill | In Latency | In Jitter | In Loss | Out Latency | Out Jitter | Out Loss |
|--------|------|-----------:|----------:|--------:|------------:|-----------:|---------:|
| Current Device | — | 0 | 0 | 0 | 0 | 0 | 0 |
| Ideal Fiber | LAN | 8 | 0 | 0 | 8 | 0 | 0 |
| Wired Broadband | Wired | 25 | 3 | 0 | 25 | 3 | 0 |
| Home Wi-Fi | Wifi | 30 | 12 | 0.2 | 30 | 15 | 0.3 |
| Standard Mobile (4G/LTE) | 4G | 45 | 20 | 0.4 | 55 | 30 | 0.5 |
| Bad Connection (3G) | 3G | 150 | 70 | 0.5 | 180 | 90 | 0.5 |

The engine (`NetworkSimulatorRegistrar.cpp`) defines these presets in a `kPresets` array with the struct order `{latencyMs, packetLossPercent, jitterMs}` per direction — the values above match it exactly. Two special entries:

- **"Current Device"** (`kDevicePreset`) is the **off state**: all six values `0`, `NetworkType::None`. It means "no artificial shaping — use the real connection," and is functionally identical to a reset. The UI only shows it when the `DeviceSimulatorNetworkOffState` flag is on; writing zeros via `settings().Network` reproduces it unconditionally. `findPreset` labels an all-zero state as "Current Device" and any other non-matching combination as "Custom".
- **"Custom"** is whatever the user has dialed in that doesn't match a named preset.

### Custom / user-saved presets — the skill can neither READ nor WRITE them

The toolbar's **Save** button persists named presets via `plugin:SetSetting("DeviceSimulatorSavedPresets", ...)`, shape `{ [simulatorName] = { { name = "...", values = { [controlId] = value } } } }`, stored per user in the shared built-in-plugin settings store.

`plugin` is `nil` in the Assistant's `execute_luau` context, so neither read (`GetSetting`) nor write (`SetSetting`) is possible from the skill. Studio Luau has no filesystem access to the settings store either, and `PluginManager():CreatePlugin()` only yields a local plugin, which can't touch the built-in plugins' saved keys.

**There is no Luau-reachable API to write/save a network preset.** Exhaustive check of the reflected schema:
- `StudioDeviceSimulatorService` (the only reflected device-sim service) has **device methods only** — `GetDeviceListAsync`, `SetDeviceAsync`, `CreateDeviceAsync`, orientation/resolution/DPI/scaling — **no preset or simulator-control methods**.
- The simulator registry (`StudioDeviceSimulatorRegistry` / `IStudioDeviceSimulatorRegistryBridge`: `getRegisteredSimulators`, `notifyControlChanged`, `getSimulatorDynamicState`) is a **plugin-component** interface reached via `plugin:GetPluginComponent(...)` — needs `plugin` (nil here) and a live generation token, and `notifyControlChanged` only applies a control value live, it does not persist a named preset.
- The only reflected `SavePreset` in Studio is `AssetImportService.SavePreset` — unrelated (asset import).

**Consequences for the skill:**
- To *apply* the effect of a custom preset, write the six `settings().Network` knobs directly (live, works). This does NOT create a toolbar dropdown entry.
- To *persist* a named preset in the toolbar, only the **user** (Save button) can do it — there is no API callable from `execute_luau`.
- To *obtain* a user's saved preset values: ask them to Apply it in the toolbar then read `settings().Network`, or paste the values.

For reference, saved `values` are keyed by engine **control ID** → property:

| Control ID | Property |
|------------|----------|
| `inbound-latency` | `InboundNetworkMinDelayMs` |
| `outbound-latency` | `OutboundNetworkMinDelayMs` |
| `inbound-jitter` | `InboundNetworkJitterMs` |
| `outbound-jitter` | `OutboundNetworkJitterMs` |
| `inbound-packet-loss` | `InboundNetworkLossPercent` |
| `outbound-packet-loss` | `OutboundNetworkLossPercent` |

Example stored shape (preset "UserPreset"): `{ ["inbound-latency"]=30, ["inbound-jitter"]=12, ["inbound-packet-loss"]=0.35, ["outbound-latency"]=30, ["outbound-jitter"]=15, ["outbound-packet-loss"]=0.41 }`.

---

## Applying a preset

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

    -- Standard Mobile (4G/LTE)
    net.InboundNetworkMinDelayMs   = clampDelay(45)
    net.OutboundNetworkMinDelayMs  = clampDelay(55)
    net.InboundNetworkJitterMs     = clampDelay(20)
    net.OutboundNetworkJitterMs    = clampDelay(30)
    net.InboundNetworkLossPercent  = clampLoss(0.4)
    net.OutboundNetworkLossPercent = clampLoss(0.5)
end)
if not ok then warn("Failed to apply network settings:", err) end
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

---

## Architecture note (why we write `settings().Network` directly)

The Studio toolbar is a generic, schema-driven renderer. It does **not** write `settings().Network` from Lua; it calls `StudioDeviceSimulatorRegistry:NotifyControlChangedAsync(simulatorName, controlId, value, generation)` — a plugin-component command that needs a live generation token and engine-defined control IDs, and is only reachable inside that plugin's context. That path is neither simpler nor reachable from `execute_luau`. Writing `settings().Network.*` hits the same `CoreNetworkSettings` singleton the registry ultimately writes to, so it is the correct and simplest programmatic path for this skill.

Relevant engine/plugin FFlags: `EnableDeviceSimulatorToolbar`, `EnableDeviceSimulatorRegistry` (master gate), `StudioDeviceSimulatorRequireExplicitControlIds`, `DeviceSimulatorNetworkSettingsLocalization`.

---

## Known limitations

1. **Loss max is 0.5%** — `0.5` = 0.5%, not 50%. Loss is a weak lever and there's no bandwidth knob, so this sim mainly stresses latency/jitter.
2. **Silent clamping** — out-of-range writes clamp without error; read back to confirm.
3. **Playtest-only effect** — applies during a local playtest (Play) or Team Test; no shaping while just editing (incl. Team Create editing).
4. **Persists across restarts** — `PERSISTENT_LOCAL_INTERNAL`, saved to local Studio settings (global across places, not in the place file). Only `/network-reset` clears it; a restart does not. Record the current values at the start; at the end, ask whether to restore them or clear to 0.
5. **NetworkPing is a client stat** — read `Stats.PerformanceStats.Ping` via `execute_luau` with `datamodel_type = "Client"` during a playtest. `:GetValue()` is `Plugin` security, so a LocalScript reads `-1`.
6. **Saved custom presets are unreadable AND unwritable from the skill** — `plugin` is nil in `execute_luau` (confirmed), and there's no reflected preset API. Ask the user to Apply the preset in the toolbar then read `settings().Network`, or paste values.
7. **Values are 32-bit `float`** — fractional loss values carry float32 imprecision (e.g. `0.3` reads back as `0.30000001192092896`). Format for display and use a tolerance when matching a read-back to a preset.

## Error handling

Always `pcall`. Likely failures:

| Symptom | Cause | Fix |
|---------|-------|-----|
| Write to a knob errors | Rare build/flag issue (properties are always present) | Report the error |
| Value doesn't match what you set | Silent clamp | Re-read; report the clamped value |
| No latency effect while playing | Editing (incl. Team Create edit), not an actual playtest | Use a local playtest (Play) or Team Test |
| `PerformanceStats.Ping` reads `-1` | Reading from a LocalScript (Plugin security) | Read via `execute_luau` with `datamodel_type = "Client"` |
| `PerformanceStats.Ping` reads `0` / `-1` or folder missing | Not the play client's stat (wrong `datamodel_type`), or connection not up yet. (Sub-millisecond values like `0.168` are normal with no shaping, not an error) | Confirm the playtest is running, target the Client datamodel, retry after a few seconds |
| Responsiveness probe: "NetProbe missing" or no server trace | Probe wasn't in the place when the playtest started, or it was created from the play server (those objects may not replicate to the client) | Stop, install the probe with `datamodel_type = "Edit"`, start again |
| Probe install returns `STOPPED: ... already exists` | The place already has an object named `__NetProbe` / `__NetJoinProbe` without the `__NetSimProbe` marker, so it may belong to the game | Don't delete or rename it. Tell the user which object collided and skip the probe-based measurements unless they resolve it |
| Movement measurement: "character did not move on W" | Custom controls, or the character faces a wall | Use the MoveTo fallback; give the character room to walk |
| Movement measurement: server catch-up `n/a` | One-way latency longer than the probe's 1s idle lead-in | Increase `IDLE` in the script |
| Loss reads back as `0.30000001192092896` | 32-bit float imprecision (not a bug) | Format with `%.2f` for display; use tolerance when matching presets |
| `plugin` is nil | `execute_luau` has no plugin global | Don't use `plugin:GetSetting/SetSetting`; use `settings().Network` + ask user |

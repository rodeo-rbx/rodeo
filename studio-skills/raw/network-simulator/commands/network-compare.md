Playtest under multiple network presets and compare ping / throughput / loss.

## What to do

1. Connect to Studio (multi-instance: `list_roblox_studios` + `set_active_studio`). **Read and record the current `settings().Network` values first** (the starting values) — don't assume zero; settings persist and may already be shaped.

2. **Pick the presets to compare.** If the user named them, use those. Otherwise default to a spread that shows contrast: **Ideal Fiber**, **Home Wi-Fi**, **Bad Connection (3G)**. Include any custom presets the user asks for.

3. **For each preset, measure real ping** (client `NetworkPing`; see SKILL.md "Verifying the real effect"):
   - Apply the preset via `execute_luau`.
   - `start_stop_play` to enter a playtest; wait a few seconds.
   - Read `Stats.PerformanceStats` via `execute_luau` with **`datamodel_type = "Client"`** (`Ping` = NetworkPing, `NetworkSent` / `NetworkReceived` KB/s — the same values as the in-game Performance Stats bar). Sample a few times and average. Do NOT use a LocalScript — its reads return `-1`.
   - If the user cares about feel (input delay, rubber-banding, hit registration), also run the responsiveness measurements for each preset (see `/network-responsiveness`). Install the probe in Edit before the first playtest, and remove it after the last one, and add the round trip and server-lag numbers to each bullet.
   - Stop the playtest, apply the next preset, repeat. (Restarting the playtest per preset gives the cleanest per-preset measurement; you can also change presets live mid-play, but a fresh playtest avoids carryover.)
   - If you truly can't drive playtests, fall back to reporting the read-back (applied) values as a matrix and tell the user live NetworkPing needs a playtest.

4. **Report** using the bullet-list comparison format in SKILL.md (**not a Markdown table**, and **not a fenced code block** — one bullet per preset) with in/out configured values and the **measured ping**. Findings: which presets cause visible latency; note throughput barely changes (loss capped at 0.5%) so this sim stresses latency/jitter, not bandwidth.

5. **Ask how to finish** (SKILL.md "Ending a test"). Stop the playtest, then ask before changing anything. If the starting values weren't 0 (e.g. the user's own preset), offer to **restore** them (recommended), **clear to 0**, or **keep** the last preset. If they were 0, offer **clear to 0** or **keep**. Do what they choose and read back to confirm. Don't clear automatically.

## Notes

- Report **measured** ping (client `NetworkPing`), not the configured inbound+outbound sum. Include the Ideal Fiber baseline so deltas are meaningful — even the baseline measures slightly above its configured sum due to real overhead.
- Shaping applies during a playtest (Play or Team Test), not while editing — compare during playtests.

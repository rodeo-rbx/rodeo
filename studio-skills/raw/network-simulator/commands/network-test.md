Apply one network preset (or custom values) and verify it took effect.

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first.

2. **Read and record the current state first** — via `execute_luau`, read all six `settings().Network` values and keep them as the starting values (SKILL.md "Ending a test"). **If shaping is already applied** (non-zero — settings persist across restarts / from the toolbar), tell the user the current values before changing anything; don't assume it's zeroed.

3. **Resolve the preset** (see SKILL.md "Presets"):
   - Built-in name or fuzzy term → use the built-in values.
   - Custom preset the user named (e.g. "UserPreset") → you **cannot** read saved presets (`plugin` is nil in `execute_luau`). Use values they gave earlier in the chat, or ask them to Apply it in the toolbar so you can read `settings().Network`, or paste the values. Don't attempt `plugin:GetSetting`. See SKILL.md "User-saved presets".
   - Raw values ("300ms latency, 0.3% loss") → apply directly. If a direction is named (e.g. "300ms inbound"), set only that direction; if no direction is given, apply to both inbound and outbound.

4. **Validate ranges, then apply** via `execute_luau` — if any requested value is out of range (loss > 0.5%, delay/jitter > the cap: 500ms when `FFlagNetworkSimulationParameterizedRanges` and `FFlagNetworkSimulationDelayJitterCap500ms` are both on, otherwise 1000ms; see SKILL.md "The six knobs"), **tell the user and don't silently clamp** (e.g. "30% loss" isn't possible — cap is 0.5%). Then write in a `pcall`. See SKILL.md "Applying conditions" and the out-of-range rule.

5. **Verify (read-back)** — re-read all six from `settings().Network` and confirm they match the target (post-clamp). This works in Edit mode and is the primary verification.

6. **Optional Play-mode confirmation** — if the user wants proof it affects traffic, start a playtest (`start_stop_play`), wait a few seconds, then read the client **`NetworkPing`** (`Stats.PerformanceStats.Ping` — the value in the in-game Performance Stats bar) via `execute_luau` with **`datamodel_type = "Client"`**, sampling a few times (see SKILL.md "Verifying the real effect"). A LocalScript can't read it (returns `-1`). Report the measured ping, not the configured latency sum.

7. **Report** using the single-test format in SKILL.md (plain text / bullets — **never a Markdown table**, the Assistant can't render them). Distinguish **configured** latency from **measured** NetworkPing. Note that shaping is live only during a playtest (Play or Team Test) and **persists across Studio restarts**. The preset stays applied (that's the point of this command); if the starting values were different, offer to restore them later, and mention `/network-reset` clears to 0.

## Reminders

- **Loss `0.5` = 0.5%, not 50%.** Never describe it as "half the packets." Max loss is 0.5%.
- **Warn on out-of-range requests** (e.g. "30% loss") instead of silently clamping — tell the user the max and let them choose.
- **Never present configured inbound+outbound latency as "measured"** — only the client `NetworkPing` (`Stats.PerformanceStats.Ping`) is a real measurement.
- Shaping applies during a playtest (Play or Team Test), not while editing.

# Custom Events

Anything that doesn't fit Economy or Funnel — adoption metrics, behavioral
signals, core-loop counters — goes through `LogCustomEvent`.

Reference: <https://create.roblox.com/docs/production/analytics/custom-events>

## Two shapes

### Counter events (default)

Fire when the action happens; no value needed. Aggregations treat the value
as `1`, so `sum` equals total count, `avg/min/max` are always `1`.

```lua
local AnalyticsService = game:GetService("AnalyticsService")

AnalyticsService:LogCustomEvent(player, "MissionStarted")
AnalyticsService:LogCustomEvent(player, "AbilityUsed")
AnalyticsService:LogCustomEvent(player, "ChestOpened")
```

### Value events

Fire with a numeric value when the metric is quantitative.

```lua
-- Mission completion duration in seconds
AnalyticsService:LogCustomEvent(player, "MissionCompletedDuration", 120)

-- Damage dealt in a single fight
AnalyticsService:LogCustomEvent(player, "BossFightDamage", damageDealt)

-- Score earned at end of round
AnalyticsService:LogCustomEvent(player, "RoundScore", finalScore)
```

For value events, the dashboard exposes `count`, `unique users`, `sum`, `avg`,
`min`, `max`, and `avg per user`.

## Use custom fields aggressively

The 100-event-name cap is global across the experience, but custom fields
have **8,000 unique value combinations**. Always lean on custom fields when
you'd otherwise generate similar event names.

```lua
-- BAD: each plant variant burns an event-name slot
AnalyticsService:LogCustomEvent(player, "PlantCabbage")
AnalyticsService:LogCustomEvent(player, "PlantTurnip")
AnalyticsService:LogCustomEvent(player, "PlantPepper")

-- GOOD: single event, plant identified by custom field
AnalyticsService:LogCustomEvent(
    player,
    "PlantSeed",
    1,
    {
        [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Plant - Cabbage",
        [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = string.format("Map - %s", currentMap),
    }
)
```

This keeps your event-name budget free for real distinct signals and lets you
visualize `PlantSeed` totals **and** per-plant breakdowns from the same
event.

## Batching to stay under the rate limit

Global rate limit is `120 + 20 * CCU` requests per minute. For high-frequency
actions (per-bullet, per-zombie-kill, per-tile-stepped), batch on the server
and flush periodically:

```lua
local zombiesKilled = {}  -- [player] = number

local function recordZombieKill(player)
    zombiesKilled[player] = (zombiesKilled[player] or 0) + 1
end

-- Flush every 30 seconds and on player leave:
local function flush(player)
    local n = zombiesKilled[player]
    if not n or n <= 0 then return end
    zombiesKilled[player] = 0
    AnalyticsService:LogCustomEvent(player, "ZombiesKilled", n)
end

task.spawn(function()
    while true do
        task.wait(30)
        for player, _ in zombiesKilled do flush(player) end
    end
end)

Players.PlayerRemoving:Connect(flush)
```

This is also the pattern recommended by the docs (`"sending 10 zombies killed
instead of 1 zombie killed ten times"`).

## Suggested event categories

Pick a small, stable set of event names per game and use custom fields to
slice them.

### Adoption / UI

| Event              | When                     | Field 01           | Field 02                | Field 03       |
| ------------------ | ------------------------ | ------------------ | ----------------------- | -------------- |
| `FeatureOpened`    | UI panel/feature opened  | `Feature - <name>` | `Source - <button/url>` | `Map - <name>` |
| `TutorialAccepted` | Tutorial prompt accepted | `Tutorial - <id>`  |                         |                |
| `SettingsChanged`  | Gameplay setting flipped | `Setting - <name>` | `Value - <on/off>`      |                |

### Core loop

| Event           | Use                              | Field 01        | Field 02        | Field 03       |
| --------------- | -------------------------------- | --------------- | --------------- | -------------- |
| `LoopStarted`   | Loop iteration begins            | `Loop - <name>` | `Map - <name>`  |                |
| `LoopCompleted` | Loop iteration ends successfully | `Loop - <name>` | `Outcome - Win` | `Map - <name>` |
| `LoopFailed`    | Loop iteration ends in failure   | `Loop - <name>` | `Reason - <id>` | `Map - <name>` |

### Behavior

| Event           | Value           | Field 01           | Field 02         |
| --------------- | --------------- | ------------------ | ---------------- |
| `AbilityUsed`   | (counter)       | `Ability - <name>` | `Class - <name>` |
| `WeaponFired`   | shots batched   | `Weapon - <name>`  | `Map - <name>`   |
| `EnemyDefeated` | enemies batched | `Enemy - <type>`   | `Map - <name>`   |
| `Loss`          | (counter)       | `Cause - <name>`   | `Map - <name>`   |

`Loss` is the broad concept — death, elimination, mission failure, race
DNF, surrender, time-out. It's genre-specific: only add it when the
game has an explicit loss / failure branch (combat, platformer,
survival, PvP, mission-based PvE, racing with DNF). Use a more specific
event name (`Death`, `Elimination`, `MatchLost`) only if the game's
losses are uniformly one kind; otherwise keep `Loss` and slice with the
`Cause` custom field.

For puzzle, simulator, social, builder, idle, or tycoon games, players
typically end a session by quitting or finishing rather than failing —
track the round / loop / match outcome via the appropriate funnel and a
custom `*Outcome` event with an `Outcome - <Quit|Finished|Failed>`
custom field instead. See `analytics-patterns.md` "Pattern: loss /
failure (only when the game has it)."

### Session quality

| Event         | Value     | Field 01        |
| ------------- | --------- | --------------- |
| `IdleTimeout` | seconds   |                 |
| `Reconnect`   | (counter) | `Reason - <id>` |

Don't add a custom session-length event — Roblox's built-in analytics
already report session length, DAU, retention, and concurrent users on
the Engagement dashboard. A `SessionDuration` custom event would
duplicate them and burn an event-name slot. Use custom events here only
for session-quality signals the platform doesn't expose
(`IdleTimeout`, `Reconnect`, etc.).

## When to choose Custom vs Economy vs Funnel

| Question                                                | Likely answer |
| ------------------------------------------------------- | ------------- |
| Did the player gain or spend a tracked resource?        | Economy       |
| Is this a step in a sequence I want drop-off rates for? | Funnel        |
| Else                                                    | Custom        |

### Don't double-log

For each gameplay trigger, pick **one** primary event. Custom is the
fallback when neither Economy nor Funnel fits, not a parallel record of
the same thing.

- **Already an economy event?** Skip the custom mirror. A coin reward
  granted via `LogEconomyEvent(..., Source, "Coins", ...)` is already
  counted on the Economy dashboard — don't also fire `RewardEarned` or
  `CoinsClaimed`. The IAP grant for a 1000-coin bundle is the same
  story; no `BundlePurchased` custom event needed.
- **Already a funnel step?** Skip the custom mirror. A `MatchFlow`
  funnel with a "Match Started" step makes a custom `MatchStarted`
  event redundant. An onboarding funnel ending in "Tutorial Completed"
  makes a custom `TutorialCompleted` event redundant.
- **Add a custom event only when** it surfaces a metric the primary
  event can't — average reward size, value distribution, an outcome
  category, a duration, or a custom-field breakdown the funnel /
  economy dashboard doesn't expose.

### Don't duplicate built-in dashboards

Roblox already exposes Engagement, Retention, Monetization, Developer
Product / Pass / Avatar Item / Subscription analytics, Acquisition,
Performance, and Error Report dashboards. Custom events that re-state
metrics from those dashboards just burn an event-name slot.

Common forbidden duplicates:

| Don't add                                           | Why                                                    | Where it's already tracked                        |
| --------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------- |
| `PlayerJoined`, `SessionStart`, `SessionDuration`   | Joins and session length are core platform metrics     | Engagement dashboard (DAU, session time)          |
| `RetentionDay1`, `ReturningPlayer`                  | Retention is computed from session timestamps          | Retention dashboard (D1 / D7 / D30)               |
| `RobuxSpent`, `BundlePurchased`, `PassActivated`    | Robux purchases per product are recorded automatically | Monetization + Developer Product / Pass analytics |
| `JoinSource`, `AcquisitionSource`, `TeleportedFrom` | Roblox attributes new users to source already          | Acquisition dashboard                             |
| `Crash`, `LowFPS`, `ErrorThrown`                    | Engine instrumentation captures these                  | Performance + Error Report                        |

See `analytics-api-reference.md` "Built-in dashboards (don't
re-instrument)" for the full table and for the three things that look
like duplicates but aren't (`Source (IAP)`, onboarding funnel step 1,
value-distribution custom events).

## Validation

After publishing, the **View Events** panel on the Custom dashboard shows a
near-real-time stream of recent events. Use it to confirm event names,
custom-field schemas, and value ranges look right before waiting 24 h for
the charts to populate.

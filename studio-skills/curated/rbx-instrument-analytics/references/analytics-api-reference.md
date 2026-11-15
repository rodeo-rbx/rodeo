# AnalyticsService API Reference

Quick reference for `AnalyticsService` method signatures, enums, and limits. The
three event types are documented in detail in:

- `analytics-economy-events.md` — sources, sinks, transactions
- `analytics-funnel-events.md` — onboarding (one-time) and recurring funnels
- `analytics-custom-events.md` — counters and value events

## Service

```lua
local AnalyticsService = game:GetService("AnalyticsService")
```

`AnalyticsService` is **server-only** and only sends data from **published**
experiences. Calls from `LocalScript`, from the client `RunContext`, or while
running in Studio are silently dropped (or no-op for analytics purposes).
Always wrap log calls in instrumentation that already runs server-side
(`Players.PlayerAdded`, `MarketplaceService.ProcessReceipt`, server `RemoteEvent`
handlers, server-driven game-loop events).

## Built-in dashboards (don't re-instrument)

Roblox already provides several analytics dashboards out of the box.
**Don't burn an event-name slot or write a custom event for any metric
in this table** — it's already collected and charted.

| Dashboard                   | Already-tracked metrics                                                                                                                                            | Don't add a custom event for                                                                                                  |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| Engagement                  | Daily Active Users (DAU), New Users, Session time, Average session time, New User First Session Retention (5/10 min), CCU                                          | `PlayerJoined`, `SessionStart`, `SessionDuration`, `SessionEnded`, anything that just counts joins or measures session length |
| Retention                   | D1 / D7 / D30 retention, daily & weekly cohorts, 7D playtime / payer conversion / revenue per user, 30D revenue per user                                           | `Retention`, `RetentionDay1`, `ReturningPlayer`, `CohortDay7`                                                                 |
| Monetization (Overview)     | Robux revenue, payer conversion %, paying users count, ARPPU, ARPDAU, hourly & daily revenue                                                                       | `RobuxSpent`, `RobuxEarned`, `PurchaseRevenue`, `PayingUser`, `ARPPU`                                                         |
| Developer Product analytics | Top developer products, sales count per product, net revenue per product, time-series                                                                              | `BundlePurchased`, `DevProductSold`, anything that just counts a `ProductId` purchase                                         |
| Pass analytics              | Pass sales, pass revenue per item                                                                                                                                  | `PassActivated`, `GamePassPurchased`                                                                                          |
| Avatar Item analytics       | Top avatar items, sales, revenue per item                                                                                                                          | `AvatarItemSold`                                                                                                              |
| Subscription analytics      | Subscriber counts, churn, revenue                                                                                                                                  | `SubscriptionStarted`, `SubscriptionRenewed`, `SubscriptionCancelled`                                                         |
| Acquisition                 | New / returning users by source (Home, Search, Charts, sponsored / search / portal ads, Teleport, Other), qualified play-through rate, cumulative new-users funnel | `JoinSource`, `AcquisitionSource`, `Referral`, `TeleportedFrom`                                                               |
| Performance                 | Crash rate, frame rate, server / client perf                                                                                                                       | `Crash`, `LowFPS`, `FrameDrop`, `MemoryWarning`                                                                               |
| Error Report                | Runtime errors with stack traces                                                                                                                                   | `ErrorThrown`, `LuaError`, `ScriptCrashed`                                                                                    |

**Complementary, not duplicates** (keep these — different question):

- **Economy `Source (IAP)`** is _not_ a duplicate of the Monetization
  dashboard. Monetization tells you how much Robux you made; the
  Economy IAP event tells you what in-game currency / item the player
  received in exchange (with `itemSku`, `customFields`, post-grant
  `endingBalance`). Both are needed to correlate revenue with what was
  granted.
- **Onboarding funnel step 1 ("Player Joined")** is _not_ a duplicate
  of DAU. The funnel uses it as the anchor for measuring drop-off
  through later steps; DAU has no notion of "step 2 reached."
- **Custom events with a `value` distribution** (`MissionDuration`,
  `BossFightDamage`) aren't duplicated by any dashboard — keep those
  when the metric is the value distribution itself, not a count.

When in doubt, open the relevant Creator Dashboard page and check
whether the chart already exists. If it does, drop the custom event.

## Method signatures

### LogEconomyEvent

```lua
AnalyticsService:LogEconomyEvent(
    player: Player,
    flowType: Enum.AnalyticsEconomyFlowType,    -- Source | Sink
    currency: string,                           -- e.g. "Coins"; max 10 currencies
    amount: number,                             -- ALWAYS positive, even for sinks
    endingBalance: number,                      -- balance AFTER this transaction
    transactionType: string,                    -- pass enum .Name, or custom string
    itemSku: string?,                           -- optional unique item id
    customFields: {[string]: string}?           -- optional, see custom-fields below
)
```

### LogOnboardingFunnelStepEvent

```lua
AnalyticsService:LogOnboardingFunnelStepEvent(
    player: Player,
    step: number,                               -- 1-based step number (max 100 steps)
    stepName: string?,                          -- optional but strongly recommended
    customFields: {[string]: string}?
)
```

Use this for the **one-time onboarding (FTUE) funnel only.** Each player has a
single onboarding funnel; repeated step logs are ignored after the first.

### LogFunnelStepEvent

```lua
AnalyticsService:LogFunnelStepEvent(
    player: Player,
    funnelName: string,                         -- groups steps; max 10 funnels
    funnelSessionId: string,                    -- distinguishes recurring sessions
    step: number,                               -- 1-based (max 100 steps per funnel)
    stepName: string?,                          -- optional but strongly recommended
    customFields: {[string]: string}?
)
```

Use for **recurring funnels** (shop, item upgrade, match flow). Always pass a
`funnelSessionId` so multiple traversals don't collapse into a single funnel.

### LogCustomEvent

```lua
AnalyticsService:LogCustomEvent(
    player: Player,
    eventName: string,                          -- max 100 unique event names
    value: number?,                             -- optional; defaults to 1 (counter)
    customFields: {[string]: string}?
)
```

Use for anything that doesn't fit Economy or Funnel. Prefer \*\*fewer event names

- custom fields\*\* over many event names (event-name cardinality is capped at
  100).

## Enums

### `Enum.AnalyticsEconomyFlowType`

| Value    | Use for                                                     |
| -------- | ----------------------------------------------------------- |
| `Source` | Player **gains** the resource (rewards, IAP, daily login)   |
| `Sink`   | Player **spends** the resource (shop, ability cost, repair) |

### `Enum.AnalyticsEconomyTransactionType`

Pass `.Name` to keep dashboards consistent with Roblox's defaults.

| Enum                 | Direction      | Typical use                                                         |
| -------------------- | -------------- | ------------------------------------------------------------------- |
| `IAP`                | Source         | Robux developer-product/game-pass purchase that grants the resource |
| `TimedReward`        | Source         | Daily/hourly bonus, login streak                                    |
| `Onboarding`         | Source         | Welcome bonus, tutorial reward                                      |
| `Shop`               | Source or Sink | Buying or selling at an in-experience store                         |
| `Gameplay`           | Source or Sink | Quest reward, kill bonus, ability cost, match payout                |
| `ContextualPurchase` | Sink           | Impulse spend (extra life, revive, skip-timer)                      |

You can pass a **custom string** (not in the enum) for `transactionType` if
needed; the dashboard supports up to 20 transactionTypes before grouping the
rest into "Other."

### `Enum.AnalyticsCustomFieldKeys`

Always reference these via `.Name` when used as a dictionary key:

```lua
{
    [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Class - Warrior",
    [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Map - IceCave",
    [Enum.AnalyticsCustomFieldKeys.CustomField03.Name] = "Level - 12",
}
```

Only `CustomField01`, `CustomField02`, and `CustomField03` are read; any other
key in the dictionary is silently ignored. **Values must be strings** —
stringify booleans/numbers (`tostring(level)`).

## Limits (per place, daily)

| Limit                           | Cap                                     | Notes                                                                       |
| ------------------------------- | --------------------------------------- | --------------------------------------------------------------------------- |
| Global rate                     | `120 + 20 * CCU` requests/min           | Across all event types                                                      |
| Currencies                      | 10                                      | Use custom fields to subdivide (e.g. `XP` + `Class` field, not `WarriorXP`) |
| `transactionType` values        | Unlimited (top 20 shown, rest "Other")  | Prefer enum defaults                                                        |
| `itemSku` values                | Unlimited (top 100 shown, rest "Other") |                                                                             |
| Funnels                         | 10                                      | One is reserved for onboarding                                              |
| Steps per funnel                | 100                                     |                                                                             |
| Custom event names              | 100                                     | Prefer custom fields over new event names                                   |
| Custom fields per event         | 3                                       | `CustomField01`, `02`, `03`                                                 |
| Unique field-value combinations | 8,000 across all 3 fields               | Past this, values group as "Other"                                          |
| Dashboard retention             | 90 days from last event                 |                                                                             |

When the rate limit is hit, additional events are dropped silently. Batch
high-frequency events (e.g. `KillsLogged` with `value = 10`) instead of firing
once per kill.

## Validation channel

After publishing, verify events in the Creator Dashboard's **View Events** panel
on the Economy / Funnel / Custom pages. Errors appear in the **Error Report**
page (e.g. invalid enum, negative amount, missing player). Charts populate
~24 hours after the first event.

## Common pitfalls

- **Calling from client.** `AnalyticsService` runs server-only; `LocalScript`
  calls do nothing. If your existing currency code lives client-side, you need
  a server `RemoteEvent` to actually log.
- **Studio runs.** Events from Studio play-tests are not recorded. Test in a
  published experience.
- **Logging on attempt instead of success.** Log after `MarketplaceService.ProcessReceipt`
  returns `PurchaseGranted`, after the DataStore write succeeds, after the
  shop transaction commits — not when the player clicks the button.
- **Negative `amount` for sinks.** `amount` is always positive; the dashboard
  flips sinks to negative for display.
- **Wrong `endingBalance`.** Pass the balance **after** the transaction. Off-by-one
  here makes wallet-balance charts useless.
- **Repeated funnel steps.** Funnels only count the first instance per player
  (one-time) or per `funnelSessionId` (recurring). Repeated calls still
  consume rate limit.
- **Skipped funnel steps.** Logging step 3 without 1 or 2 auto-completes the
  earlier steps. Use this intentionally if needed; otherwise log every step.
- **Too many event names.** 100 cap is total across the experience. Use a
  shared name like `PlantSeed` + `CustomField01 = "Plant - Cabbage"` instead
  of `PlantCabbage`, `PlantTurnip`, etc.
- **Trusting client step numbers.** Validate funnel-step numbers server-side
  before logging — see `analytics-patterns.md` "Anti-cheat" section.

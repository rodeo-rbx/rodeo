# Funnel Events

Funnels track step-by-step progress through a sequence (onboarding, shop
checkout, item upgrade, match flow). The dashboard then shows the drop-off
between each step so you can identify where players quit.

Reference: <https://create.roblox.com/docs/production/analytics/funnel-events>

## Two flavors

| API                            | Use for                                                         | Funnel slot                    |
| ------------------------------ | --------------------------------------------------------------- | ------------------------------ |
| `LogOnboardingFunnelStepEvent` | First-time-user experience (FTUE), one-time per player          | The onboarding funnel (1 slot) |
| `LogFunnelStepEvent`           | Anything that repeats — shop checkout, item upgrade, match flow | Up to 9 named funnels          |

> **Funnels complement, not duplicate, the built-in dashboards.**
> Engagement / Retention / Acquisition tell you DAU, session time, and
> D1/D7/D30 rates — but none of them break down by _per-step_ drop-off
> through a flow. That's the gap funnels fill. In particular:
>
> - Onboarding step 1 ("Player Joined") is the funnel anchor for
>   measuring drop-off through later steps. It's _not_ a duplicate of
>   DAU, even though both are triggered by `Players.PlayerAdded`.
> - Don't write a custom `OnboardingCompleted` / `TutorialCompleted`
>   event alongside the onboarding funnel's last step — that's a
>   cross-type duplicate (see `analytics-custom-events.md`
>   "Don't double-log").
>
> Full list of what's already on built-in dashboards:
> `analytics-api-reference.md` "Built-in dashboards (don't
> re-instrument)."

## Onboarding (one-time)

Use `LogOnboardingFunnelStepEvent` for the very first run-through a player
does. Funnels only count the **first** instance per player; reconnects don't
need special handling — repeated calls are ignored by the funnel itself
(though they still consume rate limit).

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local Players = game:GetService("Players")

Players.PlayerAdded:Connect(function(player)
    AnalyticsService:LogOnboardingFunnelStepEvent(player, 1, "Player Joined")
end)

-- Later, after the player completes the tutorial planting step:
AnalyticsService:LogOnboardingFunnelStepEvent(player, 2, "Plant Seed")

-- And so on:
AnalyticsService:LogOnboardingFunnelStepEvent(player, 3, "Water Plant")
AnalyticsService:LogOnboardingFunnelStepEvent(player, 4, "Harvest Plant")
AnalyticsService:LogOnboardingFunnelStepEvent(player, 5, "Sell Plant")
```

Step numbers are 1-based and stable. **Don't reuse step numbers across
different meanings.** If the FTUE changes shape, append new steps rather than
renumbering — the dashboard preserves history per number.

If you log step 3 without 1 or 2, the funnel auto-completes 1 and 2. This is
fine for graceful recovery (e.g. a returning player who already passed step
1 in a prior session) but be intentional about it.

## Recurring funnels (shop, item upgrade, match flow)

Use `LogFunnelStepEvent` and **always include a `funnelSessionId`**. Without
it, multiple traversals collapse into a single funnel and the data becomes
useless.

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local HttpService = game:GetService("HttpService")

-- Shop session: new GUID each time the shop opens
local function startShopSession(player)
    local funnelSessionId = HttpService:GenerateGUID(false)
    -- store on the player or in a server-side map
    playerShopSessions[player] = funnelSessionId

    AnalyticsService:LogFunnelStepEvent(
        player,
        "ShopCheckout",                                     -- funnelName
        funnelSessionId,
        1,                                                  -- step
        "Opened Store"                                      -- stepName
    )
end

local function shopViewItem(player, itemId)
    AnalyticsService:LogFunnelStepEvent(
        player, "ShopCheckout", playerShopSessions[player], 2, "Viewed Item"
    )
end

local function shopAddToCart(player, itemId)
    AnalyticsService:LogFunnelStepEvent(
        player, "ShopCheckout", playerShopSessions[player], 3, "Added to Cart"
    )
end

local function shopPurchaseComplete(player, itemId)
    AnalyticsService:LogFunnelStepEvent(
        player, "ShopCheckout", playerShopSessions[player], 4, "Purchase Complete"
    )
end
```

### Choosing a `funnelSessionId`

| Situation                                            | Strategy                                                            |
| ---------------------------------------------------- | ------------------------------------------------------------------- |
| Shop session — short-lived, reentrant                | `HttpService:GenerateGUID(false)` per shop open                     |
| Item upgrade — long-lived (across sessions) per item | Build a stable key, e.g. `userId .. ":" .. itemId`                  |
| Match / round flow                                   | `userId .. ":" .. matchId` so the funnel ties to the specific match |
| Anything else recurring                              | A GUID per traversal is the safe default                            |

## Anti-cheat: never trust the client

Always validate funnel-step numbers server-side. The published doc shows the
canonical pattern: client sends a `RemoteEvent`, the server checks the step
number against a known max before logging.

```lua
-- ServerScriptService: OnboardingFunnelGateway.lua
local AnalyticsService = game:GetService("AnalyticsService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local onboardingEvent = ReplicatedStorage:WaitForChild("OnboardingEvent")
local MAX_STEP = 5
local stepNames = { "Player Joined", "Plant Seed", "Water Plant", "Harvest Plant", "Sell Plant" }

onboardingEvent.OnServerEvent:Connect(function(player, args)
    if typeof(args) ~= "table" then return end
    local step = tonumber(args.step)
    if not step or step < 1 or step > MAX_STEP then
        warn(("Invalid onboarding step %s from %s"):format(tostring(step), player.Name))
        return
    end
    AnalyticsService:LogOnboardingFunnelStepEvent(player, step, stepNames[step])
end)
```

Whenever you can derive the step entirely from server-side state (player
inventory, quest progress, level), prefer that over trusting any client
signal.

## Custom fields on funnels

Filters apply to the **first step only** to avoid double-counting when the
player's segment changes mid-funnel (e.g. switches device).

```lua
AnalyticsService:LogFunnelStepEvent(
    player,
    "ShopCheckout",
    sessionId,
    1,
    "Opened Store",
    {
        [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "ShopType - Armory",
        [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Map - IceCave",
        [Enum.AnalyticsCustomFieldKeys.CustomField03.Name] = string.format("Level - %d", level),
    }
)
```

You may pass custom fields on later steps — they'll be recorded — but the
funnel dashboard segments by the first-step values.

## Common funnel templates

Use these as starting points; add or remove steps based on your game.

### Onboarding / FTUE

Anchor on `Players.PlayerAdded` for step 1, then **jump directly to the
next gameplay milestone the player actively reaches**. Don't pad with
engine lifecycle steps.

| Step | Name                 | Triggered when                                                                                        |
| ---- | -------------------- | ----------------------------------------------------------------------------------------------------- |
| 1    | Player Joined        | `Players.PlayerAdded` (funnel anchor)                                                                 |
| 2    | Tutorial Started     | Tutorial UI opened or first prompt accepted                                                           |
| 3    | Tutorial Step N      | One step per major tutorial beat (each is a player action: planted seed, used ability, talked to NPC) |
| N    | Tutorial Completed   | Tutorial finishes / closes                                                                            |
| N+1  | First Reward Claimed | First gameplay reward earned                                                                          |

For places without an explicit tutorial, replace `Tutorial Started` →
`Tutorial Completed` with the actual first-session gameplay milestones
the player has to drive: first mission started, first puzzle solved,
first plot of land claimed, first match completed, etc. The principle
is the same — each step is a player decision or completion.

**Steps to avoid in the onboarding funnel** (low signal, near-100%
conversion, waste a step number):

- `Spawned` (`CharacterAdded` — the engine spawns characters; no
  meaningful drop-off).
- `First Move` (first walk / first input — every player who isn't
  AFK fires this; the gap from step 1 is just "did they tab away?",
  which Engagement DAU vs session length already exposes).
- `Lobby Countdown Ended` / `Teleported In` / `Streaming Ready` /
  any server-driven state transition the player doesn't decide. If
  drop-off here matters at all, it's "did the player quit during the
  wait", which `SessionOutcome` with an `Outcome - Lobby` custom field
  captures more cleanly without burning a funnel slot.
- Anything that fires identically on _every_ session. Funnel steps
  should fire exactly once per player (onboarding) or once per attempt
  (recurring), driven by player progression.

If you find yourself reaching for one of these, ask: _what player
decision happens between the previous step and this one?_ If the
answer is "none, the engine handles it," drop the step.

### Shop checkout (recurring)

| Step | Name                          |
| ---- | ----------------------------- |
| 1    | Opened Store                  |
| 2    | Viewed Item                   |
| 3    | Added to Cart / Selected Item |
| 4    | Confirmed Purchase            |
| 5    | Purchase Complete             |

### Item upgrade (recurring, long-lived `funnelSessionId`)

| Step | Name              |
| ---- | ----------------- |
| 1    | Opened Upgrade UI |
| 2    | Selected Item     |
| 3    | Began Upgrade     |
| 4    | Upgrade Complete  |

### Match / round (recurring)

| Step | Name            |
| ---- | --------------- |
| 1    | Joined Lobby    |
| 2    | Match Started   |
| 3    | First Objective |
| 4    | Match Ended     |
| 5    | Reward Claimed  |

## Repeating steps

The funnel only counts the **first** instance of each step per player
(one-time) or per `funnelSessionId` (recurring). Repeated step events are
discarded for funnel purposes but still consume the global rate limit, so
don't fire them in tight loops. Throttle with a server-side dedupe set:

```lua
local logged = {}  -- [player] = { [step] = true }

local function logStepOnce(player, funnel, sessionId, step, name)
    logged[player] = logged[player] or {}
    if logged[player][step] then return end
    logged[player][step] = true
    AnalyticsService:LogFunnelStepEvent(player, funnel, sessionId, step, name)
end
```

(Reset `logged[player]` on `PlayerRemoving`.)

## Skipped steps

Logging a later step without earlier steps auto-completes the earlier ones.
Useful when:

- A returning player resumes mid-flow and you only know they're past step N.
- The player skips an optional step (the funnel will treat them as if they
  did it).

If skipping is unintentional, you'll see funnels with 100 percent
step-1→step-2 conversion that doesn't match reality — log every step
explicitly to avoid this.

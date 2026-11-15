# Analytics Instrumentation Patterns

How to wire `AnalyticsService` into typical Roblox game systems. Each section
describes the **trigger** (what to look for in the place's scripts), the
**event type** to log, and a code template the agent should adapt.

All patterns assume server-side execution. If the relevant logic currently
lives on the client, the instrumentation must be done via a server
`RemoteEvent` handler with validation (see `analytics-funnel-events.md`
"Anti-cheat" section and the "Server-side gateway" pattern below).

## Server-side gateway pattern

Many games keep currency, quest progress, or tutorial state on the client.
Don't add `LogEconomyEvent` calls to a `LocalScript` — they'll silently fail.
Instead, add a centralized server module that holds the analytics surface
area for that system, and a thin `RemoteEvent` if needed.

```lua
-- ServerScriptService/Analytics/Currency.server.lua
local AnalyticsService = game:GetService("AnalyticsService")

local Currency = {}

function Currency.applySource(player, currency, amount, transactionType, itemSku, fields)
    local wallet = WalletStore.get(player)
    wallet[currency] += amount
    WalletStore.save(player, wallet)

    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Source,
        currency,
        amount,
        wallet[currency],
        transactionType,
        itemSku,
        fields
    )
end

function Currency.applySink(player, currency, amount, transactionType, itemSku, fields)
    local wallet = WalletStore.get(player)
    if wallet[currency] < amount then return false end
    wallet[currency] -= amount
    WalletStore.save(player, wallet)

    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Sink,
        currency,
        amount,
        wallet[currency],
        transactionType,
        itemSku,
        fields
    )
    return true
end

return Currency
```

Now every existing currency mutation in the codebase becomes a single
`Currency.applySource(...)` or `Currency.applySink(...)` call, and the
analytics is automatic. This is preferred over scattering `LogEconomyEvent`
calls everywhere.

---

## Pattern: developer-product / IAP grant

**Trigger:** `MarketplaceService.ProcessReceipt`, `MarketplaceService:PromptProductPurchase`,
`PromptGamePassPurchaseFinished`.

**Event:** Economy `Source` with `transactionType = IAP`, plus a custom
`PurchaseCompleted` event if you want richer aggregation.

```lua
local MarketplaceService = game:GetService("MarketplaceService")
local AnalyticsService = game:GetService("AnalyticsService")

local PRODUCT_TO_BUNDLE = {
    [123456] = { currency = "Coins", amount = 1000, sku = "1000CoinBundle" },
    [123457] = { currency = "Gems",  amount = 50,   sku = "50GemPack"     },
}

local function processReceipt(receiptInfo)
    local player = game:GetService("Players"):GetPlayerByUserId(receiptInfo.PlayerId)
    if not player then
        return Enum.ProductPurchaseDecision.NotProcessedYet
    end

    local bundle = PRODUCT_TO_BUNDLE[receiptInfo.ProductId]
    if not bundle then
        return Enum.ProductPurchaseDecision.NotProcessedYet
    end

    -- Grant first; if grant fails, don't log analytics.
    local ok, endingBalance = pcall(grantBundle, player, bundle)
    if not ok then
        return Enum.ProductPurchaseDecision.NotProcessedYet
    end

    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Source,
        bundle.currency,
        bundle.amount,
        endingBalance,
        Enum.AnalyticsEconomyTransactionType.IAP.Name,
        bundle.sku
    )

    return Enum.ProductPurchaseDecision.PurchaseGranted
end

MarketplaceService.ProcessReceipt = processReceipt
```

---

## Pattern: in-experience shop

**Trigger:** server-side shop modules, `RemoteFunction:OnServerInvoke` for
"buy item," `MarketplaceService.PromptPurchaseFinished` for game passes.

**Event:**

- Economy `Sink` with `transactionType = Shop`, `itemSku = <item id>`, on
  successful purchase.
- Recurring funnel `ShopCheckout` for store-open / view / confirm / complete
  flow (helps surface drop-off in the purchase UI).

```lua
-- ServerScriptService/Shop/ShopGateway.server.lua
local HttpService = game:GetService("HttpService")
local AnalyticsService = game:GetService("AnalyticsService")
local Players = game:GetService("Players")

local shopSessions = {}  -- [player] = funnelSessionId

local function logShopStep(player, step, name)
    local sessionId = shopSessions[player]
    if not sessionId then return end
    AnalyticsService:LogFunnelStepEvent(
        player, "ShopCheckout", sessionId, step, name
    )
end

function Shop.openStore(player)
    shopSessions[player] = HttpService:GenerateGUID(false)
    logShopStep(player, 1, "Opened Store")
end

function Shop.viewItem(player, itemId)
    logShopStep(player, 2, "Viewed Item")
end

function Shop.confirmPurchase(player, itemId)
    logShopStep(player, 3, "Confirmed Purchase")
end

function Shop.tryBuy(player, itemId)
    local item = ITEMS[itemId]
    if not item then return false, "unknown item" end

    local granted = Currency.applySink(
        player, item.currency, item.cost,
        Enum.AnalyticsEconomyTransactionType.Shop.Name,
        itemId,
        {
            [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Category - " .. item.category,
        }
    )
    if not granted then return false, "insufficient funds" end

    Inventory.grant(player, itemId)
    logShopStep(player, 4, "Purchase Complete")
    return true
end

Players.PlayerRemoving:Connect(function(player)
    shopSessions[player] = nil
end)
```

---

## Pattern: onboarding / FTUE

**Trigger:** `Players.PlayerAdded` for step 1 (the funnel anchor); every
later step is called from a gameplay system at the moment the player
actually drives that milestone.

**Event:** `LogOnboardingFunnelStepEvent` with stable step numbers.

**Anchor on `PlayerAdded`, then jump straight to gameplay.** Don't add
intermediate engine-lifecycle steps (`CharacterAdded` / "Spawned",
"Countdown Ended", "Teleport Done"). They produce near-100% conversion
rows that waste step numbers and obscure the real drop-offs you want to
investigate. See `analytics-funnel-events.md` "Steps to avoid in the
onboarding funnel" for the full list.

```lua
-- ServerScriptService/Onboarding/OnboardingTracker.server.lua
local AnalyticsService = game:GetService("AnalyticsService")
local Players = game:GetService("Players")

local STEPS = {
    [1] = "Player Joined",
    [2] = "Tutorial Started",      -- player accepted first prompt
    [3] = "First Reward Claimed",  -- player earned their first reward
    [4] = "Tutorial Completed",    -- player finished the tutorial
}

local Onboarding = {}

function Onboarding.markStep(player, step)
    local name = STEPS[step]
    if not name then
        warn(("Unknown onboarding step %s"):format(tostring(step)))
        return
    end
    AnalyticsService:LogOnboardingFunnelStepEvent(player, step, name)
end

Players.PlayerAdded:Connect(function(player)
    Onboarding.markStep(player, 1)
end)

return Onboarding
```

Steps 2-N are called from the relevant gameplay systems at the moment
the player completes each milestone:

- `TutorialController:onPromptAccepted` → `Onboarding.markStep(player, 2)`
- `RewardService:onFirstReward` → `Onboarding.markStep(player, 3)`
- `TutorialController:onComplete` → `Onboarding.markStep(player, 4)`

Server-side state is the source of truth — never trust the client to send
the step number directly without validation.

---

## Pattern: quests / missions

**Trigger:** quest-accept, quest-complete, quest-abandon hooks in a
`QuestController` or `QuestService`.

**Event:**

- Custom `MissionStarted` / `MissionCompleted` / `MissionFailed`, all using
  `CustomField01 = "Mission - <id>"` rather than per-mission event names.
- Economy `Source` (`Gameplay`) when the quest reward currency is granted.
- Optional value event `MissionCompletedDuration`.

```lua
function Quest.complete(player, questId)
    local def = QUEST_DEFS[questId]
    local startTime = playerQuestStartTimes[player][questId]
    local duration = math.floor(workspace:GetServerTimeNow() - startTime)

    Currency.applySource(
        player, "Coins", def.reward,
        Enum.AnalyticsEconomyTransactionType.Gameplay.Name,
        questId
    )

    AnalyticsService:LogCustomEvent(
        player, "MissionCompleted", 1,
        {
            [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Mission - " .. questId,
        }
    )
    AnalyticsService:LogCustomEvent(
        player, "MissionCompletedDuration", duration,
        {
            [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Mission - " .. questId,
        }
    )
end
```

---

## Pattern: loss / failure (only when the game has it)

`Loss` is the broad concept — any state in which the player has failed
the current activity. Death is one specific shape of loss; elimination,
mission failure, race DNF, surrender, time-out, and game-over screens
are others.

**Applies when** the game has an explicit loss branch — combat games,
platformers, survival, PvP, racing with DNF, mission-based PvE,
anything with `Humanoid.Died`, `KillBrick` / lava parts, a
`PlayerKilled` server event, a `Round.Lose(player)` /
`Match.Eliminate(player)` call, or an "objective failed" / "game over"
state.

**Doesn't apply** to puzzle games, simulators, social spaces,
builders / tycoons, idle games, or any title where players normally end
a session by quitting or finishing rather than failing. For those, the
meaningful outcome signal lives on the round / loop / match flow (see
the `MatchFlow` funnel above) or on a domain-specific outcome event
(`PuzzleOutcome` with `Outcome - Solved/Abandoned`, `RaceFinished` with
`Result - Won/DNF`, `RebirthCompleted`) — not on `Humanoid.Died`, and
not on a custom session-length event (Roblox's built-in Engagement
dashboard already reports session length). If the audit found few or
zero `LOSS` pattern hits, that's the cue to skip this pattern entirely.

**Trigger:** `Humanoid.Died`, `Humanoid:GetState() == Dead`, custom
`PlayerKilled` server event, `KillBrick` / lava parts, `Round.Lose`,
`Match.Eliminate`, mission-failure handlers, surrender / forfeit
handlers.

**Event:** Custom `Loss` counter, with cause and map as custom fields.
Use a more specific name (`Death`, `Elimination`, `MatchLost`) only if
the game's losses are uniformly one kind; otherwise `Loss` plus a
`Cause` custom field keeps cardinality low.

```lua
-- ServerScriptService/Combat/LossTracker.server.lua
local Players = game:GetService("Players")
local AnalyticsService = game:GetService("AnalyticsService")

local function trackCharacter(character, cause)
    local humanoid = character:WaitForChild("Humanoid", 5)
    if not humanoid then return end
    humanoid.Died:Connect(function()
        local player = Players:GetPlayerFromCharacter(character)
        if not player then return end
        AnalyticsService:LogCustomEvent(
            player, "Loss", 1,
            {
                [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Cause - " .. (cause or "Unknown"),
                [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Map - " .. CurrentMap.name,
            }
        )
    end)
end

Players.PlayerAdded:Connect(function(player)
    player.CharacterAdded:Connect(function(character)
        trackCharacter(character, "Death - Generic")
    end)
end)
```

For specific causes (lava brick, NPC, PvP, mission timeout, surrender),
call `AnalyticsService:LogCustomEvent` directly from the losing code
path with the appropriate `Cause` field, and skip the generic listener
for those losses.

---

## Pattern: round / match flow

**Trigger:** match-start / match-end hooks in a `MatchManager`,
`Round.Begin`, `Round.End`, etc.

**Event:** Recurring funnel `MatchFlow` with `funnelSessionId = matchId`.

```lua
local AnalyticsService = game:GetService("AnalyticsService")

function Match.begin(matchId, players)
    for _, player in players do
        AnalyticsService:LogFunnelStepEvent(
            player, "MatchFlow", matchId, 1, "Match Started",
            {
                [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Mode - " .. Match.mode,
                [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Map - " .. Match.map,
            }
        )
    end
end

function Match.objectiveReached(matchId, player, objectiveId)
    AnalyticsService:LogFunnelStepEvent(
        player, "MatchFlow", matchId, 2, "First Objective"
    )
end

function Match.endRound(matchId, players, outcome)
    for _, player in players do
        AnalyticsService:LogFunnelStepEvent(
            player, "MatchFlow", matchId, 3, "Match Ended"
        )
        AnalyticsService:LogCustomEvent(
            player, "MatchOutcome", 1,
            {
                [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Outcome - " .. outcome,
                [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Mode - " .. Match.mode,
            }
        )
    end
end
```

---

## Pattern: ability / weapon usage

**Trigger:** `Tool.Activated`, ability cast handlers in a `AbilityService`,
weapon `:Fire()` calls.

**Event:** Custom `AbilityUsed` / `WeaponFired` counter, batched server-side.

```lua
-- ServerScriptService/Combat/CombatTelemetry.server.lua
local AnalyticsService = game:GetService("AnalyticsService")
local Players = game:GetService("Players")

local FLUSH_INTERVAL = 30

local pending = {}  -- [player] = { [weaponName] = count }

function CombatTelemetry.recordWeaponFire(player, weaponName)
    pending[player] = pending[player] or {}
    pending[player][weaponName] = (pending[player][weaponName] or 0) + 1
end

local function flush(player)
    local buckets = pending[player]
    if not buckets then return end
    pending[player] = nil
    for weaponName, count in buckets do
        AnalyticsService:LogCustomEvent(
            player, "WeaponFired", count,
            {
                [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Weapon - " .. weaponName,
            }
        )
    end
end

task.spawn(function()
    while true do
        task.wait(FLUSH_INTERVAL)
        for player in pending do flush(player) end
    end
end)

Players.PlayerRemoving:Connect(flush)
```

---

## Pattern: timed rewards / daily login

**Trigger:** server-side daily-login module, login-streak controller,
`PlayerAdded` + DataStore lookup.

**Event:** Economy `Source` with `transactionType = TimedReward`.

```lua
function DailyLogin.tryAward(player)
    local today = os.date("!*t").yday
    local store = LoginStore.get(player)
    if store.lastDay == today then return end

    store.streak = (store.lastDay == today - 1) and (store.streak + 1) or 1
    store.lastDay = today
    LoginStore.save(player, store)

    local reward = REWARDS[math.min(store.streak, #REWARDS)]
    Currency.applySource(
        player, reward.currency, reward.amount,
        Enum.AnalyticsEconomyTransactionType.TimedReward.Name,
        ("DailyLogin_Day%d"):format(store.streak)
    )
end
```

---

## Pattern: ProximityPrompt / interactable

**Trigger:** `ProximityPrompt.Triggered` (server-side; the event fires on
both server and client, but server is authoritative).

**Event:** Custom `Interaction` counter with the prompt id as a custom field.

```lua
local function bindPrompt(prompt, interactionId)
    prompt.Triggered:Connect(function(player)
        AnalyticsService:LogCustomEvent(
            player, "Interaction", 1,
            {
                [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Interaction - " .. interactionId,
                [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Map - " .. CurrentMap.name,
            }
        )
    end)
end
```

---

## Common discovery signals during the audit

When scanning a place, these tokens usually map to instrumentation
opportunities:

| Signal in source / tree                                                                | Likely event                                               |
| -------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `MarketplaceService.ProcessReceipt`, `PromptProductPurchase`, `PromptGamePassPurchase` | Economy `Source` (IAP)                                     |
| `:SetAttribute("Coins"`, `leaderstats.Coins`, `wallet.Coins`, `currency` mutations     | Economy events                                             |
| `Shop`, `Store`, `Buy`, `Purchase` modules / UIs                                       | Economy + ShopCheckout funnel                              |
| `Tutorial`, `Onboarding`, `FTUE`, `Welcome`                                            | Onboarding funnel                                          |
| `Quest`, `Mission`, `Objective`                                                        | Custom MissionStarted/Completed + Economy Source on reward |
| `Round`, `Match`, `Game.Begin`, `Game.End`                                             | MatchFlow funnel + custom events                           |
| `Humanoid.Died`, `KillBrick`, `Lava`, `Eliminate`, `OnFail`, `GameOver`, `Surrender`   | Custom Loss event (with `Cause` field)                     |
| `Tool.Activated`, `Weapon`, `Ability:Cast`                                             | Custom WeaponFired/AbilityUsed (batched)                   |
| `ProximityPrompt`, `ClickDetector`                                                     | Custom Interaction event                                   |
| `DailyLogin`, `LoginStreak`, `Reward`                                                  | Economy Source (TimedReward)                               |
| `Players.PlayerAdded`                                                                  | Onboarding funnel step 1 anchor                            |

For each signal, follow the corresponding pattern in this file.

## Don't double-log: cross-type deduplication

Before adding a new event, check whether an existing one already covers
the metric. There are **two** sources of duplication to rule out:

1. **Built-in dashboards.** Engagement (DAU, session time, CCU),
   Retention (D1 / D7 / D30), Monetization (Robux revenue, payer
   conversion, ARPPU), Developer Product / Pass / Avatar Item /
   Subscription analytics, Acquisition (source attribution),
   Performance (crashes, FPS), and Error Report are collected
   automatically. If the metric is on one of those pages, don't write
   _any_ event for it — see `analytics-api-reference.md`
   "Built-in dashboards (don't re-instrument)" for the full table.
2. **Cross-event-type collisions.** If the metric isn't on a built-in
   dashboard, pick **one** primary event type per gameplay trigger
   using this priority: Economy (currency / resource gain or spend) →
   Funnel (step in a sequence where drop-off matters) → Custom
   (everything else).

Common duplicates to avoid:

| Trigger                                     | Already covered by                                                                                                           | Don't also add                                                                                            |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Player joins the experience                 | Engagement dashboard (DAU)                                                                                                   | Custom `PlayerJoined` event (onboarding funnel step 1 is OK — it's the funnel anchor, not a join counter) |
| Player session ends                         | Engagement dashboard (session time)                                                                                          | Custom `SessionDuration` / `SessionEnded` event                                                           |
| Player buys 1000-coin bundle for Robux      | Economy `Source (IAP)` with `itemSku = "1000CoinBundle"` (and Monetization / Developer Product analytics for the Robux side) | Custom `BundlePurchased` event                                                                            |
| Game pass activated                         | Economy `Source (IAP)` for the in-game grant + Pass analytics for revenue                                                    | Custom `PassActivated` event                                                                              |
| Player teleports in from another experience | Acquisition dashboard ("Teleport" source)                                                                                    | Custom `JoinSource` / `TeleportedFrom` event                                                              |
| Match begins                                | `MatchFlow` funnel step "Match Started"                                                                                      | Custom `MatchStarted` event                                                                               |
| Tutorial finishes                           | Onboarding funnel last step "Tutorial Completed"                                                                             | Custom `TutorialCompleted` event                                                                          |
| Coin reward granted at quest completion     | Economy `Source (Gameplay)` for the coin grant                                                                               | Custom `CoinsEarned` / `RewardClaimed` event                                                              |
| Shop purchase succeeds                      | Economy `Sink (Shop)` + `ShopCheckout` funnel step "Purchase Complete"                                                       | Custom `ItemBought` event                                                                                 |
| Script throws a runtime error               | Error Report dashboard                                                                                                       | Custom `ErrorThrown` event                                                                                |

Add a second event for the same trigger **only** when it carries a
metric the primary event can't surface. Justified examples:

- `MatchOutcome` custom event alongside the `MatchFlow` funnel — the
  funnel can't capture the win/loss outcome value or break it down by
  `Mode` / `Map` custom fields the way a custom event can.
- `MissionCompletedDuration` custom value event alongside the Economy
  `Source` for the mission reward — the Economy dashboard tracks coin
  flow, not how long the mission took.
- A custom value event that records distribution (`avg`, `min`, `max`,
  histogram) where the primary event only counts.

When in doubt, drop the secondary event. It's cheaper to add one later
than to detangle double-counted metrics in a published place.

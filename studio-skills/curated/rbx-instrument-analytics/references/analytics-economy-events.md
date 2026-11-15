# Economy Events

Track every flow of an in-experience resource (currency, premium currency,
crafting material, energy, etc.) so the Economy dashboard can show top sinks,
top sources, average wallet balance, and net flow per category.

Reference: <https://create.roblox.com/docs/production/analytics/economy-events>

## When to log

Log **after a successful transaction** — after the wallet has been written to
the DataStore, after `ProcessReceipt` returned `PurchaseGranted`, after the
inventory item has been granted. Never log on intent (button click) because
the underlying transaction may fail.

```lua
local AnalyticsService = game:GetService("AnalyticsService")
```

## Pattern: source events (player gains)

```lua
-- Player completes a level and earns 50 coins; ending balance becomes 100.
AnalyticsService:LogEconomyEvent(
    player,
    Enum.AnalyticsEconomyFlowType.Source,
    "Coins",                                                -- currency
    50,                                                     -- amount earned
    100,                                                    -- balance AFTER
    Enum.AnalyticsEconomyTransactionType.Gameplay.Name      -- "Gameplay"
)
```

For an in-app purchase that grants a resource bundle:

```lua
-- Player buys a 1000-coin bundle for Robux. Pre-balance was 20.
AnalyticsService:LogEconomyEvent(
    player,
    Enum.AnalyticsEconomyFlowType.Source,
    "Coins",
    1000,                                                   -- amount granted
    1020,                                                   -- balance AFTER
    Enum.AnalyticsEconomyTransactionType.IAP.Name,
    "1000CoinBundle"                                        -- itemSku
)
```

Always log IAP grants from inside `MarketplaceService.ProcessReceipt`, after
returning `Enum.ProductPurchaseDecision.PurchaseGranted` (or just before, but
only on the success path).

> **`Source (IAP)` is not a duplicate of the built-in Monetization
> dashboard.** Monetization tells you Robux revenue and per-product
> sales counts (already counted automatically); the Economy IAP event
> tells you what in-game currency or item the player received in
> exchange — `currency`, `amount`, `itemSku`, `endingBalance`, and any
> `customFields`. You need both to correlate Robux revenue with the
> in-game grant. **Don't drop these events** thinking the Monetization
> dashboard already covers them. (Conversely, don't add a custom
> `BundlePurchased` / `PassActivated` event that just counts the
> purchase — Developer Product / Pass analytics handles that. See
> `analytics-api-reference.md` "Built-in dashboards.")

## Pattern: sink events (player spends)

```lua
-- Player spends 80 coins to buy DoubleJumpUpgrade in the shop. Balance after: 20.
AnalyticsService:LogEconomyEvent(
    player,
    Enum.AnalyticsEconomyFlowType.Sink,
    "Coins",
    80,                                                     -- cost (POSITIVE)
    20,                                                     -- balance AFTER
    Enum.AnalyticsEconomyTransactionType.Shop.Name,
    "DoubleJumpUpgrade"
)
```

`amount` stays positive. The dashboard flips sinks to negative when graphed.

## Choosing the transaction type

| Scenario                                                 | `flowType` | `transactionType`    | `itemSku`                 |
| -------------------------------------------------------- | ---------- | -------------------- | ------------------------- |
| Robux developer-product or game-pass grants the currency | `Source`   | `IAP`                | bundle id                 |
| Player completes quest / wins match / scores points      | `Source`   | `Gameplay`           | quest/level id (optional) |
| Daily-login bonus, hourly chest, login streak            | `Source`   | `TimedReward`        | reward id (optional)      |
| First-time-user welcome bonus                            | `Source`   | `Onboarding`         | bonus id                  |
| Player sells an item back at the shop                    | `Source`   | `Shop`               | item id                   |
| Player buys an item at the in-experience shop            | `Sink`     | `Shop`               | item id                   |
| Player pays to use an ability or fast-travel             | `Sink`     | `Gameplay`           | ability/skill id          |
| Pay-to-revive, refill timer, extra life                  | `Sink`     | `ContextualPurchase` | feature id                |
| One-off scenarios that don't match                       | either     | custom string        | optional                  |

`itemSku` is optional but **highly recommended** for shop and IAP events —
without it, the dashboard shows "N/A" in the top-sources/top-sinks tables.

## Currency naming

You get only **10 currencies**. Don't burn slots on near-duplicates — use
custom fields to subdivide instead.

```lua
-- BAD: burns 4 currency slots
"WarriorXP", "MageXP", "PaladinXP", "RogueXP"

-- GOOD: 1 currency, breakdown by class via custom field
"XP" with CustomField01 = "Class - Warrior"
```

Pick stable, lower-camel or PascalCase names (`Coins`, `Gems`, `XP`,
`CraftingMaterial`). Renaming later breaks dashboard continuity.

## Custom fields on economy events

Up to 3 `customFields`, all string values:

```lua
AnalyticsService:LogEconomyEvent(
    player,
    Enum.AnalyticsEconomyFlowType.Sink,
    "Coins",
    80,
    20,
    Enum.AnalyticsEconomyTransactionType.Shop.Name,
    "ObsidianSword",
    {
        [Enum.AnalyticsCustomFieldKeys.CustomField01.Name] = "Category - Weapon",
        [Enum.AnalyticsCustomFieldKeys.CustomField02.Name] = "Class - Warrior",
        [Enum.AnalyticsCustomFieldKeys.CustomField03.Name] = string.format("Level - %d", playerLevel),
    }
)
```

Tips:

- Including the dimension prefix (`"Class - Warrior"` rather than `"Warrior"`)
  makes filter labels self-explanatory in the dashboard.
- Use the same custom-field schema across all economy events for a given
  feature so you can cross-filter.
- Keep value cardinality bounded — once total unique values across the 3
  fields exceeds 8,000, extras group into "Other."

## Anti-pattern: logging on intent

```lua
-- WRONG: button click — purchase may fail (insufficient funds, race condition)
buyButton.MouseButton1Click:Connect(function()
    AnalyticsService:LogEconomyEvent(player, Sink, "Coins", 80, ?, Shop, "Sword")
    tryBuySword(player)
end)

-- RIGHT: log inside the success branch of the actual transaction
local function tryBuySword(player)
    local wallet = getWallet(player)
    if wallet.Coins < 80 then return false end
    wallet.Coins -= 80
    saveWallet(player, wallet)
    grantItem(player, "Sword")
    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Sink,
        "Coins", 80, wallet.Coins,
        Enum.AnalyticsEconomyTransactionType.Shop.Name,
        "Sword"
    )
    return true
end
```

## Anti-pattern: wrong ending balance

`endingBalance` must be the wallet value **after** the transaction has been
applied. Off-by-one here makes the average-wallet-balance chart wrong.

```lua
-- WRONG: pre-transaction balance
AnalyticsService:LogEconomyEvent(player, Sink, "Coins", 80, wallet.Coins + 80, ...)

-- RIGHT: post-transaction balance
wallet.Coins -= 80
AnalyticsService:LogEconomyEvent(player, Sink, "Coins", 80, wallet.Coins, ...)
```

---
name: instrument-analytics
description: Audit a Roblox experience for analytics-instrumentation opportunities and add Economy, Funnel, and Custom events via AnalyticsService. Supports a read-only audit that proposes an event plan and an instrument mode that creates a centralized analytics module and edits scripts to log events at the right places.
---

# Instrument Analytics

Adds Roblox `AnalyticsService` instrumentation to a Roblox place. The skill
runs in two modes:

- **Audit mode (read-only).** Inventory the place's mechanics (currencies,
  shops, IAPs, tutorial/quest/match systems, loss/failure states,
  abilities), map them to Economy / Funnel / Custom events, and produce
  a plan. No scripts are modified.
- **Instrument mode.** Run the audit, present the plan for approval, then
  create the analytics scaffold (a centralized server module) and edit the
  identified scripts to log events at the right places.

Infer the mode from the user's request. Requests to audit, review, or recommend
tracking are audit-only. Requests to set up, add, or instrument analytics use
instrument mode.

## When to use

Use this skill when the user asks to:

- "Set up / add analytics for my game"
- "Track economy events" / "track currencies" / "track IAP"
- "Add a funnel for onboarding / tutorial / shop"
- "Track player losses / deaths / quests / abilities / matches"
- Any reference to `AnalyticsService`, `LogEconomyEvent`,
  `LogFunnelStepEvent`, `LogOnboardingFunnelStepEvent`, `LogCustomEvent`,
  the Economy / Funnel / Custom dashboards.

## Prerequisites

- Roblox Studio open with the target place loaded
- The Roblox Studio MCP server connected to the agent
- The user understands that **events only send from a published place on the
  server.** Studio playtests and client `LocalScript` calls are no-ops for
  analytics. Confirm this up front so the user isn't surprised.

## Reference Documentation

The agent must read these reference docs whenever it's about to make a
decision or write code involving the corresponding API. They are the source
of truth for method signatures, enum values, limits, and patterns.

- [API and limits](references/analytics-api-reference.md)
- [Economy events](references/analytics-economy-events.md)
- [Funnel events](references/analytics-funnel-events.md)
- [Custom events](references/analytics-custom-events.md)
- [Discovery patterns](references/analytics-patterns.md)

---

## Mode selection

| User said                                                    | Mode       | Stop after                      |
| ------------------------------------------------------------ | ---------- | ------------------------------- |
| "audit my analytics", "what should I track"                  | Audit-only | Step 5 (deliver plan; no edits) |
| "set up analytics", "add tracking", "instrument my analytics" | Instrument | Step 8 (validation + report)    |

If the request is ambiguous, default to audit and tell the user: "I ran the
audit. Say 'go ahead and instrument' after you've reviewed the plan."

---

## Step 1: Sanity check the environment

Run via `execute_luau`. This confirms the Studio session is real, picks up
the place name, and verifies the server-only / published-only constraint
isn't going to surprise the user later.

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local RunService = game:GetService("RunService")

local report = {}
table.insert(report, "place: " .. game.Name .. " (placeId=" .. tostring(game.PlaceId) .. ")")
table.insert(report, "isStudio: " .. tostring(RunService:IsStudio()))
table.insert(report, "isServer: " .. tostring(RunService:IsServer()))
table.insert(report, "AnalyticsService methods present:")
for _, method in {
    "LogEconomyEvent", "LogOnboardingFunnelStepEvent",
    "LogFunnelStepEvent", "LogCustomEvent",
} do
    table.insert(report, ("  - %s: %s"):format(
        method, tostring(typeof(AnalyticsService[method]) == "function")))
end

return table.concat(report, "\n")
```

If any of the four methods are missing, abort: the engine version is too old
for this skill. Otherwise proceed.

If the place's `PlaceId == 0`, warn: this is an unsaved local file. Events
can't be validated against the live dashboard until the place is published.

---

## Step 2: Map the place

Build an inventory of services, scripts, and likely game systems.

### 2a: Workspace and service tree

Use `search_game_tree` with increasing depth:

1. `search_game_tree(path: "ServerScriptService", max_depth: 4)`
2. `search_game_tree(path: "ReplicatedStorage", max_depth: 4)`
3. `search_game_tree(path: "StarterPlayer", max_depth: 4)`
4. `search_game_tree(path: "Workspace", max_depth: 3)` — usually only
   relevant for `KillBrick`, `ProximityPrompt`, `Tool` placement.

Note Folder names matching: `Shop`, `Store`, `Quest`, `Mission`,
`Tutorial`, `Onboarding`, `FTUE`, `Match`, `Round`, `Currency`, `Wallet`,
`Inventory`, `Combat`, `Ability`, `Reward`. Each is a strong hint about
what events to add.

### 2b: All scripts, by side

Use `search_game_tree(instance_type: "BaseScript")` plus
`search_game_tree(instance_type: "ModuleScript")`. Categorize each:

- **Server scripts** — `Script` in `ServerScriptService`, or with
  `RunContext = Server`.
- **Client scripts** — `LocalScript`, or `Script` with
  `RunContext = Client`.
- **Shared modules** — `ModuleScript` in `ReplicatedStorage`.
- **Server-only modules** — `ModuleScript` in `ServerScriptService`.
- **Client-only modules** — `ModuleScript` in `StarterPlayerScripts` /
  `StarterGui`.

`AnalyticsService` calls must live server-side. Keep a list of which scripts
are eligible.

### 2c: Existing analytics

Check whether the place already calls `AnalyticsService`. Use `script_grep`
on these patterns:

- `AnalyticsService` (any reference)
- `LogEconomyEvent`
- `LogOnboardingFunnelStepEvent`
- `LogFunnelStepEvent`
- `LogCustomEvent`

For every hit, plan to **avoid duplicating** the existing event. If the
existing call uses a different naming convention, surface the conflict in
the plan rather than silently overriding.

---

## Step 3: Inventory candidate events

Run a code-pattern scan to find the trigger points described in
`analytics-patterns.md` "Common discovery signals during the audit." Use
`execute_luau` with the script below; the patterns are tuned for the
typical Roblox vocabulary.

```lua
local ScriptEditorService = game:GetService("ScriptEditorService")

local PATTERNS = {
    -- Economy
    { id = "IAP_RECEIPT",   group = "economy",   desc = "MarketplaceService.ProcessReceipt",
      pats = { "ProcessReceipt" } },
    { id = "IAP_PROMPT",    group = "economy",   desc = "PromptProductPurchase / PromptGamePassPurchase",
      pats = { "PromptProductPurchase", "PromptGamePassPurchase" } },
    { id = "PURCHASE_FINISHED", group = "economy", desc = "PromptPurchaseFinished / PromptGamePassPurchaseFinished",
      pats = { "PromptPurchaseFinished", "PromptGamePassPurchaseFinished" } },
    { id = "WALLET_MUT",    group = "economy",   desc = "Currency / wallet mutation",
      pats = {
          "leaderstats", "leaderstat",
          "%.Coins", "%.Gems", "%.Cash", "%.Gold", "%.XP", "%.Tokens",
          "wallet%.", "currency", "Currency",
          "SetAttribute%(.-Coins", "SetAttribute%(.-Gems", "SetAttribute%(.-Cash",
      } },
    { id = "SHOP",          group = "economy",   desc = "Shop / store / buy / purchase module",
      pats = { "Shop", "Store", "TryBuy", "Purchase", "BuyItem" } },
    { id = "REWARD",        group = "economy",   desc = "Reward grant",
      pats = { "GrantReward", "GiveReward", "AwardReward", "DailyReward", "LoginStreak", "LoginReward" } },

    -- Funnels
    { id = "PLAYER_ADDED",  group = "funnel",    desc = "Players.PlayerAdded (onboarding step 1 anchor)",
      pats = { "PlayerAdded[:%.]Connect", "Players%.PlayerAdded" } },
    { id = "CHARACTER_ADDED", group = "funnel",  desc = "CharacterAdded (post-spawn step)",
      pats = { "CharacterAdded[:%.]Connect" } },
    { id = "TUTORIAL",      group = "funnel",    desc = "Tutorial / onboarding / FTUE",
      pats = { "Tutorial", "Onboarding", "FTUE", "Welcome" } },
    { id = "MATCH",         group = "funnel",    desc = "Match / round / game flow",
      pats = { "MatchManager", "RoundManager", "Round%.Begin", "Round%.End", "Match%.Begin", "Match%.End" } },

    -- Custom (gameplay)
    { id = "LOSS",          group = "custom",    desc = "Loss / failure signals (death, elimination, fail-state, lost match)",
      pats = {
          -- death-shaped losses
          "Humanoid%.Died", "Humanoid:Die", "KillBrick", "%.Killed", ":TakeDamage",
          -- non-death loss / failure signals
          "Eliminate", "Eliminated", "Defeat", "Defeated",
          "Lose%(", "%.Lost", "OnLoss", "MatchLost", "RoundLost",
          "%.Failed", "OnFail", "GameOver", "YouLost", "YouLose",
          "Surrender", "Forfeit", "GiveUp",
      } },
    { id = "QUEST",         group = "custom",    desc = "Quest / mission / objective",
      pats = { "Quest", "Mission", "Objective" } },
    { id = "ABILITY",       group = "custom",    desc = "Ability / spell cast",
      pats = { "Ability", "Spell:Cast", "Cast%(", "UseAbility" } },
    { id = "WEAPON",        group = "custom",    desc = "Tool / weapon",
      pats = { "Tool%.Activated", "Weapon", ":Fire%(", "FireWeapon" } },
    { id = "INTERACT",      group = "custom",    desc = "ProximityPrompt / ClickDetector",
      pats = { "ProximityPrompt", "%.Triggered[:%.]Connect", "ClickDetector", "MouseClick" } },

    -- Existing analytics
    { id = "EXISTING",      group = "existing",  desc = "Already using AnalyticsService",
      pats = { "AnalyticsService", "LogEconomyEvent", "LogFunnelStepEvent",
               "LogOnboardingFunnelStepEvent", "LogCustomEvent" } },
}

local function side(inst)
    if inst:IsA("LocalScript") then return "client" end
    if inst:IsA("Script") then
        local rc = inst.RunContext
        if rc == Enum.RunContext.Client then return "client" end
        if rc == Enum.RunContext.Server then return "server" end
        return "server"
    end
    local fullName = inst:GetFullName()
    if string.find(fullName, "^StarterPlayer") or string.find(fullName, "^StarterGui") then
        return "client_module"
    elseif string.find(fullName, "^ServerScriptService") then
        return "server_module"
    elseif string.find(fullName, "^ReplicatedStorage") then
        return "shared_module"
    end
    return "unknown"
end

local results = {}
local hitsByPattern = {}
for _, p in PATTERNS do hitsByPattern[p.id] = 0 end

local function scan(service)
    if not service then return end
    for _, desc in service:GetDescendants() do
        if not (desc:IsA("Script") or desc:IsA("LocalScript") or desc:IsA("ModuleScript")) then
            continue
        end
        local ok, source = pcall(function()
            return ScriptEditorService:GetEditorSource(desc)
        end)
        if not ok or not source or #source == 0 then continue end

        local lines = string.split(source, "\n")
        local hits = {}
        for _, patDef in PATTERNS do
            local matched = {}
            for ln, line in lines do
                for _, pat in patDef.pats do
                    if string.find(line, pat) then
                        table.insert(matched, ln)
                        break
                    end
                end
            end
            if #matched > 0 then
                hits[patDef.id] = matched
                hitsByPattern[patDef.id] += #matched
            end
        end
        if next(hits) then
            table.insert(results, {
                path = desc:GetFullName(),
                side = side(desc),
                hits = hits,
            })
        end
    end
end

for _, svc in {
    game:GetService("ServerScriptService"),
    game:GetService("ReplicatedStorage"),
    game:GetService("StarterPlayer"),
    game:GetService("StarterGui"),
    workspace,
} do scan(svc) end

local out = { "Pattern hits (across all scripts):" }
for _, p in PATTERNS do
    if hitsByPattern[p.id] > 0 then
        table.insert(out, ("  [%s] %s (%s): %d"):format(
            p.group, p.id, p.desc, hitsByPattern[p.id]))
    end
end

table.insert(out, "")
table.insert(out, "Scripts with hits:")
for _, r in results do
    local ids = {}
    for id in r.hits do table.insert(ids, id) end
    table.sort(ids)
    table.insert(out, ("  [%s] %s :: %s"):format(r.side, r.path, table.concat(ids, ", ")))
end

return table.concat(out, "\n")
```

For each script with hits, follow up with `script_read` to **classify**
each finding precisely. Examples of decisions to make:

- `WALLET_MUT` hit — is this gameplay (`Source: Gameplay`), shop
  (`Sink: Shop`), or daily (`Source: TimedReward`)?
- `IAP_RECEIPT` — list every developer-product id and what it grants;
  each grant becomes one `LogEconomyEvent` (Source/IAP).
- `TUTORIAL` — derive an ordered list of tutorial beats and their step
  numbers.
- `MATCH` — figure out where match-start, match-objective, and match-end
  fire so you can wire the recurring funnel.
- `LOSS` — what does losing look like in this game (death, elimination,
  failed objective, surrender, time-out)? Decide on a single generic
  listener vs cause-specific listeners (lava, PvP, boss, mission fail).
  If the only hits are death-shaped (`Humanoid.Died`), name the event
  `Death`; if losses are broader (PvP elimination, race DNF, mission
  fail), name it `Loss` and use a `Cause` custom field.

If the place is large or scripts are long, prefer reading per-script over
bulk reads. Take notes per script in your scratch state.

### Read the game shape, not just the pattern hits

The pattern table catches generic Roblox vocabulary; it doesn't tell you
what the game _is_. Before assuming any template event applies, look at
the surrounding signals:

- **Folder / module names** in `ServerScriptService` and
  `ReplicatedStorage` (`Round`, `Match`, `Race`, `Build`, `Lobby`,
  `Save`, `Garden`, `Tycoon`, `Plot`, `Pet`, `Trade`, `Lobby`) usually
  identify the genre.
- **`Players.PlayerRemoving` handlers** show what state the game treats
  as worth saving when a player leaves; reading those handlers reveals
  the natural session boundary and what the player was doing at it.
  (Don't fire a `SessionDuration` custom event from here — Roblox's
  built-in Engagement dashboard already reports session length, DAU,
  and retention. Use this to _plan_ the right outcome event, not to
  duplicate session length.)
- **Absence of a pattern is a signal too.** Zero `LOSS` hits, zero
  `SHOP` hits, zero `TUTORIAL` hits each mean the game probably doesn't
  have that mechanic. Don't backfill events from the templates just
  because they exist in `analytics-patterns.md` — the template list is
  a menu, not a requirement.
- **Match the event to the actual loop.** A puzzle or simulator's
  meaningful loss might be quitting mid-puzzle (use a custom event
  with `Outcome - Quit/Solved/Abandoned` on session end), not death.
  A racing game's is a finish-line crossing or a DNF, not death. A
  tycoon's is a dropper purchase or rebirth, not death. Decide what
  the player is _trying_ to do, then pick the event that captures
  success / failure / drop-off for that activity.

Carry these observations into Step 4 so the plan reflects the place's
actual mechanics, not the pattern scanner's vocabulary.

---

## Step 4: Build the event plan

Map every classified hit to a concrete event. Use the templates in
`analytics-patterns.md`.

The plan must declare, for every event:

| Field                             | Example                                                                |
| --------------------------------- | ---------------------------------------------------------------------- |
| Event type                        | Economy / Onboarding / Funnel / Custom                                 |
| Trigger                           | Where in the codebase it fires (script + condition)                    |
| Server gate                       | Existing server hook, or a new RemoteEvent that needs a server gateway |
| Currency / funnelName / eventName | `Coins` / `ShopCheckout` / `MissionStarted`                            |
| flowType / step / value           | `Sink` / `2 (Viewed Item)` / `120`                                     |
| transactionType / itemSku         | `Shop` / `DoubleJumpUpgrade`                                           |
| customFields                      | `CustomField01 = "Class - Warrior"`                                    |

### Plan rules

- Respect the **10-currency cap.** If you find more than 10 distinct
  currency names, propose merging via custom fields (e.g. consolidate
  `WarriorXP`, `MageXP`, `PaladinXP` into `XP` with a `Class` custom
  field). See `analytics-economy-events.md` "Currency naming."
- Respect the **10-funnel cap.** Reserve one slot for onboarding. Don't
  propose more than 9 named funnels.
- Respect the **100-event-name cap.** Prefer custom fields over distinct
  event names. See `analytics-custom-events.md` "Use custom fields
  aggressively."
- Don't propose events the place already logs (per Step 2c). Where the
  existing schema is reasonable, extend it; where it conflicts with best
  practice, flag for the user.
- Prefer **server-side triggers**. If the only existing trigger is
  client-side, plan a server gateway (RemoteEvent + handler). Mark these
  rows in the plan as `+ gateway needed`.
- Every economy event needs `endingBalance`. If the script doesn't already
  read the post-transaction balance, include a note about how the gateway
  module exposes it.
- **One trigger, one primary event type.** For each gameplay trigger,
  pick exactly one primary event using this priority:
  1. **Economy** if it's a currency / resource gain or spend.
  2. **Funnel** if it's a step in a sequence where drop-off matters.
  3. **Custom** for everything else.

  Only add a second event for the same trigger if it carries a metric
  the primary one can't surface — e.g. a value distribution, a
  custom-field breakdown, or a duration the funnel/economy dashboard
  doesn't expose. Note the justification in Step 5D
  ("Deduplication audit"). Examples that are **not** justified:
  - `MatchStarted` custom event when `MatchFlow` funnel already has a
    "Match Started" step.
  - `BundlePurchased` custom event when an Economy `Source (IAP)` row
    already covers the same purchase.
  - `TutorialCompleted` custom event when the onboarding funnel's last
    step is "Tutorial Completed."

- **Funnel steps must be player-driven gameplay milestones, not
  engine / lifecycle transitions.** A useful funnel step is something
  the player _did_ (puzzle solved, item placed, choice made, mission
  started, reward claimed) or a clear gameplay milestone they reached
  (round started, boss encountered). Engine lifecycle events
  (`CharacterAdded` / "spawned", countdown elapsed, teleport finished,
  server-state transitions the player passively observes) are **bad
  funnel steps**: drop-off between them is either ~0% (engine always
  succeeds) or is already captured by `SessionOutcome` quit-stage
  bucketing. They turn into 99%-conversion rows that waste step numbers
  and obscure the real drop-offs. The onboarding funnel should anchor
  on `Players.PlayerAdded` (step 1) and then **jump directly to the
  next gameplay milestone the player actively reaches** — first
  tutorial prompt accepted, first puzzle/mission started, first reward
  claimed — not "Spawned" or "Countdown Ended" in between. Same rule
  applies to recurring funnels: don't pad with "Teleported In",
  "Streaming Ready", "Cutscene Played"; only steps where you'd
  actually act on drop-off.
- **Don't propose any event whose metric is already on a Roblox
  built-in dashboard.** Engagement (DAU, session time, CCU), Retention
  (D1 / D7 / D30, cohorts), Monetization (Robux revenue, payer
  conversion, ARPPU, ARPDAU), Developer Product / Pass / Avatar Item /
  Subscription analytics (per-product sales & revenue), Acquisition
  (source attribution), Performance (crashes, FPS), and Error Report
  are all collected automatically. A custom `PlayerJoined`,
  `SessionDuration`, `BundlePurchased`, `PassActivated`, `JoinSource`,
  or `Crash` event just burns an event-name slot. The full table —
  including the three "looks like a duplicate but isn't" cases
  (`Source (IAP)`, onboarding step 1, value-distribution customs) — is
  in `analytics-api-reference.md` "Built-in dashboards (don't
  re-instrument)."
- **Don't propose template events the place's mechanics don't justify.**
  The pattern table in Step 3 surfaces _candidates_, not requirements.
  Some games have no loss mechanic (puzzles, simulators, social spaces,
  builders, idle, tycoon), no shop (story-only), or no tutorial. If
  Step 3 produced few or zero hits for a given category, that's a
  signal the event isn't relevant — don't add it just to fill out the
  plan. For loss / failure signals specifically, see
  `analytics-patterns.md` "Pattern: loss / failure" for when a `Loss`
  (or genre-specific `Death`, `Elimination`) event is appropriate vs.
  when a round / loop outcome event with an `Outcome` custom field is
  the right fit. (Session length itself is already reported by Roblox's
  built-in Engagement dashboard — don't add a custom event for it.)

---

## Step 5: Deliver the plan (audit mode stops here)

Output **every** section below. If a section has no items, write "None."

### A. Existing analytics

List every script already calling `AnalyticsService`, with the call
site(s) and the schema in use (currencies, funnels, event names).

### B. Place inventory summary

- Total scripts scanned
- Server / client / module breakdown
- Existing currencies detected
- Existing shops / IAP products / quests / tutorials / matches detected

### C. Proposed events

Group by type. Use these tables verbatim. Within any single table, each
trigger should appear at most once. If a trigger needs to appear in
**more than one** table (e.g. an IAP grant that's both an Economy `Source`
and step 5 of a `ShopCheckout` funnel), every occurrence after the first
must have a matching row in section D ("Deduplication audit").

#### Economy

| Currency | Direction | transactionType | itemSku | Trigger (script:line) | customFields | Notes |
| -------- | --------- | --------------- | ------- | --------------------- | ------------ | ----- |

#### Onboarding funnel (one-time)

| Step | Step name | Trigger | customFields |
| ---- | --------- | ------- | ------------ |

#### Recurring funnels

| Funnel name | sessionId strategy | Step | Step name | Trigger |
| ----------- | ------------------ | ---- | --------- | ------- |

#### Custom events

| Event name | Counter / Value | Value source | Trigger | customFields | Notes |
| ---------- | --------------- | ------------ | ------- | ------------ | ----- |

### D. Deduplication audit

Walk every row in the Step 5C tables and check for two kinds of
duplication:

1. **Cross-table collisions** — the same trigger (script + line, or
   logical event such as "purchase grant succeeds") appears in more
   than one of the Economy / Funnel / Custom tables.
2. **Built-in dashboard duplicates** — the proposed event re-states
   a metric Roblox already collects automatically (Engagement,
   Retention, Monetization, Developer Product / Pass / Avatar Item /
   Subscription analytics, Acquisition, Performance, Error Report).
   See `analytics-api-reference.md` "Built-in dashboards (don't
   re-instrument)" for the full list.

List every duplicate of either kind and either justify it or drop it.

| Trigger / proposed event | Primary event (table + row) | Other occurrence(s) / built-in dashboard | Decision | Justification |
| ------------------------ | --------------------------- | ---------------------------------------- | -------- | ------------- |

Rules:

- **Default = drop the duplicate.** Custom events that just re-state a
  funnel step, an economy event, or a built-in dashboard metric add
  cardinality without insight.
- **Keep the duplicate only if** the secondary event carries a metric
  the primary event / built-in dashboard doesn't expose: a value
  distribution, a different custom-field breakdown, a duration, or an
  outcome category. Spell out which metric in the Justification column.
- The three "looks like a duplicate but isn't" cases stay even though
  they overlap with built-in dashboards: Economy `Source (IAP)` (carries
  the in-game grant the Monetization dashboard can't see), onboarding
  funnel step 1 ("Player Joined" — the funnel anchor for measuring
  drop-off), and any value-distribution custom event.
- If this section is non-empty after the dedup pass, also update the
  affected rows in 5C (e.g. add `(see Deduplication audit)` in the Notes
  column) so a reader following only one table can find the rationale.

### E. Gateways and infrastructure to create

List every new server-side module, gateway, or `RemoteEvent` the plan
requires. For each, give the path, purpose, and what existing
client/server code it replaces or wraps.

### F. Limit budget

Tally proposed usage against the limits:

| Limit                      | Cap | Proposed use            | OK? |
| -------------------------- | --- | ----------------------- | --- |
| Currencies                 | 10  |                         |     |
| Funnels (incl. onboarding) | 10  |                         |     |
| Custom event names         | 100 |                         |     |
| Custom fields per event    | 3   | (≤3 in every row above) |     |

If any limit is exceeded, the plan **must** be revised before instrumenting.

### G. Open questions for the user

List every decision the agent couldn't make confidently. Common ones:

- Currency unification (e.g. consolidate XP variants?)
- Whether a recurring funnel should treat session A as a continuation of
  session B
- itemSku scheme for procedurally-generated items
- Whether to instrument cosmetic-only events (camera mode, settings)

In **audit mode**, stop here and wait. In **instrument mode**, present the
plan, then explicitly ask: _"Approve this plan, or which rows should I
change/skip?"_ before doing any edits.

---

## Step 6 (instrument mode): Create the analytics scaffold

Goal: route every existing currency / quest / tutorial mutation through one
small set of server-side modules so future analytics changes are local.

### 6a: Single source-of-truth module

Create `ServerScriptService.Analytics` (or similar name; check `Step 2c` so
you don't collide with an existing module). Use `multi_edit` while Studio is
in **edit mode** to create each ModuleScript below. `multi_edit` creates the
intermediate `Analytics` Folder automatically. For each new ModuleScript, set
`className` to `ModuleScript` and use one edit whose `old_string` is empty and
whose `new_string` is the complete source shown below.

Never create a script or assign its `Source` through `execute_luau`.

#### `ServerScriptService.Analytics.Currency`

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local Currency = {}

-- Apply a currency gain. balanceAfter is the wallet value AFTER the change.
function Currency.applySource(player, currency, amount, balanceAfter,
                              transactionType, itemSku, customFields)
    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Source,
        currency,
        amount,
        balanceAfter,
        transactionType,
        itemSku,
        customFields
    )
end

-- Apply a currency spend. balanceAfter is the wallet value AFTER the change.
function Currency.applySink(player, currency, amount, balanceAfter,
                            transactionType, itemSku, customFields)
    AnalyticsService:LogEconomyEvent(
        player,
        Enum.AnalyticsEconomyFlowType.Sink,
        currency,
        amount,
        balanceAfter,
        transactionType,
        itemSku,
        customFields
    )
end

return Currency
```

#### `ServerScriptService.Analytics.Onboarding`

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local Onboarding = {}

-- Step names by step number; replace with the project's actual FTUE list.
local STEPS = {
    [1] = "Player Joined",
    -- TODO: fill in the rest of the onboarding steps from the plan.
}

function Onboarding.markStep(player, step)
    local name = STEPS[step]
    if not name then
        warn(("Onboarding: unknown step %d"):format(step))
        return
    end
    AnalyticsService:LogOnboardingFunnelStepEvent(player, step, name)
end

return Onboarding
```

#### `ServerScriptService.Analytics.Funnels`

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local HttpService = game:GetService("HttpService")
local Players = game:GetService("Players")

local Funnels = {}
local sessions = {}  -- [funnelName] = { [player] = sessionId }

function Funnels.startSession(player, funnelName, sessionId)
    sessions[funnelName] = sessions[funnelName] or {}
    sessions[funnelName][player] = sessionId or HttpService:GenerateGUID(false)
    return sessions[funnelName][player]
end

function Funnels.endSession(player, funnelName)
    if sessions[funnelName] then sessions[funnelName][player] = nil end
end

function Funnels.logStep(player, funnelName, step, stepName, customFields)
    local bucket = sessions[funnelName]
    local sessionId = bucket and bucket[player]
    if not sessionId then
        warn(("Funnels: no session for %s on funnel %s"):format(player.Name, funnelName))
        return
    end
    AnalyticsService:LogFunnelStepEvent(
        player, funnelName, sessionId, step, stepName, customFields
    )
end

Players.PlayerRemoving:Connect(function(player)
    for funnelName, bucket in sessions do
        bucket[player] = nil
    end
end)

return Funnels
```

#### `ServerScriptService.Analytics.Custom`

```lua
local AnalyticsService = game:GetService("AnalyticsService")
local Players = game:GetService("Players")

local Custom = {}

function Custom.log(player, eventName, value, customFields)
    AnalyticsService:LogCustomEvent(player, eventName, value, customFields)
end

-- Batched counter helpers: increment many times, flush once per interval.
local pending = {}  -- [player] = { [eventName] = { value = n, fields = ... } }
local FLUSH_INTERVAL = 30

function Custom.bump(player, eventName, customFields)
    pending[player] = pending[player] or {}
    local bucket = pending[player][eventName]
    if not bucket then
        bucket = { value = 0, fields = customFields }
        pending[player][eventName] = bucket
    end
    bucket.value += 1
end

local function flush(player)
    local buckets = pending[player]
    if not buckets then return end
    pending[player] = nil
    for eventName, bucket in buckets do
        AnalyticsService:LogCustomEvent(player, eventName, bucket.value, bucket.fields)
    end
end

task.spawn(function()
    while true do
        task.wait(FLUSH_INTERVAL)
        for player in pending do flush(player) end
    end
end)

Players.PlayerRemoving:Connect(flush)

return Custom
```

After all four `multi_edit` calls are accepted, use `execute_luau` to verify
that the modules exist:

```lua
local SSS = game:GetService("ServerScriptService")
local folder = SSS:FindFirstChild("Analytics")
if not folder then return "ERROR: folder missing" end
local missing = {}
for _, name in {"Currency", "Onboarding", "Funnels", "Custom"} do
    if not folder:FindFirstChild(name) then table.insert(missing, name) end
end
return #missing == 0 and "OK" or ("MISSING: " .. table.concat(missing, ", "))
```

### 6b: Fill `Onboarding.STEPS`

Replace the placeholder `STEPS` table with the actual FTUE step list from
the approved plan. Use `multi_edit` on `ServerScriptService.Analytics.Onboarding`.

### 6c: Add gateway RemoteEvents (only if needed)

For any plan rows tagged `+ gateway needed`, create a `RemoteEvent` in
`ReplicatedStorage` and a server handler under `ServerScriptService.Analytics`.
Use the validation pattern from `analytics-funnel-events.md` "Anti-cheat:
never trust the client."

Don't create gateway events the plan didn't call for — overuse adds attack
surface and rate-limit pressure.

---

## Step 7 (instrument mode): Edit scripts

Process scripts **one at a time**, in this priority order:

1. Server scripts that already mutate currencies (`WALLET_MUT` hits in
   `ServerScriptService` / server modules).
2. Server scripts that handle IAP / `ProcessReceipt`.
3. Tutorial / onboarding controllers (server-side).
4. Match / round managers (server-side).
5. Quest / mission systems (server-side).
6. Combat / loss tracking (server-side).
7. Shop / store gateways (server-side).
8. Last: anywhere a server gateway needs to be added because the existing
   logic is client-only.

For each script:

1. Read with `script_read`.
2. Identify all instrumentation points from the approved plan.
3. Apply all changes for that script in **one** `multi_edit` call. Don't
   trickle edits.
4. Verify by re-reading the script and checking every event from the plan
   is present and references `Analytics.Currency`, `Analytics.Onboarding`,
   `Analytics.Funnels`, or `Analytics.Custom`.

### Edit guidelines

- **Always require modules by `WaitForChild`**, not by index. The Analytics
  folder may not yet exist when a script first runs:

  ```lua
  local SSS = game:GetService("ServerScriptService")
  local Analytics = SSS:WaitForChild("Analytics")
  local Currency = require(Analytics:WaitForChild("Currency"))
  ```

- **Log on success, not on intent.** If an edit replaces a click handler
  that previously called `LogEconomyEvent` directly, move the call into the
  branch that runs only after the underlying transaction succeeds. See
  `analytics-economy-events.md` "Anti-pattern: logging on intent."

- **Compute `endingBalance` from the actual wallet write.** Don't reuse a
  pre-mutation copy of the value:

  ```lua
  -- WRONG
  local before = wallet.Coins
  Currency.applySink(player, "Coins", 80, before, ...)
  wallet.Coins -= 80

  -- RIGHT
  wallet.Coins -= 80
  Currency.applySink(player, "Coins", 80, wallet.Coins, ...)
  ```

- **Use `enum.Name` for transactionType.** `Enum.AnalyticsEconomyTransactionType.Shop.Name`
  yields the string `"Shop"` that the dashboard expects.

- **Use `Enum.AnalyticsCustomFieldKeys.CustomField0X.Name` for keys** in
  the custom-fields dictionary, not raw strings like `"CustomField01"`.

- **Don't break existing behavior.** Analytics calls must be additive. If
  removing an old call requires reorganizing logic, defer the cleanup and
  flag it under "Manual review needed."

- **Don't add `LogEconomyEvent` to a `LocalScript`.** It silently no-ops.
  Replace with a server gateway per Step 6c.

- **Don't introduce new currencies or event names not in the approved
  plan.** Any drift here corrupts dashboard cardinality.

- **Don't add a custom event whose trigger and meaning duplicate a
  funnel step or economy event already in the plan.** Step 5D
  ("Deduplication audit") is the source of truth: if a candidate
  duplicate isn't listed there with a justification, drop it from this
  edit and surface it under "Manual review needed" in the final report.
  Re-running the audit is cheaper than backing out double-logged events
  from a published place.

### Flag-for-review situations

Don't try to edit; surface in the final report:

- The script uses a custom replication framework (Roact, Knit, Matter,
  ReplicaService, etc.) and the analytics call needs to integrate with the
  framework's lifecycle. Suggest the integration point but don't auto-edit.
- The script ties currency to a third-party DataStore wrapper and the
  post-mutation balance isn't easily recoverable. Suggest where to thread
  the value through.
- The script handles batched events from a server-authoritative loop where
  per-event logging would exceed the rate limit; suggest the batching
  pattern from `analytics-custom-events.md` but let the user shape the
  policy.

---

## Step 8 (instrument mode): Validate and report

### 8a: Re-scan

Re-run the Step 3 scan. The output should now show:

- `EXISTING` hits in every script the plan called out — at least once per
  proposed event.
- No new hits in unexpected locations.

### 8b: Static checks

Via `execute_luau`, walk the new `Analytics` folder and verify:

```lua
local SSS = game:GetService("ServerScriptService")
local folder = SSS:FindFirstChild("Analytics")
local out = {}
if not folder then return "ERROR: Analytics folder missing" end
for _, child in folder:GetChildren() do
    local ok, mod = pcall(require, child)
    table.insert(out, ("%s: %s"):format(child.Name, ok and "loaded" or ("ERROR " .. tostring(mod))))
end
return table.concat(out, "\n")
```

All four modules must report `loaded`. Anything else means the scaffold
script source has a syntax error — fix before reporting completion.

### 8c: Smoke playtest checklist

Tell the user to:

1. Save the place.
2. Publish to Roblox.
3. Run a normal session in the published place.
4. Visit the Creator Dashboard → Analytics → Economy / Funnel / Custom →
   "View Events" panel. The events should appear within ~1 minute.
5. Wait ~24 h before expecting charts to populate.

If events don't appear in **View Events**:

- Confirm the place was actually published (not a Studio-only run).
- Confirm the script side is server (`AnalyticsService` is server-only).
- Visit the Analytics → Error Report page for runtime errors.

### 8d: Final report

Produce the report below; **every** section is required.

#### 1. Mode and scope

Audit-only or instrument; requested analytics scope; place name + id.

#### 2. Scaffold created (instrument mode only)

| Path | Purpose |
| ---- | ------- |

#### 3. Scripts edited (instrument mode only)

| Path | Events added | Notes |
| ---- | ------------ | ----- |

#### 4. Final event schema

Repeat the Plan tables from Step 5C, but as the **shipped** schema (with
the correct line numbers from the edited files).

#### 5. Limit budget — actual

| Limit | Cap | Used | Headroom |
| ----- | --- | ---- | -------- |

#### 6. Manual review needed

Anything flagged during Step 7. For each, explain what's blocking the
automated edit and what the user must do.

#### 7. Validation results

- Re-scan summary (new `EXISTING` hits, vs the plan)
- Module-load smoke test result
- Studio playtest steps the user should run
- Where to verify in Roblox dashboard

#### 8. Recommended next steps

- Top 3 dashboards to monitor first (Economy net flow, onboarding drop-off,
  whichever recurring funnel is most actionable for this game).
- Custom-field schema cheat sheet for the team (so future events use
  consistent values).
- When to add a second pass (after first 2 weeks of data once dashboards
  populate).

---

## Out of scope (always)

Flag for manual review rather than attempting:

- Designing brand-new in-game systems just to have something to track
  (analytics should reflect existing mechanics, not invent them).
- Adding currencies the place doesn't already use.
- Restructuring the existing networking layer beyond a thin server gateway.
- Migrating away from a third-party analytics framework. If the place uses
  one, integrate alongside it (or surface the conflict).
- Changing a place's monetization model. Track what's there; let the user
  decide what to add.

---

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

---

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

---

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

---

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

---

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

---

## Command Workflows

The sections below define the audit-only and instrumentation modes. Follow the matching workflow when the user requests that mode.

### `/analytics-audit`

Use the `instrument-analytics` skill in audit-only mode.

Inspect the experience, identify instrumentation opportunities, and deliver
the complete event plan. Do not create or modify anything.

### `/analytics-instrument`

Use the `instrument-analytics` skill in instrument mode.

First audit the experience and present the event plan. Do not modify scripts
until the user explicitly approves that plan. After approval, instrument and
validate the approved events.

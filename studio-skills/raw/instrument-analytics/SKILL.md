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

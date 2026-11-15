---
name: rbx-process-receipt-misuse
description: "Audit Developer Product receipt handling for ProcessReceipt mistakes. Use to review or debug Developer Product purchases, including out-of-experience purchases that fail."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

# ProcessReceipt misuse detection

Audit an experience's Developer Product receipt handling and return a
`PASS` / `FAIL` / `REVIEW` verdict with evidence. This is an **audit skill, not
an enforcement gate** — read the relevant scripts, reason about the failure
paths, and when the evidence is genuinely inconclusive return `REVIEW` rather
than guessing.

**Default behavior: report only.** Never modify an audited script. Only apply a
fix if the user explicitly asks for one after seeing the report.

## When to Use

Trigger this skill when the user asks to audit, review, or debug Developer
Product purchases or receipt handling.

- Keywords: `ProcessReceipt`, `Developer Product`, `receipt`, `PurchaseGranted`,
  `NotProcessedYet`, `PromptProductPurchaseFinished`, `Store tab`, `EDP`,
  `Personalized Shop`, `purchase lost`, `bought but didn't get`.
- Common phrasings: "check / audit my ProcessReceipt code", "is my receipt
  handling safe?", "why do Store-tab (Shop / EDP) purchases fail?", "players
  paid out of game and never got the item", "are my purchases getting lost or
  double-granted?"

## Do NOT use when

- The user is not asking about purchases, receipts, or Developer Products.
- The user asked to *build* a purchase flow from scratch — that is authoring,
  not auditing. Build it correctly using "Recommended fixes" below instead.
- The question is only about Game Passes or subscriptions (different APIs).

## Disambiguating fuzzy input

Users rarely say "ProcessReceipt". Map their words to the audit:

| User says | Read it as |
|---|---|
| "Store tab", "Shop", "Personalized Shop", "buying from the app", "EDP", "out of game" | **Out-of-experience** purchase — the case that only `ProcessReceipt` fulfills |
| "purchase disappeared", "paid but got nothing", "lost Robux" | Silent purchase loss — check the failure path returns `NotProcessedYet`, not `PurchaseGranted` |
| "got it twice", "double reward on rejoin" | Idempotency / `PurchaseId` persistence (Section B) |
| "gamepass" | Gamepasses are **not** Developer Products — say so and scope the audit to Developer Products only |

## Background: why this matters

Roblox calls `MarketplaceService.ProcessReceipt` after a Developer Product is
purchased and keeps re-calling it until the handler returns
`Enum.ProductPurchaseDecision.PurchaseGranted`. **Out-of-experience** purchases
(Store tab / EDP / Personalized Shops) only work when fulfillment goes through
`ProcessReceipt`, because those buyers are **not in the game** — any fulfillment
wired to an in-experience event never fires for them. A handler that returns
`PurchaseGranted` before the item is actually granted permanently closes the
receipt with no retry, silently losing the purchase.

## How to scan

Use the read-only script tools Studio Assistant provides. Do not rely on any
tool outside Studio Assistant.

### Step 1 — Find candidate scripts

Use `script_grep` (or `script_search`) to search every script source for the
receipt signal terms, then note the `path:line` of each hit:

```
ProcessReceipt, receiptInfo, PromptProductPurchaseFinished, PromptPurchaseFinished,
PurchaseGranted, NotProcessedYet, ProductPurchaseDecision,
DataStoreService, GetDataStore, GetAsync, SetAsync, UpdateAsync,
PurchaseId, GetPlayerByUserId, LocalPlayer, PlayerAdded
```

Also use `search_game_tree` to list scripts whose **names** suggest purchase
handling (`Receipt`, `Purchase`, `Product`, `Marketplace`, `Monetization`,
`Shop`, `Store`, `DeveloperProduct`) to catch handlers that reference the API
indirectly.

### Step 2 — Read the receipt-handling scripts

For each flagged script, read the full source with `script_read` / `read_script`
(and `inspect_instance` for structure). **Follow the control flow**: if the
`ProcessReceipt` handler calls helper functions or `require`s modules to perform
the grant, read those too before deciding. The verdict depends on what happens
on the *failure* path, not just the happy path.

## Detection taxonomy

Classify the experience as exactly one of `PASS`, `FAIL`, or `REVIEW`.

### Section A blockers → `FAIL`

Any one of these is a `FAIL` (they break out-of-experience purchases):

1. **`prompt_finished_misuse`** — `PromptProductPurchaseFinished` (or
   `PromptPurchaseFinished`) is used to *fulfill* Developer Product purchases
   instead of `ProcessReceipt`. This event only fires for in-experience buyers,
   so Store-tab / EDP buyers never get their item. Also fires when **no**
   `MarketplaceService.ProcessReceipt` is assigned in any server script but a
   `PromptProductPurchaseFinished` handler grants items.

2. **`no_player_existence_check`** (silent purchase loss) — the handler can
   return `PurchaseGranted` when the grant did **not** actually happen. Signals:
   - `Players.LocalPlayer` referenced inside a server-side `ProcessReceipt`
     (always `nil` on the server).
   - A broad `pcall`/error path that swallows failures and still returns
     `PurchaseGranted`.
   - In-memory state (e.g. a callbacks table or player-data object populated by
     `PlayerAdded`) required to fulfill the grant, which may not be ready when
     `ProcessReceipt` fires on join, **and** the failure path returns
     `PurchaseGranted` instead of `NotProcessedYet`.

   > Key distinction: a missing nil-check is **not** a `FAIL` by itself. It is
   > only a `FAIL` when the failure/error path returns `PurchaseGranted` (silent
   > permanent loss) rather than `NotProcessedYet` (safe retry).

   > **Do NOT flag the opposite pattern as a blocker.** Code that checks
   > `Players:GetPlayerByUserId(receiptInfo.PlayerId)` and returns
   > `NotProcessedYet` when it is `nil` is the **correct, safe** pattern — this
   > is exactly what "Recommended fixes" tells developers to do, so it can
   > never itself be a `FAIL`. Do not reason that "out-of-experience buyers are
   > never in the game so this retries forever": `ProcessReceipt` only fires
   > while evaluating a live session for that player (at purchase time if
   > in-experience, or on their next join if not), so the player is expected to
   > be present when the handler runs. Roblox keeps re-invoking `ProcessReceipt`
   > on each subsequent join until it returns `PurchaseGranted`, which is
   > precisely how out-of-experience purchases get fulfilled — a transient nil
   > player handled with `NotProcessedYet` is safe deferral, not silent loss.

### Section B warnings → report, but do NOT change the verdict

Report these as warnings; they do **not** by themselves make the
out-of-experience eligibility verdict `FAIL`:

1. **`no_datastore_persistence`** — fulfillment uses only volatile state
   (leaderstats, `Value` objects, in-memory tables) with no durable `PurchaseId`
   idempotency. Retries on rejoin can double-grant; grants are lost on restart.
   (May be intentional for session-scoped boosts.)
2. **`unconditional_purchase_granted`** — `PurchaseGranted` returned as a
   default/fallthrough, or without confirming the product matched and the grant
   succeeded. (If this means the grant can be skipped while still closing the
   receipt, treat it as Section A silent loss → `FAIL`.)

   > **Do NOT confuse this with a missing idempotency check.** Code that
   > performs the grant, confirms it succeeded (e.g. a successful `pcall`
   > around `SetAsync`), and only *then* returns `PurchaseGranted` is not
   > "unconditional" — it's conditioned on success. The absence of a
   > `GetAsync`-before-write check (so a retried receipt re-runs the grant) is
   > an idempotency gap, not a silent-loss risk — that's `no_datastore_persistence`
   > (Section B), never `unconditional_purchase_granted` or a Section A escalation.
   > The `unconditional_purchase_granted` → `FAIL` escalation is only for a
   > handler that can return `PurchaseGranted` **without any confirmation the
   > grant happened at all** (e.g. no persistence/side-effect check whatsoever
   > on the success path).
3. **`multiple_overwritten_callbacks`** — more than one script assigns
   `MarketplaceService.ProcessReceipt`; the later assignment silently overwrites
   earlier ones, so some products never get fulfilled.

   > This id belongs in `section_b_warnings`, **never** in
   > `section_a_blockers`, even though the impact sounds severe. Only escalate
   > to `FAIL` if you separately find a Section A blocker (e.g. one of the
   > overwritten handlers itself has a `no_player_existence_check` or
   > `prompt_finished_misuse` issue) — the mere fact of multiple assignments is
   > not, by itself, a Section A blocker.

### `PASS`

`MarketplaceService.ProcessReceipt` is assigned in a server script and no
Section A blocker is present. Safe patterns:
- `PurchaseGranted` returned only after a verified, persisted grant.
- `NotProcessedYet` returned whenever the player, data, or callbacks are not
  ready.
- Durable fulfillment keyed by `PlayerId` + `PurchaseId` (idempotent).

### `REVIEW`

The scan is ambiguous and manual review is needed. Examples:
- No `ProcessReceipt` **and** no clear `PromptProductPurchaseFinished` grant
  logic (possibly a donation-only experience where the Robux transfer *is* the
  product).
- A third-party module handles receipts and its internals are not visible.
- Grant logic is too indirect to verify confidently from the available scripts.

> When **no** `ProcessReceipt` callback exists, the engine auto-approves
> receipts — this is intentional. Flag as `REVIEW`, **not** `FAIL`, unless a
> `PromptProductPurchaseFinished` grant path is also present (then it is Section
> A `prompt_finished_misuse` → `FAIL`).

## Verdict decision rules

1. `FAIL` if any Section A blocker fired.
2. Otherwise `REVIEW` if the receipt path is absent/ambiguous per above.
3. Otherwise `PASS`.
4. Section B warnings are always reported but never flip the verdict on their own.

**Common mistake to avoid:** `multiple_overwritten_callbacks` and
`no_datastore_persistence` are **always** `section_b_warnings`, by definition,
never `section_a_blockers` — this is fixed by their category, not by how
severe their impact reads. Before finalizing the JSON artifact, check every id
you put in `section_a_blockers` against the exact three ids listed under
"Section A blockers" above (`prompt_finished_misuse`,
`no_player_existence_check`, `unconditional_purchase_granted`). If an id you're
about to report isn't one of those three, it belongs in `section_b_warnings`
and must not affect the verdict, no matter how the impact reads in your own
summary text.

## Output format

Return a concise report to the user containing:

- **Verdict**: `PASS`, `FAIL`, or `REVIEW`.
- **Section A blockers** found (with the category id from above).
- **Section B warnings** found (with the category id).
- **Scripts inspected**: full instance paths.
- **Evidence**: short snippets or `path:line` references for each finding.
- **Reasoning**: one or two lines per finding, focused on the failure path.
- **Recommended fixes** (see below).

If the caller (or an eval) asks for a machine-readable artifact, also use
`execute_luau` to create a `StringValue` named `ProcessReceiptMisuseReport` under
`ServerStorage` whose `Value` is compact JSON with fields `verdict`,
`section_a_blockers`, `section_b_warnings`, `scripts_inspected`, `summary`, and
`recommended_fixes`. Creating that artifact is the only write this skill
performs, and only on request — never edit the audited scripts unless the user
asks for a fix.

## Tone

Frame findings as opportunities to protect revenue, not as scolding. Lead with
the impact ("Out-of-experience buyers currently lose this purchase") and pair
every blocker with the concrete fix. Keep it factual and specific — cite the
`path:line` so the developer can jump straight to it.

## Verification — how you know it worked

- Every blocker/warning you report cites a real `path:line` you actually read.
- The verdict follows the decision rules above (a Section B warning alone never
  produces `FAIL`; absent/ambiguous receipt logic is `REVIEW`, not `FAIL`).
- If you produced the JSON artifact, it parses and its `verdict` matches the
  prose report.

## Recommended fixes

- Fulfill Developer Products in `MarketplaceService.ProcessReceipt`, not
  `PromptProductPurchaseFinished`.
- Return `Enum.ProductPurchaseDecision.NotProcessedYet` whenever the player,
  data, or callbacks are not ready — never `PurchaseGranted` on a failure path.
- Return `PurchaseGranted` only after the grant **and** its persistence succeed.
- Track each `receiptInfo.PurchaseId` durably (DataStore) for idempotency so
  retries and rejoins do not double-grant.
- Use a single central `ProcessReceipt` dispatcher; do not assign it in multiple
  scripts.

## Worked example

**User:** "Players say they bought the coin pack from the Store tab but never
got coins. Can you check my receipt code?"

1. `script_grep` for `ProcessReceipt` / `PromptProductPurchaseFinished` →
   one hit: `game.ServerScriptService.Purchases:14` assigns a
   `PromptProductPurchaseFinished` handler; no `ProcessReceipt` anywhere.
2. `script_read` `game.ServerScriptService.Purchases` → the coin grant lives
   entirely inside the `PromptProductPurchaseFinished` callback.
3. Verdict: **FAIL** — `prompt_finished_misuse`. That event only fires for
   buyers who are in the experience, so Store-tab buyers are never fulfilled.

**Report:**
> **Verdict: FAIL**
> **Section A blocker:** `prompt_finished_misuse` — `Purchases:14` fulfills the
> coin pack from `PromptProductPurchaseFinished`, which never fires for Store
> tab / EDP buyers, so out-of-experience purchases are silently lost.
> **Scripts inspected:** `game.ServerScriptService.Purchases`
> **Fix:** Move the coin grant into `MarketplaceService.ProcessReceipt`, return
> `PurchaseGranted` only after the grant succeeds, and `NotProcessedYet`
> otherwise. Track `receiptInfo.PurchaseId` in a DataStore so retries don't
> double-grant.

## Relationship to the source detector

This skill mirrors the Virtual Product Content Understanding (VPCU)
`process_receipt_misuse` scan (execution plan `process_receipt_misuse.yaml`,
definition `eligibility_verdict`). Keep the taxonomy and verdict semantics in
sync with that scan so Studio results match the offline scanner.

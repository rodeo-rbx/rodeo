---
name: rbx-search-asset
description: How to use the search_asset tool to find assets in the Creator Store and the user's, group's, or universe's inventory before inserting them. Read before calling search_asset.
---

# Searching for Assets with search_asset

## When to Use

`search_asset` searches for assets across the Creator Store (public marketplace) and Creator Inventory (user, group, and universe). Use it to find assets by keyword before inserting them with `insert_asset`. It returns a list of matching assets with metadata (name, type, source, price).

## Choosing a Scope

`scope` controls where to search:

- `"auto"` (default): waterfalls through universe inventory → the universe's owning group → user inventory → Creator Store. Best for general "find me an X" requests.
- `"creator_store"`: searches only the marketplace. Use it when the user wants marketplace assets or paid assets, or when using price or creator filters.
- `"user"`: searches the user's personal inventory only.
- `"group"`: searches the universe's owning group by default. Pass `groupId` or `groupName` to search a different group instead. If the universe has no owning group and you don't pass a `groupId`, the response lists the user's groups so you can retry.
- `"universe"`: searches the current universe's inventory only.

With `scope = "auto"` you can narrow the waterfall:

- `includeSources`: only search these sources, e.g. `["user", "creator_store"]`. Values: `user`, `group`, `universe`, `creator_store`.
- `excludeSources`: skip these sources. Same values.

### Targeting a specific group

When the user names a particular group ("in my group X" / "in group 12345"), set `scope = "group"` AND pass `groupId` or `groupName` to constrain the search to that one group.

- `groupId`: the numeric group ID.
- `groupName`: matched case-insensitively against the user's groups. If the name doesn't match any of them, the response includes a `groups` field listing valid options.
- `groupId` and `groupName` are mutually exclusive, and only valid with `scope = "group"`. Combining them with any other scope (including `"auto"`) is rejected.

## Filters

### query

The search term. It can be empty when filtering by `assetType` alone (e.g. to list all packages). Supports multi-term search with `+` (e.g. `red+car`) and exact phrases with quotes (e.g. `"red+car"`).

### assetType

Set this when the user asks for a specific asset type. Values: `Model`, `Audio`, `Mesh`, `MeshPart`, `Image`, `Decal`, `Video`, `Package`.

- The inventory API requires exactly one asset type per call and defaults to `Model` when omitted, so finding other types requires an explicit `assetType` (e.g. `Image`, `Audio`).
- Use `Image` for user-uploaded images. Most decals and textures uploaded today are stored as Image, not Decal.
- Use `Package` when the user asks about packages. Packages are Models with a Package subtype, so do not search for the word "package" as a query.

### maxResults

Number of results to return, 1-20. Defaults to 5.

### Price filters (Creator Store only)

If the user asks for paid or premium assets, or names a price range, set `scope = "creator_store"` and use:

- `priceFilter`: `"free"` returns only free assets, `"paid"` only paid assets, `"all"` (default) both.
- `minPriceCents` / `maxPriceCents`: a price range in cents, e.g. `minPriceCents = 100, maxPriceCents = 5000`.

### Other Creator Store filters

These are ignored for inventory scopes:

- `verifiedCreatorsOnly`: only return results from verified creators.
- `facets`: extra keywords that narrow results by related concepts. For a "lion" search: `mane`, `safari`, `realistic`, `animated`. Available facets depend on the query.
- `tags`: category labels like `Vehicle`, `Airplane`, `Fantasy`.

### Audio duration

Only when `assetType = "Audio"`:

- `audioMinDuration`: minimum duration in seconds.
- `audioMaxDuration`: maximum duration in seconds.

## Reading the Response

- **Attribution:** each inventory result includes `creatorId` (string, the group or user ID that owns the asset) and `creatorName` (the group or user name when known). For `source = "inventory"` results that come from a group, `creatorId` is the group ID.
- **Groups:** every `scope = "group"` or `scope = "auto"` response includes a `groups` field listing `{ id, name }` for each of the user's groups (when the user has any). Use it to make a follow-up `scope = "group"` + `groupId` call when the default group doesn't match what the user wants.
- **Context:** every response includes a `context` field describing the current Studio session: `{ userId, universeId, isPublished, creatorType ("User"/"Group" when published), creatorId, creatorName }`. Use it to ground answers about which place is being edited and to disambiguate "in my group" requests.
- **Thumbnails:** each result includes a `thumbnailUrl` (Roblox Thumbnails API). Fetch it to get JSON with `data[0].imageUrl` pointing to a CDN image of the asset. This is useful for visually comparing assets before inserting.

## Inserting Results

- Inventory results (`source = "inventory"`) are always insertable.
- Creator Store results may occasionally be restricted. If an insert fails, try the next result.
- **Cross-owner inserts:** when an inventory result's `creatorId` differs from `context.creatorId` (the place's owner), inserting it brings another owner's asset into this place. The user may be able to view the asset without intending, or having the authority, to share it across owners. Before calling `insert_asset` on a cross-owner result, name the source (the asset's `creatorName`) and the destination (`context.creatorName` or the universe), and ask the user for explicit consent.

## Examples

- "Find me a tree" → `{ "query": "tree" }`
- "Find a free sword from the marketplace by a verified creator" → `{ "query": "sword", "scope": "creator_store", "priceFilter": "free", "verifiedCreatorsOnly": true }`
- "List the packages in my group Acme Studios" → `{ "query": "", "scope": "group", "groupName": "Acme Studios", "assetType": "Package" }`
- "Find a rain sound under 30 seconds" → `{ "query": "rain", "assetType": "Audio", "audioMaxDuration": 30 }`

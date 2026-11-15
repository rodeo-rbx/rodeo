---
name: permissions
description: Work out which MCP permissions a task needs, check which are granted, and ask the user for exactly the missing ones. Covers Open Cloud scopes for open_cloud_invoke, upload directories, the Network capability and HTTP allowlist, and data store and memory store access from Luau. Use before any Open Cloud call, upload, data store, memory store or HTTP access, and whenever one fails with "has not been granted", a 403, a capability error, an "HTTP allowlist" denial, a 502 "Access forbidden", or "API access is not enabled".
---

# MCP Permissions

**MCP only.** These gates belong to the MCP connection and don't apply to the Assistant running inside Studio.

The user sets them per place in **Assistant Settings → MCP Permissions**. You can't read them directly, so: **find what you need → check what's granted → ask for exactly what's missing.**

**How Open Cloud auth works over MCP:** instead of an API key, Studio requests a short-lived OAuth token for each `open_cloud_invoke` call (the "MCP token"), limited to the scopes the user has switched on. Some endpoints accept API keys but not this token, so a scope can be granted and still be refused.

## 1. Find what you need

- **Open Cloud:** each endpoint's docs page lists its scopes on a `**Scopes:**` line (see `rbx-open-cloud-usage`). Find them in the catalog below to name the right toggle.
- **Uploads:** the file must be inside an approved upload directory.
- **Data stores and memory stores:** use `DataStoreService` and `MemoryStoreService` through `execute_luau`, never Open Cloud. The *Save & load data* scopes gate these engine calls. Data stores also need the game setting **Game Settings → Security → Enable Studio Access to API Services**.
- **HTTP from Luau:** needs the `Network` capability, and the domain must be in the HTTP allowlist.
- **Cross-server messages:** `MessagingService` only runs on a game server, so it can't be used from Studio. Tell the user rather than looking for a workaround.

## 2. Check what's granted, before acting

Probe with a harmless GET that carries the scopes. If one isn't granted, the token request fails **before** anything is sent, so this is safe even for write scopes:

```
open_cloud_invoke(method: "GET", endpoint: "/cloud/v2/universes/<game.GameId>", scopes: ["asset:read", "asset:write"])
```

- `200`: all granted.
- `Could not acquire an Open Cloud access token: Scope '<scope>' has not been granted…`: that scope is off. The probe stops at the first missing scope, so probe one at a time for a full list.
- A misspelled or unknown scope gives the same error, so copy names exactly from the catalog.

Upload directories, the HTTP allowlist and the Studio API access setting can't be probed. If the task needs them, ask up front.

## 3. Ask for exactly what's missing

In one message, name the **section** and **exact toggle**, say what it unlocks and why it's needed, and ask for the smallest grant. For example:

> To upload your model I need **Publish & manage assets → `asset:write`** (upload) and **`asset:read`** (check it finished). Could you switch those on in Assistant Settings → MCP Permissions?

After the user confirms, probe again, then continue.

**Sensitive toggles:** never bundle these into a general request. Say plainly what each allows, and get a clear yes:
- **Money:** `developer-product:write`, `game-pass:write`, `ad.campaign:write` (spends Robux), `ad.billing:read`
- **Players:** `universe.user-restriction:write` (bans), `user.user-notification:write`
- **Security:** `universe.secret:write`, `universe.place.luau-execution-session:write`
- **Publishing and sharing:** `universe:write` (includes publishing and server restarts), `universe-places:write`, `asset-permissions:write`, `creator-store-product:write`

## Scope catalog

This maps scopes to toggles. It says nothing about what this user has enabled, or whether an endpoint accepts the MCP token, so **always probe and judge by the result**.

| Section | Scopes | Unlocks |
|---|---|---|
| Look at your game | `universe:read` · `universe.place:read` · `universe.analytics:read` · `universe.place.instance:read` | Experience settings and configs · place settings and version history · analytics · reading instances remotely |
| Edit your game | `universe:write` · `universe.place:write` · `universe.place.instance:write` | Experience settings, configs, experiments, publishing, server restarts · place settings and version notes · editing instances remotely |
| Users, groups & events¹ | `user.advanced:read`, `user.social:read`, `user.inventory-item:read` · `user.user-notification:write` | Profiles and inventory · notifying users |
| | `group:read`, `group-forum:read` · `group:write` | Group roles, join requests, forums · managing them |
| | `universe.event:read` · `universe.event:write` | Game events |
| | `creator-store-save:read` · `creator-store-save:write` | The user's saved Creator Store items |
| Save & load data | `universe-datastores.*`, `universe.ordered-data-store.*` · `memory-store.*` | `DataStoreService` · `MemoryStoreService` |
| Connect to the internet | `Network` (a script capability) · `universe-messaging-service:publish` | HTTP from Luau, together with the domain allowlist · cross-server messages |
| Publish & manage assets | `asset:read`, `asset:write` · `asset-permissions:write` | Uploading and checking assets · sharing them |
| | `creator-store-product:read` · `creator-store-product:write` | Creator Store search and listings · selling |
| | `universe-places:write` · `universe.thumbnail:read`, `:write` | Publishing place versions · homepage thumbnails |
| Money & monetization | `developer-product:*` · `game-pass:*` | Developer products and game passes, with prices |
| | `ad.campaign:read`, `:write`, `ad.billing:read` · `universe.subscription-product.subscription:read` | Ad campaigns and billing · a user's subscription |
| Player consequences | `universe.user-restriction:read`, `:write` | Reading bans · banning and unbanning |
| Security & credentials | `universe.secret:read` · `universe.secret:write` | Secret names (never values) and the public key · creating secrets, which needs that public key |
| | `universe.place.luau-execution-session:read`, `:write` | Running Luau in the cloud and reading its logs |

¹ The section listing `group:*`, `user.*`, `universe.event:*` and `creator-store-save:*`.

**Scopes not in the dialog** can't be granted over MCP: badge scopes, `universe-datastores.control:*`, `universe-datastores.objects:create`, analytics alerts, and any `legacy-*` scope. Point the user to the Creator Dashboard for those.

## Errors

1. **`…Scope '<scope>' has not been granted…`**: off or misspelled. If it's a dialog scope, ask for it (step 3).
2. **HTTP 403 or 401 from the endpoint.** The tool's text always suggests granting a scope; don't take that at face value. Probe the endpoint's scopes first:
   - **Probe fails:** ask for the missing scope.
   - **Probe passes:** the endpoint doesn't accept the MCP token. The user can't fix this in MCP Permissions, so don't ask again. Explain it, then use a Studio tool (`execute_luau`, `inspect_instance`, `search_game_tree`) or point them to the Creator Dashboard.
   - **A business reason**, such as "No ad account is associated with this user": the permission worked. Explain what's missing.
3. **`The current thread cannot call '<function>' (lacking capability <capability>)`**: only `Network` can be granted, under *Connect to the internet*. Other capabilities can't be, so say so.
4. **`HttpService request denied: domain <domain> is not in the Assistant HTTP allowlist`**: ask the user to add the domain under *Connect to the internet → HTTP domain allow list*.
5. **Data stores in Luau.** `DataStoreService` exists even with its scopes off, so check each call's result.
   - **`502 … Access forbidden`**: the matching *Save & load data* scope is off. Reads need `universe-datastores.objects:read`; writes need `universe-datastores.objects:update`.
   - **`403: Cannot write to DataStore from studio if API access is not enabled`**, or **`Studio access to APIs is not allowed`**: ask the user to turn on **Game Settings → Security → Enable Studio Access to API Services**. That's a game setting, not MCP Permissions.
6. **Memory stores in Luau, `InternalError: Internal Error.`**: usually the `memory-store.*` scopes are off. Probe them first.
7. **`MessagingService can only be executed by game server`**: there's no Studio route; tell the user.
8. **File not in an approved upload directory**: ask the user to approve its directory, or to move the file into an approved one themselves. Don't copy or move it yourself.

## Don't circumvent permission controls

Permission settings are the user's choice:
- **Don't start a playtest, or otherwise run the game, to do a blocked operation from game code.** It may not be subject to the same gates.
- Don't try other APIs, endpoints, domains or execution contexts to get the effect of something the user *blocked*.
- If the user declines, accept it.

Using a Studio tool after an endpoint refuses a *granted* scope is fine; the user didn't block it.

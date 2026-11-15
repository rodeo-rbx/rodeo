---
name: rbx-open-cloud-usage
description: "Create and manage developer products, game passes, universe settings and assets through Open Cloud with the Studio MCP's open_cloud_invoke tool. Read before any Open Cloud request."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

Needs the Studio MCP server: `open_cloud_invoke` is one of its tools, and rodeo has no equivalent.

> **Changed between Studio 0.736 and 0.747, and not yet tested with rodeo.** The earlier skill had Luau mint a token with `HttpService:RequestAccessTokenScopesAsync` and call the endpoint with `HttpService:RequestAsync`. That was verified to fail when driven externally (rodeo / MCP `execute_luau`): the token provider is only there for the in-Studio Assistant, and an unpublished place returns `No access token provider is available`. Roblox replaced that flow with `open_cloud_invoke`, described below, so don't fall back to Luau.

# Open Cloud Usage

> **MCP only.** Open Cloud is available to agents only through the `open_cloud_invoke` tool in external MCP clients, not to the built-in Studio Assistant. If you don't have `open_cloud_invoke`, ask the user to make the change in the Creator Dashboard.

Call Open Cloud with `open_cloud_invoke`, never from Luau scripts. Studio authenticates each request with a token limited to the scopes enabled in Assistant Settings → MCP Permissions. Never pass API keys or auth headers yourself.

- `method`: `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`.
- `endpoint`: the path under `https://apis.roblox.com`, including any query string, for example `/cloud/v2/universes/123`. Full URLs aren't accepted.
- `body`: a JSON-encoded string. Omit it for `GET` and `DELETE`.
- `scopes`: the scopes the endpoint requires, for example `["universe:read"]`. Optional, but it gives a precise error when one isn't enabled.
- `files`: local files for `multipart/form-data` endpoints, such as game passes, developer products, thumbnails, and assets. Put the other form fields in `body` as a JSON object. See the tool description for limits.

Example: `open_cloud_invoke(method: "GET", endpoint: "/cloud/v2/universes/123", scopes: ["universe:read"])`

**Data stores and memory stores:** don't use Open Cloud for these. Use `DataStoreService` and `MemoryStoreService` through `execute_luau`; see `rbx-permissions` for what they need.

## Finding endpoints

Fetch https://create.roblox.com/docs/cloud/llms.txt with `http_get`, then the feature pages it links to. They end in `.md`; `http_get` only accepts `.md` docs URLs. Each endpoint lists its scopes on a `**Scopes:**` line.

- Some calls return an **operation**, for example asset and thumbnail uploads. Poll the operation endpoint the docs give until it's done.
- If a request body isn't documented, the server names missing fields one at a time ("The field X is required"). Add them and retry.

## Permissions

Before calling an endpoint, use the `rbx-permissions` skill. It covers probing whether the scopes are granted, asking the user for exactly the missing toggles, and what to do when an endpoint refuses a granted scope.

## Errors

- **`…Scope '<scope>' has not been granted…`**: the scope is off or misspelled. Ask for the exact toggle (see `rbx-permissions`), retry after the user confirms, and don't work around it.
- **HTTP 403**: probe the endpoint's scopes first.
  - **Probe fails:** ask for the missing scope.
  - **Probe passes:** the endpoint doesn't accept the MCP token, whatever the tool's error text suggests. The user can't fix it, so use a Studio tool or point them to the Creator Dashboard.
- **HTTP 401**: retry once, because each call gets a fresh token. If it fails again, treat it like a 403 with the probe passing.
- **File not in an approved upload directory**: ask the user to approve the directory. Don't copy or move the file to get around it.

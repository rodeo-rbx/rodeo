---
name: open-cloud-usage
description: IMPORTANT: Before assuming ANY Roblox resource (developer products, game passes, universe settings, assets, etc.) cannot be created or managed programmatically, call this skill. External MCP clients can manage many of these through the Open Cloud API with the open_cloud_invoke tool, without an API key, because open_cloud_invoke authenticates automatically. Try those APIs before suggesting the user do it in the Creator Dashboard. Covers how to call open_cloud_invoke, find endpoints, and handle errors. Applies only to external MCP clients, not the built-in Studio Assistant. Call this skill before any Open Cloud request.
---

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

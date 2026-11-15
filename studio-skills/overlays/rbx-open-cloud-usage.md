---
description: Create and manage developer products, game passes, universe settings and assets through Open Cloud with the Studio MCP's open_cloud_invoke tool. Read before any Open Cloud request.
---

Needs the Studio MCP server: `open_cloud_invoke` is one of its tools, and rodeo has no equivalent.

> **Changed between Studio 0.736 and 0.747, and not yet tested with rodeo.** The earlier skill had Luau mint a token with `HttpService:RequestAccessTokenScopesAsync` and call the endpoint with `HttpService:RequestAsync`. That was verified to fail when driven externally (rodeo / MCP `execute_luau`): the token provider is only there for the in-Studio Assistant, and an unpublished place returns `No access token provider is available`. Roblox replaced that flow with `open_cloud_invoke`, described below, so don't fall back to Luau.

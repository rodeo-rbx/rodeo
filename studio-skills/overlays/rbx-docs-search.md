---
description: Look up Roblox documentation with http_get: Engine API reference, creator guides and Open Cloud docs. Use when you need accurate API details or how-to guidance.
---

`http_get` supports the `query` / `context_lines` / `return_full` params below and enforces the `.md`/`llms.txt` rule — use it for cheap multi-page keyword checks. No play mode needed. When Studio isn't connected, plain `WebFetch` against the same `create.roblox.com/.../<Name>.md` URLs returns full docs (without the cheap query-miss behavior).

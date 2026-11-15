---
name: rbx-docs-search
description: "Look up Roblox documentation with http_get: Engine API reference, creator guides and Open Cloud docs. Use when you need accurate API details or how-to guidance."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

`http_get` supports the `query` / `context_lines` / `return_full` params below and enforces the `.md`/`llms.txt` rule — use it for cheap multi-page keyword checks. No play mode needed. When Studio isn't connected, plain `WebFetch` against the same `create.roblox.com/.../<Name>.md` URLs returns full docs (without the cheap query-miss behavior).

# Roblox Documentation Lookup

Use the `http_get` tool to fetch official Roblox documentation as clean markdown.

The tool supports an optional `query` parameter that searches the fetched content for a keyword. When a query is provided, only matching sections are returned (or a short "no match" message), saving context window space. Misses are cheap — they cost one line in context. Prefer using `query` unless you already know you need the full document.

## Engine API reference

The API name in the URL is PascalCase and matches the Roblox API name exactly.

| Type | URL pattern |
|------|-------------|
| Classes | `https://create.roblox.com/docs/reference/engine/classes/<ClassName>.md` |
| Datatypes | `https://create.roblox.com/docs/reference/engine/datatypes/<TypeName>.md` |
| Enums | `https://create.roblox.com/docs/reference/engine/enums/<EnumName>.md` |
| Globals | `https://create.roblox.com/docs/reference/engine/globals/<GlobalName>.md` |
| Libraries | `https://create.roblox.com/docs/reference/engine/libraries/<LibName>.md` |
| Engine index | `https://create.roblox.com/docs/reference/engine/llms.txt` |
| Deprecated APIs | `https://create.roblox.com/docs/reference/engine/deprecated.md` |

The engine index lists classes, datatypes, enums, globals, and libraries with one-line summaries. It does NOT list individual methods or properties — search the index for related class/domain terms, then fetch the candidate class doc.

## Creator guides

Use creator guides for workflows, requirements, and how-tos that are not spelled out on Engine API pages (avatar setup, rigging, character specs, Character Controller Library, etc.).

| Type | URL pattern |
|------|-------------|
| Any guide | `https://create.roblox.com/docs/<path>.md` |
| Site index | `https://create.roblox.com/docs/llms.txt` |

Examples:

```
http_get(url: "https://create.roblox.com/docs/avatar-setup/auto-setup-requirements.md")
http_get(url: "https://create.roblox.com/docs/art/characters/specifications.md")
http_get(url: "https://create.roblox.com/docs/characters/character-controller-library/controllers.md")
```

When an Engine API page links to a guide (e.g. auto-setup requirements), fetch that `.md` URL directly — do not stop at "see requirements" without loading the guide.

Use `query` on large guides the same way as engine API pages.

## Workflow

1. If you KNOW the exact API and need the full reference:
   ```
   http_get(url: "https://create.roblox.com/docs/reference/engine/classes/Part.md")
   ```

2. If you need requirements or how-to guidance (avatar setup, rigging, movement controllers):
   ```
   http_get(url: "https://create.roblox.com/docs/avatar-setup/auto-setup-requirements.md")
   http_get(url: "https://create.roblox.com/docs/art/characters/specifications.md", query: "higher-fidelity")
   ```

3. If you need to SEARCH within a page, use the query parameter:
   ```
   http_get(url: "https://create.roblox.com/docs/reference/engine/classes/Part.md", query: "Anchored")
   http_get(url: "https://create.roblox.com/docs/reference/engine/classes/BasePart.md", query: "Anchored")
   ```
   Misses cost almost nothing. If you find a match, fetch the full page with only `url`.

4. For more context around a match without returning the full document:
   ```
   http_get(url: "...", query: "Anchored", context_lines: 10)
   ```

5. For the full document when the query matches, without a second call:
   ```
   http_get(url: "...", query: "Anchored", return_full: true)
   ```

6. If you don't know which API to look up, use the engine index:
   ```
   http_get(url: "https://create.roblox.com/docs/reference/engine/llms.txt")
   ```

7. To discover creator guides by topic, use the site index or search with `query`:
   ```
   http_get(url: "https://create.roblox.com/docs/llms.txt", query: "avatar-setup")
   ```

8. If the user references a deprecated API:
   ```
   http_get(url: "https://create.roblox.com/docs/reference/engine/deprecated.md")
   ```

## Rules

- URLs must end with `.md` or be `llms.txt`.
- Do NOT guess API names you are unsure about; use `llms.txt` to confirm first.
- Prefer `query` when checking multiple pages to save context.
- Engine API pages often link to creator guides — follow those links with `http_get`.

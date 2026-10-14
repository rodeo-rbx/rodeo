# lute adapter

`--adapter lute` makes these aliases resolve to the folders here:

| Alias | Folder | Source |
|-------|--------|--------|
| `@lute/*` | `lute/` | Lute's built-in modules, from the Lute process behind the run |
| `@std/*` | `std/` | Lute's standard library, verbatim |
| `@batteries/*` | `batteries/` | Lute's batteries, verbatim |

`std/` and `batteries/` are copied unmodified from
[luau-lang/lute](https://github.com/luau-lang/lute) at `1219ec5`, the commit the
`submodules/lute` submodule pins: `lute/std/libs/` and `batteries/`, without their
`.luaurc` files (their `@batteries` path is Lute's repo layout; the adapter
provides the alias). License: MIT, see `LICENSE`.

They run in Studio, on the `@lute/*` modules in `lute/`. Those are Lute's own:
a run that requires one starts the pinned Lute (`rodeo-client/src/runtime/runtimes.rs`)
and calls it through `@rodeo-bridge` (`../_bridge/`). `io` and `task` stay in
Studio, on the run's stdio and Studio's scheduler, and so does `time`, whose
Durations cross as values; `process` keeps the run's args and exit.

Update flow: move `submodules/lute`, copy both folders again verbatim, update
the commit above and the Lute pin in `runtimes.rs` to the release built from
it, and run `tests/adapters/lute.test.ts`. A module that no longer works gets a
manifest entry there, not an edit here.

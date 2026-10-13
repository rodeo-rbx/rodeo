# lute adapter

`--adapter lute` makes these aliases resolve to the folders here:

| Alias | Folder | Source |
|-------|--------|--------|
| `@lute/*` | `lute/` | rodeo's shims for Lute's built-in modules, over `@rodeo` |
| `@std/*` | `std/` | Lute's standard library, verbatim |
| `@batteries/*` | `batteries/` | Lute's batteries, verbatim |

`std/` and `batteries/` are copied unmodified from
[luau-lang/lute](https://github.com/luau-lang/lute) at `1219ec5`, the commit the
`submodules/lute` submodule pins: `lute/std/libs/` and `batteries/`, without their
`.luaurc` files (their `@batteries` path is Lute's repo layout; the adapter
provides the alias). License: MIT, see `LICENSE`.

They run on the `@lute/*` shims in `lute/`. Built-ins Studio can't provide
(`crypto`, `debugger`, `luau`, `net`, `syntax`, `vm`) are shims that error when
required, so the `@std` modules built on them (`@std/luau`, `@std/net`,
`@std/syntax`) do too.

Update flow: move `submodules/lute`, copy both folders again verbatim, update
the commit above, and run `tests/adapters/lute.test.ts`. A module that no longer
works gets a manifest entry there, not an edit here.

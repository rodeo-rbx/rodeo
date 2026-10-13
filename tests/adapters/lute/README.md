# lute adapter conformance tests

Verbatim copies of [lute](https://github.com/luau-lang/lute)'s own tests for its
standard library and built-in modules, executed through
`rodeo run --adapter lute` so the adapter (rodeo's `@lute/*` shims, plus lute's
`@std` and `@batteries` running on them) is held to lute's ground truth.

- Source: `luau-lang/lute` `tests/std/`, `tests/lute/` and `tests/testutil.luau`,
  copied into `tests/` here at `1219ec5`, the commit the `submodules/lute`
  submodule pins (and the adapter's `std`/`batteries` copies come from). The
  layout matches lute's repo, since some tests read data at paths relative to
  its root (`tests/std/JSONParsingTestSuite`).
- License: MIT (see `LICENSE`, copied from the lute repository).
- Update flow: move `submodules/lute`, copy the folders again verbatim
  (together with `rodeo-adapters/lute/std` and `batteries`), re-run the harness,
  reconcile the manifest.

Files are intentionally unmodified. The harness (`../lute.test.ts`) copies
`tests/` to a scratch directory per test file and appends the runner call that
`lute test` would make, then classifies the result against its manifest: whole
files that can't run (`out-of-scope`, `gap`), and known-failing cases of the
files that do (`CASE_GAPS`). A test that can't pass under rodeo gets a manifest
entry with a reason, never an edit.

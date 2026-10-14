# lune adapter conformance tests

Verbatim copies of [lune](https://github.com/lune-org/lune)'s complete test
suite, executed through `rodeo run --adapter lune` so the adapter is held to
lune's ground truth. Every module but `@lune/roblox` is bridged to the lune
process behind the run; the roblox tests are vendored anyway and recorded as
out-of-scope.

- Source: `lune-org/lune` `tests/` and its root `.luaurc` (the `@tests`
  aliases), copied into `tests/` and `.luaurc` here at `7f1849c`, the commit
  the `submodules/lune` submodule pins, so the provenance is checkable from a
  fresh clone. The layout matches lune's repo root, which lune runs every test
  from.
- License: MPL-2.0 (see `LICENSE.txt`, copied from the lune repository).
- Update flow: pull `submodules/lune`, re-copy `tests/` and `.luaurc`
  verbatim, re-run the harness, reconcile the manifest.

Files are intentionally unmodified — a test that can't pass under rodeo is
recorded in the harness manifest (`gap` with a reason, or `out-of-scope`),
never edited to pass.

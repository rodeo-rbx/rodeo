# Design notes

## Concurrent runs and log capture

Multiple runs can be active in the same Studio at the same time, and this is
expected to work. Each run captures the DOM's `LogService` output for its
lifetime, including output that did not come from that run. That is
intentional: the capture is what the Output window showed while the run was
live, not a stream attributed to the run.

A run that needs logs specific to itself should write them through the
`@rodeo/io` and `@rodeo/stream` APIs. Those are per-run by construction.

## Parallel serves

Several serves, of the same or different rodeo builds, can run on one machine
at once. The port is the separation convention: every command resolves it as
`--port`, then `RODEO_PORT`, then 44872, so a project pins its port next to its
rodeo version. Each studio backend installs its own plugin file, named by
build and port, and owns that file's lifetime.

Studios a serve launched belong to that serve only. Studios opened by hand
belong to nobody and connect to every running serve. `rodeo state` is per
serve: its own Studios plus the hand-opened ones.

Studio watches the one shared plugins folder and hot-loads a new plugin file
into every open Studio, which is how hand-opened Studios join a serve. That
does not disturb the plugin another serve already has loaded: a run in one
serve's Studio survives another serve starting and stopping, another project's
one-shot `rodeo run --place`, and unrelated files appearing in the folder, with
no reconnect (tests/cli/operations/pluginFolderChurn.test.ts). What the
per-backend file removed is the permanent corruption a single shared plugin
file caused when a different build overwrote it (issue #12).

## Captures

Every capture, in every DOM, is read from the file the engine writes for it in
Studio's per-user capture directory, never from an EditableImage. Promoting the
capture's temporary texture into one is refused in a session's DOMs and capped
at 8192 pixels a side everywhere, which on a 2x display rules out anything past
a 4096 viewport, and it means reading the whole RGBA frame through Luau; the
file has neither problem and the simulator's full 7680 by 4320 works on every
display. That directory is shared by all Studio processes, so rodeo snapshots
it before the capture and accepts exactly one complete PNG that appears
afterwards at the viewport's size. Two candidates at once is an error to
retry, never a guess, and a frame that is entirely black is refused rather
than written. The image is resampled to the viewport by default so UI offsets
map 1:1 onto pixels; `resample = false` keeps the frame at its rendered size
instead.

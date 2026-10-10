---
title: Installation
---

## Install the rodeo CLI

```bash
mise use ubi:revvy02/rodeo
# or
rokit add revvy02/rodeo
```

Verify:

```bash
rodeo --version
```

## Studio plugin

There is no separate install step. The plugin is embedded in the CLI. Each serve writes its own copy to Studio's local plugins folder when it starts, named `rodeo-<build>-<port>.rbxm` after the build and the port it listens on:

```bash
rodeo serve
# or, which starts a serve if none is running on the port
rodeo run --place
```

One plugin file per serve means two serves never overwrite each other's plugin, so different rodeo versions can run side by side on one machine, each on its own port (see [Port](/rodeo/getting-started/cli-usage/#port)).

Launched Studios connect only to the serve that launched them. A Studio you open manually connects to every running serve and appears in each one's `rodeo state`.

Versions before 1.5 write a single shared `rodeo.rbxm`. This build never touches that file, so an older rodeo keeps working alongside; delete `rodeo.rbxm` by hand once no older version remains.

## Generate type definitions

```bash
rodeo setup
```

Writes `@rodeo` typedefs to `~/.rodeo/typedefs/<version>/` and registers them in `.rodeo/.luaurc` so your editor can type-check `require("@rodeo/fs")` etc.

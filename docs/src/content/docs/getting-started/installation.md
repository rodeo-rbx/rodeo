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

If the project has a `.claude/skills` or `.agents/skills` folder, `setup` also installs the rodeo agent skill there as `rodeo/SKILL.md`. A copy for an older rodeo is replaced; one for the same or a newer rodeo is left as it is.

### Studio skills

In those same folders, `setup` compares the Studio skills with the latest on GitHub. These are Roblox's Studio Assistant skills (debugging, scene analysis, virtual input and more), each a folder named `rbx-<name>`, with notes on using them through rodeo; they live in [`studio-skills/curated`](https://github.com/rodeo-rbx/rodeo/tree/main/studio-skills/curated). `setup` says which ones the project lacks and which differ from the latest, and asks before it installs and replaces them:

```
Studio skills in .claude/skills: 1 not installed (rbx-docs-search), 1 differs from the latest (rbx-debug)
Install and replace them with the latest? [y/N]
```

- A skill differs when the content of one of its files does. The skills have no version, so a copy you edited differs too, and a yes replaces your edits.
- Only the files that are missing or differ are downloaded and written. Other files in a skill's folder, and skills that aren't rodeo's, are left alone.
- Outside a terminal (a script, an agent) `setup` only says what it found. `rodeo setup --studio-skills update` installs and replaces without asking, and `--studio-skills skip` leaves the skills alone and makes no request.
- The check makes two requests to GitHub's API, which allows 60 an hour for each address without a login. When GitHub can't be reached, `setup` warns and finishes its other work.

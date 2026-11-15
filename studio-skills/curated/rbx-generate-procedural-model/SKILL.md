---
name: rbx-generate-procedural-model
description: "How to call the Studio MCP's generate_procedural_model tool to build 3D objects from primitive parts. Read before calling it."
---

## Outside the Studio Assistant (rodeo / Studio MCP)

This is one of Roblox's built-in Studio Assistant skills, repackaged by rodeo for agents that drive Studio through rodeo or the Studio MCP server. Use this copy over the same-named skill from the Studio MCP's `skill` tool: that one is the original, without the notes in this section.

The tool names in the text are the Assistant's:

- `execute_luau` is the Studio MCP tool of that name, or `rodeo run`. `rodeo run --context mcp` runs code at the same identity as `execute_luau`; plain `rodeo run` uses plugin identity, which not every API here accepts. The `rodeo` skill covers modes and contexts.
- The other tools (`screen_capture`, `get_console_output`, `http_get`, `start_stop_play` and so on) are the Studio MCP tools of the same name: `mcp__Roblox_Studio__<tool>` when the server is registered as `Roblox_Studio`.
- `script_search`, `script_read`, `script_grep` and `multi_edit` work on Studio's copy of the scripts. In a project synced from files, read and edit the files instead.
- Every Studio MCP call takes a `studio_id` from `list_roblox_studios`. `set_active_studio` no longer exists; where the text says to call it, pass `studio_id` instead.

Every skill in this set has a name starting with `rbx-`. The text sometimes leaves the prefix off: its `virtual-input` skill is `rbx-virtual-input` here.

Needs the Studio MCP server: `generate_procedural_model` is one of its tools, and rodeo has no equivalent.

# Generating Procedural Models with generate_procedural_model

## When to Use

`generate_procedural_model` creates 3D objects built from primitive parts (blocks, spheres, cylinders, wedges) as a ProceduralModel with configurable attributes. Use it when the user wants to:

- Create or build a 3D object, model, character, creature, vehicle, building, scenery, or any physical thing in the workspace
- Generate something from a reference image
- Create an object with tunable parameters (e.g. "add attributes to control head size, arm length, color")
- Build anything described as "procedural", "parametric", "configurable", or "with attributes"

## What It Produces

The output is a ProceduralModel: a scripted model whose appearance is controlled by user-editable attributes (like size, color, proportions). The user can tweak these attributes after generation without regenerating.

The tool inserts the generated model into the workspace automatically. You do not need to run any code afterward.

## Run It in the Background

This is a long-running generative tool. Default to calling it with `async` set to `true`. The generation then runs in the background and the call returns immediately with a jobId. Handle other tasks while it runs, and only call `wait_job_finished` when you actually need the result.

## Arguments

### prompt (required)

A text description of what to create and what attributes to expose.

- Pass the user's own words.
- If the user mentions specific configurable properties (e.g. "head size", "arm length", "wheel count"), include them in the prompt so the generated model exposes them as editable attributes.
- If an image is attached and the user gave no text description, pass an empty string.
- Do NOT describe or interpret the attached image. Only pass the user's own words.

### attachedImageUri (optional)

The image URI (`IMAGEID_<id>`) from the user's attached image. Always pass this when the user provides a reference image.

### segmentation (optional)

Controls how the model is broken into parts. Pick based on the user's wording:

- Omit it (or pass `"auto"`) when the user did NOT mention parts or segmentation (e.g. "generate a car"). The tool derives the parts automatically.
- `"none"` when the user explicitly asked for no parts or a single piece (e.g. "generate a car with no parts", "as one mesh", "single piece").
- `"explicit"` when the user named specific parts (e.g. "a car with body and wheels"). You MUST also pass `partNames` with the user's listed parts.

### partNames (optional)

The part names that define the structure of the generated model. Accepts a comma-separated string (e.g. `body, left wheel, right wheel, door`) or a JSON array of strings (e.g. `["body", "left wheel", "right wheel", "door"]`).

- Required when `segmentation` is `"explicit"`.
- At most 8 parts. Extra parts are dropped.

## Examples

- "Make a dragon" → `{ "prompt": "Make a dragon", "async": true }`
- "Build a car with body and four wheels, and let me change the wheel size" → `{ "prompt": "Build a car with body and four wheels, and let me change the wheel size", "segmentation": "explicit", "partNames": "body, front left wheel, front right wheel, back left wheel, back right wheel", "async": true }`
- User attaches an image with no text → `{ "prompt": "", "attachedImageUri": "IMAGEID_<id>", "async": true }`

# Roblox Studio MCP — Tool Reference

Generated from the StudioMCP proxy's `tools-cache.json` (`~/Library/Roblox/StudioMCP/`) — the
live tool set the connected Studio advertised (cached 2026-11-10, Studio 0.747). This is what
`mcp__Roblox_Studio__*` exposes.

**38 tools** (below). `list_roblox_studios` returns the connected Studio instances; every
other tool takes a required `studio_id` from it, so each call names its target. The old
`set_active_studio` is gone.

---

## `auto_setup_avatar`

Rig a static Workspace Model as an R15 avatar and place the result beside it.
Pass `instance_path` whenever the Model is known; omit it only to use the
user's single selected Model. Do not set Selection with execute_luau.
Usually takes 3-5 minutes.

| param | type | req | description |
|---|---|---|---|
| `align_front_angle` | boolean |  | Fix the front to negative Z. Defaults to false, which lets the engine infer it. Use true for ambiguous models that already face negative Z. |
| `classifications` | array<object> |  | Labels for Models and BaseParts under the input. A Model covers every BasePart inside it. Omit to let the engine decide. Unlisted parts become Body; an R15 body with a Humanoid is skipped. |
| `decal_to_dynamic_head` | boolean |  | Create a dynamic head from a 2D face Decal or texture. Defaults to false. |
| `enable_additional_head_processing` | boolean |  | Align the head cage to facial features for a better makeup fit. Defaults to true. |
| `instance_path` | string |  | Workspace path to the input Model, preferably from inspect_instance. Pass it whenever known; omit only for the user's single selected Model. |
| `r15_plus` | boolean |  | Add optional upper-torso, neck, hand, and foot joints for more articulation. Defaults to false. |
| `reduce_triangles` | boolean |  | Development only: reduce triangles for validation. Defaults to false. Omit for Platform, which always reduces them. |
| `setup_type` | string |  | "Platform" (default) creates marketplace-compatible avatars. "Development" creates experience-only characters such as NPCs. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `character_navigation`

Navigate the character to the given position, or instance.

| param | type | req | description |
|---|---|---|---|
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `instance_path` | string |  | The instance path that the character should be moved to. for example: "game.Workspace.Part","game.Workspace.Model". The path should start with game, LocalPlayer, or Workspace. |
| `speed_multiplier` | number |  | speed multiplier, default is 1.0, 0.5 is half speed, 2.0 is double speed. minimum is 0.1, maximum is 10.0. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `x` | number |  | The x coordinate to move the character to. Required if instance_path is not provided. |
| `y` | number |  | The y coordinate to move the character to. Required if instance_path is not provided. |
| `z` | number |  | The z coordinate to move the character to. Required if instance_path is not provided. |

## `create_skill`

Create a new custom skill. The skill will be published and immediately available to the assistant.

| param | type | req | description |
|---|---|---|---|
| `skill_body` | string | ✔ | The markdown body content — instructions the agent follows when the skill is invoked. |
| `skill_description` | string | ✔ | One-line description of what the skill does and when to use it. Written in third person. |
| `skill_name` | string | ✔ | Unique name for the skill. Lowercase letters, numbers, underscores, dashes only. Cannot start with 'rbx-'. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `edit_skill`  _(destructive)_

Edit an existing user skill. Can update the name, description, and/or body. Only user-created skills can be edited.

| param | type | req | description |
|---|---|---|---|
| `new_description` | string |  | New one-line description for the skill. Omit to keep current description. |
| `new_name` | string |  | New name for the skill. Lowercase letters, numbers, underscores, dashes only. Cannot start with 'rbx-'. Omit to keep current name. |
| `new_skill_body` | string |  | New markdown body content for the skill. This replaces the entire body. Omit to keep current body. |
| `skill_name` | string | ✔ | The current name of the skill to edit. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `execute_luau`  _(destructive, open-world)_

Executes Luau code in Roblox Studio. Returns the result of the executed code or an error message if the code fails to execute.

Code runs on Studio's main thread; until it yields, Studio is frozen, the script can't be stopped, and the MCP connection may drop. For heavy work (bulk instance creation or edits, procedural or terrain generation, large loops), split it into logical chunks (e.g. per region, or batches of ~1000 items) and call `task.wait()` once per chunk, not per item. When creating many instances, build each chunk in a container outside the DataModel and parent it before that chunk's `task.wait()`. The cost of a .Parent write into the DataModel scales with the total size of the subtree being inserted (all descendants), so parenting one large container is itself a single unyielding operation that can freeze Studio or drop the connection, no matter how incrementally the container was built. Keeping each flushed subtree small (one chunk's worth) bounds the cost of every .Parent write. A loop that waits for state to change (e.g. `while not ready do`) must call `task.wait()` inside it.

| param | type | req | description |
|---|---|---|---|
| `code` | string | ✔ | The Luau code to execute. |
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `generate_animation`

Generates new R15+ animations from a prompt, or upscales an existing R6 animation to R15.
Use this tool whenever the user wants to create an animation, motion, or movement for a character.
Pass a 'textPrompt' to create a new animation, or an 'inputAssetId' to upscale an existing R6 animation.

| param | type | req | description |
|---|---|---|---|
| `duration` | number |  | Length of the animation in seconds (1 to 30). Only pass this if the user asks for a specific duration; otherwise the best duration is chosen automatically. |
| `inputAssetId` | string |  | Asset id of an existing R6 animation clip to upscale. |
| `instance_path` | string |  | Full Studio path to the rig Model to animate, e.g. "Workspace.Rig1", preferably from inspect_instance. Only pass this if the user specifies a rig. Otherwise omit it, and the tool will use the rig selected in Studio, or insert a default rig if none is selected. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `textPrompt` | string |  | The animation description exactly as provided by the user. Do not paraphrase, summarize, or add detail. |

## `generate_avatar`  _(open-world)_

Generate a new rigged, animatable R15 avatar (body, head, and clothing) from a text prompt using Roblox's avatar creation pipeline, then upload it as a persistent asset and insert it into Workspace in front of the camera. Use this when the user asks to create, generate, or design a new avatar or character from a description. To rig a Model that already exists in the place, use auto_setup_avatar instead. Long-running (usually several minutes): prefer setting 'async' to true and only call wait_job_finished when you need the result.

| param | type | req | description |
|---|---|---|---|
| `dynamicHead` | boolean |  | true generates a dynamic (facially animatable) head, false a static head. Omit to use the default. |
| `pipelineName` | string |  | npc generates the avatar with clothing baked in. modesty generates the body only, with a modesty layer. Omit to use the default. |
| `r15Plus` | boolean |  | true generates an R15+ avatar with a HumanoidRigDescription and DigitsRigDescriptions, false a standard R15 avatar. Omit to use the default. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `textPrompt` | string | ✔ | Description of the avatar to generate, e.g. 'a knight in silver armor with a red cape'. |

## `generate_material`  _(open-world)_

Returns the BaseMaterial and Name of the generated MaterialVariant. To use, these must both be set to the Material and MaterialVariant properties of BaseParts, respectively.

| param | type | req | description |
|---|---|---|---|
| `async` | boolean |  | Run this tool asynchronously and return immediately with a jobId, without blocking on completion. Use this when you want to kick off the work (e.g. several generations at once, or when the user does not need to wait for the result) and continue with other tasks. The tool runs in the background and updates its own UI. Poll the returned jobId with the wait_job_finished tool only when you actually need the final result. If you DO need the result before proceeding, omit this argument (or set it false) and call the tool normally. |
| `baseMaterial` | string | ✔ |  |
| `materialDescription` | string | ✔ |  |
| `materialId` | string | ✔ |  |
| `materialPattern` | string | ✔ |  |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `generate_mesh`  _(open-world)_

Generates a textured mesh from a prompt using AI.

| param | type | req | description |
|---|---|---|---|
| `async` | boolean |  | Run this tool asynchronously and return immediately with a jobId, without blocking on completion. Use this when you want to kick off the work (e.g. several generations at once, or when the user does not need to wait for the result) and continue with other tasks. The tool runs in the background and updates its own UI. Poll the returned jobId with the wait_job_finished tool only when you actually need the final result. If you DO need the result before proceeding, omit this argument (or set it false) and call the tool normally. |
| `maxTriangles` | number |  | The maximum number of triangles for the generated mesh. If provided, this must be between 12 and 20000 (inclusive). |
| `partNames` | string |  | List of part names defining the schema for the generated mesh. Accepts a comma-separated string (e.g. 'body, left wheel, right wheel') or a JSON array of strings (e.g. ['body', 'left wheel', 'right wheel']). Required when segmentation='explicit'. Maximum 8 parts (excess will be truncated). |
| `segmentation` | string |  | Controls how the mesh is broken into parts. Pick based on the user's wording: - Omit (or "auto"): user did NOT mention parts/segmentation (e.g. "generate a car"). The tool will derive parts automatically via an internal LLM call. - "none": user explicitly asked for no parts / a single piece (e.g. "generate a car with no parts", "as one mesh", "single piece"). - "explicit": user named specific parts (e.g. "a car with body and wheels"). You MUST also pass partNames with the user's listed parts (max 8). |
| `size` | object |  | The generation's bounding box size. The generation will try to fit within this volume. Try to approximate a good size based on textPrompt. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `textPrompt` | string | ✔ | The text prompt describing the mesh to generate. |

## `generate_procedural_model`  _(open-world)_

Creates 3D objects built from primitive parts (blocks, spheres, cylinders, wedges) as a ProceduralModel with configurable attributes.
Use this tool when the user wants to:
- Create or build a 3D object, model, character, creature, vehicle, building, scenery, or any physical thing in the workspace
- Generate something from a reference image
- Create an object with tunable parameters (e.g. "add attributes to control head size, arm length, color")
- Build anything described as "procedural", "parametric", "configurable", or "with attributes"

The output is a ProceduralModel: a scripted model whose appearance is controlled by user-editable attributes (like size, color, proportions). The user can tweak these attributes after generation without regenerating.

The tool automatically inserts the generated model into the workspace. You do not need to run any code afterward.

CRITICAL: This is a long-running generative tool. Default to calling it with the 'async' argument set to true, which runs the generation in the background and returns immediately with a jobId. Handle other tasks while the generation is in-progress, and only call the 'wait_job_finished' tool when you absolutely need the result.

| param | type | req | description |
|---|---|---|---|
| `async` | boolean |  | Run this tool asynchronously and return immediately with a jobId, without blocking on completion. Use this when you want to kick off the work (e.g. several generations at once, or when the user does not need to wait for the result) and continue with other tasks. The tool runs in the background and updates its own UI. Poll the returned jobId with the wait_job_finished tool only when you actually need the final result. If you DO need the result before proceeding, omit this argument (or set it false) and call the tool normally. |
| `attachedImageUri` | string |  | The image URI (IMAGEID_<id>) from the user's attached image. Always pass this when the user provides a reference image. |
| `partNames` | string |  | List of part names that define the structure of the generated model. Accepts a comma-separated string (e.g. 'body, left wheel, right wheel, door') or a JSON array of strings (e.g. ['body', 'left wheel', 'right wheel', 'door']). Required when segmentation='explicit'. Maximum 8 parts (excess will be truncated). |
| `prompt` | string | ✔ | A text description of what to create and what attributes to expose. Pass the user's own words. If they mention specific configurable properties (e.g. "head size", "arm length", "wheel count"), include those in the prompt so the generated model exposes them as editable attributes. If an image is attached and the user gave no text description, pass an empty string. Do NOT describe or interpret the attached image — only pass the user's own words. |
| `segmentation` | string |  | Controls how the model is broken into parts. Pick based on the user's wording: - Omit (or "auto"): user did NOT mention parts/segmentation (e.g. "generate a car"). The tool will derive parts automatically via an internal LLM call. - "none": user explicitly asked for no parts / a single piece (e.g. "generate a car with no parts", "as one mesh", "single piece"). - "explicit": user named specific parts (e.g. "a car with body and wheels"). You MUST also pass partNames with the user's listed parts (max 8). |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `generate_scene`

Generates a scene of models and parts from a text prompt and/or reference images, placed within an optional bounding box, and runs the generated script in the DataModel.

| param | type | req | description |
|---|---|---|---|
| `boundingBoxPosition` | object |  | World-space position of the area to generate the scene in. |
| `boundingBoxSize` | object |  | Size (in studs) of the area to generate the scene in. |
| `images` | array<object> |  | Reference images that guide the scene. Each is an image content object ({ type = "image", data = <base64>, mimeType = <e.g. "image/png"> }) or the URI of an attached image. Either this or textPrompt must be provided. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `textPrompt` | string |  | Text description of the desired scene. Either this or images must be provided. |
| `useMeshGen` | boolean |  | Whether to generate meshes for scene objects. Defaults to true. |

## `generate_texture`  _(destructive, open-world)_

Re-textures an existing mesh using AI, given a text prompt describing the desired look. Requires a source MeshPart or Model. Replaces the source in place.

| param | type | req | description |
|---|---|---|---|
| `hintImage` | object |  | Optional reference image guiding the texture style. When provided, textPrompt may be an empty string. |
| `instance_path` | string |  | Full Studio path to the source MeshPart or Model, e.g. "Workspace.Model1.Part2". Resolved directly — no live Studio selection required. If omitted, falls back to the current Studio selection (which must be exactly one MeshPart or Model). |
| `model` | string |  | Generation mode. "Fast" (default) trades quality for speed. "Quality" is a research preview and can vary in results; it first generates a reference image from an automatically selected view, so it takes longer. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `textPrompt` | string | ✔ | Text description of the desired texture/appearance, e.g. "rusty metal". |

## `get_console_output`  _(read-only)_

Get the console output from the Studio output log.

| param | type | req | description |
|---|---|---|---|
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `get_studio_state`  _(read-only)_

Get the state of the studio, including current play state and available datamodel types. If the studio state is not expected, please call start_stop_play tool to start or stop the play.

| param | type | req | description |
|---|---|---|---|
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `http_get`  _(read-only, open-world)_

Fetches the content of a URL via HTTP GET request. Returns the response body as text.

Optionally searches the fetched content for a keyword (query parameter). When a query is provided:
- Returns only the matching sections with surrounding context lines, saving context window space.
- Returns a short "no match" message if the keyword isn't found.
- Use context_lines to control how many lines of context around each match (default: 3).
- Use return_full: true to get the entire document when the keyword matches.

Without a query, the full response body is returned.

Only the following URL patterns are allowed:
- https://create.roblox.com/docs/reference/engine (Roblox Engine API docs — URLs must end with .md or be llms.txt)
- https://create.roblox.com/docs/cloud (Roblox Cloud API docs — URLs must end with .md or be llms.txt)
- https://create.roblox.com/docs/performance-optimization (Roblox Performance Optimization docs — URLs must end with .md or be llms.txt)
- https://github.com/Roblox/libmp (Roblox LibMP repository — URLs must end with .md or be llms.txt)
- https://create.roblox.com/docs (Roblox Creator docs (guides, tutorials, etc.) — URLs must end with .md or be llms.txt)

Any URL that does not match one of the above rules will be rejected.
Only GET requests are supported. The full URL must be provided.

Examples:
- http_get(url: "https://create.roblox.com/docs/reference/engine/classes/Part.md")
- http_get(url: "https://create.roblox.com/docs/reference/engine/classes/Part.md", query: "Anchored")
- http_get(url: "https://create.roblox.com/docs/reference/engine/classes/BasePart.md", query: "Position", context_lines: 10)
- http_get(url: "https://create.roblox.com/docs/reference/engine/classes/Part.md", query: "Anchored", return_full: true)

| param | type | req | description |
|---|---|---|---|
| `context_lines` | number |  | Lines of surrounding context per match. Only meaningful when query is provided. Default: 3. |
| `query` | string |  | Optional keyword to search for in the fetched content (case-insensitive literal match). When provided, only matching sections are returned. |
| `return_full` | boolean |  | If true and query matches, return the entire document instead of just matched sections. Only meaningful when query is provided. Default: false. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `url` | string | ✔ | The full URL to fetch. Must match one of the allowed URL patterns. |

## `insert_asset`  _(open-world)_

Inserts an asset into the game by its numeric Roblox asset ID.
Use this tool when you have a specific asset ID to insert directly, rather than searching the Creator Store.
The asset will be loaded, validated, and placed in the scene.
Always provide assetName when you know the name of the asset (e.g. from search results or user request). The inserted instance will be named using assetName — if omitted, it defaults to a generic name.
Supports models, meshes, images/decals, audio, video, animations, and packages.

| param | type | req | description |
|---|---|---|---|
| `assetId` | string | ✔ | Numeric Roblox asset ID to insert. |
| `assetName` | string |  | Name for the inserted instance in the game tree. Always provide this when you know the asset name (e.g. from search_asset results). If omitted, defaults to a generic name. |
| `assetType` | string |  | Asset type hint. If provided, skips the metadata API lookup and uses this type directly. Use when the caller already knows the asset type (e.g. from inventory search results). 'Image' and 'Decal' both insert as a Decal instance. |
| `parentPath` | string |  | DataModel path to parent the asset under (e.g. game.Workspace.Folder1). Defaults to workspace. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `inspect_instance`  _(read-only)_

Inspect a specific Roblox instance to get all its properties, attributes, and children summary.
Returns detailed information about instances including all readable properties, custom attributes, 
and a comprehensive summary of immediate children and recursive child counts.
If multiple instances match the same path, all matches will be returned.
Use this tool after using GameTree to explore specific instances in detail.

SINGLE MATCH OUTPUT FORMAT:
{
  "name": "MyPart",
  "className": "Part",
  "path": "Workspace.MyPart",
  "uniqueId": "1234567890ABCDEF",
  "properties": {
    "Anchored": true,
    "CanCollide": true,
    "CFrame.Position": "0, 10, 0",
    "CFrame.Rotation": "1, 0, 0, 0, 1, 0, 0, 0, 1",
    // ... all other readable properties
  },
  "attributes": {
    "CustomTag": "PlayerSpawn",
    "SpawnDelay": 5,
    "IsActive": true
  },
  "children": {
    "immediateChildren": [
      {
        "className": "Part",
        "path": "Workspace.MyPart.ChildPart"
      },
      {
        "className": "Model",
        "path": "Workspace.MyPart.ChildModel"
      }
    ],
    "childrenCount": 2,
    "totalDescendants": 8
  }
}

CHILDREN SUMMARY FIELDS:
- immediateChildren: Array of direct children with className and path only (no name, childCount, or uniqueId)
- childrenCount: Number of direct children
- totalDescendants: Total count of all descendants (children, children's children, etc.)

uniqueId is included for the main inspected instance when available. This is a unique identifier for each instance in the DataModel.

MULTIPLE MATCHES OUTPUT FORMAT:
{
  "matches": [
    { 
      "name": "MyPart", 
      "className": "Part", 
      "path": "Workspace.MyPart",
      "uniqueId": "RBX1234567890ABCDEF",
      "properties": {...}, 
      "attributes": {...},
      "children": {...}
    },
    { 
      "name": "MyPart", 
      "className": "MeshPart", 
      "path": "Workspace.MyPart",
      "uniqueId": "RBX9876543210FEDCBA",
      "properties": {...}, 
      "attributes": {...},
      "children": {...}
    }
  ],
  "count": 2,
  "totalFound": 2,
  "note": "Multiple instances found with path 'Workspace.MyPart'. All 2 matches are included below."
}

WHEN TOO MANY MATCHES (>20):
{
  "matches": [ /* first 20 matches with full details */ ],
  "count": 20,
  "totalFound": 45,
  "note": "Multiple instances found with path 'Workspace.Part'.",
  "warning": "Found 45 total matches, but only showing first 20 due to output limits. Consider using a more specific path to narrow down results."
}

PATH SPECIFICATION:
- Use dot notation: "Workspace.Model.Part"
- Can include or omit "game." prefix
- Path must match instance hierarchy
- If multiple instances have the same name at the same level, all will be returned
- Maximum of 20 matches will be returned (with warning if more exist)

EXAMPLES:
- Inspect with children: InspectInstance(path: "Workspace.Baseplate")
- Multiple matches: InspectInstance(path: "Workspace.Part") → Returns up to 20 Parts with children summary
- Case-insensitive: InspectInstance(path: "workspace.baseplate")
- Inspect script: InspectInstance(path: "ServerScriptService.MainScript")
- Inspect GUI: InspectInstance(path: "StarterGui.ScreenGui.Frame")
- With game prefix: InspectInstance(path: "game.Workspace.Model")

CHILDREN SUMMARY USE CASES:
- Understand instance hierarchy structure
- Count total descendants for performance considerations
- See immediate children for navigation purposes
- Identify complex models with many nested children
- Plan traversal strategies for large hierarchies

| param | type | req | description |
|---|---|---|---|
| `path` | string | ✔ | Path to the instance(s) to inspect using dot notation (case-insensitive). Returns detailed properties, attributes, and children summary. If multiple instances match, up to 20 will be returned with a warning if more exist. Example: 'Workspace.Model.Part' or 'workspace.model.part' |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `list_roblox_studios`  _(read-only)_

Lists the connected Roblox Studio instances so a call can be directed at one. Several instances are commonly open at once, so every tool call must include a `studio_id` identifying the target instance — call this tool first to obtain the ids. Each result has an `id` and a human-readable `name` (including its place id or file name); use the `name` to choose the intended instance, confirming with the user before modifying their game. Results include `mode` (`Edit`/`Play`) and `state` depending on Studio state, and `persistedToCloud`, `placeId`, `universeId` or `file` depending on how the place is saved.

_No parameters._

## `list_skills`  _(read-only)_

Lists every available skill with its source and a description of when to use it. Call this before starting a task to find skills that cover it, then load one with skill.

| param | type | req | description |
|---|---|---|---|
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `multi_edit`  _(destructive)_

Makes multiple edits to a single script in one operation. More efficient than multiple single edits.
Can also create new scripts if the path doesn't exist (will need to specify className).

Example Call (correct):
- Args: {"file_path":"ReplicatedStorage.Platformer.Constants","edits":[{"old_string":"JUMP_COOLDOWN = 0.15","new_string":"JUMP_COOLDOWN = 0.3"}]}

Path Format:
- Use dot notation like "game.ServerScriptService.MyScript"

Creating New Scripts:
- If the script doesn't exist, it will be created using the provided className
- className is required when creating new scripts
- First edit with empty old_string ("") sets the initial content
- Subsequent edits work normally on the created content

Before Using:
- Read or search file to understand existing script contents
- For new scripts, provide className and start with empty old_string to set initial content

Important Notes:
- All edits are applied in sequence, in the order provided
- Each edit operates on the result of the previous edit
- All edits must be valid for the operation to succeed - atomic operation
- Plan edits carefully to avoid conflicts between sequential operations

Critical Requirements:
- old_string must match script contents exactly (including whitespace)
- old_string and new_string must be different
- className required only when creating new scripts
- For new files: first edit can have empty old_string to set initial content

When Making Edits:
- Ensure all edits result in correct, runnable code
- Don't leave code in a broken state
- Use replace_all for renaming variables across the entire script

| param | type | req | description |
|---|---|---|---|
| `className` | string |  | The class name of the script to create (e.g., 'Script', 'LocalScript', 'ModuleScript'). Required only when creating new scripts, leave empty if script already exists. |
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `edits` | array<object> | ✔ | An array of edit operations. For new scripts, first edit can have empty old_string to set initial content. |
| `file_path` | string | ✔ | The dot-notation path of the script (e.g., 'game.ServerScriptService.MyScript'). Will be created if it doesn't exist. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `open_cloud_invoke`  _(destructive, open-world)_

Sends an authenticated request to a Roblox Open Cloud API and returns the status code and response body.

Authentication is handled automatically: the request runs with the Assistant's permissions, using an access token limited to the Open Cloud scopes the creator has granted. Never pass API keys or auth headers yourself.

Usage:
- endpoint is the path under https://apis.roblox.com, starting with / and including any query string (for example /cloud/v2/universes/123/developer-products). Full URLs are not accepted.
- body is a JSON-encoded string, sent with Content-Type: application/json.
- scopes lists the Open Cloud scopes the endpoint needs, for example ["universe:write"]. Optional, but passing them produces a precise error if the creator has not granted one.
- Read the endpoint's reference at https://create.roblox.com/docs/cloud with http_get to find its endpoint, method, body shape, and required scopes.

Errors:
- A denied or ungranted scope, or a 403 response, means the creator has not allowed that permission. Ask them to enable it in the Assistant MCP Permissions settings. Do not try to work around it.
- A 401 response means the token was rejected. Retry once.

Examples:
- open_cloud_invoke(method: "GET", endpoint: "/cloud/v2/universes/123", scopes: ["universe:read"])
- open_cloud_invoke(method: "PATCH", endpoint: "/cloud/v2/universes/123?updateMask=voiceChatEnabled", body: '{"voiceChatEnabled":true}', scopes: ["universe:write"])

File uploads:
- files uploads local asset files as a multipart/form-data request. Each entry is { name, path, contentType? }: name is the form field (the Assets API uses "fileContent"), path is an absolute path on the creator's machine, and contentType defaults from the file extension.
- With files, body must be a JSON object whose keys become form fields. Object values are JSON-encoded, so pass the Assets API "request" field as an object.
- Only asset files can be uploaded (bmp, fbx, flac, glb, gltf, jpeg, jpg, mov, mp3, mp4, ogg, png, rbxm, rbxmx, tga, wav), up to 5 files of 20 MB each.
- Files must be inside a directory the creator has approved in Assistant Settings -> MCP Permissions. If a file is rejected, ask the creator to approve its directory there.
- Asset uploads return an operation. Poll GET /assets/v1/operations/{operationId} until done is true to get the asset ID.
- open_cloud_invoke(method: "POST", endpoint: "/assets/v1/assets", body: '{"request":{"assetType":"Model","displayName":"Tree","description":"A tree","creationContext":{"creator":{"userId":"123"}}}}', files: [{ name: "fileContent", path: "/Users/me/tree.fbx" }], scopes: ["asset:read", "asset:write"])

| param | type | req | description |
|---|---|---|---|
| `body` | string |  | JSON-encoded request body. Omit for GET and DELETE. With files, a JSON object of form fields. |
| `endpoint` | string | ✔ | Open Cloud path under https://apis.roblox.com, starting with / and including any query string. |
| `files` | array<object> |  | Local files to upload as multipart/form-data. Requires POST, PUT, or PATCH. |
| `method` | string | ✔ | HTTP method. |
| `scopes` | array<string> |  | Open Cloud scopes the endpoint requires, such as "universe:read" or "game-pass:write". |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `save_place`  _(open-world)_

Saves the place open in Roblox Studio, the same way the user pressing Ctrl+S (File > Save) would. Never publishes: players do not see the change until the user publishes.

Where it saves depends on how the place was opened: places from Roblox (including Team Create) are saved to Roblox as a new place version; places opened from a .rbxl/.rbxlx file are written back to that file.

Call this when the user asks to save, or at the end of a change they asked you to save. Do not call it after every edit: each save to Roblox creates a version in the place's version history.

The result's status is one of:
- Saved: done. version is the new place version (Roblox saves) and filePath the file written (local saves).
- NotNeeded: there were no unsaved changes; nothing was written.
- NeedsUserAction: Studio can only save this place through a dialog (it was never saved, needs Save As, or needs to ask about script changes from a playtest). Relay the message to the user; once they save it themselves, later calls work.
- Busy: a playtest, or another save or publish, is in progress. Retry once it has finished; for a playtest, ask the user to stop it, or stop it yourself.
- Failed: the save failed; message says why.

If notesSaved is false, notesError says why. The save itself still happened; do not save again to retry the notes.

| param | type | req | description |
|---|---|---|---|
| `notes` | string |  | Optional title for this save in the place's version history. Only pass it when the user asked to label the save. With no changes it still saves, but the title can go on the latest version instead of a new one, replacing that version's notes. Must pass text moderation, and titles longer than version history allows are shortened (notesTruncated). Ignored for local place files. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `screen_capture`  _(read-only)_

Capture current edit-time screen, return the image data. If camera_position and look_at_position are provided, the camera will be temporarily set to the camera_position and look at the look_at_position.

| param | type | req | description |
|---|---|---|---|
| `camera_position` | array<number> |  | The position of the camera to capture the screen from. |
| `capture_id` | string | ✔ | Capture identifier such as 'ScreenCapture_1', 'ScreenCapture_2', etc. |
| `look_at_position` | array<number> |  | The position to look at. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `script_grep`  _(read-only)_

Runs a search for a string pattern over all script contents in the game. To avoid overwhelming output, the results are capped at 50 matches.

| param | type | req | description |
|---|---|---|---|
| `query` | string | ✔ | The string or Luau pattern to search for. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `script_read`  _(read-only)_

Reads a script from the Roblox workspace. The output will be returned with line numbers in format: LINE_NUMBER→LINE_CONTENT.
When using this tool to gather information, ensure you gather the COMPLETE context to fulfill the user's request.

Usage:
- This tool reads the entire script by default to provide complete context
- You can read multiple scripts in parallel for efficiency
- If you need to find specific symbols across multiple scripts, use the grep search tool
- Avoid re-reading the same script unless the script may have changed since the last read
- For very large scripts (thousands of lines), consider using grep_search to locate specific functions or patterns first

Path Format:
- The path needs to be a full path, with no wildcard matching
- Use dot notation like "game.ServerScriptService.MyScript"
- Script must already exist (use `file_search` or `grep_search` to find the script path first)
- Will only read existing scripts, never creates new ones

| param | type | req | description |
|---|---|---|---|
| `end_line_one_indexed_inclusive` | integer |  | The one-indexed line number to end reading at (inclusive). Required if should_read_entire_file is false. |
| `should_read_entire_file` | boolean |  | Whether to read the entire script. Defaults to true. Must be set to false when providing a line range. |
| `start_line_one_indexed` | integer |  | The one-indexed line number to start reading from (inclusive). Required if should_read_entire_file is false. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `target_file` | string | ✔ | The dot-notation path of the script to read (e.g., 'game.ServerScriptService.MyScript') |

## `script_search`  _(read-only)_

Fast script search based on fuzzy matching against script names.
Use if you know part of the script name but don't know where it's located exactly.
Response will be capped to 10 results.
Make your query more specific if need to filter results further.
Note: Pattern matching is not supported such as asterisk (*) or question mark (?) wildcards.

| param | type | req | description |
|---|---|---|---|
| `keywords` | string | ✔ | The comma-separated keywords string to search for in the game's scripts. Each keyword is case-insensitive. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `search_asset`  _(read-only, open-world)_

Searches for assets across Creator Store (public marketplace) and Creator Inventory (user/group/universe).
Use this tool to find assets by keyword before inserting them with insert_asset.
Returns a list of matching assets with metadata (name, type, source, price).

Scope controls where to search:
- 'auto' (default): waterfalls through universe inventory → universe's owning group → user inventory → Creator Store. Best for general "find me an X" requests.
- 'creator_store': searches only the marketplace. Use this when the user wants marketplace assets, paid assets, or when using price/creator filters.
- 'user': searches the user's personal inventory only.
- 'group': searches the universe's owning group by default. Pass groupId or groupName to search a different group instead. If the universe has no owning group and you don't pass a groupId, the response lists the user's groups so you can retry.
- 'universe': searches the current universe's inventory only.

Targeting a specific group: when the user names a particular group ("in my group X" / "in group 12345"), set scope='group' AND pass groupId or groupName to constrain to that one group. groupId/groupName are only valid with scope='group'; combining with any other scope (including 'auto') is rejected.

Result attribution: each inventory result includes `creatorId` (string, the group or user ID that owns the asset) and `creatorName` (the group/user name when known). `creatorId` semantics depend on `source`: for source='inventory' coming from a group, it's the group ID.

Every scope='group' or scope='auto' response includes a `groups` field listing { id, name } for each of the user's groups (when the user has any). Use these to make a follow-up scope='group' + groupId call when the default (the universe's owning group, or the first relevant group) doesn't match what the user wants.

Every response also includes a `context` field describing the current Studio session: { userId, universeId, isPublished, creatorType ('User'/'Group' when published), creatorId, creatorName }. Use this to ground answers about which place is being edited and to disambiguate "in my group" requests.

Cross-owner inserts caution: when an inventory result's creatorId differs from context.creatorId (the place's owner), inserting it brings another owner's asset into this place. The user may have access to view the asset but might not intend or have the authority to share it across owners. Before calling insert_asset on a cross-owner result, name the source (the asset's creatorName) and the destination (context.creatorName or universe), and ask the user for explicit consent.

When to use price filters: If the user asks for paid/premium assets or specifies a price range, set scope='creator_store' and use priceFilter, minPriceCents, and/or maxPriceCents. These filters only apply to Creator Store searches.
When to use asset type: If the user asks for a specific asset type (audio, decals, meshes, packages, etc.), set assetType to filter results. The inventory API requires exactly one assetType per call and defaults to 'Model' when omitted, so cross-type discovery requires explicit assetType=Image / Audio / etc. Packages are stored as Models with a Package subtype — set assetType='Package' to find them (do not search for the word "package" as a query).
Inventory results (source='inventory') are always insertable. Creator Store results may occasionally be restricted — if insert fails, try the next result.
Each result includes a thumbnailUrl (Roblox Thumbnails API). Fetch it to get JSON with data[0].imageUrl pointing to a CDN image of the asset — useful for visually comparing assets before inserting.

| param | type | req | description |
|---|---|---|---|
| `assetType` | string |  | Filter by asset type. Use 'Image' for user-uploaded images (most decals/textures uploaded today are stored as Image, not Decal). Use 'Package' when the user asks about packages — packages are Models with a Package subtype, so a keyword search for 'package' won't find them. |
| `audioMaxDuration` | number |  | Maximum audio duration in seconds (only when assetType='Audio'). |
| `audioMinDuration` | number |  | Minimum audio duration in seconds (only when assetType='Audio'). |
| `excludeSources` | array<string> |  | With scope='auto': skip these sources from the waterfall. |
| `facets` | array<string> |  | Additional keywords to refine the search (Creator Store only, ignored for inventory scopes). Facets narrow results by related concepts — e.g. for a 'lion' search: 'mane', 'safari', 'realistic', 'animated'. Available facets depend on the query. |
| `groupId` | string |  | Numeric group ID to constrain group inventory searches to a single group. Only valid with scope='group'. Mutually exclusive with groupName. |
| `groupName` | string |  | Group name to constrain group inventory searches to a single group. Matched case-insensitively against the user's groups. Only valid with scope='group'. If the name doesn't match any of the user's groups, the response includes a `groups` field listing valid options. Mutually exclusive with groupId. |
| `includeSources` | array<string> |  | With scope='auto': only search these sources (e.g. ['user', 'creator_store']). |
| `maxPriceCents` | number |  | Maximum price in cents. Requires scope='creator_store'. Use with minPriceCents for a price range. |
| `maxResults` | number |  | Number of results to return (1-20, default 5). |
| `minPriceCents` | number |  | Minimum price in cents. Requires scope='creator_store'. Use with maxPriceCents for a price range (e.g. minPriceCents=100, maxPriceCents=5000). |
| `priceFilter` | string |  | Price filter. Requires scope='creator_store'. 'free' returns only free assets, 'paid' returns only paid assets, 'all' (default) returns both. |
| `query` | string |  | Search term. Can be empty when filtering by assetType alone (e.g. to list all packages). Supports multi-term search with '+' (e.g. 'red+car') and exact phrase with quotes (e.g. '"red+car"'). |
| `scope` | string |  | Where to search. 'auto' (default) waterfalls through all available sources. Use explicit scope to target a single source. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `tags` | array<string> |  | Tags to filter by (Creator Store only, ignored for inventory scopes). Tags are category labels like 'Vehicle', 'Airplane', 'Fantasy'. |
| `verifiedCreatorsOnly` | boolean |  | Only return results from verified creators (Creator Store only). |

## `search_game_tree`  _(read-only)_

Explore the Roblox game hierarchy tree with flat JSON output.
Returns an array of JSON objects representing instances in the Data Model, showing names, paths, and relationships.
Use optional filters to narrow down results by path, instance type, or keywords.
For detailed inspection of a specific instance's properties and attributes, use the Inspect Instance tool.

OUTPUT FORMAT:
Array of objects with:
{
  "name": "InstanceName",
  "className": "Part", 
  "fullPath": "Workspace.Folder.InstanceName",
  "parentName": "Folder",
  "childSummary": "5 children (3 Part, 2 Script)",  // Present if depth limit reached and children exist
  "unexploredChildCount": 5  // Present if depth limit reached with matching children
}

DEPTH LIMITING:
- Default max_depth: 3 levels from start point (absolute traversal depth)
- Absolute max: 10 levels
- max_depth limits how deep into the hierarchy we traverse, regardless of filters
- Beyond max_depth, children are summarized with counts by class type
- Nodes at depth limit show: "unexploredChildCount" and "childSummary"

EXAMPLES (every call must pass datamodel_type: use "Edit" in edit mode, or "Client"/"Server" during a playtest -- "Edit" is rejected while playing):
- Full tree (3 levels): search_game_tree(datamodel_type: "Edit")
- Workspace only: search_game_tree(datamodel_type: "Edit", path: "Workspace")
- ServerScriptService: search_game_tree(datamodel_type: "Edit", path: "ServerScriptService")
- All base scripts: search_game_tree(datamodel_type: "Edit", instance_type: "BaseScript")
- All parts: search_game_tree(datamodel_type: "Edit", instance_type: "Part")
- By keywords: search_game_tree(datamodel_type: "Edit", keywords: "player, character")
- Deep exploration: search_game_tree(datamodel_type: "Edit", max_depth: 6)
- Combined filters: search_game_tree(datamodel_type: "Edit", path: "Workspace", instance_type: "Part", keywords: "red")
- Client tree during a playtest: search_game_tree(datamodel_type: "Client", path: "Players")
- Server tree during a playtest: search_game_tree(datamodel_type: "Server", path: "Workspace")

| param | type | req | description |
|---|---|---|---|
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `head_limit` | number |  | Maximum number of results to return. Default: 200. Prevents overwhelming output for large trees. |
| `instance_type` | string |  | Filter by ClassName using IsA() check. Examples: 'BasePart', 'BaseScript', 'GuiObject', 'Model', 'Folder'. Case sensitive. |
| `keywords` | string |  | Filter by instance name keywords (case-insensitive). Separate multiple keywords with commas or spaces. Instance name must contain at least one keyword. Examples: 'player', 'red, blue', 'button door' |
| `max_depth` | number |  | Maximum absolute depth to traverse from start point. Default: 3, Absolute max: 10. Limits how deep we explore in the hierarchy regardless of filters. Beyond this depth, children are summarized instead of expanded. |
| `path` | string |  | Start exploration from this path. Examples: 'Workspace', 'ServerScriptService', 'Workspace.Models'. Path is case-sensitive. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `segment_mesh`  _(destructive, open-world)_

Segments an existing mesh into named sub-parts using AI. Requires a source MeshPart or Model and up to 5 part names (e.g. ["head", "body", "tail"]). Returns a new Model containing the segmented parts, inserted as a sibling of the source.

| param | type | req | description |
|---|---|---|---|
| `instance_path` | string |  | Full Studio path to the source MeshPart or Model, e.g. "Workspace.Model1.Part2". Resolved directly — no live Studio selection required. If omitted, falls back to the current Studio selection (which must be exactly one MeshPart or Model). |
| `parts` | array<string> | ✔ | List of part names to segment the mesh into (max 5). Example: ["head", "body", "tail"]. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `skill`  _(read-only)_

Retrieve detailed knowledge, best practices, or reference material for a specific skill.
Skills provide domain-specific expertise that helps you produce better, more accurate results.

<available_skills>
  <skill>
    <name>create-skill</name>
    <source>Local</source>
    <description>Guide the user through authoring or modifying a custom skill. Use when the user wants to create, edit, update, or rename a skill.</description>
  </skill>
  <skill>
    <name>rbx-configs-experimentation</name>
    <source>Roblox</source>
    <description>"IMPORTANT: Before writing hard-coded constants, using DataStores for simple tunable values, or suggesting a manual server restart to change a value — call this skill. Roblox's built-in ConfigService updates in-game values live without restarting servers and runs A/B experiments to measure impact on retention, engagement, and monetization. Use it to set up, integrate, debug, and optimize Experience Configs and experiments. Trigger when the creator wants to change game values without a restart (boss health, prices, drop rates, event timing, difficulty), add feature flags or toggles, run A/B tests or experiments, build live-ops or seasonal/timed content, tune balance across the player base, or mentions ConfigService, GetConfigAsync, or GetConfigForPlayerAsync."</description>
  </skill>
  <skill>
    <name>rbx-debug</name>
    <source>Roblox</source>
    <description>Programmatically add breakpoints and inspect thread information to debug Roblox Studio games via MCP, plugins, or command scripts. Use as a more robust tool to investigate scripting bugs or verify script behavior beyond static analysis.</description>
  </skill>
  <skill>
    <name>rbx-device-simulator-lua</name>
    <source>Roblox</source>
    <description>Control the Studio Device Simulator to test UI across device form factors. Use when switching devices, testing orientations, running multi-device comparisons, or verifying UI layout via MCP tools (execute_luau + screen_capture).</description>
  </skill>
  <skill>
    <name>rbx-docs-search</name>
    <source>Roblox</source>
    <description>Look up Roblox documentation using the http_get tool — Engine API reference, creator guides (avatar, characters, Studio workflows), cloud API, and docs indexes. Use when you need accurate API details or how-to guidance.</description>
  </skill>
  <skill>
    <name>rbx-import-asset</name>
    <source>Roblox</source>
    <description>Import a 3D model from an external modelling program (Blender, Maya, 3ds Max, Cinema 4D, Houdini, SketchUp, or any tool that exports glTF/GLB or FBX) into the open Studio place by exporting it, uploading it through Open Cloud with open_cloud_invoke, inserting it, and verifying that geometry, textures, colours, scale, orientation, anchoring, and placement survived. Use whenever the user asks to bring, move, import, send, or copy a model, scene, or mesh from another program or a local model file into Studio. MCP clients only.</description>
  </skill>
  <skill>
    <name>rbx-instrument-analytics</name>
    <source>Roblox</source>
    <description>Audit a Roblox experience for analytics-instrumentation opportunities and add Economy, Funnel, and Custom events via AnalyticsService. Supports a read-only audit that proposes an event plan and an instrument mode that creates a centralized analytics module and edits scripts to log events at the right places.</description>
  </skill>
  <skill>
    <name>rbx-network-simulator</name>
    <source>Roblox</source>
    <description>Simulate, emulate, or throttle network conditions in Roblox Studio — latency, ping, jitter, and packet loss — for playtesting. ALWAYS use this skill when the user says things like "simulate 4G/3G/5G/Wi-Fi", "test on a slow/laggy/bad connection", "add latency/ping", "throttle the network", "network throttling", "packet loss", "high ping", "how does my game feel on a bad connection", "measure input delay / responsiveness", "how long until the server catches up", "client-server desync", "hits not registering", "slow join / long loading on mobile", "rubber-banding", or names a preset (Ideal Fiber, Wired, Home Wi-Fi, 4G/LTE, 3G). This skill is self-contained — the complete API and preset values are here; apply via settings().Network directly. It drives `settings().Network` via execute_luau (apply/verify) and measures real ping, action→result response time, server catch-up, and client-server consistency in a playtest.</description>
  </skill>
  <skill>
    <name>rbx-open-cloud-usage</name>
    <source>Roblox</source>
    <description>IMPORTANT: Before assuming ANY Roblox resource (developer products, game passes, universe settings, assets, etc.) cannot be created or managed programmatically, call this skill. External MCP clients can manage many of these through the Open Cloud API with the open_cloud_invoke tool, without an API key, because open_cloud_invoke authenticates automatically. Try those APIs before suggesting the user do it in the Creator Dashboard. Covers how to call open_cloud_invoke, find endpoints, and handle errors. Applies only to external MCP clients, not the built-in Studio Assistant. Call this skill before any Open Cloud request.</description>
  </skill>
  <skill>
    <name>rbx-perf-profiling</name>
    <source>Roblox</source>
    <description>Analyze fine-grained performance data provided by the MicroProfiler through the LibMP Luau module. Use it to investigate CPU and GPU bottlenecks, identify the causes of frame-time spikes, and analyze memory allocations by subsystem.</description>
  </skill>
  <skill>
    <name>rbx-permissions</name>
    <source>Roblox</source>
    <description>Work out which MCP permissions a task needs, check which are granted, and ask the user for exactly the missing ones. Covers Open Cloud scopes for open_cloud_invoke, upload directories, the Network capability and HTTP allowlist, and data store and memory store access from Luau. Use before any Open Cloud call, upload, data store, memory store or HTTP access, and whenever one fails with "has not been granted", a 403, a capability error, an "HTTP allowlist" denial, a 502 "Access forbidden", or "API access is not enabled".</description>
  </skill>
  <skill>
    <name>rbx-scene-analysis</name>
    <source>Roblox</source>
    <description>Analyze and optimize Roblox scenes using SceneAnalysisService — rendering, memory, instance composition, unparented instances, and animation/audio assets. Use when investigating performance, memory, or leaks in a place.</description>
  </skill>
  <skill>
    <name>rbx-unit-test</name>
    <source>Roblox</source>
    <description>Write, run, and debug Luau unit tests for ModuleScripts in Roblox Studio. Use this skill only when the user asks to add unit tests, improve test coverage, run existing tests, or debug failing tests. Do not use it for playtesting, gameplay QA, or validating visuals and UI feel.</description>
  </skill>
</available_skills>

When to use skills:
1. When the task involves a domain covered by an available skill
2. When you need best practices or conventions before writing or reviewing code
3. When you are unsure about the correct patterns or APIs for a specific area

Important:
1. Invoke the skill BEFORE writing code or taking action, not after
2. Follow the returned guidance closely in your response
3. Do not mention a skill without actually invoking it first

| param | type | req | description |
|---|---|---|---|
| `skill_name` | string | ✔ | The name of the skill to retrieve. Options: create-skill, rbx-configs-experimentation, rbx-debug, rbx-device-simulator-lua, rbx-docs-search, rbx-import-asset, rbx-instrument-analytics, rbx-network-simulator, rbx-open-cloud-usage, rbx-perf-profiling, rbx-permissions, rbx-scene-analysis, rbx-unit-test |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `start_stop_play`

Start play the game or stop the play.

| param | type | req | description |
|---|---|---|---|
| `is_start` | boolean | ✔ | true to start the game, false to stop the game and return to edit mode. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `store_image`

Load an image from a local file path and return an IMAGEID_<id> URI that can be passed to other tools (e.g. as attachedImageUri for generate_procedural_model).
Use this tool when you need to convert a local image file into an image URI that other tools accept.
Supported formats: png, jpg, jpeg. Max file size: 5MB.

| param | type | req | description |
|---|---|---|---|
| `filePath` | string | ✔ | Absolute path to a local image file (png, jpg, or jpeg). |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `subagent`

Launch a specialized subagent to handle complex, multi-step tasks autonomously.
Subagents are independent AI assistants with restricted tool access and specialized instructions.

<available_subagents>
  <subagent>
    <name>explore</name>
    <description>Fast agent specialized for exploring codebases and searching for information. Use this when you need to:
- Find files by patterns (e.g., "scripts with 'player' in the name")
- Search code for keywords or patterns (e.g., "how are touch events handled?")
- Answer questions about the codebase structure
- Query the current game state via Luau execution
- Understand existing game architecture before making changes
- Gather context from multiple files before deciding on an approach</description>
  </subagent>
  <subagent>
    <name>playtest</name>
    <description>Agent specialized for playtesting Roblox experiences. Use this when you need to:
- Run playtests that involve moving through the world and interacting with objects like a human player.
- Verify observable outcomes through world state, GUI changes, and console output.
- For playtest requests, prefer using this subagent over directly orchestrating individual tools.
- The agent can use start_stop_play - no need to call start_stop_play.</description>
  </subagent>
</available_subagents>

When to use subagents:
1. When you need to explore the codebase before making decisions
2. When a task requires multiple search/read steps across many files
3. When you're uncertain about the right approach and need autonomous investigation
4. To separate concerns (e.g., explore first, then modify)

Important:
1. Subagents return a final text result summarizing their work
2. You cannot have a back-and-forth conversation with a subagent
3. Subagents cannot spawn other subagents (no nesting)
4. Choose the most specific subagent for your task

| param | type | req | description |
|---|---|---|---|
| `description` | string | ✔ | A short 3-5 word description of the task in present continuous tense (e.g., 'Exploring player spawn logic', 'Finding lighting scripts'). Used for display. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `subagent_type` | string | ✔ | The type of subagent to invoke. Options: explore, playtest, screen_capture, unit_test |
| `task` | string | ✔ | Detailed task description and instructions for the subagent. Be specific about what you need the subagent to do and what information to return. |

## `submit_feedback`  _(open-world)_

Sends feedback about Roblox Studio and its MCP tools to the Roblox team that builds them. Call it when a tool fails or behaves unexpectedly, a capability you need is missing, a workflow takes more steps or workarounds than it should, the user hits friction in Studio, or a tool description, result, or documentation is unclear. Only send feedback about Studio and its tools, not about the user's game or the user. If the user has asked you not to send feedback, do not call this tool. After it succeeds, tell the user in one short sentence that you sent feedback and what it was about, and that they can ask you to stop sending feedback, then continue with the task. One specific issue per call: the tools involved, if any, what you tried, what happened, and what would have worked better. Do not include source code, personal information, or credentials.

| param | type | req | description |
|---|---|---|---|
| `feedback` | string | ✔ | At most 4000 bytes. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `upload_image`  _(open-world)_

Upload a batch of images from http server to Roblox Asset Server, returning imagePath to assetId map, such as {"http://localhost/image.png" : "rbxassetid://12345678", "https://www.figma.com/api/mcp/asset/6dcff81e-b394-4640-b3b1-123456789" : "rbxassetid://12345679"}.

| param | type | req | description |
|---|---|---|---|
| `imagePaths` | array<string> | ✔ | An array of image paths to be uploaded. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `user_keyboard_input`

Send one or more keyboard inputs to the game in order. Each step is keyDown, keyUp, keyPress (keyDown then keyUp), textInput, or wait.

| param | type | req | description |
|---|---|---|---|
| `actions` | array<object> | ✔ | Ordered list of keyboard actions to perform. |
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `user_mouse_input`

Send one or more mouse actions in order: moveTo, mouseButtonDown, mouseButtonUp, mouseButtonClick, scrollUp, scrollDown, or wait. For mouseButtonDown, mouseButtonUp, and mouseButtonClick, set mouse_button to "left" or "right". After the first step that sets position (x/y or instance_path), later steps may omit coordinates and reuse that position.

| param | type | req | description |
|---|---|---|---|
| `actions` | array<object> | ✔ | Ordered list of mouse actions to perform. |
| `datamodel_type` | string | ✔ | The target datamodel to operate on, the tool can only be performed in those datamodel types. call get_studio_state to get current available datamodel types. if the target datamodel is not available in current mode, consider using start_stop_play to switch to the desired mode and then use the tools. This is a required argument. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |

## `wait_job_finished`  _(read-only)_

Wait for a background job to finish. Returns the final status and details when the job reaches a terminal state (Completed, Failed, or Cancelled).

Pass the jobId returned when a tool was called with its 'async' argument set to true.

Only call this tool when:
- The user EXPLICITLY asks to wait for a backgrounded job to finish (e.g. "wait for it to complete", "let me know when it's done")
- You need to confirm a job's result before performing a follow-up action that depends on it

| param | type | req | description |
|---|---|---|---|
| `jobId` | string | ✔ | The jobId returned when a tool was called with its 'async' argument set to true. |
| `studio_id` | string | ✔ | Selects Roblox Studio instance, use the list_roblox_studios tool to get available instances |
| `timeout` | number |  | Maximum time in seconds to wait for the job to finish. Defaults to 600 seconds. |

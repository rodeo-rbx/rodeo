---
name: server-authority-helper
description: Assists in debugging and designing Server Authority compliant scripts for Client Side Prediction.
---

# Server Authority Skill
This skill understands the Server Authority API and how to build scripts that work with client side prediction. It can review scripts for compliance with the core pillars of the Server Authority API, refactor non-compliant scripts, and author new server-authoritative gameplay.

## When to Use
- User mentions they want to convert their game to use Server Authority
- User mentions they are using Server Authority
- User asks for help debugging mispredictions or rollbacks

## How to Use This Skill
Pick the command that matches what the user wants, and read the listed files BEFORE acting. If none clearly applies, ask the user which one they want.

| User wants… | Run command | Read first |
|---|---|---|
| To find problems in an existing game | `server_authority_audit` | `api_reference/3_core_rules.md`, `api_reference/4_anti_patterns.md` |
| To fix / migrate a game to Server Authority | `server_authority_convert` | `api_reference/3_core_rules.md`, `api_reference/4_anti_patterns.md` |
| To build a new gameplay mechanic | `server_authority_create` | `api_reference/3_core_rules.md`, `code_samples/1_SimulationTemplate.luau` |
| To understand a concept or ask a question | `server_authority_explain` | `api_reference/1_concepts.md` |

## Glossary
- **Simulation**: the core gameplay logic that runs on BOTH client and server, bound via `RunService:BindToSimulation()`.
- **Predicted instance**: an instance the client simulates ahead of the server. Its SimulationAccess properties and attributes are rolled back on a misprediction. Whether a given instance is predicted is often runtime- and proximity-based (Automatic prediction) and cannot be fully determined from source — confirm at runtime with `RunService:GetPredictionStatus()`.
- **SimulationAccess property**: an engine property (for example `BasePart.CFrame`) that the prediction system synchronizes. Only these properties/methods may be read or written inside the simulation.
- **Attribute-backed state**: custom gameplay data stored in an Attribute on a predicted instance so it survives rollback. Raw Luau variables do NOT survive rollback.
- **Misprediction / rollback / resimulation**: when the server's state differs from the client's prediction, the client resets to the server state and re-runs the simulation.
- **InputAction**: the only client-authoritative input. It is replayed during resimulation. Use it instead of `UserInputService` / `ContextActionService` inside the simulation.

## Core Rules (summary)
Full detail in `api_reference/3_core_rules.md`; violation → fix table in `api_reference/4_anti_patterns.md`.
1. **Shared simulation** — core gameplay logic lives in a ModuleScript run on client AND server, updated only via `RunService:BindToSimulation()` (or `BasePart.Touched` / `InputAction.StateChanged`).
2. **Instance prediction** — only predicted instances are simulated. Skip any instance whose `RunService:GetPredictionStatus()` is `Enum.PredictionStatus.None`. Force prediction with `RunService:SetPredictionMode()`, called inside `task.defer()`.
3. **Synchronized data** — back all dynamic simulation state with Attributes on predicted instances. Never rely on raw Luau variables for state that affects the simulation.
4. **Input** — read player input only through an `InputAction` parented under a Player. Never use `UserInputService` / `ContextActionService` inside the simulation.

## Recommended Scripting Architecture
Recommended scripting architecture is to have your core game loop implemented in a single Module Script in a method called Simulate() that is passed into `RunService:BindToSimulation()` from a Start() method that is called on both Client and Server.
> See code_samples/1_SimulationTemplate.luau

Loading the Module script should be done with two child scripts below the Module Script. One with RunContext = Client, one with RunContext = Server. Each should simply be:

> ReplicatedStorage.Simulation.Client (Client Script)
```lua
require(script.Parent).Start()
```

> ReplicatedStorage.Simulation.Server (Server Script)
```lua
require(script.Parent).Start()
```

## Reference Material
Detailed API documentation lives in `api_reference/`:
- `api_reference/1_concepts.md` — core concepts of server authority and client side prediction
- `api_reference/2_spec.md` — the Server Authority API specification
- `api_reference/3_core_rules.md` — the core rules scripts must follow to be compliant
- `api_reference/4_anti_patterns.md` — common anti patterns and their fixes

Worked examples live in `code_samples/`, starting with `code_samples/1_SimulationTemplate.luau`.


---

## `api_reference/concepts.md`

# Server Authority API Concepts
In a server authority model, the server is the single source of truth for the entire game state, and clients are only trusted to report their own inputs. This architecture is the core netcode foundation of a fair, competitive game because it prevents entire classes of cheating like flyhacks or speedhacks by never trusting a client to report its own position or state.

In a naive server-owned system, clients would simply send their inputs to the server and display the results of the game sent back by the server. While technically correct, such a system would face significant input latency because every player action would have to travel to the server, be processed, and have the result sent back to the client before it could be displayed. For most games, especially those fast‑paced, that round‑trip delay would cause gameplay to feel laggy, unresponsive, and unplayable.

In Roblox's server authority model, latency is compensated for by having clients instantly predict the effects of their inputs in addition to sending them to the server. For example, when a player presses a key, the client doesn't wait for the server to respond; instead it predicts a few frames ahead of the last known server state. This allows the client to show the result of the input action instantly, effectively hiding network latency and making the game feel responsive.

Sometimes the client will get its prediction wrong (misprediction) and, because of network latency, the client will not know that it has made a mistake for a few frames. For example:

Your client predicts that you've moved forward, but on the server another player used a stun ability that stopped you from moving. The client and server now have different states.

When a misprediction is detected, the client must correct its prediction based on the server's authoritative state. If the authoritative state is different from the client's predicted state, the client must roll back and resimulate its predicted frames. This system of client‑side prediction, rollback, and resimulation is known as "latency compensation" and helps make server‑authoritative multiplayer games feel smooth and responsive.

Mispredictions are a normal and expected part of the server authority architecture because clients inherently cannot predict other players' inputs, so actions by other players will always trigger some amount of misprediction. When tuned correctly, the resulting corrections are small and should be imperceptible to players. Read further on advanced techniques to smooth out and mask network artifacts.

## Concepts
The server authority system runs on a few core concepts as follows.

### Client prediction
Through client prediction, the client simulates a few frames ahead of the last known server state to predict the effects of player inputs immediately. This hides input latency, but the prediction may later turn out to be incorrect (client misprediction) and thus require correction. The client tries to simulate just far enough ahead of the last known authoritative server state so that its inputs arrive on the server at the intended frame. The number of frames the client will predict ahead of the known server state is based on the latency behind the client and server.

### Client misprediction
When the client receives the authoritative state from the server, it checks that state against a historical record of what it predicted locally for that frame. When there is a difference between what the client predicted and what the server actually did, this is a misprediction. Mispredictions can occur for several reasons, including shifts in network latency, other players acting in ways the client didn't anticipate, the game running certain logic exclusively on the server, etc.

If the authoritative state is different from the client's predicted state, the client must roll back and resimulate.

### Rollback and resimulation
When a client detects a misprediction, it must reset to the server's authoritative state and then resimulate to jump back to its predicted frame. Based on the network latency, the client tries to simulate just far enough ahead of the last known authoritative server state so that its inputs arrive on the server at the intended frame.

### Network Ownership and Prediction
In the server authority model, you're able to keep the core gameplay objects server‑owned without incurring the input latency cost normally associated with server ownership. Things like cars, player characters, or other gameplay‑critical objects can remain server‑owned, even when interacting with other players.

By default, Roblox will automatically predict properties with SimulationAccess near the local player Character, but if you want more fine‑grained control, you can explicitly force an instance's prediction on or off with RunService:SetPredictionMode(). You should try to keep the number of predicted instances as small as possible. There's no value in spending CPU cycles resimulating something a player will never see.

### Simulation Sync
In the server authority model, the client and the server must both run the core simulation, and the client's simulation needs to be able to roll back and resimulate when a misprediction occurs. To enable this, write your core logic inside functions bound through RunService:BindToSimulation() in a ModuleScript that's initialized on both the client and server.
During a resimulation, Roblox will re-run the functions bound to the simulation via BindToSimulation(). Processing player inputs, interacting with synchronized physics objects, and updating the core game state should live inside those bound functions.

### State Sync with Attributes
Roblox automatically synchronizes all properties with simulation access on predicted instances. For custom data, attributes are the primary way to synchronize instances marked as predicted; on such instances, any mismatch in attribute values between the server's source of truth and the client's prediction will trigger a full rollback and resimulation.

Use attributes to store game data that affects your core simulation, including player health, ammunition, inventory, or custom rules (see limitations).

Only write to attributes on predicted instances from within functions bound through BindToSimulation() to ensure that Roblox can fully track relevant game states and resimulate properly.

Note that changes to the Attribute Schema on an instance are more expensive than changes to the attribute values. That means that adding new attributes, deleting existing attributes or changing the types of attributes has a higher bandwidth cost than simply changing the values of attributes. Thus changing the values of attributes without changing the types should always be preferred.

### Instance Stitching
Instances can be predictively created, and the Roblox engine will try to merge the Client created instance with the Server created Instance when the Server instance is received. Instances that are created from within the Simulation context get a deterministically created GUID that should match the GUID created on the server for the same instance.
When creating a new Instance you can freely set non-simulation access properties such as Name, Size, or Parent on an Instance before it is parented into the DataModel.

### SimulationAccess Properties
Many properties and methods in the engine API reference include the Simulation Access label, for example BasePart.CFrame. Properties with this label will be predicted by the server authority system. Additionally, only properties and methods with this label can be accessed inside functions bound with RunService:BindToSimulation().

### InputActions
In a server-authoritative game, the primary way for a client to affect the game's state is through the Input Action System. These inputs are sent to the server and are replayed during resimulation on the client. As a result, InputActions should be used for all inputs that affect the core simulation and they should be checked for sanity before they're processed.

To reiterate, do not use traditional events like UserInputService.InputBegan in the core simulation of a server-authoritative game.

Note that InputContexts must be a descendent of a Player so that the engine knows who has ownership over the InputContext. One approach is to add your InputContexts to a folder under ReplicatedStorage and use a Script under ServerScriptService to clone the InputContexts per player:

```lua
ServerScriptService/InputSetup.lua
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local InputsFolder = ReplicatedStorage:WaitForChild("Inputs")

local function onPlayerAdded(player)
	local clone = InputsFolder:Clone()
	clone.Parent = player
end

Players.PlayerAdded:Connect(onPlayerAdded)

for _, player in Players:GetPlayers() do
	onPlayerAdded(player)
end
```

# Game Design
Network latency between the client and server will fundamentally create network artifacts.
Remote player's inputs can never be predicted. You will not know that a remote player has pressed the Jump button until ~100 milliseconds after they have jumped.
Remote player inputs thus cause mispredictions. The magnitude of those mispredictions are increased if the instantaneous response to input is large.
This means that high acceleration movement will create more visual artifacts than low acceleration movement.
Take for example, a game mechanic where a player can press a button to instantly explode a bomb. This bomb has a huge impact on the game state, applying large forces to a large number of objects.
We will never predict that the player has pushed the button, and thus we will predict the bomb did not explode and the visual impact of mispredicting the bomb explosion will be large.
We can slightly adjust this game mechanic to greatly lower the visual impact of misprediction. We can instead make it so pressing the button _lights a fuse_ that starts a countdown before the bomb explodes.
Now our misprediction is on the lighting of the fuse, a very minor detail. The explosion of the bomb can be perfectly predicted once the fuse timer is synchronized.
This design principle can be applied to lessen the visual impact of network artifacts on your game.
We can summarize it like this:
Slower responses to input can be used to hide or lessen visual artifacts.


---

## `api_reference/spec.md`

```lua
RunService:BindToSimulation(
function:function, frequency:Enum.StepFrequency, priority:number
):RBXScriptConnection
```
- summary:
	Binds a custom function to be called at a fixed frequency which is independent of the frame rate.
- code_samples:
	- RunService-BindToSimulation
- parameters:
	- `function`: The function to call. This function will be passed one parameter called `deltaTime` which shows how much time passed between the beginning of the previous simulation step and the beginning of the current simulation step.
	- `frequency`: Optional `Enum.StepFrequency` value indicating the frequency at which to call the bound function. If not provided, the default frequency will be used.
	- `priority`: Optional priority of the binding as an integer; it determines the order in which bound functions are called within a simulation step. Lower numbers are called first. If two bindings have the same priority, the order between them is unspecified. Defaults to 2000.
- returns:
	- An `Datatype.RBXScriptConnection` that can be disconnected to unbind the function from the simulation step.
- description:
`BindToSimulation()` binds a custom function to be called at a fixed frequency which is independent of the frame rate. In the server authority model, bound functions will also get called when the client needs to resimulate. This method is only available when `Class.Workspace.UseFixedSimulation` is enabled.

Note that since the bound function is meant for physics controllers and prediction, an error will trigger if you read or write from unsynchronized properties or call an unsynchronized method. This error is meant to help guide you in writing a pure, synchronized simulation.

To synchronize inaccessible properties, you should store the relevant data in [attributes](../../../studio/properties.md#instance-attributes) and update the relevant property in `Class.RunService.PostSimulation|PostSimulation` or `Class.RunService.RenderStepped|RenderStepped` by reading from attributes. This will leverage attribute synchronization to keep the property fully synchronized through the rollback mechanism.

```lua
RunService:BindToAnimation(
function:function, frequency:Enum.StepFrequency, priority:number
):RBXScriptConnection
```
- summary:
	Binds a custom function to be called at a fixed frequency before Animators update when fixed simulation is enabled, or immediately before `BindToSimulation` callbacks otherwise.
- parameters:
	- `function`: The function to call. This function will be passed one parameter called `deltaTime`, which shows the elapsed time since the callback's previous invocation.
	- `frequency`: Optional `Enum.StepFrequency` value indicating the frequency at which to call the bound function. Defaults to `Enum.StepFrequency.Hz30`.
	- `priority`: Optional priority of the binding as an integer; it determines the order in which bound functions are called within a simulation step. Lower numbers are called first. If two bindings have the same priority, the order between them is unspecified. Defaults to 2000.
- returns:
	- A `Datatype.RBXScriptConnection` that can be disconnected to unregister the callback.
    `BindToAnimation()` binds a custom function to be called at a fixed frequency which is independent of the frame rate. When `Class.Workspace.UseFixedSimulation` is enabled, these functions are invoked before Animators update, making this the fixed-frequency counterpart to `Class.RunService.PreAnimation|PreAnimation`. When fixed simulation is disabled, they instead run immediately before `Class.RunService:BindToSimulation()|BindToSimulation()` callbacks, so no pre-animation ordering should be assumed. In the server authority model, bound functions will also get called when the client needs to resimulate.

- description:
Like `Class.RunService:BindToSimulation()|BindToSimulation()`, the bound function is meant for physics controllers and prediction, so an error will trigger if you read or write from unsynchronized properties or call an unsynchronized method. This error is meant to help guide you in writing a pure, synchronized simulation.

To synchronize inaccessible properties, you should store the relevant data in [attributes](../../../studio/properties.md#instance-attributes) and update the relevant property in `Class.RunService.PostSimulation|PostSimulation` or `Class.RunService.RenderStepped|RenderStepped` by reading from attributes. This will leverage attribute synchronization to keep the property fully synchronized through the rollback mechanism.

```lua
RunService:GetPredictionStatus(context:Instance):Enum.PredictionStatus
```
- summary:
  	Checks the `Enum.PredictionStatus` of a specific context instance, useful for debugging scripts affecting multiple instances where some might be predicted and others might not.
- parameters:
	- `context`: The `Class.Instance` for which to check prediction status.
- returns:
	- The `Enum.PredictionStatus` of the given instance, indicating whether it is predicted, authoritative, or not participating in prediction.
- description:
Clients can use this method to check the `Enum.PredictionStatus` of a specific context instance, useful for debugging scripts affecting multiple instances (vehicle controllers, custom physics, etc.) where some might be predicted and others might not. This method is also useful for debugging and observing the effects of `Enum.PredictionMode.Automatic` prediction mode.

```lua
RunService:SetPredictionMode(
context:Instance, mode:Enum.PredictionMode
)
```
- summary:
	Sets the prediction mode for an `Class.Instance` to an `Enum.PredictionMode` value.
- parameters:
	- `context`: The `Class.Instance` for which to set the prediction mode.
	- `mode`: The `Enum.PredictionMode` to set for the context instance.
- description:
Sets the prediction mode for an `Class.Instance` (`context`) to an `Enum.PredictionMode` value. Mismatches between the physics properties or attributes for predicted instances will cause a rollback and resimulation on the client. Only has effect when called on the client.

```lua
RunService:IsResimulating():boolean
```
- summary:
	Returns whether the client is currently in a resimulation step after a
misprediction in the server authority model.
- returns:
	- Whether the engine is currently resimulating.
- description:
Returns `true` if the engine is currently replaying simulation steps after
detecting a misprediction in the server authority model; returns `false`
otherwise. When the server's authoritative state diverges from the client's
predicted state, the client rolls back and rapidly resimulates all affected
steps within a single frame. During that resimulation pass,
`Class.RunService:BindToSimulation()|BindToSimulation()` callbacks and
property-changed signals fire again for each replayed step, which can cause
unwanted side effects such as duplicate sound playback or redundant network
requests.

```lua
time()
```
- summary:
	Returns the amount of time in seconds that has elapsed since the current
game instance started running.
- returns:
	- number: The elapsed time, in seconds, since the current game instance started running.
- description:
Returns the amount of time, in seconds, that has elapsed since the current
game instance started running. If the current game instance is not
running, this will be `0`.

If `Class.Workspace.AuthorityMode` is `Enum.AuthorityMode.Server`, this
value is synchronized between client and server.


---

## `api_reference/core_rules.md`

# Core Rules For Server Authority Scripting

### Rule 1 Shared Simulation
The game simulation must execute on both client and server.
The entire game simulation should exist in one or several ModuleScripts that are executed on both client and server.

Typical approach is to have a Module Script in ReplicatedStorage with a Start() method that is required() by a child Script with RunContext = Server and another child Script with RunContext = Client.

The game simulation's update logic must only be updated with RunService:BindToSimulation() and not Heartbeat, RenderStepped, or other update signals.

The instances that make up the game simulation should not be touched outside of this shared context.

Note that you can also use InputAction.StateChanged signal, as it runs under the simulation context.
Note that you can also use BasePart:Touched signal for core game logic.
The shared simulation should be as simple as possible to achieve the gameplay goals of the experience. Particles, for instance, can be client side only, created through LocalScripts or scripts with RunContext=Client. This is a performance optimization to keep cosmetic things out of the shared simulation.

The Shared Simulation operates on a subset of the datamodel. It should only touch Predicted Instances' Properties marked as SimulationAccess and Attributes.

We use RunService:GetPredictionStatus(instance) to see if we are Enum.PredictionStatus.Authoritative or Enum.PredictionStatus.Predicted or Enum.PredictionStatus.None. If RunService:GetPredictionStatus(instance) returns Enum.PredictionStatus.None, then we do not need to process scripts for that instance.

- Core Game Simulation executes on both client and server via a Module Script
- Core Game Simulation exists in RunService:BindToSimulation(), BasePart.Touched, and/or InputAction.StateChanged
- Core Game Simulation is not mutated outside of these contexts ever
- Core Game Simulation is as simple as possible for the game mechanics

#### Intentional Server Side Only Game Mechanic
Note that occasionally, Creators may choose to not predict a certain gameplay event, like a Kill or a Goal being scored. By not predicting something, the game will never _mispredict_ that event.

For example, if you predict a players death, you may be incorrect and have to undo their death on a misprediction. This can be very visually jarring.

In those cases, its better to simply guard the relevant code section in `if RunService:IsServer()`.
This causes latency on the event as the client waits for the server data, but means there is no chance of mispredicting the event happening.

The lua api call time() is synchronized between client and server. os.time() is not.

Recommended design is to have a Module Script in Replicated Storage:
> ReplicatedStorage/Simulation (ModuleScript)
```lua
local module = {}
local RunService:RunService = game:GetService("RunService")
local Players:Players = game:GetService("Players")
local ReplicatedStorage:ReplicatedStorage = game:GetService("ReplicatedStorage")

---------
-- Simulation must run on both Client and Server.
-- We can do this by calling Start() with a Server Script and a Client Script.
---------
function module.Start()
    -- If you have custom InputContexts, clone them from a template InputContext
    -- under ReplicatedStorage.
	if RunService:IsServer() then
		Players.PlayerAdded:Connect(function(player:Instance)
			local inputContextTemplate:InputContext = ReplicatedStorage.MyInputContext
			local newInputContext:InputContext = inputContextTemplate:Clone()
			newInputContext.Parent = player
		end)
	end
	RunService:BindToSimulation(module.Simulate)
end

function module.Simulate(deltaTime:number)
	for _, player:Player in game.Players:GetPlayers() do	
		if player.Character == nil then continue end
		local humanoidRootPart:BasePart = player.Character:FindFirstChild("HumanoidRootPart")
		if humanoidRootPart == nil then continue end
		if RunService:GetPredictionStatus(humanoidRootPart) == Enum.PredictionStatus.None then continue end
		module.SimulatePlayer(player, humanoidRootPart, deltaTime)
	end	
end

function module.SimulatePlayer(player:Player, humanoidRootPart:BasePart, deltaTime:number)
    -- TODO make a fun game
    -- by reading from InputActions under the Player
    -- and writing to SimulationAccess properties and Attributes
    -- on predicted instances
end

return module
```
With two child scripts underneath this script:

> ReplicatedStorage/ModuleScript/Client (Script with RunContext = Client)
```lua
require(script.Parent).Start()
```
> ReplicatedStorage/ModuleScript/Server (Script with RunContext = Server)
```lua
require(script.Parent).Start()
```

### Rule 2 Instance Prediction
Only a subset of the DataModel is part of this client side prediction system. Instances that are Predicted are simulated on the Client ahead of the Server data, and divergences on their SimulationAccess properties and their attributes cause a full rollback and resimulation.

On the Client, the local player's HumanoidRootPart, their Animator and their Humanoid are always predicted. By default, all other Instances have Enum.PredictionMode.Automatic as their default prediction mode. In this Automatic prediction mode, nearby BaseParts become predicted as they become close to the local player's HumanoidRootPart.

When a BasePart becomes predicted, the entire mechanism that it is a part of becomes predicted. Determining the extent of that mechanism is tricky, so it's preferable to write attributes for custom simulation data onto the Root Part for a given simulation or to manually mark the relevant instance as predicted.

> Note: Because Automatic prediction is runtime- and proximity-based, you cannot determine from source alone whether a given instance is predicted. When auditing, treat "is this instance predicted?" as a candidate finding to confirm at runtime with RunService:GetPredictionStatus(), not a static conclusion.

On the Client, remote players are not included in the automatic prediction system. Predicting remote players requires marking their HumanoidRootPart, Animator and Humanoid as explicitly predicted. The Players.<PlayerName>:Player Instance is not predicted and should not be predicted as it is not under the workspace. Only Instances under the workspace can be predicted.

Inputs are not broadcast from client to client by default, so full prediction of remote players can be improved by manually replicating inputs from client to client. An example of this is included in the Utilities module script below.

Automatic prediction should be an effective balance for most Creators, but non-BasePart instances will need to use RunService:SetPredictionMode(<instance>, Enum.PredictionMode.On) in order to be made predicted.

When choosing to force prediction on an instance calling RunService:SetPredictionMode(<instance>, Enum.PredictionMode.On) needs to be done in task.defer() to escape the Simulation context briefly.

> Using task.defer() to allow SetPredictionMode to be called from within the Simulation
> mostly useful when creating new instances from within the Simulation
```lua
task.defer(function()
	RunService:SetPredictionMode(instance, Enum.PredictionMode.On)
end)
```

> Using RunService:GetPredictionStatus() to only run simulation code for instances that should be simulated
```lua
if RunService:GetPredictionStatus(myInstance) ~= Enum.PredictionStatus.None then
    SimulateInstance(myInstance)
end
```

> Using RunService:SetPredictionMode() to control what is and is not predicted
```lua
local instance:Instance = script.Parent
RunService:SetPredictionMode(instance, Enum.PredictionMode.Off)
```

### Rule 3 Synchronized Data
All custom data for the simulation must be synchronized by being written into the datamodel. The recommended technique is to use Attributes on Predicted Instances. The Simulation can read static data from raw luau variables or can traverse the DataModel - this is fine. Dynamic data that is critical to the Simulation needs to be backed by Attributes on PredictedInstances. Raw unsynchronized lua state is not rolled back and reading from dynamic lua state in the simulation callbacks is considered an error. Instead, that dynamic data should be written to an Attribute on a Predicted Instance every frame.

This is the rule that is most easy to miss and most likely to cause bugs in Creators games. All data touched by BindToSimulation() (and other simulation callbacks) should be read from and then written to Attributes on Predicted Instances. To confirm adherence to this rule, this skill needs to confirm that all dynamic data is read from an Attribute, changes are written to an Attribute, and that the instances are all predicted, whether via understanding Auto Prediction or manually marking the relevant instance as predicted.

> Using attributes to store custom data
```lua
function module.SimulatePlayer(player:Player, humanoidRootPart:BasePart, deltaTime:number)
	-- Use an Attribute for the player's Mana
	local mana = humanoidRootPart:GetAttribute("Mana") or 100.0
    -- todo do something more interesting with mana, like cast a spell!
	mana = math.min(100.0, mana + deltaTime * 33.0)
	-- Make sure to write the Mana back into an Attribute to sync it.
	humanoidRootPart:SetAttribute("Mana", mana)
end
```

### Rule 4 Input
Inputs are the only data that Clients have authority over. Normally, this is Key Presses, Mouse Clicks and other nominal input mechanics. Inputs are sent from the Client to the Server via InputActions that are parented to an InputContext that is parented to a Player. InputActions that are descendent of a Player are owned by that Player and that Players Client sends the states of those InputActions to the Server. This is how the Server reads Input from the clients.

Input Actions have a StateChanged signal. This signal also runs in the Simulation context when the InputAction is a child of a player. InputAction.StateChanged can therefore touch the core simulation

InputActions:GetState() only contains the current state. To detect new button presses there are two options: Use InputAction.StateChanged or encode each frames InputAction:GetState() into an Attribute on a Predicted Instance such that one can read the previous frames state via an attribute.
Here is a code sample of the latter option:

> Storing the current input state in an Attribute so it can be read the next frame to detect changes
```lua
function module.SimulatePlayer(player: Player, humanoid: Humanoid, humanoidRootPart: BasePart, deltaTime: number)
	local jumpAction = player:FindFirstChild("JumpAction", true)
	local jumpHeld = jumpAction ~= nil and jumpAction:GetState()
	local jumpPrevHeld = humanoidRootPart:GetAttribute("JumpPrevHeld") or false
	humanoidRootPart:SetAttribute("JumpPrevHeld", jumpHeld)
	if jumpHeld and not jumpPrevHeld then
		-- new jump press detected
		doCustomJump()
	end
end
```


---

## `api_reference/anti_patterns.md`

# Server Authority Anti Patterns

## Violation → Fix Quick Reference
Use this table when auditing or converting. Each row is a pattern to search for, why it breaks under Server Authority, and the fix.

| Bad pattern (search for this) | Why it breaks | Fix |
|---|---|---|
| `UserInputService.InputBegan` / `ContextActionService` driving gameplay | Not replayed during resimulation, so inputs desync | Read input via `InputAction:GetState()` / `InputAction.StateChanged` |
| A `LocalScript` writing to the local player's `Humanoid` / `HumanoidRootPart` | Client no longer has network ownership; server never runs the LocalScript | Move logic into a ModuleScript run on client AND server via `RunService:BindToSimulation()` |
| Gameplay state held in a raw Luau variable (module-level or upvalue) | Not rolled back on a misprediction, so it desyncs | Store it in an Attribute on a predicted instance; read it back each frame |
| Updating gameplay in `Heartbeat` / `RenderStepped` / `Stepped` | Not part of the simulation, not resimulated | Use `RunService:BindToSimulation()` (or `BasePart.Touched` / `InputAction.StateChanged`) |
| `RunService:SetPredictionMode()` called directly inside the simulation | Wrong context; errors | Wrap the call in `task.defer(function() ... end)` |
| Simulating an instance without checking prediction status | Runs sim logic for instances that are not predicted | Skip when `RunService:GetPredictionStatus(instance) == Enum.PredictionStatus.None` |
| Reading/writing a non-SimulationAccess property inside the simulation | Errors; property is not synchronized | Store the value in an Attribute and apply it in `PostSimulation` / `RenderStepped` |
| `os.time()` / `tick()` for gameplay timing | Not synchronized between client and server | Use `time()` |

## Anti Pattern 1 — LocalScript operating on LocalPlayer
In Legacy Authority, clients had Network Ownership over their local player's `Humanoid` and `HumanoidRootPart`, so they could write to the `HumanoidRootPart` to control movement. In Server Authority this is broken: the server does not run LocalScripts, and the local player no longer has Network Ownership over their `Humanoid` / `HumanoidRootPart`. This violates **Rule 1 (Shared Simulation)**. Move the logic into a ModuleScript loaded on client and server and drive it with `RunService:BindToSimulation()`.

```lua
-- ❌ Bad: LocalScript writing directly to the character (client-only, not simulated)
local player = game.Players.LocalPlayer
game:GetService("UserInputService").InputBegan:Connect(function(input)
	if input.KeyCode == Enum.KeyCode.Space then
		player.Character.HumanoidRootPart:ApplyImpulse(Vector3.new(0, 5000, 0))
	end
end)
```

```lua
-- ✅ Good: ModuleScript run on client AND server, driven by the simulation and an InputAction
local RunService:RunService = game:GetService("RunService")
local module = {}

function module.Start()
	RunService:BindToSimulation(module.Simulate)
end

function module.Simulate(deltaTime:number)
	for _, player:Player in game.Players:GetPlayers() do
		local hrp:BasePart? = player.Character and player.Character:FindFirstChild("HumanoidRootPart")
		if hrp == nil then continue end
		if RunService:GetPredictionStatus(hrp) == Enum.PredictionStatus.None then continue end
		local jumpAction:InputAction? = player:FindFirstChild("JumpAction", true)
		if jumpAction ~= nil and jumpAction:GetState() then
			hrp:ApplyImpulse(Vector3.new(0, 5000, 0))
		end
	end
end

return module
```

## Anti Pattern 2 — Unsynchronized Luau Data
Raw Luau variables are not synchronized between client and server and are not rolled back during client rollback and resimulation. You must back all custom simulation state with Attributes on predicted instances. This is the most common and hardest-to-spot violation.

```lua
-- ❌ Bad: fuel lives in a Luau variable, so it is never rolled back and will desync
local fuel = 100.0
function module.Simulate(deltaTime:number)
	fuel = math.max(0.0, fuel - deltaTime)
end
```

```lua
-- ✅ Good: fuel is stored in an Attribute on a predicted instance
function module.Simulate(deltaTime:number, humanoidRootPart:BasePart)
	local fuel = humanoidRootPart:GetAttribute("JetpackFuel") or 100.0
	fuel = math.max(0.0, fuel - deltaTime)
	humanoidRootPart:SetAttribute("JetpackFuel", fuel)
end
```

## Anti Pattern 3 — Predicting Too Much
The simulation is expensive for the client to predict because it must run many times per frame to stay ahead of the server state. Keep the simulation as cheap as possible to achieve the desired gameplay effect. Decouple the renderer from the simulation so cosmetic work (particles, explosion effects, smoothing) stays out of the simulation and only the gameplay-critical state is predicted.

```lua
-- ❌ Bad: spawning cosmetic particles inside the simulation (predicted and resimulated every step)
function module.Simulate(deltaTime:number)
	local sparks = Instance.new("ParticleEmitter")
	sparks.Parent = workspace.Effects
end
```

```lua
-- ✅ Good: keep cosmetics out of the simulation; render them client-side from simulation state
-- (see code_samples/4c_SimulationEvents.luau for the Simulation Events pattern)
if RunService:IsClient() and not RunService:IsResimulating() then
	task.defer(function()
		spawnSparksEffect()
	end)
end
```


---

## `code_samples/SimulationTemplate.luau`

```lua
local RunService:RunService = game:GetService("RunService")
local Players:Players = game:GetService("Players")
local ReplicatedStorage:ReplicatedStorage = game:GetService("ReplicatedStorage")

local module = {}

function module.Start()
    -- If you have custom InputContexts, clone them from a template InputContext
    -- under ReplicatedStorage.
	if RunService:IsServer() then
		Players.PlayerAdded:Connect(function(player:Instance)
			local inputContextTemplate:InputContext = ReplicatedStorage.MyInputContext
			local newInputContext:InputContext = inputContextTemplate:Clone()
			newInputContext.Parent = player
		end)
	end
	RunService:BindToSimulation(module.Simulate)
end

function module.Simulate(deltaTime:number)
	for _, player:Player in game.Players:GetPlayers() do	
		if player.Character == nil then continue end
		local humanoidRootPart:BasePart = player.Character:FindFirstChild("HumanoidRootPart")
		if humanoidRootPart == nil then continue end
		if RunService:GetPredictionStatus(humanoidRootPart) == Enum.PredictionStatus.None then continue end
		module.SimulatePlayer(player, humanoidRootPart, deltaTime)
	end	
end

function module.SimulatePlayer(player:Player, humanoidRootPart:BasePart, deltaTime:number)
    -- TODO make a fun game
    -- by reading from InputActions under the Player
    -- and writing to SimulationAccess properties and Attributes
    -- on predicted instances
end

return module
```


---

## `code_samples/JetpackSimulation.luau`

```lua
-- Jetpack Game Mechanic
-- ReplicatedStorage/JetpackSimulation (ModuleScript)
-- Implemented correctly for the Server Authority Client Side Prediction System
-- Shows how basic movement mechanics can be implemented safely and correctly.
local RunService:RunService = game:GetService("RunService")
local module = {}
---------
-- RULE 1 
-- Simulation must run on both Client and Server.
-- We can do this by calling Start() with a Server Script and a Client Script.
---------
function module.Start()
	---------
	-- RULE 1
	-- Use BindToSimulation() for update logic - not RenderStepped or Heartbeat.
	---------
	-- Add custom JetpackInputContext under each player when they connect.
	if game.RunService:IsServer() then
		game.Players.PlayerAdded:Connect(function (player:Instance)
			local inputContextTemplate:InputContext = game.ReplicatedStorage.Misc.JetpackInputContext
			local newInputContext:InputContext = inputContextTemplate:Clone()
			newInputContext.Parent = player
		end)
	end
	
	-- Simulation callbacks are called multiple times during resimulation.
	-- You can also write to the simulation using InputAction.StateChanged.
	-- InputActions under Players are rolled back and replayed
	RunService:BindToSimulation(module.SimulateJetpacks)
end

function module.SimulateJetpack(player:Player, humanoidRootPart:BasePart, deltaTime:number)
	---------
	-- RULE 3
	-- All data must be synchronized, no raw unsync'd luau data.
	-- Use InputActions under Players for inputs and Attributes for custom outputs.
	---------
	
	---------
	-- Get the Jump Input Action under the Player
	---------
	local jumpAction:InputAction = player:FindFirstChild("JumpAction", true)
	if jumpAction == nil then return end
	
	---------
	-- Use an Attribute for the Jetpack Fuel
	---------
	local fuel = humanoidRootPart:GetAttribute("JetpackFuel") or 100.0
	
	---------
	-- Actual jetpack simulation is simple.
	---------
	if jumpAction:GetState() then
		if fuel > 0 and humanoidRootPart.AssemblyLinearVelocity.Y < 5.0 then
			humanoidRootPart:ApplyImpulse(Vector3.new(0.0, 67.0, 0.0))
		end
		fuel = math.max(0.0, fuel - deltaTime * 33.0)
	else
		fuel = math.min(100.0, fuel + deltaTime * 33.0)
	end
	
	---------
	-- Make sure to write the Fuel back into an Attribute to sync it.
	---------
	humanoidRootPart:SetAttribute("JetpackFuel", fuel)
end

function module.SimulateJetpacks(deltaTime:number)
	for _, player:Player in game.Players:GetPlayers() do	
		if player.Character == nil then continue end
		local humanoidRootPart:BasePart = player.Character:FindFirstChild("HumanoidRootPart")
		if humanoidRootPart == nil then continue end
		
		-- Rule 2 Instance Prediction
		-- GetPredictionStatus() tells us if we are simulating this instance
		-- if GetPredictionStatus() returns None, we dont need to process this players jetpack.
		if RunService:GetPredictionStatus(humanoidRootPart) == Enum.PredictionStatus.None then
			continue
		end
		module.SimulateJetpack(player, humanoidRootPart, deltaTime)
	end	
end

return module
```


---

## `code_samples/CoinPickupSimulation.luau`

```lua
-- Picking up a Coin
-- This script shows how to use the Touched callback and attributes to do a basic game mechanic, picking up a coin and adding jetpack fuel.
-- The visual effect for the coin is implemented in a Client Script called CoinVisual.luau
local RunService:RunService = game:GetService("RunService")
local module = {}

function module.Start()
	game.Players.PlayerAdded:Connect(module.PlayerAdded)
	for _, player in game.Players:GetPlayers() do
		module.PlayerAdded(player)
	end
end

function module.CharacterAdded(player:Player, character:Model)
	-- get the humanoid root part
	local humanoidRootPart:BasePart = character:WaitForChild("HumanoidRootPart")
	-- Listen to the touch callback for all players on client and server
	-- In theory, we may only need to listen to predicted players touched callbacks on the client
	-- But making that handle changes to what is and isnt predicted at runtime is challenging
	-- So we take the robust route and listen to all players touched callbacks and check prediction status
	-- inside the callback.
	humanoidRootPart.Touched:Connect(function(otherPart:BasePart)
		module.PlayerTouchedPart(humanoidRootPart, otherPart)
	end)
end

function module.PlayerAdded(player:Player)
	if player.Character ~= nil then
		module.CharacterAdded(player, player.Character)
	end
	player.CharacterAdded:Connect(function(character:Model)
		module.CharacterAdded(player, character)
	end)
end

function module.PlayerTouchedPart(humanoidRootPart:BasePart, otherPart:BasePart)
	-- Rule 2 Instance Prediction
	if RunService:GetPredictionStatus(humanoidRootPart) == Enum.PredictionStatus.None then return end
	if RunService:GetPredictionStatus(otherPart) == Enum.PredictionStatus.None then return end
	if otherPart.Name == "Coin" then
		-- use the isPickedUp attribute to see if we've picked up the coin
		local isPickedUp = otherPart:GetAttribute("IsPickedUp")
		if not isPickedUp then
			-- Rule 3 Synchronized data through attributes
			otherPart:SetAttribute("IsPickedUp", true)
			local jetpackFuel = humanoidRootPart:GetAttribute("JetpackFuel") or 0.0
			jetpackFuel = jetpackFuel + 50.0
			humanoidRootPart:SetAttribute("JetpackFuel", jetpackFuel)
		end
	end
end

return module
```


---

## `code_samples/CoinVisual.luau`

```lua
local coin = script.Parent
-- Heres the visual effect for the previously shown pickup coin mechanic. Its written in a Client script child of the Coin model.
-- This is a Renderer script, it can (and should) use unsynced luau data to indicate what was previously rendered
-- The pattern in a Renderer effect is to compare the current (predicted) simulation state to what has been rendered
-- and to update the renderer effectively.
local wasPickedUp = false

local pickupSound:Sound = coin:WaitForChild("PickupSound")
local coinMesh:MeshPart = coin:WaitForChild("CoinMesh")
local pointLight:PointLight = coinMesh:WaitForChild("PointLight")
local impactEmitter:ParticleEmitter = coin:WaitForChild("Center"):WaitForChild("ImpactEmitter")
local sparkleEmitter:ParticleEmitter = coin:WaitForChild("SparkleEmitter")

function playPickupEffect()
	coinMesh.Transparency = 0.8
	pointLight.Enabled = false
	impactEmitter:Emit(1)
	sparkleEmitter:Emit(6)
	pickupSound:Play()
end

function stopPickupEffect()
	coinMesh.Transparency = 0
	pointLight.Enabled = true
end

game.RunService.RenderStepped:Connect(function(deltaTime: number) 
	local isPickedUp = coin:GetAttribute("IsPickedUp")
	if wasPickedUp ~= isPickedUp then
		if isPickedUp then
			playPickupEffect()
		else
			-- should only possible if a misprediction occurs
			-- but is good practice
			stopPickupEffect()
		end
		wasPickedUp = isPickedUp
	end	
end)
```


---

## `code_samples/RocketLauncherSimulation.luau`

```lua
--!strict
-- Rocket Launcher Game Mechanic Module Script
-- ReplicatedStorage/RocketLauncher (ModuleScript)
-- This example shows creating a new instance and using task.defer to mark it as always predicted.
-- It shows how we can use Simulation Events to render an explosion only on client using data produced by the simulation (See #simulation-events)
-- This script requires a RocketTemplate BasePart, ReplicatedStorage/SimulationEvents Module Script
-- and ReplicatedStorage/RaycastInput Client Script.
local module = {}
local RunService:RunService = game:GetService("RunService")
local rocketCooldown = 0.5
local rocketsParent:Folder = game.Workspace.Rockets
local explosionEffect
local rocketSpeed = 2.0
local rocketTemplate = game.ReplicatedStorage.Misc.RocketTemplate

-- Make input action template
local function makeRocketInputContext():InputContext
	local inputContext = Instance.new("InputContext")
	inputContext.Name = "RocketInputContext"

	local fireRocketInputAction = Instance.new("InputAction")
	fireRocketInputAction.Name = "FireRocket"
	fireRocketInputAction.Type = Enum.InputActionType.Bool
	fireRocketInputAction.Parent = inputContext

	local fireRocketBinding = Instance.new("InputBinding")
	fireRocketBinding.Type = Enum.InputBindingType.Automatic
	fireRocketBinding.KeyCode = Enum.KeyCode.MouseLeftButton
	fireRocketBinding.Parent = fireRocketInputAction
	
	local raycastAction = Instance.new("InputAction")
	raycastAction.Name = "RaycastAction"
	raycastAction.Type = Enum.InputActionType.Direction3D
	raycastAction.Parent = inputContext

	local raycastBinding = Instance.new("InputBinding")
	raycastBinding.Type = Enum.InputBindingType.Scriptable
    raycastBinding.Name = "RaycastBinding"
	raycastBinding.Parent = raycastAction
	
	return inputContext
end

local function FireRocket(position:Vector3, direction:Vector3)
	print("FireRocket()", time())
	local newRocket:BasePart = rocketTemplate:Clone()
	newRocket.Parent = rocketsParent
	newRocket:SetAttribute("age", 0.0)
	newRocket.VectorForce.Force = Vector3.new(0.0, newRocket.Mass * workspace.Gravity, 0.0)
	newRocket.CFrame = CFrame.new(position, position + direction)
	newRocket:ApplyImpulse(direction.Unit * rocketSpeed)
	if RunService:IsClient() then
		task.defer(function ()
			RunService:SetPredictionMode(newRocket, Enum.PredictionMode.On)
		end)
	end
end

local function ExplodeRocket(rocket:BasePart)
	print("ExplodeRocket", time())
	-- TODO use custom events for the explosion effect
	local r:number = rocket:GetAttribute("ExplosionRadius") or 5.0
	if RunService:IsClient() and not RunService:IsResimulating() then
		task.defer(function()
			if explosionEffect == nil then
				explosionEffect = require(game.ReplicatedStorage.Effects.ExplosionEffect)
			end
			explosionEffect.explode(rocket.Position, r)
		end)
	end
	
	-- Apply a force all parts near the explosion
	local force:number = rocket:GetAttribute("ExplosionForce") or 20.0
	local overlapParams:OverlapParams = OverlapParams.new()
	overlapParams.CollisionGroup = "RocketExplosion"
	local touchingParts:{BasePart} = game.Workspace:GetPartBoundsInRadius(rocket.Position, r, overlapParams)
	for _, part:BasePart in touchingParts do
		if part.Name == "Block" then part.Anchored = false end
		local v:Vector3 = part.Position - rocket.Position
		if part.Name == "Ball" then
			local explosionForce:Vector3 = v.Unit * 2000.0
			part:ApplyImpulse(explosionForce)
			continue
		end
		local explosionForce:Vector3 = v.Unit * force
		explosionForce = explosionForce + Vector3.yAxis * 10.0
		part:ApplyImpulse(explosionForce)
	end
end

local function SimulateRocket(rocket:BasePart, deltaTime:number)
	-- Store a timer on each rocket as an Attribute
	local age = rocket:GetAttribute("age")
	age += deltaTime
	rocket:SetAttribute("age", age)
	if age > 3.0 then
		task.defer(function()
			rocket:Destroy()
		end)
		return
	end
	local overlapParams:OverlapParams = OverlapParams.new()
	overlapParams.CollisionGroup = "RocketExplosion"
	local touchingParts:{BasePart} = workspace:GetPartsInPart(rocket, overlapParams)
	for _, part:BasePart in touchingParts do
 		-- print("touching", part)
		-- explode
		ExplodeRocket(rocket)
		task.defer(function()
			rocket:Destroy()
		end)
		break
	end
end

local function SimulateRocketLauncher(rocketLauncher:Tool?, player:Player, humanoidRootPart:BasePart, deltaTime:number)
	if rocketLauncher == nil then return end
	-- Read all custom state via Attributes on Predicted Instances
	local cooldown:number = humanoidRootPart:GetAttribute("Cooldown") or 0.0
	if cooldown > 0.0 then
		cooldown = math.max(0.0, cooldown - deltaTime)
		humanoidRootPart:SetAttribute("Cooldown", cooldown)
	end
	-- Read input using InputAction under a player
	local fireRocketAction:InputAction? = player:FindFirstChild("FireRocket", true)
	local isInFpsAction:InputAction? = player:FindFirstChild("RotationAction", true)
	local isInFps:boolean = isInFpsAction ~= nil and isInFpsAction:GetState()
	local raycastAction:InputAction? = player:FindFirstChild("RaycastAction", true)
	local raycastPosition:Vector3 = if raycastAction ~= nil then raycastAction:GetState() else Vector3.zero
	if fireRocketAction ~= nil and fireRocketAction:GetState() and cooldown == 0.0 then
		local spawnPosition
		local direction
		if isInFps then
			local cameraCframe:CFrame = player:GetCameraState().CFrame
			direction = cameraCframe.LookVector
			local offset = Vector3.new(0.8, -0.5, 0.15)
			spawnPosition = cameraCframe.Position + cameraCframe.RightVector * offset.X + Vector3.yAxis * offset.Y + cameraCframe.LookVector * offset.Z
		else
			local attachment:Attachment = rocketLauncher.Handle.MissleSpawnLocation
			spawnPosition = attachment.WorldPosition
			local cameraCframe:CFrame = player:GetCameraState().CFrame
			direction = (raycastPosition - spawnPosition).Unit
		end
		
		FireRocket(spawnPosition + direction, direction)
		-- Reset rocket launcher cooldown by writing into Attributes
		humanoidRootPart:SetAttribute("LastRocket", time())
		humanoidRootPart:SetAttribute("Cooldown", rocketCooldown)
	end
end

local function SimulateAll(deltaTime:number)
	for _, player in game.Players:GetPlayers() do
		local character:Model? = player.Character
		if character == nil then continue end
		local humanoidRootPart:BasePart? = character:FindFirstChild("HumanoidRootPart")
		if humanoidRootPart == nil then continue end
		if RunService:GetPredictionStatus(humanoidRootPart) == Enum.PredictionStatus.None then continue end
		local rocketLauncher:Tool? = module.GetEquippedTool(player)
		SimulateRocketLauncher(rocketLauncher, player, humanoidRootPart, deltaTime)
	end
	
	local rockets:{BasePart} = rocketsParent:GetChildren()
	for _, rocket:BasePart in rockets do
		if rocket:IsA("BasePart") then
			SimulateRocket(rocket, deltaTime)
		end
	end
end

function module.GetEquippedTool(player:Player):Tool?
	if player.Character == nil then return nil end
	local tool:Tool? = player.Character:FindFirstChildOfClass("Tool")
	if tool and tool.Name == "RocketLauncher" then
		return tool
	end
	return nil
end

function module.Start()
	-- Create custom input context for each player
	if RunService:IsServer() then
		game.Players.PlayerAdded:Connect(function (player:Instance)
			local newInputContext:InputContext = makeRocketInputContext()
			newInputContext.Parent = player
		end)
	end
	RunService:BindToSimulation(SimulateAll, Enum.StepFrequency.Hz60)
end

return module
```


---

## `code_samples/RaycastInput.luau`

```lua
-- Client Script in ReplicatedStorage that pumps the raycast info into an InputAction.
-- This script is necessary to get the Raycast info into an InputAction for RocketLauncherSimulation to know 
-- in what direction to fire the Rocket.
local Players:Players = game:GetService("Players")
local UserInputService:UserInputService = game:GetService("UserInputService")
local Workspace = game:GetService("Workspace")

local player:Player = Players.LocalPlayer
local inputBinding:InputBinding = player:WaitForChild("RocketInputContext"):WaitForChild("RaycastAction"):WaitForChild("RaycastBinding")
local camera:Camera = Workspace.CurrentCamera

if player.Character == nil then
	player.CharacterAdded:Wait()
end

local raycastParams = RaycastParams.new()
raycastParams.FilterType = Enum.RaycastFilterType.Exclude
raycastParams.FilterDescendantsInstances = {player.Character}

local maxDistance = 1000

local function getMouseWorldPosition(): Vector3
	local mouseLocation = UserInputService:GetMouseLocation()
	local unitRay = camera:ViewportPointToRay(mouseLocation.X, mouseLocation.Y)
	local raycastResult = Workspace:Raycast(unitRay.Origin, unitRay.Direction * maxDistance, raycastParams)
	if raycastResult then
		return raycastResult.Position
	end
	return unitRay.Origin + (unitRay.Direction * maxDistance)
end

game.RunService.RenderStepped:Connect(function(deltaTime: number) 
	local raycastPosition:Vector3 = getMouseWorldPosition()
	inputBinding:Fire(raycastPosition)
end)
```


---

## `code_samples/SimulationEvents.luau`

```lua
-- !strict
-- Simulation Events Module Script
-- ReplicatedStorage/SimulationEvents (ModuleScript)
-- By decoupling the renderer from the simulation we gain greater power over the rendering, allowing us to hide network artifacts.
-- See #smoothing section for one example of smoothing out visual artifacts from sudden unpredicted movement.
-- Another technique is to have the simulation produce 'Simulation Events' that are then rendered client side.
-- Simulation Events are simply small chunks of server authoritative data that are predicted to have happened, and ultimately confirmed by server data or deleted.
-- The insight in this design is that SimulationEvents are just more State.
-- This system is a way you can procedurally write _more state_ into the simulation with an ergonomic API.
-- It can be used to capture data from a moment in the simulation so that it can be rendered effectively on the client.
-- An example is a rocket exploding. We want to render an explosion effect on the clients. We dont want to use server authority for the cosmetic effect of the explosion, the server doesnt need to know
-- where each chunk of visual debris is. The explosion _effect_ is just cosmetic, it doesnt affect the simulation. So instead of creating all the explosion pieces inside the simulation and using server authority to correct it,
-- we encapsulate just the right subset of data that the renderer needs to be able to show an explosion effect. We do this through the SimulationEvents module script.
local module = {}
local CollectionService = game:GetService("CollectionService")
local RunService = game:GetService("RunService")
local utils = require(game.ReplicatedStorage.Utilities.Utils)
local frameCount = 0
local clientInitialized = false
local eventsParent = utils.sharedInstance(game.Workspace, "Folder", "SimulationEvents")
assert(eventsParent ~= nil)
local eventDestroyTime = 3.0

type EventData = {[string]:any}
type EventDataMap = {
	[Instance]:EventData
}

module.RenderEventType = table.freeze({
	New = 1,
	Change = 2,
	Delete = 3,
})
export type RenderEventType = number

export type RenderEventArgs = {
	instance:Instance,
	attributes:any,
	attributesPrevious:any,
	attributesDiff:any,
	type:RenderEventType,
}

export type RenderEventCallback = (RenderEventArgs)->()

local renderingEvents:EventDataMap = {}
local callbackMap:{[string]:RenderEventCallback} = {}
local destroyedEvents:{Instance} = {}

function module.CreateEvent(tag:string, data:any)
	local event = Instance.new("Folder")
	assert(eventsParent ~= nil)
	event.Name = string.format("%s_%d", tag, math.round(time() * 60))
	event.Parent = eventsParent
	for key, value in data do
		event:SetAttribute(key, value)
	end
	
	event:SetAttribute("tag", tag)
	
	if RunService:IsServer() then
		task.delay(eventDestroyTime, function()
			event:Destroy()
		end)
	else
		task.defer(function()
			game.RunService:SetPredictionMode(event, Enum.PredictionMode.On)
		end)
	end
	return event
end

function module.RenderEvents(tag:string, callback:RenderEventCallback)
	assert(RunService:IsClient())
	if not clientInitialized then
		clientInitialized = true
		module.InitializeClient()
	end
	callbackMap[tag] = callback
end

function module.InitializeClient()
	assert(RunService:IsClient())
	
	local function tableDiff(a:any, b:any):any
		local diff:any = nil
		if b == nil then return a end
		for k, v in a do
			if b[k] ~= v then
				if diff == nil then
					diff = {}
				end
				diff[k] = v
			end
		end
		return diff
	end

	local function render(deltaTime:number)
		frameCount = frameCount + 1
		
		-- Look for new events
		for _, event:Instance in eventsParent:GetChildren() do
			if renderingEvents[event] == nil then
				local attributes = event:GetAttributes()
				renderingEvents[event] = attributes
				local tag:string = attributes["tag"]
				local callback:RenderEventCallback = callbackMap[tag]
				if callback ~= nil then
					callback({
						instance = event,
						attributes = attributes,
						attributesPrevious = nil,
						attributesDiff = attributes,
						type = module.RenderEventType.New
					})
				end
			end
		end
		
		-- Look for changed and deleted events
		for event:Instance, previousAttributes:EventData in renderingEvents do
			local attributes = event:GetAttributes()
			local tag = attributes["tag"]
			local callback:RenderEventCallback = callbackMap[tag]
			if callback == nil then
				continue
			end
			
			if event.Parent == nil then
				-- deleted event
				callback({
					instance = event,
					attributes = nil,
					attributesPrevious = previousAttributes,
					attributesDiff = {},
					type = module.RenderEventType.Delete
				})
				renderingEvents[event] = nil
			else
				local diff = tableDiff(attributes, previousAttributes)
				if diff ~= nil then
					-- changed event
					callback({
						instance = event,
						attributes = attributes,
						attributesPrevious = previousAttributes,
						attributesDiff = diff,
						type = module.RenderEventType.Change
					})
					renderingEvents[event] = attributes
				end
			end
		end
	end
	
	RunService:BindToRenderStep("renderEvents", Enum.RenderPriority.Character.Value, render)
end

return module
```


---

## `code_samples/SmoothingModule.luau`

```lua
-- ReplicatedStorage.SmoothingModule
-- Can be used to improve the visual appearance of client side prediction
-- by smoothing out the position jumps associated with mispredictions.
-- ie: it turns sudden movements into smooth movements.
-- This can be used by placing SmoothMe Client Script under a Part and setting the Target attribute
-- on that script to the part to smoothly track.
-- The smoothing engages if the difference in position the rendered object and the target object
-- is between 'SmoothEngageMin' and 'SmoothEngageMax'
-- This means it will usually be disabled, and the parent object will be in the exact
-- same location as the target object.
-- Tune EngageMin, EngageMax, Time and Disengage to values correct for your game
-- (based on the speed and acceleration of the movement).
-- Smoothing can be disabled for one frame via 'DisableForNextFrame'
-- in order to not smooth intentional jumps like teleports or respawns.
-- Default values are provided.
local TweenService:TweenService = game:GetService("TweenService")

export type SmoothData = {
	velocity:Vector3,
	position_offset:Vector3,
	is_smoothing:boolean,
	params:SmoothParams,
	last_target:Vector3,
	target_delta:Vector3,
	projected_delta:Vector3,
}

export type SmoothParams = {
	DisableOneFrame:boolean,
	Always:boolean,
	Never:boolean,
	VisualizeDebug:boolean,
	EngageMin:number,
	EngageMax:number,
	ReEngageMin:number,
	ReEngageMax:number,
	Disengage:number,
	Time:number,
}
export type SmoothParameters = {
	DisableOneFrame:boolean?,
	Always:boolean?,
	Never:boolean?,
	VisualizeDebug:boolean?,
	EngageMin:number?,
	EngageMax:number?,
	ReEngageMin:number?,
	ReEngageMax:number?,
	Disengage:number?,
	Time:number?,
	Fancy:boolean?,
}

local smoothDefaults:SmoothParams = {
	DisableOneFrame = true,
	Always = false,
	Never = false,
	VisualizeDebug = false,
	EngageMin = 1.0,
	EngageMax = 30.0,
	ReEngageMin = 10.0,
	ReEngageMax = 30.0,
	Disengage = 0.05,
	Time = 0.1,
	Fancy = true,
}

local module = {}

function module.new(parameters:SmoothParameters?):SmoothData
	local s:SmoothData = {
		velocity = Vector3.zero,
		position = Vector3.zero,
		position_offset = Vector3.zero,
		is_smoothing = false,
		smooth_velocity = Vector3.zero,
		params = table.clone(smoothDefaults),
		last_target = Vector3.zero
	}
	if parameters ~= nil then
		for k,v in parameters do
			s.params[k] = v
		end
	end
	return s
end

local function projectVector(vectorA:Vector3, vectorB:Vector3):Vector3
	local dotProduct = vectorA:Dot(vectorB)
	local bMagnitudeSquared = vectorB.Magnitude^2
	if bMagnitudeSquared == 0 then
		return Vector3.new(0, 0, 0)
	end
	local projection = (dotProduct / bMagnitudeSquared) * vectorB
	return projection
end

function module.SmoothPosition(position:Vector3, targetPosition:Vector3, data:SmoothData, deltaTime:number):Vector3
	local position_delta = (position - targetPosition)
	local position_delta_magnitude = position_delta.Magnitude
	local target_delta = data.last_target - targetPosition
	if data.params.DisableOneFrame then
		data.is_smoothing = false
		data.params.DisableOneFrame = false
	elseif data.params.Always then
		data.is_smoothing = true
		data.position_offset = position_delta
	elseif data.params.Never then
		data.is_smoothing = false
	elseif not data.is_smoothing and position_delta.Magnitude >= data.params.EngageMin and position_delta.Magnitude < data.params.EngageMax then
		data.is_smoothing = true
		data.position_offset = position_delta
		data.last_target = targetPosition
	elseif data.is_smoothing then
		if target_delta.Magnitude >= data.params.ReEngageMin then
			data.is_smoothing = true
			print("REENGAGE", target_delta.Magnitude)
			data.position_offset = position_delta
			data.last_target = targetPosition
		end
	end
	target_delta = data.last_target - targetPosition
	
	if data.is_smoothing then
		if data.params.Fancy then
			if target_delta.Magnitude > 0.0 and target_delta:Dot(data.position_offset) > 0 then
				local projected_delta = projectVector(target_delta, data.position_offset.Unit)
				data.position_offset = data.position_offset - projected_delta
			end
		end
		
		data.position_offset, data.velocity = TweenService:SmoothDamp(data.position_offset, Vector3.zero, data.velocity, data.params.Time, 1000.0, deltaTime)
		position = targetPosition + data.position_offset		
		if not data.params.Always and data.position_offset.Magnitude < data.params.Disengage then
			--print("DISENGAGE", data.position_offset.Magnitude)
			data.is_smoothing = false
		end
	end

	if not data.is_smoothing then
		data.velocity = Vector3.zero
		data.position_offset = Vector3.zero
		position = targetPosition
	end
	data.last_target = targetPosition
	
	return position
end
return module
```


---

## `code_samples/SmoothMe.luau`

```lua
-- Client Script (Script with RunContext=Client)
-- That uses ReplicatedStorage/SmoothingModule to dampen out the visual
-- impact of mispredictions on the position of an object
-- Using the smoothing module script involves adding this Client Script under a Renderer object that is made Anchored, Not Collidable and Visible in the Workspace and setting a "Target" Instance Attribute on the Script that points at the Simulated object.
local smoothingModule = require(game.ReplicatedStorage.SmoothingModule)

local smoothData = smoothingModule.new({
	Time=0.15,
	EngageMin=3.0,
	EngageMax=20.0,
	Disengage=1.0,
})

local render:Part = script.Parent

local function getTargetPosition()
	local targetHandle:InstanceHandle? = script:GetAttribute("Target")
	if targetHandle == nil or typeof(targetHandle) ~= "InstanceHandle" then
		return nil, nil
	end
	local target = targetHandle:Get()
	if target == nil then
		return nil, nil
	end
	return target.Position, target.Rotation
end

render.CanCollide = false
render.Anchored = true
render.CanQuery = false
render.CanTouch = false

game.RunService.RenderStepped:Connect(function(deltaTime: number)
	local targetPosition, targetRotation = getTargetPosition()
	if targetPosition == nil then
		targetPosition = smoothData.last_target
	end
	
	smoothData.position = script.Parent.Position
	render.Position = smoothingModule.SmoothPosition(render.Position, targetPosition, smoothData, deltaTime)
	if targetRotation ~= nil then
		render.Rotation = targetRotation
	end
end)
```


---

## `commands/server_authority_audit.md`

---
name: server_authority_audit
description: Audit all scripts in the game for Server Authority compliance and produce a report of core gameplay mechanics and their issues.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first.

2. Read the rules before auditing: `api_reference/3_core_rules.md` and `api_reference/4_anti_patterns.md` (especially the Violation → Fix Quick Reference table).

3. Find Core Gameplay Mechanics.
Examine all scripts for the core gameplay mechanics. Examples of core gameplay mechanics are operations on the Humanoid and HumanoidRootPart, physics operations, point scoring, health, etc.

Search for these terms:
- Writing to LocalPlayer, Humanoid, Character, HumanoidRootPart
- BasePart APIs: AssemblyLinearVelocity, ApplyImpulse, Touched, and others
- RemoteEvent APIs: FireServer, FireClient, FireAllClients
- Legacy input: UserInputService, ContextActionService
- Update signals: Heartbeat, RenderStepped, Stepped
- Timing: os.time(), tick()

4. Examine Each Mechanic Against This Checklist.
Work through both groups for every mechanic.

**Group 1 — Mechanical checks.** These are decidable from the script text; decide them yourself and report them with high confidence.
- [ ] No legacy input: no `UserInputService` / `ContextActionService` drives gameplay (input comes from an `InputAction` under the Player instead).
- [ ] No non-simulation update loops: gameplay is not updated in `Heartbeat` / `RenderStepped` / `Stepped`; it uses `RunService:BindToSimulation()`, `BasePart.Touched`, or `InputAction.StateChanged`.
- [ ] Timing uses `time()`, not `os.time()` / `tick()`.
- [ ] Any `RunService:SetPredictionMode()` call is wrapped in `task.defer()`.
- [ ] Gameplay logic lives in a ModuleScript run on BOTH client and server, not in a LocalScript.

**Group 2 — Judgment checks.** You often CANNOT decide these from the script alone. Do NOT assert pass/fail — mark them "Needs review" and state what a human must confirm.
- [ ] All dynamic state that affects the simulation is read from and written back to an Attribute on a predicted instance (no raw Luau variable holds gameplay state). Requires judging which data is dynamic and gameplay-critical.
- [ ] The instances it touches are actually predicted. Automatic prediction is runtime/proximity-based and cannot be confirmed from source — treat this as a candidate finding and recommend confirming at runtime with `RunService:GetPredictionStatus()`.
- [ ] It skips instances where `RunService:GetPredictionStatus(instance) == Enum.PredictionStatus.None`.
- [ ] It only reads/writes SimulationAccess properties and Attributes inside the simulation.

5. Produce the report as a table with one row per mechanic:

| Mechanic | Script (path) | Script type | Rule/anti-pattern violated | Symptom | Suggested fix | Confidence |
|---|---|---|---|---|---|---|

- Script type is one of: ModuleScript, Client Script, Server Script, LocalScript.
- Confidence is **High** for findings from a Group 1 (mechanical) check, and **Needs review** for findings that depend on a Group 2 (judgment) check.
- If a mechanic passes every check, list it with "Compliant" in the violation column.
- Use the Violation → Fix Quick Reference table in `api_reference/4_anti_patterns.md` to fill in the fix column.


---

## `commands/server_authority_convert.md`

---
name: server_authority_convert
description: Refactor core gameplay scripts to be Server Authority compliant so they work with client side prediction.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first.

2. Read the rules before converting: `api_reference/3_core_rules.md` and `api_reference/4_anti_patterns.md` (especially the Violation → Fix Quick Reference table).

3. Audit the game first. Run the same steps as `server_authority_audit`: find the core gameplay mechanics and check each one against the compliance checklist, producing the report table of mechanics and their violations.

4. Present the report to the user and ask them to approve the set of changes before editing any scripts. Do not edit scripts until the user approves.

5. Apply the refactors ONE MECHANIC AT A TIME. Do not batch edits across mechanics — finish and verify one before starting the next. For each approved mechanic, in order:
   1. Apply the refactor using the fixes from `api_reference/4_anti_patterns.md`. Common conversions:
      - Move gameplay logic out of LocalScripts into a ModuleScript required by a Client script (RunContext = Client) and a Server script (RunContext = Server). See `code_samples/1_SimulationTemplate.luau`.
      - Replace `Heartbeat` / `RenderStepped` / `Stepped` update loops with `RunService:BindToSimulation()`.
      - Replace `UserInputService` / `ContextActionService` with `InputAction` reads under the Player.
      - Move dynamic gameplay state out of raw Luau variables into Attributes on predicted instances.
      - Guard simulation code so it only runs when `RunService:GetPredictionStatus(instance) ~= Enum.PredictionStatus.None`.
      - Wrap any `RunService:SetPredictionMode()` call in `task.defer()`.
   2. Re-run the audit checklist against the changed script and confirm the violation is resolved and nothing new was introduced.
   3. If the correct fix is ambiguous, or depends on a Group 2 judgment check (for example whether an instance is actually predicted), STOP and ask the user before continuing. Do not guess.

6. When all approved mechanics are done, report a summary: what is now compliant, what still needs the user's attention, and any mechanics you skipped and why.


---

## `commands/server_authority_create.md`

---
name: server_authority_create
description: Create a new gameplay mechanic that is Server Authority compliant and works with client side prediction.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first

2. Ask the user for a game mechanic idea if they havent already provided one.

3. Create the game mechanic(s) using the concepts and rules in this skill.


---

## `commands/server_authority_explain.md`

---
name: server_authority_explain
description: Answer questions about Server Authority using this skill's concepts, code samples, and API reference.
user_invocable: true
---

## What to do

1. Connect to Studio — if multiple instances are open, call `list_roblox_studios` and `set_active_studio` first

2. Ask the user for a question or answer their existing question using the server authority concepts, code samples and API reference.

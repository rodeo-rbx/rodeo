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

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

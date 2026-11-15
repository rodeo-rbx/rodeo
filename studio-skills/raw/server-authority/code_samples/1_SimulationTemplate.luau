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

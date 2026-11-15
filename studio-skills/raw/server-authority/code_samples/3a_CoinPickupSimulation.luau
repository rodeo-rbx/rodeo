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

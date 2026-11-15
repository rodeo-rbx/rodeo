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

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

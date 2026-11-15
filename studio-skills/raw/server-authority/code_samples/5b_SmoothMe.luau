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

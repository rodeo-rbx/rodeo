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

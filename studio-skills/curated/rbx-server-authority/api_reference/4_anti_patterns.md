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

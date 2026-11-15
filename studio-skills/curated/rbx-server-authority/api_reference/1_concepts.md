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
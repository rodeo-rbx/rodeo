```lua
RunService:BindToSimulation(
function:function, frequency:Enum.StepFrequency, priority:number
):RBXScriptConnection
```
- summary:
	Binds a custom function to be called at a fixed frequency which is independent of the frame rate.
- code_samples:
	- RunService-BindToSimulation
- parameters:
	- `function`: The function to call. This function will be passed one parameter called `deltaTime` which shows how much time passed between the beginning of the previous simulation step and the beginning of the current simulation step.
	- `frequency`: Optional `Enum.StepFrequency` value indicating the frequency at which to call the bound function. If not provided, the default frequency will be used.
	- `priority`: Optional priority of the binding as an integer; it determines the order in which bound functions are called within a simulation step. Lower numbers are called first. If two bindings have the same priority, the order between them is unspecified. Defaults to 2000.
- returns:
	- An `Datatype.RBXScriptConnection` that can be disconnected to unbind the function from the simulation step.
- description:
`BindToSimulation()` binds a custom function to be called at a fixed frequency which is independent of the frame rate. In the server authority model, bound functions will also get called when the client needs to resimulate. This method is only available when `Class.Workspace.UseFixedSimulation` is enabled.

Note that since the bound function is meant for physics controllers and prediction, an error will trigger if you read or write from unsynchronized properties or call an unsynchronized method. This error is meant to help guide you in writing a pure, synchronized simulation.

To synchronize inaccessible properties, you should store the relevant data in [attributes](../../../studio/properties.md#instance-attributes) and update the relevant property in `Class.RunService.PostSimulation|PostSimulation` or `Class.RunService.RenderStepped|RenderStepped` by reading from attributes. This will leverage attribute synchronization to keep the property fully synchronized through the rollback mechanism.

```lua
RunService:BindToAnimation(
function:function, frequency:Enum.StepFrequency, priority:number
):RBXScriptConnection
```
- summary:
	Binds a custom function to be called at a fixed frequency before Animators update when fixed simulation is enabled, or immediately before `BindToSimulation` callbacks otherwise.
- parameters:
	- `function`: The function to call. This function will be passed one parameter called `deltaTime`, which shows the elapsed time since the callback's previous invocation.
	- `frequency`: Optional `Enum.StepFrequency` value indicating the frequency at which to call the bound function. Defaults to `Enum.StepFrequency.Hz30`.
	- `priority`: Optional priority of the binding as an integer; it determines the order in which bound functions are called within a simulation step. Lower numbers are called first. If two bindings have the same priority, the order between them is unspecified. Defaults to 2000.
- returns:
	- A `Datatype.RBXScriptConnection` that can be disconnected to unregister the callback.
    `BindToAnimation()` binds a custom function to be called at a fixed frequency which is independent of the frame rate. When `Class.Workspace.UseFixedSimulation` is enabled, these functions are invoked before Animators update, making this the fixed-frequency counterpart to `Class.RunService.PreAnimation|PreAnimation`. When fixed simulation is disabled, they instead run immediately before `Class.RunService:BindToSimulation()|BindToSimulation()` callbacks, so no pre-animation ordering should be assumed. In the server authority model, bound functions will also get called when the client needs to resimulate.

- description:
Like `Class.RunService:BindToSimulation()|BindToSimulation()`, the bound function is meant for physics controllers and prediction, so an error will trigger if you read or write from unsynchronized properties or call an unsynchronized method. This error is meant to help guide you in writing a pure, synchronized simulation.

To synchronize inaccessible properties, you should store the relevant data in [attributes](../../../studio/properties.md#instance-attributes) and update the relevant property in `Class.RunService.PostSimulation|PostSimulation` or `Class.RunService.RenderStepped|RenderStepped` by reading from attributes. This will leverage attribute synchronization to keep the property fully synchronized through the rollback mechanism.

```lua
RunService:GetPredictionStatus(context:Instance):Enum.PredictionStatus
```
- summary:
  	Checks the `Enum.PredictionStatus` of a specific context instance, useful for debugging scripts affecting multiple instances where some might be predicted and others might not.
- parameters:
	- `context`: The `Class.Instance` for which to check prediction status.
- returns:
	- The `Enum.PredictionStatus` of the given instance, indicating whether it is predicted, authoritative, or not participating in prediction.
- description:
Clients can use this method to check the `Enum.PredictionStatus` of a specific context instance, useful for debugging scripts affecting multiple instances (vehicle controllers, custom physics, etc.) where some might be predicted and others might not. This method is also useful for debugging and observing the effects of `Enum.PredictionMode.Automatic` prediction mode.

```lua
RunService:SetPredictionMode(
context:Instance, mode:Enum.PredictionMode
)
```
- summary:
	Sets the prediction mode for an `Class.Instance` to an `Enum.PredictionMode` value.
- parameters:
	- `context`: The `Class.Instance` for which to set the prediction mode.
	- `mode`: The `Enum.PredictionMode` to set for the context instance.
- description:
Sets the prediction mode for an `Class.Instance` (`context`) to an `Enum.PredictionMode` value. Mismatches between the physics properties or attributes for predicted instances will cause a rollback and resimulation on the client. Only has effect when called on the client.

```lua
RunService:IsResimulating():boolean
```
- summary:
	Returns whether the client is currently in a resimulation step after a
misprediction in the server authority model.
- returns:
	- Whether the engine is currently resimulating.
- description:
Returns `true` if the engine is currently replaying simulation steps after
detecting a misprediction in the server authority model; returns `false`
otherwise. When the server's authoritative state diverges from the client's
predicted state, the client rolls back and rapidly resimulates all affected
steps within a single frame. During that resimulation pass,
`Class.RunService:BindToSimulation()|BindToSimulation()` callbacks and
property-changed signals fire again for each replayed step, which can cause
unwanted side effects such as duplicate sound playback or redundant network
requests.

```lua
time()
```
- summary:
	Returns the amount of time in seconds that has elapsed since the current
game instance started running.
- returns:
	- number: The elapsed time, in seconds, since the current game instance started running.
- description:
Returns the amount of time, in seconds, that has elapsed since the current
game instance started running. If the current game instance is not
running, this will be `0`.

If `Class.Workspace.AuthorityMode` is `Enum.AuthorityMode.Server`, this
value is synchronized between client and server.

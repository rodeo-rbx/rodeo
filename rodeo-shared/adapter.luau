-- The run's line to its runtime bridges (`--adapter`): sends a bridge message
-- (rodeo-adapters/_bridge/codec.luau) for Lune or Lute to the run client,
-- which relays it to that runtime's process and returns the reply.
-- Factory: (rpc, runtime) -> adapter. `runtime` is the caller's proto module.

return function(rpc, runtime)
	local adapter = {}

	function adapter.call(runtimeName: string, payload: buffer): buffer
		local response: runtime.AdapterCallResponse = rpc.call(
			"adapter_call",
			runtime.AdapterCallRequest.new({ runtime = runtimeName, payload = payload })
		)
		return response.payload or buffer.create(0)
	end

	return adapter
end

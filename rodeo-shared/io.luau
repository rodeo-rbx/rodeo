-- @rodeo/io API over typed RPC.
-- Factory: (rpc, runtime) -> io. `runtime` is the caller's proto module,
-- passed in rather than read from _G: several rodeo plugins can share one VM.
return function(rpc, runtime)
	local io = {}

	io.stdin = { __handle = "stdin" }
	io.stdout = { __handle = "stdout" }
	io.stderr = { __handle = "stderr" }

	function io.read(): string
		local resp: runtime.StreamReadLineResponse = rpc.call("stream_read_line",
			runtime.StreamReadLineRequest.new({ handle = "stdin" }))
		return resp.data
	end

	return io
end

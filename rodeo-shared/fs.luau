-- @rodeo/fs API over typed RPC.
-- Factory: (rpc, runtime) -> fs. `runtime` is the caller's proto module,
-- passed in rather than read from _G: several rodeo plugins can share one VM.
return function(rpc, runtime)
	local fs = {}

	local nextFileId = 0

	function fs.open(path: string, mode: string?)
		nextFileId += 1
		local handle = `file:{nextFileId}:{path}`
		rpc.call("stream_open", runtime.StreamOpenRequest.new({
			handle = handle,
			path = path,
			mode = mode or "r",
		}))
		return { __handle = handle }
	end

	function fs.remove(path: string)
		rpc.call("fs_remove", runtime.FsRemoveRequest.new({ path = path }))
	end

	function fs.stat(path: string): runtime.FsStatResponse
		return rpc.call("fs_stat", runtime.FsStatRequest.new({ path = path }))
	end

	function fs.type(path: string): string
		local resp: runtime.FsTypeResponse = rpc.call("fs_type", runtime.FsTypeRequest.new({ path = path }))
		return resp.type or "unknown"
	end

	function fs.mkdir(path: string)
		rpc.call("fs_mkdir", runtime.FsMkdirRequest.new({ path = path }))
	end

	function fs.exists(path: string): boolean
		local resp: runtime.FsExistsResponse = rpc.call("fs_exists", runtime.FsExistsRequest.new({ path = path }))
		return if resp.exists ~= nil then resp.exists else false
	end

	function fs.copy(src: string, dest: string)
		rpc.call("fs_copy", runtime.FsCopyRequest.new({ src = src, dest = dest }))
	end

	function fs.listdir(path: string)
		local resp: runtime.FsListdirResponse = rpc.call("fs_listdir", runtime.FsListdirRequest.new({ path = path }))
		return resp.entries
	end

	function fs.rmdir(path: string)
		rpc.call("fs_rmdir", runtime.FsRmdirRequest.new({ path = path }))
	end

	return fs
end

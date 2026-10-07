-- Stream read/write/close primitives for handles returned by `fs.open`,
-- `io` stdio handles, and `process.create` stdio.
-- Bundled require: `local stream = require("@rodeo/stream")`.
--
-- NOTE: typedef-only. The runtime lives in rodeo-pkg/rodeo/shims/stream.luau.

local singleton = require("./_internal/singleton")

--- Opaque stream identifier. Pass these to `stream.read`/`stream.write`/
--- `stream.close`; you don't access `__handle` directly.
export type StreamHandle = {
	__handle: string,
}

local stream = {}

--- Reads from `handle`. Returns the next chunk as a string, or `nil` on EOF.
--- For a process's stdout or stderr, a read returns as soon as any output is available, up to `size` bytes (default 4096).
function stream.read(handle: StreamHandle, size: number?): string?
	return (singleton :: any).stream.read(handle, size)
end

--- Writes `data` to `handle`. `data` is converted to a string via `tostring`.
function stream.write(handle: StreamHandle, data: any)
	(singleton :: any).stream.write(handle, data)
end

--- Reads all remaining bytes from `handle` as a `buffer`. Use for binary
--- data; for text use `read`.
function stream.readBytes(handle: StreamHandle): buffer
	return (singleton :: any).stream.readBytes(handle)
end

--- Writes the bytes in `data` to `handle`. Use for binary data; for text
--- use `write`.
function stream.writeBytes(handle: StreamHandle, data: buffer)
	(singleton :: any).stream.writeBytes(handle, data)
end

--- Closes `handle`. Subsequent reads/writes error.
function stream.close(handle: StreamHandle)
	(singleton :: any).stream.close(handle)
end

return stream

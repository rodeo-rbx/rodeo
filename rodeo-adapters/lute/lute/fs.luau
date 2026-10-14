-- @lute/fs: Lute's own, from the Lute process behind this run (`--adapter lute`).
require("@lute/time") -- Lute's durations arrive as @lute/time's
local bridge = require("@rodeo-bridge")

local lute = bridge.module("lute", "@lute/fs")
local fs = table.clone(lute)

-- Arguments reach Lute as copies, so the buffer Lute filled comes back and
-- is copied into the caller's.
function fs.readIntoBuffer(file, buf: buffer, fileOffset: number, numBytes: number, bufferOffset: number?): number
	local filled, read = bridge.callOut("lute", lute.readIntoBuffer, 2, file, buf, fileOffset, numBytes, bufferOffset)
	buffer.copy(buf, 0, filled)
	return read
end

return table.freeze(fs)

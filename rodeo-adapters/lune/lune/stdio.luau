-- @lune/stdio: Lune's own, from the Lune process behind this run
-- (`--adapter lune`), except reading and writing this run's stdin, stdout and
-- stderr, which stay in order with print.
local bridge = require("@rodeo-bridge")
local rodeo = bridge.apis()

local function writer(handle)
	return function(s: string | buffer)
		if type(s) == "buffer" then
			rodeo.stream.writeBytes(handle, s)
		else
			rodeo.stream.write(handle, s)
		end
	end
end

local stdio = table.clone(bridge.module("lune", "@lune/stdio"))
stdio.write = writer(rodeo.io.stdout)
stdio.ewrite = writer(rodeo.io.stderr)

function stdio.readLine(): string
	return (string.gsub(rodeo.io.read(), "\r?\n$", ""))
end

function stdio.readToEnd(): string
	return buffer.tostring(rodeo.stream.readBytes(rodeo.io.stdin))
end

return table.freeze(stdio)

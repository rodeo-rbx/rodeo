-- @lute/process: Lute's own, from the Lute process behind this run
-- (`--adapter lute`), with this run's args and exit. `env` holds Lute's
-- environment; a write to it is made there before the script's next call to
-- Lute, so the children Lute starts see it.
require("@lute/time") -- Lute's durations arrive as @lute/time's
local bridge = require("@rodeo-bridge")
local rodeo = bridge.apis()

local lute = bridge.module("lute", "@lute/process")
local process = table.clone(lute)
process.args = rodeo.process.args
process.env = bridge.mirror("lute", lute.env)
process.exit = rodeo.process.exit

return table.freeze(process)

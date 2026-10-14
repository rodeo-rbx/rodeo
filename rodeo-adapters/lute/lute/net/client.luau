-- @lute/net/client: Lute's own, from the Lute process behind this run (`--adapter lute`).
require("@lute/time") -- Lute's durations arrive as @lute/time's
return require("@rodeo-bridge").module("lute", "@lute/net/client")

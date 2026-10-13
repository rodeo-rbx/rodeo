-- @lute/io shim — maps to rodeo's io/stream.
local g = _G.__rodeo_procs[_G.__rodeo_eid]
return {
	write = function(...: string)
		local parts = table.pack(...)
		for i = 1, parts.n do
			g.stream.write(g.io.stdout, parts[i])
		end
	end,
	read = function(): string
		return g.io.read()
	end,
}

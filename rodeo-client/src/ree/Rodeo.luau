-- rodeo's harness for ree (roblox-engine-emulator): `rodeo run --ree` runs `ree run --harness` on this file.
-- It mounts the rodeo plugin's modules (the same built plugin a Studio loads), takes them and itself out of the
-- DataModel again so the run's script sees the place as a Studio run would, and hands over to the plugin's ree entry
-- (rodeo-plugin/src/library/ree.luau), which runs the script and answers its @rodeo/* calls through Context.HostCall.
return function(Context)
	local root = Context.Mount("ReplicatedStorage/RodeoPlugin", Context.RodeoPlugin)
	local entry = require(root.plugin.library.ree)
	root.Parent = nil
	local fixtures = script.Parent
	if fixtures and fixtures.Name == "RURFixtures" and #fixtures:GetChildren() == 1 then
		fixtures.Parent = nil
	end
	Context.RodeoPlugin = nil
	entry(Context)
end

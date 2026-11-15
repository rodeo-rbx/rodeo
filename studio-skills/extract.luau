--!strict
-- Extracts the Roblox Studio Assistant's built-in skills out of the shipped
-- Assistant plugin (Assistant.rbxm) into raw/ as a tree of plain .md files.
-- raw/ is the pristine reference; build.luau makes curated/ from it (see README).
--
-- The skills are stored as StringValues under
--   Assistant/Packages/_Index/AssistantSkills/AssistantSkills/<skill>/...
-- with this shape per skill:
--   SKILL            main skill doc (frontmatter + body)
--   SKILL-combined   convenience: SKILL + every command/spec concatenated
--   QuerySpecs/API_spec   reference data shapes (optional)
--   commands/<name>  sub-skill command docs (optional)
-- A skill's code samples are ModuleScripts instead, written out as .luau files.
--
-- Usage (run from this directory):
--   lune run extract.luau                 -- autodetect Studio, write ./raw
--   lune run extract.luau <rbxm> <outdir>
--
-- No network, no Studio running required: reads the local install's bytes.
-- Re-run after a Studio update to refresh the reference, then run build.luau.

local fs = require("@lune/fs")
local process = require("@lune/process")
local roblox = require("@lune/roblox")

local CANDIDATES = {
	"/Applications/RobloxStudio.app/Contents/Resources/BuiltInStandalonePlugins/Packed/Assistant.rbxm",
	"/Applications/RobloxStudio.app/Contents/Resources/BuiltInStandalonePlugins/Optimized_Embedded_Signature/Assistant.rbxm",
}

local rbxmPath = process.args[1]
if not rbxmPath then
	for _, c in CANDIDATES do
		if fs.isFile(c) then
			rbxmPath = c
			break
		end
	end
end
assert(rbxmPath, "Assistant.rbxm not found in default locations; pass its path as arg 1")

local outDir = process.args[2] or "raw"

print("reading  " .. rbxmPath)
local model = roblox.deserializeModel(fs.readFile(rbxmPath))

-- Locate the inner registry folder: a Folder named "AssistantSkills" that
-- directly contains the known "rbx-debug" skill folder.
local registry: any = nil
local function findRegistry(inst: any)
	for _, c in inst:GetChildren() do
		if c.Name == "AssistantSkills" and c:FindFirstChild("rbx-debug") then
			registry = c
		end
		findRegistry(c)
	end
end
for _, root in model do
	findRegistry(root)
end
assert(registry, "AssistantSkills registry not found inside the rbxm")

if fs.isDir(outDir) then
	fs.removeDir(outDir)
end
fs.writeDir(outDir)

local fileCount = 0
local skillCount = 0

-- The code block under the "## `<path>`" heading of a SKILL-combined doc.
local function quotedIn(combined: string, path: string): string?
	local _, headingEnd = string.find(combined, "## `" .. path .. "`\n", 1, true)
	if not headingEnd then
		return nil
	end
	local fenceStart, fenceEnd = string.find(combined, "```%w*\n", headingEnd)
	local nextHeading = string.find(combined, "\n## `", headingEnd, true)
	if not fenceStart or not fenceEnd or (nextHeading and nextHeading < fenceStart) then
		return nil
	end
	local closing = string.find(combined, "\n```", fenceEnd, true)
	return if closing then string.sub(combined, fenceEnd + 1, closing) else nil
end

-- `rel` is the path inside the skill, and `combined` the skill's SKILL-combined text if it has one.
local function dump(inst: any, dir: string, rel: string, combined: string?)
	for _, c in inst:GetChildren() do
		if c.ClassName == "Folder" then
			local sub = dir .. "/" .. c.Name
			fs.writeDir(sub)
			dump(c, sub, rel .. c.Name .. "/", combined)
		elseif c.ClassName == "StringValue" then
			fs.writeFile(dir .. "/" .. c.Name .. ".md", c.Value)
			fileCount += 1
			print(string.format("  %-58s %7d chars", dir .. "/" .. c.Name .. ".md", #c.Value))
		elseif c.ClassName == "ModuleScript" then
			-- The plugin ships compiled, so a ModuleScript (a skill's code sample) has no
			-- source to read. SKILL-combined quotes each one under its path, minus the
			-- ordering prefix ("1_", "4c_") the instance name carries. Take the text from
			-- there and write it under the instance's name, which is the path the skill's
			-- other docs point to.
			local stem = string.gsub(c.Name, "^%d+%a?_", "")
			local source = combined
				and (quotedIn(combined, rel .. stem .. ".luau") or quotedIn(combined, rel .. c.Name .. ".luau"))
			local path = dir .. "/" .. c.Name .. ".luau"
			if source then
				fs.writeFile(path, source)
				fileCount += 1
				print(string.format("  %-58s %7d chars", path, #source))
			else
				print("  ! " .. path .. ": a ModuleScript with no source, and SKILL-combined doesn't quote it")
			end
		end
	end
end

for _, skillFolder in registry:GetChildren() do
	if skillFolder.ClassName == "Folder" then
		skillCount += 1
		local dir = outDir .. "/" .. skillFolder.Name
		fs.writeDir(dir)
		local combined = skillFolder:FindFirstChild("SKILL-combined")
		dump(skillFolder, dir, "", if combined then combined.Value else nil)
	end
end

print(string.format("\nextracted %d skills (%d files) -> %s/", skillCount, fileCount, outDir))

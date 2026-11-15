--!strict
-- Builds curated/ (what gets shipped) from raw/ (Roblox's text, written by extract.luau)
-- and overlays/ (rodeo's own notes). Each curated skill is its raw skill with:
--   - an rbx- prefix on its name, unless it has one already,
--   - overlays/_header.md placed above Roblox's text, then overlays/<name>.md if there is one,
--   - SKILL-combined.md left out, since it repeats the skill's other files.
-- curated/ is rebuilt from scratch on every run, so never edit it: change an overlay instead.
--
-- An overlay can open with a frontmatter block that replaces Roblox's description:
--   ---
--   description: One or two plain sentences, written out on this one line.
--   ---
-- Roblox writes its descriptions for the Assistant, and most are too long to keep their
-- place in an agent's skill listing.
--
-- An overlay can also place notes inside Roblox's text. A line of the form
--   <!-- after: <the start of a line in the raw SKILL.md> -->
-- puts the paragraphs that follow it after that line. The start has to match exactly one
-- line, so the build fails when a Studio update rewrites or removes the line.
--
-- Usage (run from this directory):
--   lune run build.luau

local fs = require("@lune/fs")
local process = require("@lune/process")
local stdio = require("@lune/stdio")

local RAW = "raw"
local OVERLAYS = "overlays"
local OUT = "curated"

-- Raw skills that curated/ leaves out, and why.
local SKIPPED = {
	["create-skill"] = "authors skills in the Assistant's own skill store",
	["perf-profiling-ref"] = "a stub with the same description as perf-profiling",
	["unit-test-testservice"] = "a variant of unit-test with a near-identical description",
}

-- The longest name the Agent Skills spec allows (agentskills.io/specification).
local MAX_NAME = 64
-- Claude Code drops descriptions from its skill listing once the listing outgrows its
-- budget, so the build points out any description longer than this.
local DESCRIPTION_TARGET = 200

-- Some skills name a tool as {ToolNames.X}, which the Assistant fills in as it loads the
-- skill. X is the title of a Studio MCP tool with the spaces taken out ("Grep Search" is
-- script_grep; STUDIO_MCP_TOOLS.md is generated from the same list).
local TOOL_NAMES = {
	ExecuteLuau = "`execute_luau`",
	FileSearch = "`script_search`",
	GameTree = "`search_game_tree`",
	GetConsoleOutput = "`get_console_output`",
	GrepSearch = "`script_grep`",
	InspectInstance = "`inspect_instance`",
	MultiEdit = "`multi_edit`",
	ReadFile = "`script_read`",
	ScreenCapture = "`screen_capture`",
	StartStopPlay = "`start_stop_play`",
	-- The Assistant's own tool for asking the user something. The Studio MCP has none.
	QuestionAnswer = "your ask-the-user tool",
}

local ANCHOR = "^<!%-%- after: (.-) %-%->$"

type Insert = { anchor: string, block: { string } }
type Overlay = { description: string?, top: { string }, inserts: { Insert } }
type File = { path: string, content: string }

local function fail(message: string): never
	stdio.ewrite("build.luau: " .. message .. "\n")
	return process.exit(1)
end

-- `text` with its {ToolNames.X} placeholders filled in.
local function withToolNames(text: string, path: string): string
	-- fail() can't run inside the gsub callback: exiting yields, and a callback can't.
	local unknown: string? = nil
	local filled = string.gsub(text, "{ToolNames%.(%w+)}", function(tool)
		if not TOOL_NAMES[tool] then
			unknown = unknown or tool
		end
		return TOOL_NAMES[tool]
	end)
	if unknown then
		return fail(path .. " names {ToolNames." .. unknown .. "}, which TOOL_NAMES doesn't cover")
	end
	return filled
end

local function sortedDir(path: string): { string }
	local names = fs.readDir(path)
	table.sort(names)
	return names
end

-- Drops the blank lines at both ends.
local function trimmed(lines: { string }): { string }
	local first, last = 1, #lines
	while first <= last and string.match(lines[first], "^%s*$") do
		first += 1
	end
	while last >= first and string.match(lines[last], "^%s*$") do
		last -= 1
	end
	return table.move(lines, first, last, 1, {})
end

local function readOverlay(path: string): Overlay
	local overlay: Overlay = { top = {}, inserts = {} }
	local lines = string.split(fs.readFile(path), "\n")
	local first = 1
	if lines[1] == "---" then
		local close = table.find(lines, "---", 2)
		if not close then
			return fail(path .. " opens a frontmatter block and never closes it")
		end
		for index = 2, close - 1 do
			local description = string.match(lines[index], "^description:%s*(.-)%s*$")
			if description and description ~= "" then
				overlay.description = description
			elseif string.match(lines[index], "%S") then
				return fail(path .. ': its frontmatter takes one "description: ..." line, not "' .. lines[index] .. '"')
			end
		end
		first = close + 1
	end

	local into = overlay.top
	for index = first, #lines do
		local line = lines[index]
		local anchor = string.match(line, ANCHOR)
		if anchor then
			local insert: Insert = { anchor = anchor, block = {} }
			table.insert(overlay.inserts, insert)
			into = insert.block
		else
			table.insert(into, line)
		end
	end
	overlay.top = trimmed(overlay.top)
	for _, insert in overlay.inserts do
		insert.block = trimmed(insert.block)
	end
	return overlay
end

-- A raw SKILL.md with the curated name, the header and the skill's overlay applied.
local function curatedDoc(raw: string, rawPath: string, name: string, header: string, overlay: Overlay?): string
	local _, frontmatterEnd = string.find(raw, "\n---\n", 1, true)
	if string.sub(raw, 1, 4) ~= "---\n" or not frontmatterEnd then
		return fail(rawPath .. " has no frontmatter")
	end
	local frontmatter, renames = string.gsub(string.sub(raw, 1, frontmatterEnd), "\nname:[^\n]*", function()
		return "\nname: " .. name
	end)
	if renames ~= 1 then
		return fail(rawPath .. " has " .. renames .. " name fields in its frontmatter, not one")
	end

	if overlay and overlay.description then
		if string.find(frontmatter, "\ndescription:[^\n]*\n[ \t]") then
			return fail(rawPath .. " spreads its description over several lines, which this build can't replace")
		end
		-- Quoted, so that punctuation in the text can't be read as YAML.
		local quoted = '"' .. string.gsub(string.gsub(overlay.description, "\\", "\\\\"), '"', '\\"') .. '"'
		local replaced
		frontmatter, replaced = string.gsub(frontmatter, "\ndescription:[^\n]*", function()
			return "\ndescription: " .. quoted
		end)
		if replaced ~= 1 then
			return fail(rawPath .. " has " .. replaced .. " description fields in its frontmatter, not one")
		end
	end

	local description = string.match(frontmatter, "\ndescription:%s*([^\n]*)") or ""
	description = string.match(description, '^"(.*)"$') or description
	if #name > MAX_NAME then
		print(string.format("  ! %s: the name is %d characters, over the %d allowed", name, #name, MAX_NAME))
	end
	if #description > DESCRIPTION_TARGET then
		print(
			string.format(
				"  ! %s: the description is %d characters, over the %d aimed for (shorten it in %s/%s.md)",
				name,
				#description,
				DESCRIPTION_TARGET,
				OVERLAYS,
				name
			)
		)
	end

	local body = trimmed(string.split(string.sub(raw, frontmatterEnd + 1), "\n"))
	local blockAfter: { [number]: { string } } = {}
	for _, insert in if overlay then overlay.inserts else {} do
		local matches = {}
		for index, line in body do
			if string.sub(line, 1, #insert.anchor) == insert.anchor then
				table.insert(matches, index)
			end
		end
		if #matches ~= 1 then
			return fail(
				string.format(
					'%s/%s.md: "%s" starts %d lines of %s, not exactly one',
					OVERLAYS,
					name,
					insert.anchor,
					#matches,
					rawPath
				)
			)
		end
		blockAfter[matches[1]] = insert.block
	end

	local out = { frontmatter, header, "" }
	if overlay and #overlay.top > 0 then
		table.insert(out, table.concat(overlay.top, "\n"))
		table.insert(out, "")
	end
	for index, line in body do
		table.insert(out, line)
		if blockAfter[index] then
			table.insert(out, "")
			table.insert(out, table.concat(blockAfter[index], "\n"))
		end
	end
	return table.concat(out, "\n") .. "\n"
end

-- Adds a raw skill's files to `files` as they go into curated/, with `doc` as its SKILL.md.
local function addSkillFiles(files: { File }, from: string, rel: string, doc: string)
	for _, entry in sortedDir(from) do
		local source = from .. "/" .. entry
		if fs.isDir(source) then
			addSkillFiles(files, source, rel .. entry .. "/", doc)
		elseif rel == "" and entry == "SKILL.md" then
			table.insert(files, { path = entry, content = doc })
		elseif not (rel == "" and entry == "SKILL-combined.md") and string.sub(entry, 1, 1) ~= "." then
			local content = fs.readFile(source)
			if string.sub(entry, -3) == ".md" then
				content = withToolNames(content, source)
			end
			table.insert(files, { path = rel .. entry, content = content })
		end
	end
end

local header = table.concat(trimmed(string.split(fs.readFile(OVERLAYS .. "/_header.md"), "\n")), "\n")

-- Every overlay has to belong to a skill this build writes, so one left behind by a
-- skill Roblox renamed or removed is an error, not a silently dropped note.
local unusedOverlays: { [string]: boolean } = {}
for _, entry in sortedDir(OVERLAYS) do
	if entry ~= "_header.md" and string.sub(entry, -3) == ".md" then
		unusedOverlays[string.sub(entry, 1, -4)] = true
	end
end

type Skill = { name: string, files: { File }, overlaid: boolean }

-- Work out every file before writing anything, so a failure leaves the last build alone.
local skills: { Skill } = {}
local skipped = 0
for _, rawName in sortedDir(RAW) do
	local rawDir = RAW .. "/" .. rawName
	if not fs.isDir(rawDir) then
		continue
	end
	if SKIPPED[rawName] then
		skipped += 1
		continue
	end

	local name = if string.sub(rawName, 1, 4) == "rbx-" then rawName else "rbx-" .. rawName
	local overlay = if unusedOverlays[name] then readOverlay(OVERLAYS .. "/" .. name .. ".md") else nil
	unusedOverlays[name] = nil

	local docPath = rawDir .. "/SKILL.md"
	local doc = curatedDoc(withToolNames(fs.readFile(docPath), docPath), docPath, name, header, overlay)
	local files: { File } = {}
	addSkillFiles(files, rawDir, "", doc)
	table.insert(skills, { name = name, files = files, overlaid = overlay ~= nil })
end

for name in unusedOverlays do
	fail(OVERLAYS .. "/" .. name .. ".md has no skill to go on: this build makes no " .. name)
end

if fs.isDir(OUT) then
	fs.removeDir(OUT)
end
fs.writeDir(OUT)

for _, skill in skills do
	for _, file in skill.files do
		local path = OUT .. "/" .. skill.name .. "/" .. file.path
		local dir = string.match(path, "^(.*)/") :: string
		if not fs.isDir(dir) then
			fs.writeDir(dir)
		end
		fs.writeFile(path, file.content)
	end
	local note = if skill.overlaid then "   + " .. OVERLAYS .. "/" .. skill.name .. ".md" else ""
	print(string.format("  %-34s %3d files%s", skill.name, #skill.files, note))
end

print(string.format("\nbuilt %d skills -> %s/ (left out %d)", #skills, OUT, skipped))

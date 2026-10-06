--[[
	require_resolver.luau - Require resolution and uncacheable module traversal

	Handles converting require() paths to actual instances, cloning module trees
	to bypass Roblox's require cache, and cleaning up after execution.
]]

local constants = require(script.Parent.constants)
local reloadPrelude = require(script.Parent.reload_prelude)
local utils = require(script.Parent.utils)

-- Noop logger for functions called without one
local noop = function() end
local noopLog = { debug = noop, info = noop, warn = noop, error = noop }

-- Find the topmost ModuleScript ancestor (root of the module tree)
local function findRootModule(module: ModuleScript): ModuleScript
	local ancestor = module:FindFirstAncestorOfClass("ModuleScript")
	if ancestor then
		return findRootModule(ancestor)
	end
	return module
end

-- Get absolute instance path from game using bracket notation
-- e.g. game["Workspace"]["Part"] or game["ReplicatedStorage"]["Modules"]["Utils"]
-- Returns nil if the instance is not a descendant of game
local function getAbsoluteInstancePath(instance: Instance): string?
	if instance == game then
		return "game"
	end

	local parts = {}
	local current = instance

	while current and current ~= game do
		table.insert(parts, 1, `["{current.Name}"]`)
		current = current.Parent
	end

	-- If we didn't reach game, the instance has no valid path
	if current ~= game then
		return nil
	end

	return "game" .. table.concat(parts, "")
end

-- Extract content from outermost parentheses, handling nested parens
-- Returns: content string and ending position, or nil if not found
local function extractParenthesisContent(str: string, startPos: number): (string?, number?)
	local openParenPos = string.find(str, "%(", startPos)
	if not openParenPos then
		return nil, nil
	end

	local depth = 0
	local pathStart = openParenPos + 1
	local pathEnd = nil

	for i = openParenPos, #str do
		local char = string.sub(str, i, i)
		if char == "(" then
			depth = depth + 1
		elseif char == ")" then
			depth = depth - 1
			if depth == 0 then
				pathEnd = i - 1
				return string.sub(str, pathStart, pathEnd), i
			end
		end
	end

	return nil, nil
end

-- Extract all require() calls from source code
local function extractRequires(source: string): { string }
	local requires = {}
	local pos = 1

	while true do
		-- Find next "require" keyword
		local requireStart = string.find(source, "require%s*%(", pos)
		if not requireStart then
			break
		end

		-- Check if this require is commented out (-- before require on same line)
		-- Find the start of the current line by searching backwards for newline
		local lineStart = 1
		for i = requireStart - 1, 1, -1 do
			if string.sub(source, i, i) == "\n" then
				lineStart = i + 1
				break
			end
		end
		local lineBeforeRequire = string.sub(source, lineStart, requireStart - 1)
		if string.find(lineBeforeRequire, "%-%-") then
			-- This require is in a comment, skip it
			pos = requireStart + 1
			continue
		end

		-- Extract the full require call with nested parentheses
		local content, endPos = extractParenthesisContent(source, requireStart)
		if content and endPos then
			-- Reconstruct the full require string
			local fullRequire = string.sub(source, requireStart, endPos)
			table.insert(requires, fullRequire)
			pos = endPos + 1
		else
			-- Skip this occurrence and continue
			pos = requireStart + 1
		end
	end

	return requires
end

-- Convert Roblox string path to instance
-- Handles @self, ./, ../
local function getRobloxStringPathInstance(rootInstance: Instance, pathString: string): ModuleScript?
	local current = rootInstance

	-- Remove quotes if present
	pathString = string.gsub(pathString, "^[\"']", "")
	pathString = string.gsub(pathString, "[\"']$", "")

	-- Handle @self prefix (means the script itself)
	if string.sub(pathString, 1, 5) == "@self" then
		current = rootInstance
		pathString = string.sub(pathString, 6) -- Remove "@self"
		if string.sub(pathString, 1, 1) == "/" then
			pathString = string.sub(pathString, 2) -- Remove leading /
		end
	else
		-- Require explicit ./ or ../ prefix (Luau spec compliance)
		-- ./ means script.Parent (sibling)
		-- ../ means go up one level (chainable)

		-- Start from script.Parent for relative paths
		current = rootInstance.Parent or rootInstance

		-- Must have ./ or ../ prefix
		if string.sub(pathString, 1, 2) == "./" then
			pathString = string.sub(pathString, 3) -- Remove "./"
		elseif string.sub(pathString, 1, 3) == "../" then
			-- Handle chained ../
			while string.sub(pathString, 1, 3) == "../" do
				current = current.Parent or current
				pathString = string.sub(pathString, 4) -- Remove "../"
			end
		else
			-- No valid prefix (./, ../, or @) - return nil per Luau spec
			return nil
		end
	end

	-- If path is now empty, return current
	if pathString == "" then
		if typeof(current) == "Instance" and current:IsA("ModuleScript") then
			return current
		end
		return nil
	end

	-- Split remaining path by / and navigate
	local parts = string.split(pathString, "/")
	for _, part in parts do
		if part ~= "" then
			local child = current:FindFirstChild(part)
			if not child then
				return nil
			end
			current = child
		end
	end

	-- Verify final result is a ModuleScript
	if typeof(current) == "Instance" and current:IsA("ModuleScript") then
		return current
	end

	return nil
end

-- Resolve a require path to an actual instance
-- Two-phase resolution: try loadstring first, then string path conversion
local function getRequireInstance(requireString: string, currentScript: Instance, log: any?): ModuleScript?
	local l = log or noopLog
	l.debug(`Resolving require: {requireString} from {currentScript:GetFullName()}`)

	-- Extract the path from require(path) using helper function
	local pathExpr, _ = extractParenthesisContent(requireString, 1)
	if not pathExpr then
		l.debug(`Failed to extract path from require string`)
		return nil
	end

	-- Strip quotes and whitespace from the path expression
	pathExpr = string.gsub(pathExpr, "^%s*[\"']?", "") -- Remove leading whitespace and quotes
	pathExpr = string.gsub(pathExpr, "[\"']?%s*$", "") -- Remove trailing quotes and whitespace

	-- Phase 1: Evaluate path using temp module (avoids loadstring/setfenv VM de-optimization)
	local success, result = pcall(function()
		local tempModule = Instance.new("ModuleScript")
		tempModule.Name = "__rodeo_resolver"
		-- Assign script to currentScript so child paths like script.SomeChild work.
		-- %q-escape the name: inline scripts are named after their own source, which
		-- can contain quotes/newlines that would otherwise break this string literal
		-- (e.g. a require written as game:GetService("ReplicatedStorage").Foo).
		tempModule.Source = 'local ok, r = pcall(function()\nscript = script.Parent and script.Parent[' .. string.format("%q", currentScript.Name) .. ']\nreturn ' .. pathExpr .. '\nend)\nreturn ok and r or nil'
		tempModule.Parent = currentScript.Parent
		local resolved = require(tempModule)
		tempModule:Destroy()
		return resolved
	end)

	-- If temp module succeeded and returned a ModuleScript, use it
	if success and typeof(result) == "Instance" and result:IsA("ModuleScript") then
		l.debug(`Phase 1 (temp module): Resolved to {result:GetFullName()}`)
		return result
	end

	-- Phase 2: If loadstring failed or didn't return an instance, try string path conversion
	-- This handles "@self/path", "./relative", etc.
	l.debug(`Phase 1 failed, trying Phase 2 (string conversion)`)
	local resolved = getRobloxStringPathInstance(currentScript, pathExpr)
	if resolved then
		l.debug(`Phase 2: Resolved to {resolved:GetFullName()}`)
	else
		l.debug(`Phase 2: Failed to resolve`)
	end
	return resolved
end

-- Source assignment rejects strings of 200,000 bytes and up (see
-- utils.updateSource); below that, plain assignment avoids a yield per module.
local MAX_ASSIGNED_SOURCE = 199999

local function describe(target: any): string
	if typeof(target) == "Instance" then
		return target:GetFullName()
	end
	return tostring(target)
end

export type ReloadSession = {
	-- Give the run's entry module (and any modules under it) the require
	-- wrapper. The entry is already the run's own instance.
	adopt: (module: ModuleScript) -> (),
	-- Swap copies in, ahead of time, for every require the module's source
	-- spells out in a form that evaluates on its own (a path from game or
	-- script, or a ./, ../, @self string), then for their requires in turn.
	traverse: (module: ModuleScript) -> (),
	-- The plugin end of the require wrapper: maps a require argument to the
	-- run's copy of its module, swapping one in on first use. `from` is the
	-- module calling require.
	resolve: (target: any, from: Instance?) -> any,
	-- Restore the originals' names and destroy the copies.
	cleanup: () -> (),
}

-- One --reload-requires run's set of fresh module copies. Each copied module
-- tree replaces its original under the original's name (the original is
-- renamed aside), so anything that finds a module by name or path during the
-- run, string requires included, reaches the copy. Every copy, and the entry,
-- also starts with the reload prelude, whose wrapper sends require arguments
-- the traversal couldn't evaluate ahead of time to `resolve` (issue #33).
local function createReloadSession(executionId: string?, log: any?): ReloadSession
	local l = log or noopLog

	-- Process-scoped suffix so concurrent processes don't collide
	local eid = if executionId and executionId ~= "" then executionId else ""
	local suffix = utils.makeIgnoreSuffix(eid)
	local suffixPattern = utils.makeIgnoreSuffixPattern(eid)

	-- Modules this run owns: the entry's tree and every copy swapped in.
	local owned: { [ModuleScript]: true } = {}
	-- Original module → the copy standing in for it this run.
	local copyOf: { [ModuleScript]: ModuleScript } = {}
	local clonedRoots: { ModuleScript } = {}
	local disabledInstances: { ModuleScript } = {}
	local visited: { [ModuleScript]: true } = {}
	local warned: { [string]: true } = {}
	local closed = false

	-- Prelude plus the two attributes it finds this run's mapper by (the same
	-- pair the @rodeo shims read on the entry).
	local function prepare(module: ModuleScript)
		owned[module] = true
		local source = module.Source
		local injected = reloadPrelude.inject(source)
		-- An empty Source may not mean an empty module (LinkedSource, or a
		-- DataModel that doesn't carry the text): writing the prelude there
		-- would replace the code. Such a copy is still fresh, just unwrapped.
		if source ~= "" and injected ~= source then
			if #injected <= MAX_ASSIGNED_SOURCE then
				module.Source = injected
			else
				utils.updateSource(module, injected)
			end
		end
		module:SetAttribute("rodeoPlugin", constants.PLUGIN_KEY)
		module:SetAttribute("rodeoExecutionId", eid)
	end

	local function prepareTree(root: ModuleScript)
		prepare(root)
		for _, descendant in root:GetDescendants() do
			if descendant:IsA("ModuleScript") then
				prepare(descendant)
			end
		end
	end

	-- Record which copy stands in for each original module under `original`,
	-- so a require that reaches an original (a reference held before the
	-- swap, or a child of a root copied for a sibling) maps to its copy.
	-- Clone keeps child order but leaves out non-Archivable children: pair by
	-- position, falling back to the first unpaired child of the same name and
	-- class.
	local function pairCopies(original: Instance, copy: Instance)
		if original:IsA("ModuleScript") and copy:IsA("ModuleScript") then
			copyOf[original] = copy
		end
		local copyChildren = copy:GetChildren()
		local paired: { [Instance]: true } = {}
		local function matches(child: Instance, candidate: Instance?): boolean
			return candidate ~= nil
				and not paired[candidate]
				and candidate.Name == child.Name
				and candidate.ClassName == child.ClassName
		end
		local position = 0
		for _, child in original:GetChildren() do
			if not child.Archivable then
				continue
			end
			position += 1
			local counterpart = copyChildren[position]
			if not matches(child, counterpart) then
				counterpart = nil
				for _, candidate in copyChildren do
					if matches(child, candidate) then
						counterpart = candidate
						break
					end
				end
			end
			if counterpart then
				paired[counterpart] = true
				pairCopies(child, counterpart)
			end
		end
	end

	-- Copy a root module tree (topmost ModuleScript) and swap the copy in
	-- under the original's name.
	local function swapRoot(moduleRoot: ModuleScript): ModuleScript
		local existing = copyOf[moduleRoot]
		if existing then
			return existing
		end

		local rootCopy = moduleRoot:Clone()
		if rootCopy == nil then
			error(`{moduleRoot:GetFullName()} has Archivable off, so it can't be copied`, 0)
		end
		-- Before parenting, so the copy enters the DataModel already wrapped.
		prepareTree(rootCopy)
		pairCopies(moduleRoot, rootCopy)
		table.insert(clonedRoots, rootCopy)
		l.debug(`Cloned root: {moduleRoot.Name}`)

		-- Swap: rename original root, insert clone as sibling
		local name = moduleRoot.Name
		moduleRoot.Name = name .. suffix
		rootCopy.Parent = moduleRoot.Parent
		l.debug(`Swapped: {name} <-> {moduleRoot.Name}`)

		table.insert(disabledInstances, moduleRoot)
		return rootCopy
	end

	local function traverse(currentModule: ModuleScript)
		if visited[currentModule] then
			return
		end
		visited[currentModule] = true

		l.debug(`Processing: {currentModule:GetFullName()}`)
		local source = currentModule.Source
		local requires = extractRequires(source)
		l.debug(`Found {#requires} require() calls`)

		for _, requirePath in requires do
			l.debug(`Resolving: {requirePath}`)
			local requireModule = getRequireInstance(requirePath, currentModule, l)

			if not requireModule then
				-- Typically a path through a local variable, which is nil when
				-- the argument is evaluated on its own. The require wrapper
				-- swaps that module in when the require runs.
				l.debug(`Failed to resolve ahead of time, left to the require wrapper`)
				continue
			end

			l.debug(`Resolved to: {requireModule:GetFullName()}`)

			-- If already cloned, still recurse to discover external deps
			if owned[requireModule] then
				l.debug(`Already in cloned subtree, recursing for external deps`)
				traverse(requireModule)
				continue
			end

			-- Find root ancestor of this module (topmost ModuleScript)
			local moduleRoot = findRootModule(requireModule)
			l.debug(`Root module: {moduleRoot:GetFullName()}`)

			local swapped, rootCopy = pcall(swapRoot, moduleRoot)
			if not swapped then
				l.warn(
					`--reload-requires couldn't reload {moduleRoot:GetFullName()}, so it returns the cached module: {rootCopy}`
				)
				continue
			end

			-- Recurse into cloned root
			traverse(rootCopy)
		end
	end

	local function resolveNow(target: any, from: Instance?): any
		if typeof(target) == "Instance" then
			if not target:IsA("ModuleScript") or owned[target] then
				return target
			end
			local copy = copyOf[target]
			if copy then
				return copy
			end
			swapRoot(findRootModule(target))
			l.debug(`Reloaded at require time: {target:GetFullName()}`)
			return copyOf[target] or target
		elseif type(target) == "string" and typeof(from) == "Instance" then
			-- The real require resolves the string relative to the calling
			-- module and finds the module by name, so swapping the copy in
			-- under that name is all it takes.
			local found = getRobloxStringPathInstance(from, target)
			if found then
				resolveNow(found, from)
			elseif string.sub(target, 1, 1) == "@" and string.sub(target, 1, 5) ~= "@self" and not warned[target] then
				-- An alias. (A ./, ../ or @self path that doesn't resolve
				-- names a missing module, which the real require reports.)
				warned[target] = true
				l.warn(
					`--reload-requires can't resolve require("{target}") in {from:GetFullName()}: `
						.. "only ./, ../ and @self paths are resolved, so it may return the cached module"
				)
			end
		end
		return target
	end

	local function resolve(target: any, from: Instance?): any
		-- The run is over and its copies are gone: a thread it left running
		-- gets the plain require.
		if closed then
			return target
		end
		local ok, result = pcall(resolveNow, target, from)
		if ok then
			return result
		end
		l.warn(`--reload-requires couldn't reload {describe(target)}, so it returns the cached module: {result}`)
		return target
	end

	local function cleanup()
		closed = true
		l.debug(`Cleanup START`)

		-- PHASE 1: Restore original names
		l.debug(`Restoring {#disabledInstances} original names`)
		for _, instance in disabledInstances do
			instance.Name = string.gsub(instance.Name, suffixPattern, "")
		end

		-- PHASE 2: Destroy only the root clones (children are destroyed automatically)
		l.debug(`Destroying {#clonedRoots} root clones`)
		for _, root in clonedRoots do
			l.debug(`  Destroying root: {root.Name}`)
			root:Destroy()
		end

		l.debug(`Cleanup END`)
	end

	return {
		adopt = prepareTree,
		traverse = traverse,
		resolve = resolve,
		cleanup = cleanup,
	}
end

return {
	getAbsoluteInstancePath = getAbsoluteInstancePath,
	createReloadSession = createReloadSession,
	extractRequires = extractRequires,
	extractParenthesisContent = extractParenthesisContent,
	getRobloxStringPathInstance = getRobloxStringPathInstance,
	getRequireInstance = getRequireInstance,
}

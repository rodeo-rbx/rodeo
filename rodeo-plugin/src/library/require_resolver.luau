--[[
	require_resolver.luau - Require resolution and uncacheable module traversal

	Handles converting require() paths to actual instances, cloning module trees
	to bypass Roblox's require cache, and cleaning up after execution.
]]

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

local function recurse(currentModule: ModuleScript, clonedModules: { [ModuleScript]: true }, clonedRoots: { ModuleScript }, disabledInstances, visited: { [ModuleScript]: true }, suffix: string, log: any)
	if visited[currentModule] then
		return
	end
	visited[currentModule] = true

	log.debug(`Processing: {currentModule:GetFullName()}`)
	local source = currentModule.Source
	local requires = extractRequires(source)
	log.debug(`Found {#requires} require() calls`)

	for _, requirePath in requires do
		log.debug(`Resolving: {requirePath}`)
		local requireModule = getRequireInstance(requirePath, currentModule, log)

		if not requireModule then
			log.debug(`Failed to resolve, skipping`)
			continue
		end

		log.debug(`Resolved to: {requireModule:GetFullName()}`)

		-- If already cloned, still recurse to discover external deps
		if clonedModules[requireModule] then
			log.debug(`Already in cloned subtree, recursing for external deps`)
			recurse(requireModule, clonedModules, clonedRoots, disabledInstances, visited, suffix, log)
			continue
		end

		-- Find root ancestor of this module (topmost ModuleScript)
		local moduleRoot = findRootModule(requireModule)
		log.debug(`Root module: {moduleRoot:GetFullName()}`)

		-- Clone entire root subtree
		local rootCopy = moduleRoot:Clone()
		table.insert(clonedRoots, rootCopy)
		clonedModules[rootCopy] = true
		log.debug(`Cloned root: {moduleRoot.Name}`)

		-- Track all descendant ModuleScripts for skip checking
		for _, descendant in rootCopy:GetDescendants() do
			if descendant:IsA("ModuleScript") then
				clonedModules[descendant] = true
			end
		end

		-- Swap: rename original root, insert clone as sibling
		local name = moduleRoot.Name
		moduleRoot.Name = name .. suffix
		rootCopy.Parent = moduleRoot.Parent
		log.debug(`Swapped: {name} <-> {moduleRoot.Name}`)

		table.insert(disabledInstances, moduleRoot)

		-- Recurse into cloned root
		recurse(rootCopy, clonedModules, clonedRoots, disabledInstances, visited, suffix, log)
	end
end

-- Setup uncachable requires by cloning and renaming modules
local function setupUncachableRequireModuleTraversal(rootInstance: ModuleScript, executionId: string?, log: any?): () -> ()
	local l = log or noopLog
	l.debug(`START for {rootInstance:GetFullName()}`)

	-- Process-scoped suffix so concurrent processes don't collide
	local eid = if executionId and executionId ~= "" then executionId else ""
	local suffix = utils.makeIgnoreSuffix(eid)
	local suffixPattern = utils.makeIgnoreSuffixPattern(eid)

	local clonedModules = {}
	local clonedRoots = {}
	local disabledInstances = {}
	recurse(rootInstance, clonedModules, clonedRoots, disabledInstances, {}, suffix, l)

	l.debug(`COMPLETE: {#clonedRoots} roots cloned, {#disabledInstances} originals renamed`)

	return function()
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
end

-- ── Changed modules under the require cache (issue #10) ─────────────────
--
-- A cached run reaches the live module, which the VM evaluated once from the
-- Source it had then. When that Source has changed since (a package patched
-- on disk, a file synced in), the run used to execute the old code with
-- nothing to say so. So the plugin keeps the Source each module had when a
-- run in this DOM first required it, and a run whose require graph reaches a
-- module whose Source differs gets a fresh copy of it, and of every module in
-- the graph that requires it (those would otherwise hand the run their cached
-- copy of the old one). Everything else stays live.
--
-- One plugin instance runs per DOM, so module-level state is per DOM. A
-- module the VM cached before this plugin first saw it (a play DOM's scripts
-- at startup, another plugin, or before this plugin reloaded) is baselined at
-- the Source it has at that first sight.
local cachedSources: { [ModuleScript]: string } = {}
-- The changed Source a notice was last printed for, so one edit prints once.
local noticedSources: { [ModuleScript]: string } = {}

-- Methods a require path may call on its way to the module. The graph is
-- read before the run starts, so none may yield: a WaitForChild on a child
-- that a script creates later would hang the run, so it reads as
-- FindFirstChild.
local PATH_METHODS: { [string]: (Instance, string, string) -> Instance? } = {
	GetService = function(instance, name)
		return if instance == game then game:GetService(name) else nil
	end,
	WaitForChild = function(instance, name)
		return instance:FindFirstChild(name)
	end,
	FindFirstChild = function(instance, name, rest)
		return instance:FindFirstChild(name, string.match(rest, "^,%s*true%s*$") ~= nil)
	end,
	FindFirstChildOfClass = function(instance, name)
		return instance:FindFirstChildOfClass(name)
	end,
	FindFirstChildWhichIsA = function(instance, name)
		return instance:FindFirstChildWhichIsA(name)
	end,
	FindFirstAncestor = function(instance, name)
		return instance:FindFirstAncestor(name)
	end,
	FindFirstAncestorOfClass = function(instance, name)
		return instance:FindFirstAncestorOfClass(name)
	end,
	FindFirstAncestorWhichIsA = function(instance, name)
		return instance:FindFirstAncestorWhichIsA(name)
	end,
}

-- Resolve a require argument without running any code: an instance path from
-- game, workspace, script or an alias in `scope`, through children, .Parent,
-- an ObjectValue's .Value and PATH_METHODS; or a ./, ../, @self string. nil
-- for anything else (a call result, a loop variable, a computed name).
local function resolvePath(expr: string, from: ModuleScript, scope: { [string]: Instance }): Instance?
	expr = string.match(expr, "^%s*(.-)%s*$") :: string
	expr = string.gsub(expr, "%s*::.*$", "") -- a type cast
	local _, text = string.match(expr, "^([\"'])(.*)%1$")
	if text then
		return getRobloxStringPathInstance(from, text)
	end

	local base, pos = string.match(expr, "^([%a_][%w_]*)()")
	local current: Instance?
	if base == "game" then
		current = game
	elseif base == "workspace" then
		current = workspace
	elseif base == "script" then
		current = from
	elseif base then
		current = scope[base]
	end

	while current and pos <= #expr do
		local name, after = string.match(expr, "^%s*%.%s*([%a_][%w_]*)()", pos)
		if name then
			if name == "Parent" then
				current = current.Parent
			else
				local child = current:FindFirstChild(name)
				if child == nil and name == "Value" and current:IsA("ObjectValue") then
					child = current.Value
				end
				current = child
			end
		else
			local key
			_, key, after = string.match(expr, "^%s*%[%s*([\"'])(.-)%1%s*%]()", pos)
			if key then
				current = current:FindFirstChild(key)
			else
				local method, arg, rest
				method, _, arg, rest, after =
					string.match(expr, "^%s*:%s*([%a_][%w_]*)%s*%(%s*([\"'])(.-)%2%s*([^)]*)%)()", pos)
				local call = method and PATH_METHODS[method]
				if not call then
					return nil
				end
				current = call(current, arg, rest)
			end
		end
		pos = after
	end
	return current
end

-- True when `at` sits after a `--` on its line.
local function inLineComment(source: string, at: number): boolean
	local lineStart = 1
	for i = at - 1, 1, -1 do
		if string.byte(source, i) == 10 then
			lineStart = i + 1
			break
		end
	end
	return string.find(string.sub(source, lineStart, at - 1), "--", 1, true) ~= nil
end

-- A module's `local NAME = ...` declarations and require(...) arguments, in
-- source order. Scanning is the costly part of reading the graph, so it is
-- kept per module until the Source changes.
local scans: { [ModuleScript]: { source: string, steps: { { at: number, alias: string?, expr: string } } } } = {}

local PATH_BASES = { game = true, workspace = true, script = true }

local function scanRequires(module: ModuleScript, source: string)
	local cached = scans[module]
	if cached and cached.source == source then
		return cached.steps
	end
	local found = {}
	for at, name, rest in string.gmatch(source, "()%f[%w_]local%s+([%a_][%w_]*)([^\n;]*)") do
		table.insert(found, { at = at, alias = name, rest = rest })
	end
	for at in string.gmatch(source, "()%f[%w_]require%s*%(") do
		table.insert(found, { at = at })
	end
	table.sort(found, function(a, b)
		return a.at < b.at
	end)

	-- Most locals aren't paths; keep only those built on game, workspace,
	-- script or an earlier path local, so later runs resolve just those.
	local steps = {}
	local pathNames: { [string]: true } = {}
	for _, item in found do
		if item.alias then
			-- `local NAME = expr` or `local NAME: Type = expr`, up to a `;` or the line's end
			local expr = string.match(item.rest, "^%s*=%s*(.*)$") or string.match(item.rest, "^%s*:[^=]*=%s*(.*)$")
			local base = expr and string.match(expr, "^([%a_][%w_]*)")
			if base and (PATH_BASES[base] or pathNames[base]) and not inLineComment(source, item.at) then
				pathNames[item.alias] = true
				table.insert(steps, { at = item.at, alias = item.alias, expr = string.gsub(expr, "%s*%-%-.*$", "") })
			elseif pathNames[item.alias] then
				-- Declared again as something else: it no longer names a path.
				pathNames[item.alias] = nil
				table.insert(steps, { at = item.at, alias = item.alias, expr = "" })
			end
		else
			local arg = extractParenthesisContent(source, item.at)
			if arg and not inLineComment(source, item.at) then
				table.insert(steps, { at = item.at, expr = arg })
			end
		end
	end
	scans[module] = { source = source, steps = steps }
	return steps
end

-- The modules `module`'s source requires, as far as reading it can tell.
-- Aliases count from their declaration on, so the common
-- `local RS = game:GetService("ReplicatedStorage")` then `require(RS.Foo)`
-- resolves. Requires through loop variables or computed names don't.
local function staticRequireTargets(module: ModuleScript, source: string): { ModuleScript }
	local scope: { [string]: Instance } = {}
	local targets = {}
	for _, step in scanRequires(module, source) do
		local ok, found = pcall(resolvePath, step.expr, module, scope)
		local instance = if ok then found else nil
		if step.alias then
			-- A path that doesn't resolve clears the name too.
			scope[step.alias] = instance
		elseif instance and instance:IsA("ModuleScript") then
			table.insert(targets, instance)
		end
	end
	return targets
end

-- Every module reachable from `entry` by requires its sources spell out:
-- each one's current Source, and the modules it requires.
local function collectRequireGraph(
	entry: ModuleScript
): ({ [ModuleScript]: string }, { [ModuleScript]: { ModuleScript } })
	local sources: { [ModuleScript]: string } = {}
	local edges: { [ModuleScript]: { ModuleScript } } = {}
	local pending = { entry }
	while #pending > 0 do
		local module = table.remove(pending) :: ModuleScript
		if sources[module] == nil then
			local source = module.Source
			sources[module] = source
			local targets = staticRequireTargets(module, source)
			edges[module] = targets
			for _, target in targets do
				if sources[target] == nil then
					table.insert(pending, target)
				end
			end
		end
	end
	return sources, edges
end

-- For a cached run: swap fresh copies in for the module trees in the run's
-- require graph whose Source changed since this DOM cached them, and for the
-- trees that require those. Returns a cleanup that puts the originals back,
-- or nil when nothing changed.
local function setupChangedModuleReload(entry: ModuleScript, executionId: string?, log: any?): (() -> ())?
	local l = log or noopLog
	local sources, edges = collectRequireGraph(entry)

	-- The entry and anything under it are this run's own fresh instances.
	local function ownedByRun(module: ModuleScript): boolean
		return module == entry or module:IsDescendantOf(entry)
	end

	local changed: { [ModuleScript]: string } = {}
	local reloadRoots: { [ModuleScript]: true } = {}
	local queue: { ModuleScript } = {}
	for module, source in sources do
		local cachedSource = cachedSources[module]
		if ownedByRun(module) or cachedSource == nil then
			continue
		end
		if cachedSource == source then
			noticedSources[module] = nil
			continue
		end
		changed[module] = source
		local root = findRootModule(module)
		if not reloadRoots[root] then
			reloadRoots[root] = true
			table.insert(queue, root)
		end
	end

	-- Trees are copied whole (a module's siblings and children resolve
	-- inside its tree), so dependents are tracked between tree roots.
	local requirers: { [ModuleScript]: { ModuleScript } } = {}
	for module, targets in edges do
		if ownedByRun(module) then
			continue
		end
		local fromRoot = findRootModule(module)
		for _, target in targets do
			local toRoot = findRootModule(target)
			if toRoot ~= fromRoot and not ownedByRun(target) then
				requirers[toRoot] = requirers[toRoot] or {}
				table.insert(requirers[toRoot], fromRoot)
			end
		end
	end
	while #queue > 0 do
		local root = table.remove(queue) :: ModuleScript
		for _, requirer in requirers[root] or {} do
			if not reloadRoots[requirer] then
				reloadRoots[requirer] = true
				table.insert(queue, requirer)
			end
		end
	end

	-- Everything else is required through the cache this run, which holds (or
	-- is about to hold) the Source it has now.
	for module, source in sources do
		if cachedSources[module] == nil and not ownedByRun(module) and not reloadRoots[findRootModule(module)] then
			cachedSources[module] = source
		end
	end

	if next(reloadRoots) == nil then
		return nil
	end

	local noticed = {}
	for module, source in changed do
		if noticedSources[module] ~= source then
			noticedSources[module] = source
			table.insert(noticed, module:GetFullName())
		end
	end
	if #noticed > 0 then
		table.sort(noticed)
		l.info(
			`{table.concat(noticed, ", ")} changed since first required here; `
				.. "this run loads fresh copies of changed modules and of the modules that require them"
		)
	end

	local eid = if executionId and executionId ~= "" then executionId else ""
	local suffix = utils.makeIgnoreSuffix(eid)
	local suffixPattern = utils.makeIgnoreSuffixPattern(eid)
	local originals: { ModuleScript } = {}
	local copies: { ModuleScript } = {}
	for root in reloadRoots do
		-- A tree holding the run's own entry stays put: copying it would
		-- copy the entry too.
		if entry:IsDescendantOf(root) then
			continue
		end
		local copy = root:Clone()
		if copy == nil then
			l.warn(`can't reload {root:GetFullName()}: Archivable is off, so the run gets the cached module`)
			continue
		end
		l.debug(`Reloading changed module tree: {root:GetFullName()}`)
		local name = root.Name
		root.Name = name .. suffix
		copy.Parent = root.Parent
		table.insert(originals, root)
		table.insert(copies, copy)
	end

	return function()
		for _, original in originals do
			original.Name = string.gsub(original.Name, suffixPattern, "")
		end
		for _, copy in copies do
			copy:Destroy()
		end
	end
end

return {
	getAbsoluteInstancePath = getAbsoluteInstancePath,
	setupUncachableRequireModuleTraversal = setupUncachableRequireModuleTraversal,
	setupChangedModuleReload = setupChangedModuleReload,
	extractRequires = extractRequires,
	extractParenthesisContent = extractParenthesisContent,
	getRobloxStringPathInstance = getRobloxStringPathInstance,
	getRequireInstance = getRequireInstance,
}

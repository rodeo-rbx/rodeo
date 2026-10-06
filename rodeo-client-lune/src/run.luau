-- Types for Dom.runCode — mirrors rodeo-client-ts/src/run.ts.

export type LogFilter = {
	enableWarn: boolean?,
	enableError: boolean?,
	enableInfo: boolean?,
	enableOutput: boolean?,
	enableLogs: boolean?,
}

-- Common run options (every tier). `context` is the run context the code
-- executes as (cf. Roblox Script.RunContext).
export type DomRunCodeOpts = {
	source: string?,
	file: string?,
	sourcemap: string?,
	context: ("plugin" | "server" | "client" | "elevated" | "cmdbar")?,
	showReturn: boolean?,
	reloadRequires: boolean?,
	verbose: boolean?,
	scriptArgs: { string }?,
	profile: string?,
	-- Write the script's return value to this host-side path. `.luau`/`.lua`
	-- emits Luau source (e.g. `return { pos = Vector3.new(1,2,3) }`); any
	-- other extension emits JSON-encoded tagged structs.
	returnFile: string?,
	logFilter: LogFilter?,
}

-- Routed tiers (client.runCode / studio.runCode) add routing fields.
-- `dom.runCode` uses DomRunCodeOpts (pinned — no routing).
export type RunCodeOpts = DomRunCodeOpts & {
	mode: ("edit" | "run" | "test" | "play")?,
	domKind: ("edit" | "server" | "client")?,
}

export type RunResult = {
	ok: boolean,
	output: string,
	exitCode: number,
	-- Master-minted run id, usable with `client.kill` / `listProcesses`.
	executionId: string?,
	-- JSON-parsed script return value. `nil` if the script returned nothing,
	-- if a return file captured the value instead, if the value exceeded the
	-- 2MiB wire cap with showReturn set (printed in full, omitted here;
	-- without showReturn an over-cap value fails the run), or if the return
	-- payload failed to parse (the latter is swallowed defensively). Access
	-- via bracket notation because `return` is a Luau reserved keyword:
	-- `result["return"]`.
	["return"]: any?,
	-- Why the run failed, when it did: the module's own error (with its
	-- trace), the runner's (a context the Studio can't provide, a run that
	-- failed to start, an oversized return), or the connection's. `nil` for
	-- a successful or killed run.
	error: string?,
}

return {}

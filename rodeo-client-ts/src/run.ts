//! Types only — runCode implementation now lives in `client.ts` as a thin
//! wrapper over the canonical-client daemon. These types are kept in this
//! file so consumers importing `./run.js` see no breakage.

export type LogFilter = {
  enableWarn?: boolean;
  enableError?: boolean;
  enableInfo?: boolean;
  enableOutput?: boolean;
  enableLogs?: boolean;
};

/** Routing fields — used by the serve-wide (`client.runCode`) and
 *  session-scoped (`studio.runCode`) tiers. `dom.runCode` omits them (a
 *  pinned DOM does no routing); only `context` applies there. */
export type RouteOpts = {
  /** Studio mode to converge to (auto-transitions). */
  mode?: "edit" | "run" | "test" | "play";
  /** Which DOM receives the script (usually inferred). `edit` targets the edit
   *  DOM even while a test/play session runs. */
  domKind?: "edit" | "server" | "client";
};

type CommonRunOpts = {
  source?: string;
  file?: string;
  sourcemap?: string;
  /** Run context the code executes as (cf. Roblox Script.RunContext). */
  context?: "plugin" | "server" | "client" | "elevated" | "cmdbar";
  showReturn?: boolean;
  reloadRequires?: boolean;
  verbose?: boolean;
  scriptArgs?: string[];
  profile?: string;
  /** Write the script's return value to this host-side path. `.luau`/`.lua`
   *  emits Luau source (e.g. `return { pos = Vector3.new(1,2,3) }`); any
   *  other extension emits JSON-encoded tagged structs. */
  returnFile?: string;
  logFilter?: LogFilter;
};

/** Options for the routed tiers (`client.runCode` / `studio.runCode`). */
export type RunCodeOpts = CommonRunOpts & RouteOpts;

/** Options for `dom.runCode` — a pinned DOM, so no routing fields. */
export type DomRunCodeOpts = CommonRunOpts;

export type RunResult = {
  ok: boolean;
  output: string;
  exitCode: number;
  /** Master-minted run id, usable with `client.kill` / `listProcesses`. */
  executionId?: string;
  /** JSON-parsed script return value. `undefined` if the script returned
   *  nothing, if a returnFile captured the value instead, if the value
   *  exceeded the 2MiB wire cap with showReturn set (printed in full,
   *  omitted here; without showReturn an over-cap value fails the run), or
   *  if the return payload failed to parse (the latter is swallowed
   *  defensively — never throws at the consumer). */
  return?: unknown;
  /** Why the run failed, when it did: the module's own error (with its
   *  trace), the runner's (a context the Studio can't provide, a run that
   *  failed to start, an oversized return), or the connection's.
   *  `undefined` for a successful or killed run. */
  error?: string;
};

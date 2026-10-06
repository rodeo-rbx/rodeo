use crate::util::config;
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, Subcommand, ValueEnum};

/// Routing flag enums. Each maps to its lowercase wire string via `as_str`;
/// `shared::target::RouteSpec` does the actual defaults + validation.
#[derive(Copy, Clone, Debug, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ModeArg { Edit, Run, Test, Play }

#[derive(Copy, Clone, Debug, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum DomKindArg { Edit, Server, Client }

#[derive(Copy, Clone, Debug, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ContextArg { Plugin, Server, Client, Elevated, Cmdbar }

impl ModeArg {
    pub fn as_str(self) -> &'static str {
        match self { Self::Edit => "edit", Self::Run => "run", Self::Test => "test", Self::Play => "play" }
    }
}
impl DomKindArg {
    pub fn as_str(self) -> &'static str {
        match self { Self::Edit => "edit", Self::Server => "server", Self::Client => "client" }
    }
}
impl ContextArg {
    pub fn as_str(self) -> &'static str {
        match self { Self::Plugin => "plugin", Self::Server => "server", Self::Client => "client", Self::Elevated => "elevated", Self::Cmdbar => "cmdbar" }
    }
}

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Red.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Yellow.on_default());

#[derive(Parser)]
#[command(name = "rodeo", about = "Command-line interface for Roblox Studio")]
#[command(version = rodeo_proto::BUILD_ID, styles = STYLES)]
pub struct Cli {
    /// Enable debug output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start persistent server (no Studio launch — use `run --place` for that)
    Serve {
        /// Master port. Resolution: this flag, then RODEO_PORT, then 44872.
        /// The studio backend listens on port + 1.
        #[arg(long, env = "RODEO_PORT")]
        port: Option<u16>,

        /// Run as master only (central orchestrator)
        #[arg(long, conflicts_with = "studio_mode")]
        master: bool,

        /// Run as studio backend only (connects to master)
        #[arg(long = "studio", conflicts_with = "master")]
        studio_mode: bool,

        /// Master host to connect to (for --studio)
        #[arg(long = "master-host", default_value = "localhost")]
        master_host: String,

        /// Master port to connect to (for --studio). Resolution: this flag,
        /// then RODEO_PORT, then 44872.
        #[arg(long = "master-port", env = "RODEO_PORT")]
        master_port: Option<u16>,

        /// Parent PID — exit when this process dies
        #[arg(long)]
        ppid: Option<u32>,
    },

    /// Run a script in Studio
    Run {
        /// Path to the script to execute, or '-' for stdin
        script: Option<String>,

        /// Execute source code passed as string. Hyphen-leading values are
        /// legal — Luau hotcomments (`--!native`, `--!optimize 2`) start
        /// scripts, and clap would otherwise reject them as flags.
        #[arg(short, long, allow_hyphen_values = true)]
        source: Option<String>,

        /// Path to sourcemap.json for instance resolution
        #[arg(long)]
        sourcemap: Option<String>,

        /// Path to file for execution output (prints/logs)
        #[arg(long)]
        output: Option<String>,

        /// Path to file for return value JSON
        #[arg(long = "return")]
        return_file: Option<String>,

        /// Print return value to stdout
        #[arg(long)]
        show_return: bool,

        /// Studio mode to run in (auto-transitions Studio). Defaults to edit;
        /// never inferred from --context/--dom, so a server/client run must pass
        /// --mode explicitly (e.g. --mode run --context server).
        #[arg(long, value_enum, help_heading = "Targeting")]
        mode: Option<ModeArg>,

        /// Which DOM receives the script: edit, server, or client (usually
        /// inferred). `edit` targets the edit DOM even while a session runs.
        #[arg(long = "dom", value_name = "DOM", value_enum, help_heading = "Targeting")]
        dom_kind: Option<DomKindArg>,

        /// Identity level the code executes at: plugin, server (server-runtime
        /// identity), client (client-runtime identity), elevated (command bar
        /// via StudioMCP), or cmdbar (command bar via the launch bootstrap's
        /// bridge — edit DOM of a rodeo-launched Studio, no StudioMCP needed).
        #[arg(long, value_enum, help_heading = "Targeting")]
        context: Option<ContextArg>,

        /// Scope routing to one studio by id (from `rodeo state`; unique prefix ok).
        #[arg(long = "studio-id", help_heading = "Targeting", conflicts_with = "place")]
        studio_id: Option<String>,

        /// Disable warning output
        #[arg(long)]
        no_warn: bool,

        /// Disable error output
        #[arg(long)]
        no_error: bool,

        /// Disable info output
        #[arg(long)]
        no_info: bool,

        /// Disable print statements
        #[arg(long)]
        no_print: bool,

        /// Disable all output
        #[arg(long)]
        no_output: bool,

        /// Re-evaluate instance requires instead of reusing the VM's require
        /// cache. By default a require resolves to the live module the game is
        /// already using, unless the module's source changed since it was
        /// first required; this gives the run its own fresh copies of all of
        /// them.
        #[arg(long)]
        reload_requires: bool,

        /// Removed in 1.3.0: caching is the default now. Present only to fail
        /// loudly — a silently-flipped default would mean runs quietly reading
        /// different module state than before.
        #[arg(long = "cache-requires", hide = true)]
        cache_requires_removed: bool,

        /// Script arguments (passed after --)
        #[arg(last = true)]
        script_args: Vec<String>,

        /// Parent PID — exit when this process dies
        #[arg(long)]
        ppid: Option<u32>,

        #[command(flatten)]
        server: ServerArgs,

        #[command(flatten)]
        place: PlaceArgs,

        #[command(flatten)]
        fflags: FflagArgs,
    },

    /// Show the canonical rodeo state: studios, their DOMs, and runs
    State {
        /// Print the raw state snapshot as JSON
        #[arg(long)]
        json: bool,

        #[command(flatten)]
        server: ServerArgs,
    },

    /// Kill a run or close a Studio by id
    Kill {
        /// Run ID or Studio ID (from `rodeo state`; prefixes accepted).
        /// Killing a Studio fails its active runs as disconnected.
        id: String,

        #[command(flatten)]
        server: ServerArgs,
    },

    /// Save the Studio place
    Save {
        /// Studio ID to save (from `rodeo state`; prefixes accepted).
        /// Defaults to the only connected Studio.
        id: Option<String>,

        /// Copy saved file to this output path (overrides the default of
        /// copying back to the launch's SOURCE_PATH)
        #[arg(long)]
        out: Option<String>,

        #[command(flatten)]
        server: ServerArgs,
    },

    /// Generate type definitions and configure .luaurc
    Setup,


    /// Internal: master server process
    #[command(name = "__master", hide = true)]
    InternalMaster {
        #[arg(long)]
        port: u16,
        /// Parent PID — exit when this process dies
        #[arg(long)]
        ppid: Option<u32>,
    },

    /// Internal: studio backend process
    #[command(name = "__studio-backend", hide = true)]
    InternalStudioBackend {
        /// Local port for plugin WebSocket connections
        #[arg(long)]
        port: u16,
        /// Master host to connect to
        #[arg(long)]
        master_host: String,
        /// Master port to connect to
        #[arg(long)]
        master_port: u16,
        /// Parent PID — exit when this process dies
        #[arg(long)]
        ppid: Option<u32>,
    },

    /// Internal: process source (bundle + shim + resolve)
    #[command(name = "__process_source", hide = true)]
    ProcessSource {
        /// Script file to process
        script: Option<String>,
        /// Inline source to process. Hyphen-leading values are legal (Luau
        /// hotcomments) — see Run's `source`.
        #[arg(long, allow_hyphen_values = true)]
        source: Option<String>,
        /// Path to rojo sourcemap.json
        #[arg(long)]
        sourcemap: Option<String>,
    },

    /// Internal: canonical JSON-RPC 2.0 client over NDJSON on stdin/stdout.
    /// Spawned by language wrappers (rodeo-client-ts, rodeo-client-luau).
    #[command(name = "__spawn_canonical_client", hide = true)]
    SpawnCanonicalClient {
        /// Master host
        #[arg(long, default_value = "localhost")]
        host: String,
        /// Master port. Resolution: this flag, then RODEO_PORT, then 44872.
        #[arg(long, env = "RODEO_PORT")]
        port: Option<u16>,
    },
}

/// Shared args for connecting to a running rodeo server
#[derive(clap::Args, Clone)]
pub struct ServerArgs {
    /// Host of running server
    #[arg(long, default_value = "localhost")]
    pub host: String,

    /// Master port of the running server. Resolution: this flag, then
    /// RODEO_PORT, then 44872.
    #[arg(long, env = "RODEO_PORT", default_value_t = config::SERVE_PORT)]
    pub port: u16,
}

/// Shared args for launching Studio/Player and targeting DOMs
#[derive(clap::Args, Clone, Default)]
pub struct PlaceArgs {
    /// Launch Studio: empty (no value), place ID (number), or file path (.rbxl/.rbxlx)
    #[arg(long = "place", num_args = 0..=1, default_missing_value = "", help_heading = "Launch")]
    pub place: Option<String>,

    /// Pin the run to a specific DOM by id (from `rodeo state`; unique prefix
    /// ok). Only --context may accompany it — no mode/dom routing.
    #[arg(long = "dom-id", help_heading = "Targeting")]
    pub dom_id: Option<String>,

    /// Universe ID (resolved from place ID if omitted)
    #[arg(long = "place.universe", value_name = "UNIVERSE_ID", help_heading = "Launch")]
    pub place_universe: Option<u64>,

    /// Bring Studio to the front on launch (default: background). Studio only
    /// takes keyboard focus when it opens on the display you are working on;
    /// on another display it is raised there and your focus stays put
    #[arg(long)]
    pub focus: bool,

    /// Keep Studio/Player running after rodeo exits
    #[arg(long = "detach", help_heading = "Launch")]
    pub detached: bool,

    /// Allow-list of Studio dock widgets to keep visible; everything else
    /// (panels, ribbon, command bar) is hidden. `none` hides all; a comma list
    /// keeps those (aliases: output, explorer, properties, editor, toolbox,
    /// assistant, ribbon, commandbar, rodeo (this serve's own panel); or a raw panel ID). Restored on exit.
    #[arg(long = "show-widgets", value_name = "WIDGETS", help_heading = "Launch")]
    pub show_widgets: Option<String>,

    /// Enable microprofiler auto-capture and collect dumps (optional: output directory)
    #[arg(long = "profile", num_args = 0..=1, default_missing_value = "", help_heading = "Profiling")]
    pub profile: Option<String>,

    /// Save Studio place on exit, optionally to a specific path
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub save: Option<String>,
}

impl PlaceArgs {
    /// Convert to PlaceTarget. Returns Ok(None) if no launch requested.
    ///
    /// A file path is resolved to an absolute path against the run client's cwd
    /// — the serve that opens the place may run in a different directory, so a
    /// relative path would otherwise be resolved against the serve's cwd (wrong
    /// file, or a fresh place). Canonicalize also fails fast if the file is
    /// missing rather than silently launching an empty place.
    pub fn to_target(&self) -> anyhow::Result<Option<crate::studio_backend::PlaceTarget>> {
        use crate::studio_backend::PlaceTarget;
        let Some(val) = self.place.as_deref() else {
            return Ok(None);
        };
        if val.is_empty() {
            Ok(Some(PlaceTarget::Empty))
        } else if let Ok(pid) = val.parse::<u64>() {
            Ok(Some(PlaceTarget::PlaceId {
                place_id: pid,
                universe_id: self.place_universe,
            }))
        } else {
            let abs = std::fs::canonicalize(val)
                .map_err(|e| anyhow::anyhow!("place file '{val}' not found: {e}"))?;
            if !abs.is_file() {
                anyhow::bail!("place path '{val}' is not a file, expected an .rbxl/.rbxlx place");
            }
            Ok(Some(PlaceTarget::File(abs.to_string_lossy().into_owned())))
        }
    }
}

/// Shared args for FFlag configuration
#[derive(clap::Args, Clone, Default)]
pub struct FflagArgs {
    /// Set FFlag override (Key=Value, repeatable)
    #[arg(long = "fflag.override", value_name = "KEY=VALUE", help_heading = "FFlags")]
    pub fflag_override: Vec<String>,

    /// Load FFlag overrides from a JSON file
    #[arg(long = "fflag.file", value_name = "PATH", help_heading = "FFlags")]
    pub fflag_file: Option<String>,
}

#[cfg(test)]
mod port_resolution_tests {
    use super::*;

    // One test body: RODEO_PORT is process-global, so the cases must not
    // interleave with each other.
    #[test]
    fn port_resolves_flag_then_env_then_default() {
        std::env::remove_var("RODEO_PORT");
        match Cli::try_parse_from(["rodeo", "state"]).unwrap().command {
            Commands::State { server, .. } => assert_eq!(server.port, config::SERVE_PORT),
            _ => unreachable!(),
        }

        std::env::set_var("RODEO_PORT", "46123");
        match Cli::try_parse_from(["rodeo", "state"]).unwrap().command {
            Commands::State { server, .. } => assert_eq!(server.port, 46123),
            _ => unreachable!(),
        }
        match Cli::try_parse_from(["rodeo", "serve"]).unwrap().command {
            Commands::Serve { port, master_port, .. } => {
                assert_eq!(port, Some(46123));
                assert_eq!(master_port, Some(46123));
            }
            _ => unreachable!(),
        }
        match Cli::try_parse_from(["rodeo", "__spawn_canonical_client"]).unwrap().command {
            Commands::SpawnCanonicalClient { port, .. } => assert_eq!(port, Some(46123)),
            _ => unreachable!(),
        }

        // The flag wins over the env var.
        match Cli::try_parse_from(["rodeo", "state", "--port", "46200"]).unwrap().command {
            Commands::State { server, .. } => assert_eq!(server.port, 46200),
            _ => unreachable!(),
        }
        std::env::remove_var("RODEO_PORT");
    }
}

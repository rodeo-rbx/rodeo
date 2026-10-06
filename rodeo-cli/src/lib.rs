pub mod cli;
mod cli_run;
mod commands;
mod master;
mod studio_backend;
mod shared;
mod runtime;
mod util;

use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Commands};
use util::config;

fn build_banner() -> String {
    let left_label = format!(" v{} ", rodeo_proto::BUILD_ID);
    let right_label = " rvy ";
    let pad = 3;

    let art: &[&str] = &[
        "██████╗  ██████╗ ██████╗ ███████╗ ██████╗",
        "██╔══██╗██╔═══██╗██╔══██╗██╔════╝██╔═══██╗",
        "██████╔╝██║   ██║██║  ██║█████╗  ██║   ██║",
        "██╔══██╗██║   ██║██║  ██║██╔══╝  ██║   ██║",
        "██║  ██║╚██████╔╝██████╔╝███████╗╚██████╔╝",
        "╚═╝  ╚═╝ ╚═════╝ ╚═════╝ ╚══════╝ ╚═════╝",
    ];

    let max_w = art.iter().map(|l| l.chars().count()).max().unwrap();
    let inner = max_w + pad * 2;
    let left_len = left_label.chars().count();
    let right_len = right_label.chars().count();
    let fill = inner.saturating_sub(left_len + right_len + 4);

    let mut out = String::new();

    out += &format!(
        "\x1b[33m╭──{}{}{}──╮\x1b[0m\n",
        left_label,
        "─".repeat(fill),
        right_label,
    );

    let empty_row = format!(
        "\x1b[33m│\x1b[0m{}\x1b[33m│\x1b[0m\n",
        " ".repeat(inner),
    );
    out += &empty_row;

    for line in art {
        let w = line.chars().count();
        let right_pad = inner - pad - w;
        out += &format!(
            "\x1b[33m│\x1b[0m{}\x1b[1;31m{}\x1b[0m{}\x1b[33m│\x1b[0m\n",
            " ".repeat(pad),
            line,
            " ".repeat(right_pad),
        );
    }

    out += &empty_row;
    out += &format!("\x1b[33m╰{}╯\x1b[0m", "─".repeat(inner));

    out
}

/// Long flags clap parses as `Vec<T>` — directive and CLI both contribute, so
/// these are exempt from the directive-token override filter. Keep in sync if
/// new repeatable args are added to `Commands::Run` (or other commands that
/// participate in the directive splice).
const REPEATABLE_DIRECTIVE_FLAGS: &[&str] = &["--fflag.override"];

/// Directive flags that a differently-named user flag overrides, because the
/// two pick the Studio in incompatible ways: a directive's `--place` documents
/// how to launch the script standalone, while a user's `--studio-id` or
/// `--dom-id` points it at a Studio that is already open (issue #28). Without
/// this, `--studio-id` hit clap's conflict with the directive's `--place`,
/// and `--dom-id` launched a Studio only to run in another DOM and close it.
/// The reverse holds too: a user's `--place` overrides a directive's pin.
const DIRECTIVE_FLAGS_OVERRIDDEN_BY: &[(&str, &[&str])] = &[
    ("--place", &["--studio-id", "--dom-id"]),
    ("--studio-id", &["--place"]),
    ("--dom-id", &["--place"]),
];

/// Directive flags that go with a directive flag the table above drops: the
/// directive's `--save` saves the Studio its `--place` launches, so once a
/// user's `--studio-id`/`--dom-id` drops that `--place`, keeping `--save`
/// would apply it to the Studio the user pointed the script at instead of
/// the one the directive meant to launch. A `--save` the user types applies.
const DIRECTIVE_FLAGS_DROPPED_WITH: &[(&str, &str)] = &[("--save", "--place")];

/// Drop directive flag tokens whose long-name matches a user-supplied CLI
/// flag, so clap's "argument cannot be used multiple times" rejection
/// doesn't fire on directive↔CLI overlap. Scalar override semantics: user
/// CLI wins for any flag they passed, and for the directive flags their
/// flags conflict with (`DIRECTIVE_FLAGS_OVERRIDDEN_BY`, plus what goes with
/// those, `DIRECTIVE_FLAGS_DROPPED_WITH`). Repeatable Vec flags (see
/// `REPEATABLE_DIRECTIVE_FLAGS`) pass through unfiltered so values
/// accumulate.
fn filter_directive_for_overrides(directive: &[String], user_after_run: &[String]) -> Vec<String> {
    fn flag_names(tokens: &[String]) -> std::collections::HashSet<&str> {
        tokens
            .iter()
            .filter(|t| t.starts_with("--") && t.len() > 2)
            .map(|t| t.split('=').next().unwrap())
            .collect()
    }
    let user_flags = flag_names(user_after_run);
    let directive_flags = flag_names(directive);
    let conflicts = |flag: &str| {
        DIRECTIVE_FLAGS_OVERRIDDEN_BY
            .iter()
            .any(|(f, by)| *f == flag && by.iter().any(|b| user_flags.contains(b)))
    };
    let overridden = |flag: &str| {
        user_flags.contains(flag)
            || conflicts(flag)
            || DIRECTIVE_FLAGS_DROPPED_WITH
                .iter()
                .any(|(f, with)| *f == flag && directive_flags.contains(with) && conflicts(with))
    };

    let mut out = Vec::with_capacity(directive.len());
    let mut i = 0;
    while i < directive.len() {
        let tok = &directive[i];
        if tok.starts_with("--") && tok.len() > 2 {
            let flag_name = tok.split('=').next().unwrap();
            let repeatable = REPEATABLE_DIRECTIVE_FLAGS.contains(&flag_name);
            if overridden(flag_name) && !repeatable {
                i += 1;
                let has_inline_value = tok.contains('=');
                if !has_inline_value
                    && i < directive.len()
                    && !directive[i].starts_with("--")
                {
                    i += 1;
                }
                continue;
            }
        }
        out.push(directive[i].clone());
        i += 1;
    }
    out
}

/// The argv a `rodeo run` with a directive is re-parsed from: the user's argv
/// with the directive's flag tokens spliced in after the user's flags.
fn splice_directive(argv: &[String], directive_flags: &[String]) -> Vec<String> {
    let run_idx = argv.iter().position(|a| a == "run")
        .expect("matched Run subcommand but no 'run' in argv");
    let user_after_run: &[String] = &argv[run_idx + 1..];
    // Split at the user's `--`: script_args is `last = true`,
    // so anything after the first bare `--` is captured raw —
    // directive flag tokens spliced after it would be eaten as
    // script arguments instead of parsed as flags (issue #8).
    // The tail (including the `--` itself) must stay at the
    // very end of the spliced argv; only the tokens before it
    // are user *flags* for override purposes.
    let dd = user_after_run.iter().position(|t| t == "--");
    let (user_flags, user_tail) = match dd {
        Some(i) => (&user_after_run[..i], &user_after_run[i..]),
        None => (user_after_run, &[][..]),
    };
    let filtered = filter_directive_for_overrides(directive_flags, user_flags);
    // User flags + positional first, then directive tokens.
    // The user's positional (script path) must be parsed
    // before any `num_args = 0..=1` flags in the directive
    // (e.g. `--place`), otherwise clap greedily consumes the
    // positional as the flag's value and downstream tries to
    // open the script as a place file ("failed to parse
    // binary place"). Override semantics still hold because
    // `filter_directive_for_overrides` already dropped any
    // directive flag the user also passed.
    let mut spliced = argv[..=run_idx].to_vec();
    spliced.extend(user_flags.iter().cloned());
    spliced.extend(filtered);
    spliced.extend(user_tail.iter().cloned());
    spliced
}

/// Library entry point: parses CLI, dispatches subcommands. main.rs is the
/// thin binary wrapper that builds the tokio runtime and calls this.
pub async fn run() {
    // Load .env file if present (shell env vars take precedence)
    let _ = dotenvy::dotenv();

    // Tell launch-control to dispatch helper invocations through us via
    // the `__launch-control` subcommand. Single binary — no separate
    // helper file to deploy or unpack.
    if let Ok(exe) = std::env::current_exe() {
        launch_control::set_helper_invocation(exe, vec!["__launch-control".into()]);
    }

    let banner = build_banner();
    let matches = Cli::command()
        .before_help(banner.clone())
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)
        .unwrap_or_else(|e| e.exit());

    // If this is `rodeo run <script>`, read the script's `@rodeo run …`
    // directive (if any) and splice its flag tokens into argv right after
    // the `run` subcommand. Then re-parse via clap so directive flags flow
    // through the same arg pipeline as the CLI — no per-field merge code,
    // adding a new CLI arg works in directives automatically.
    //
    // CLI precedence: any flag the user passed on the CLI is removed from
    // the directive tokens before splicing, so clap doesn't see duplicate
    // occurrences (which it rejects by default for scalar `Option<T>`).
    // Vec-typed flags in `REPEATABLE_DIRECTIVE_FLAGS` are exempt — both
    // directive and CLI values accumulate.
    let (cli, directive_script_args) = match &cli.command {
        Commands::Run { script: Some(script_arg), .. } => {
            let resolved = commands::process_source::directive::resolve_script_path(script_arg);
            match std::fs::read_to_string(&resolved)
                .ok()
                .and_then(|c| commands::process_source::directive::parse_directive(&c))
            {
                Some(tokens) if !tokens.flag_args.is_empty() || !tokens.script_args.is_empty() => {
                    let argv: Vec<String> = std::env::args().collect();
                    let spliced = splice_directive(&argv, &tokens.flag_args);
                    let re_parsed = Cli::command()
                        .before_help(banner)
                        .get_matches_from(spliced);
                    let cli = Cli::from_arg_matches(&re_parsed)
                        .unwrap_or_else(|e| e.exit());
                    (cli, tokens.script_args)
                }
                _ => (cli, Vec::new()),
            }
        }
        _ => (cli, Vec::new()),
    };

    let verbose = cli.verbose || std::env::var("RODEO_VERBOSE").is_ok();
    util::log::init();

    // Long-running subprocesses (master, studio-backend, player-backend,
    // studio-daemon) capture structured JSON logs to .rodeo/.temp/logs/ in
    // addition to stderr — see util::log_capture. All other commands
    // (run, ps, kill, save, etc.) keep the existing stderr-only subscriber.
    //
    // For the master specifically, the bootstrap UUID doubles as `master_id`
    // advertised to backends in `RegisterResponse` — we stash it in an Option
    // so the `InternalMaster` branch below can pass it into `run_master`.
    let subprocess_role: Option<&'static str> = match &cli.command {
        Commands::InternalMaster { .. } => Some("master"),
        Commands::InternalStudioBackend { .. } => Some("studio-backend"),
        _ => None,
    };
    let master_bootstrap_id: Option<String> = if let Some(role) = subprocess_role {
        let bootstrap_id = uuid::Uuid::new_v4().to_string();
        util::log_capture::init(role, &bootstrap_id);
        if role == "master" { Some(bootstrap_id) } else { None }
    } else {
        // Initialize tracing subscriber (existing behavior for non-subprocess commands)
        use tracing_subscriber::EnvFilter;
        let quiet_serve = !verbose && matches!(&cli.command, Commands::Run { .. });

        if quiet_serve && std::env::var_os("RUST_LOG").is_none() {
            std::env::set_var("RODEO_QUIET", "1");
        }

        let filter = EnvFilter::try_from_env("RUST_LOG")
            .unwrap_or_else(|_| {
                if verbose {
                    EnvFilter::new("rodeo=debug")
                } else if quiet_serve {
                    EnvFilter::new("rodeo=warn")
                } else {
                    EnvFilter::new("rodeo=info")
                }
            });
        use std::io::IsTerminal;
        let no_color = std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty());
        let force_color = std::env::var("FORCE_COLOR").is_ok_and(|v| !v.is_empty());
        let use_ansi = !no_color && (force_color || std::io::stderr().is_terminal());

        let no_timestamps = std::env::var("RODEO_NO_TIMESTAMPS").is_ok();
        if no_timestamps {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_writer(std::io::stderr)
                .with_ansi(use_ansi)
                .without_time()
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_writer(std::io::stderr)
                .with_ansi(use_ansi)
                .init();
        }
        None
    };

    let result = match cli.command {
        Commands::Serve { port, master, studio_mode, master_host, master_port, ppid } => {
            if let Some(ppid) = ppid { parent_exit::on_parent_exit(ppid); }
            let mode = if master {
                commands::serve::ServeMode::Master
            } else if studio_mode {
                let master_port = master_port.unwrap_or(config::SERVE_PORT);
                let serve = matches.subcommand_matches("serve").expect("matched Serve but no serve matches");
                commands::serve::ServeMode::Studio {
                    port: cli::studio_backend_port(serve, port, master_port),
                    master_host,
                    master_port,
                }
            } else {
                commands::serve::ServeMode::Combined
            };
            commands::serve::main(port, mode).await
        }
        Commands::Run { script, source, sourcemap, output, return_file, show_return, mode, dom_kind, context, studio_id, no_warn, no_error, no_info, no_print, no_output, reload_requires, cache_requires_removed, script_args, ppid, server, place, fflags } => {
            if cache_requires_removed {
                eprintln!(
                    "rodeo: --cache-requires was removed — caching instance requires is the default now.\n\
                     Drop the flag for the same behavior, or pass --reload-requires to re-evaluate them\n\
                     (the old default: each run gets freshly-initialized copies of the require tree)."
                );
                std::process::exit(2);
            }
            if let Some(ppid) = ppid { parent_exit::on_parent_exit(ppid); }
            let script_args = if script_args.is_empty() { directive_script_args } else { script_args };
            commands::run::main(commands::run::RunArgs {
                script, source, sourcemap, output, return_file, show_return,
                mode: mode.map(|m| m.as_str().to_string()),
                dom_kind: dom_kind.map(|d| d.as_str().to_string()),
                context: context.map(|c| c.as_str().to_string()),
                studio_id,
                no_warn, no_error, no_info, no_print, no_output,
                reload_requires, script_args,
                server, place, fflags,
                verbose,
            }).await
        }
        Commands::State { json, server } => commands::state::main(&server.host, server.port, json).await,
        Commands::Kill { id, server } => commands::kill::main(&id, &server.host, server.port).await,
        Commands::Save { id, out, server } => commands::save::main(id.as_deref(), &server.host, server.port, out).await,
        Commands::Setup => commands::setup::main(),
        Commands::InternalMaster { port, ppid } => {
            if let Some(ppid) = ppid { parent_exit::on_parent_exit(ppid); }
            let master_id = master_bootstrap_id.unwrap_or_default();
            commands::serve::run_master(port, master_id).await
        }
        Commands::InternalStudioBackend { port, master_host, master_port, ppid } => {
            if let Some(ppid) = ppid { parent_exit::on_parent_exit(ppid); }
            commands::serve::run_studio_backend(port, &master_host, master_port).await
        }
        Commands::ProcessSource { script, source, sourcemap } => {
            commands::process_source::main(script, source, sourcemap)
                .map_err(|e| { eprintln!("{e}"); e })
        }
        Commands::SpawnCanonicalClient { host, port } => {
            commands::spawn_canonical_client::main(host, port.unwrap_or(config::SERVE_PORT)).await
        }
    };

    if let Err(e) = result {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod directive_splice_tests {
    use super::*;
    use clap::Parser;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    struct Parsed {
        studio_id: Option<String>,
        dom_id: Option<String>,
        place: Option<String>,
        place_universe: Option<u64>,
        save: Option<String>,
        show_return: bool,
    }

    /// Parse `argv` the way `run()` does when the script carries `directive`.
    fn parse(argv: &[&str], directive: &[&str]) -> Result<Parsed, clap::Error> {
        let spliced = splice_directive(&strings(argv), &strings(directive));
        match Cli::try_parse_from(spliced)?.command {
            Commands::Run { studio_id, show_return, place, .. } => Ok(Parsed {
                studio_id,
                dom_id: place.dom_id,
                place: place.place,
                place_universe: place.place_universe,
                save: place.save,
                show_return,
            }),
            _ => unreachable!(),
        }
    }

    // Issue #28: re-running a script whose directive launches a place against
    // an already-open Studio. The user's pin wins; the directive's other
    // flags still apply.
    #[test]
    fn a_cli_studio_id_overrides_a_directive_place() {
        for directive in [
            &["--place", "build.rbxl", "--show-return"][..],
            &["--place=build.rbxl", "--show-return"][..],
            // Bare --place (empty place) followed by another flag.
            &["--place", "--show-return"][..],
        ] {
            let p = parse(&["rodeo", "run", "script.luau", "--studio-id", "abc"], directive)
                .unwrap_or_else(|e| panic!("{directive:?}: {e}"));
            assert_eq!(p.studio_id.as_deref(), Some("abc"), "{directive:?}");
            assert_eq!(p.place, None, "{directive:?}");
            assert!(p.show_return, "{directive:?}");
        }
        let p = parse(&["rodeo", "run", "script.luau", "--studio-id=abc"], &["--place", "build.rbxl"]).unwrap();
        assert_eq!((p.studio_id.as_deref(), p.place), (Some("abc"), None));
    }

    // --dom-id used to keep the directive's --place: the run launched a
    // Studio, ran in the pinned DOM instead, and closed the launch unused.
    #[test]
    fn a_cli_dom_id_overrides_a_directive_place() {
        let p = parse(&["rodeo", "run", "script.luau", "--dom-id", "d0m"], &["--place", "123", "--place.universe", "456"]).unwrap();
        assert_eq!(p.dom_id.as_deref(), Some("d0m"));
        assert_eq!(p.place, None);
        // Only --place itself is dropped; launch-only modifiers are inert
        // without it.
        assert_eq!(p.place_universe, Some(456));
    }

    // The directive's --save saves the Studio its --place launches; with that
    // launch replaced by a pin, the save goes too. A --save the user types,
    // or a directive --save with no --place of its own, still applies.
    #[test]
    fn a_directive_save_goes_with_the_place_a_pin_drops() {
        for directive in [&["--place", "build.rbxl", "--save"][..], &["--save", "out.rbxl", "--place", "build.rbxl"][..]] {
            for pin in [&["--studio-id", "abc"][..], &["--dom-id", "d0m"][..]] {
                let argv: Vec<&str> = ["rodeo", "run", "script.luau"].iter().chain(pin).copied().collect();
                let p = parse(&argv, directive).unwrap_or_else(|e| panic!("{directive:?} {pin:?}: {e}"));
                assert_eq!((p.place, p.save), (None, None), "{directive:?} {pin:?}");
            }
        }
        let p = parse(&["rodeo", "run", "script.luau", "--studio-id", "abc", "--save", "mine.rbxl"], &["--place", "build.rbxl", "--save"]).unwrap();
        assert_eq!(p.save.as_deref(), Some("mine.rbxl"));
        let p = parse(&["rodeo", "run", "script.luau", "--studio-id", "abc"], &["--save", "out.rbxl"]).unwrap();
        assert_eq!(p.save.as_deref(), Some("out.rbxl"));
        // A user --place replaces only the directive's place; its --save
        // applies to the user's launch as before.
        let p = parse(&["rodeo", "run", "script.luau", "--place", "other.rbxl"], &["--place", "build.rbxl", "--save"]).unwrap();
        assert_eq!((p.place.as_deref(), p.save.as_deref()), (Some("other.rbxl"), Some("")));
    }

    #[test]
    fn a_cli_place_overrides_a_directive_pin() {
        let p = parse(&["rodeo", "run", "script.luau", "--place", "other.rbxl"], &["--studio-id", "abc"]).unwrap();
        assert_eq!((p.studio_id, p.place.as_deref()), (None, Some("other.rbxl")));
        let p = parse(&["rodeo", "run", "script.luau", "--place"], &["--dom-id", "d0m"]).unwrap();
        assert_eq!((p.dom_id, p.place.as_deref()), (None, Some("")));
    }

    #[test]
    fn without_a_cli_pin_the_directive_place_stands() {
        let p = parse(&["rodeo", "run", "script.luau"], &["--place", "build.rbxl"]).unwrap();
        assert_eq!(p.place.as_deref(), Some("build.rbxl"));
        // An unrelated user flag doesn't drop it either.
        let p = parse(&["rodeo", "run", "script.luau", "--show-return"], &["--place", "build.rbxl"]).unwrap();
        assert_eq!(p.place.as_deref(), Some("build.rbxl"));
    }

    // Typed together on the command line the two still conflict: nothing says
    // which one the user meant.
    #[test]
    fn a_pin_and_place_both_typed_conflict() {
        assert!(parse(&["rodeo", "run", "script.luau", "--studio-id", "abc", "--place", "build.rbxl"], &[]).is_err());
        assert!(parse(&["rodeo", "run", "script.luau", "--dom-id", "d0m", "--place", "build.rbxl"], &[]).is_err());
    }
}

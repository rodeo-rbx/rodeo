/// Bundle a script's external requires into a single source string via darklua_core.
///
/// Two-pass pipeline:
///   1. (optional, if sourcemap exists) convert_require with roblox target + sourcemap
///      → converts Wally package requires to Roblox instance paths
///   2. darklua bundle with luau require mode: inlines filesystem deps, and
///      `@rodeo` plus the enabled adapters' aliases (see `adapters`), which are
///      written to disk next to the bundle and bundled like any other module.

use anyhow::{bail, Context, Result};
use super::adapters;
use darklua_core::{
    process, BundleConfiguration, Configuration, GeneratorParameters, Options, Resources,
};
use darklua_core::rules::bundle::BundleRequireMode;
use std::path::{Path, PathBuf};

pub struct BundleOptions {
    pub sourcemap: Option<String>,
    pub verbose: bool,
    /// Adapters to enable (`--adapter`), by name.
    pub adapters: Vec<String>,
}

pub fn bundle(script_path: &str, options: &BundleOptions) -> Result<String> {
    let resources = Resources::from_file_system();
    let abs_script = std::fs::canonicalize(script_path)
        .context(format!("cannot resolve script path '{script_path}'"))?;

    // Per-invocation tmp dir + converted-file path so concurrent `rodeo run`
    // procs (parallel test harnesses, scripted launches) don't race on
    // shared `.rodeo-bundle-tmp/` and `.rodeo-converted.luau` paths.
    let invocation_id = uuid::Uuid::new_v4();
    let tmp = PathBuf::from(format!(".rodeo-bundle-tmp-{invocation_id}"));
    std::fs::create_dir_all(&tmp)
        .context("failed to create temp dir")?;
    let converted_filename = format!(".rodeo-converted-{invocation_id}.luau");

    let result = bundle_inner(&resources, &abs_script, &tmp, &converted_filename, options);

    // Always cleanup
    let _ = std::fs::remove_dir_all(&tmp);
    if let Some(parent) = abs_script.parent() {
        let converted = parent.join(&converted_filename);
        if converted.exists() {
            let _ = std::fs::remove_file(&converted);
        }
    }

    result
}

fn bundle_inner(
    resources: &Resources,
    script_path: &Path,
    tmp: &Path,
    converted_filename: &str,
    options: &BundleOptions,
) -> Result<String> {
    let mut input_path = script_path.to_path_buf();

    // Pass 1 (optional): convert requires via sourcemap to Roblox instance paths
    if let Some(ref sourcemap) = options.sourcemap {
        let sm_path = Path::new(sourcemap);
        if sm_path.exists() {
            let abs_sourcemap = std::fs::canonicalize(sm_path)
                .context("cannot resolve sourcemap")?;

            // Output next to original so darklua resolves relative requires correctly
            let converted_path = script_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(converted_filename);

            let config = Configuration::empty()
                .with_generator(GeneratorParameters::RetainLines)
                .with_rule(convert_require_rule(
                    "luau",
                    "roblox",
                    Some(&abs_sourcemap),
                    true,
                )?);

            let opts = Options::new(script_path)
                .with_configuration(config)
                .with_output(&converted_path);

            let convert_tree = process(resources, opts)
                .map_err(|e| anyhow::anyhow!("pass 1 (convert_require) failed: {e}"))?;
            check_darklua_result(convert_tree, "convert the requires of", script_path)?;

            if !converted_path.exists() {
                bail!(
                    "darklua failed to convert requires for '{}' (check .luaurc and sourcemap for errors)",
                    script_path.display()
                );
            }

            input_path = converted_path;
        }
    }

    // Pass 2: darklua bundle (inlines filesystem deps and the alias trees)
    let bundled_path = tmp.join("bundled.luau");

    let alias_trees = adapters::write_alias_trees(&tmp.join("aliases"), &options.adapters)?;
    let require_mode = luau_require_mode(&alias_trees)?;
    // A require through an alias that doesn't resolve stays as written, and
    // only errors in Studio if it runs: code written for another runtime can
    // require that runtime's modules on a path Studio never takes.
    let bundle_config = BundleConfiguration::new(require_mode).with_keep_unresolved_aliases(true);

    let config = Configuration::empty()
        .with_generator(GeneratorParameters::RetainLines)
        .with_rule(convert_require_rule("luau", "path", None, false)?)
        .with_bundle_configuration(bundle_config);

    let opts = Options::new(&input_path)
        .with_configuration(config)
        .with_output(&bundled_path);

    let bundle_tree = process(resources, opts)
        .map_err(|e| anyhow::anyhow!("pass 2 (bundle) failed: {e}"))?;
    check_darklua_result(bundle_tree, "bundle", &input_path)?;

    if !bundled_path.exists() {
        bail!(
            "darklua failed to bundle '{}' (check .luaurc for JSON syntax errors)",
            input_path.display()
        );
    }

    // Read bundled output
    let bundled = std::fs::read_to_string(&bundled_path)
        .context("failed to read bundled output")?;

    if options.verbose {
        tracing::debug!("bundled {} bytes", bundled.len());
    }

    Ok(bundled)
}

/// darklua reports what went wrong (a parse error, a require it can't
/// resolve) in the worker tree rather than as `process`'s error, and then
/// writes no output. Fail with those messages.
fn check_darklua_result(tree: darklua_core::WorkerTree, action: &str, path: &Path) -> Result<()> {
    let Err(errors) = tree.result() else {
        return Ok(());
    };
    let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
    bail!(
        "darklua couldn't {action} '{}':\n- {}",
        path.display(),
        messages.join("\n- ")
    )
}

/// The luau require mode for bundling: `@rodeo` and the enabled adapters'
/// aliases point at their written trees and take precedence over the
/// project's `.luaurc`, which typically maps `@lune`/`@lute`/`@std`/`@rodeo`
/// to type definitions. Other aliases still come from `.luaurc`.
fn luau_require_mode(alias_trees: &[(&str, PathBuf)]) -> Result<BundleRequireMode> {
    let aliases: serde_json::Map<String, serde_json::Value> = alias_trees
        .iter()
        .map(|(alias, path)| (alias.to_string(), path.display().to_string().into()))
        .collect();
    // darklua_core doesn't export the luau require mode's type; configure via serde.
    serde_json::from_value(serde_json::json!({
        "name": "luau",
        "aliases": aliases,
        "prefer_aliases": true,
    }))
    .context("failed to create the luau require mode")
}

/// Create a convert_require rule via JSON5 deserialization.
/// darklua_core's ConvertRequire fields are private, so we configure via serde.
fn convert_require_rule(
    current: &str,
    target: &str,
    sourcemap: Option<&Path>,
    use_luau_config: bool,
) -> Result<Box<dyn darklua_core::rules::Rule>> {
    // Build the rule config with serde_json (not string interpolation): the
    // sourcemap path is interpolated as a JSON string, and on Windows it
    // contains backslashes (`C:\...`, often a `\\?\` verbatim prefix) which are
    // invalid JSON escapes — `format!` produced malformed JSON that failed to
    // deserialize. serde_json escapes the path correctly.
    let current_json = match current {
        "luau" => serde_json::json!({ "name": "luau", "use_luau_configuration": use_luau_config }),
        "path" => serde_json::json!({ "name": "path" }),
        other => bail!("unknown require mode: {other}"),
    };

    let target_json = match target {
        "path" => serde_json::json!({ "name": "path" }),
        "roblox" => match sourcemap {
            Some(sm) => serde_json::json!({
                "name": "roblox",
                "rojo_sourcemap": sm.display().to_string(),
                "indexing_style": "wait_for_child",
            }),
            None => serde_json::json!({ "name": "roblox" }),
        },
        other => bail!("unknown target require mode: {other}"),
    };

    let rule = serde_json::json!({
        "rule": "convert_require",
        "current": current_json,
        "target": target_json,
    });

    serde_json::from_value::<Box<dyn darklua_core::rules::Rule>>(rule)
        .context("failed to create convert_require rule")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory holding `files`, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("rodeo-bundle-test-{}", uuid::Uuid::new_v4()));
            for (name, content) in files {
                let path = dir.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, content).unwrap();
            }
            Self(dir)
        }

        fn bundle(&self, entry: &str) -> Result<String> {
            self.bundle_with(entry, &[])
        }

        fn bundle_with(&self, entry: &str, adapters: &[&str]) -> Result<String> {
            let options = BundleOptions {
                sourcemap: None,
                verbose: false,
                adapters: adapters.iter().map(|a| a.to_string()).collect(),
            };
            let entry = self.0.join(entry).to_str().unwrap().to_string();
            // darklua recurses deeply over the adapters' modules; an unoptimized
            // build needs more than a test thread's stack (rodeo bundles on its
            // `__process_source` process's main thread).
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || bundle(&entry, &options))
                .unwrap()
                .join()
                .unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_failed_bundle_reports_darklua_errors() {
        let scratch = Scratch::new(&[("main.luau", "return require('./missing')")]);

        let err = format!("{:#}", scratch.bundle("main.luau").unwrap_err());

        assert!(err.contains("missing"), "{err}");
        assert!(!err.contains("check .luaurc"), "{err}");
    }

    #[test]
    fn a_require_through_an_unknown_alias_stays_as_written() {
        let scratch = Scratch::new(&[(
            "main.luau",
            "local fs = if game then nil else require('@std/fs')\nreturn fs",
        )]);

        let bundled = scratch.bundle("main.luau").unwrap();

        assert!(bundled.contains("require('@std/fs')"), "{bundled}");
    }

    #[test]
    fn rodeo_requires_always_bundle() {
        let scratch = Scratch::new(&[("main.luau", "local fs = require('@rodeo/fs')\nreturn fs")]);

        let bundled = scratch.bundle("main.luau").unwrap();

        assert!(!bundled.contains("require('@rodeo/fs')"), "{bundled}");
        assert!(bundled.contains("__rodeo_procs"), "{bundled}");
    }

    #[test]
    fn an_adapter_require_stays_as_written_until_its_adapter_is_enabled() {
        let scratch = Scratch::new(&[("main.luau", "local fs = require('@lune/fs')\nreturn fs")]);

        let without = scratch.bundle("main.luau").unwrap();
        let with = scratch.bundle_with("main.luau", &["lune"]).unwrap();

        assert!(without.contains("require('@lune/fs')"), "{without}");
        assert!(!with.contains("require('@lune/fs')"), "{with}");
        assert!(with.contains("__rodeo_procs"), "{with}");
    }

    #[test]
    fn an_enabled_adapter_overrides_the_luaurc_alias_and_otherwise_luaurc_is_followed() {
        let scratch = Scratch::new(&[
            ("main.luau", "return require('@lune/fs')"),
            ("typedefs/lune/fs.luau", "return 'from luaurc typedefs'"),
            (".luaurc", r#"{ "aliases": { "lune": "./typedefs/lune" } }"#),
        ]);

        let without = scratch.bundle("main.luau").unwrap();
        let with = scratch.bundle_with("main.luau", &["lune"]).unwrap();

        assert!(without.contains("from luaurc typedefs"), "{without}");
        assert!(!with.contains("from luaurc typedefs"), "{with}");
        assert!(with.contains("__rodeo_procs"), "{with}");
    }

    #[test]
    fn an_adapter_module_bundles_the_modules_it_requires() {
        // @lute/fs requires @lute/time for its Duration timestamps.
        let scratch = Scratch::new(&[("main.luau", "return require('@lute/fs')")]);

        let bundled = scratch.bundle_with("main.luau", &["lute"]).unwrap();

        assert!(!bundled.contains("require(\"@lute/time\")"), "{bundled}");
        assert!(!bundled.contains("require('@lute/time')"), "{bundled}");
    }

    #[test]
    fn a_lute_module_bundles_as_a_call_to_lute() {
        let scratch = Scratch::new(&[("main.luau", "return require('@lute/net/client')")]);

        let bundled = scratch.bundle_with("main.luau", &["lute"]).unwrap();

        assert!(bundled.contains(r#"module("lute", "@lute/net/client")"#), "{bundled}");
    }

    #[test]
    fn an_unknown_adapter_fails_the_bundle() {
        let scratch = Scratch::new(&[("main.luau", "return 1")]);

        let err = format!("{:#}", scratch.bundle_with("main.luau", &["nope"]).unwrap_err());

        assert!(err.contains("unknown adapter 'nope'"), "{err}");
    }

    #[test]
    fn a_hex_literal_larger_than_64_bits_bundles() {
        let scratch = Scratch::new(&[
            ("main.luau", "return require('./value')"),
            ("value.luau", "return 0x1_0000_0000_0000_0000"),
        ]);

        let bundled = scratch.bundle("main.luau").unwrap();

        assert!(bundled.contains("0x1_0000_0000_0000_0000"), "{bundled}");
    }
}

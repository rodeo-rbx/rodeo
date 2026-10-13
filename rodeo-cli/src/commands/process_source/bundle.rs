/// Bundle a script's external requires into a single source string via darklua_core.
///
/// Two-pass pipeline:
///   1. (optional, if sourcemap exists) convert_require with roblox target + sourcemap
///      → converts Wally package requires to Roblox instance paths
///   2. darklua bundle with luau require mode (inlines filesystem deps, skips adapter requires)
///
/// Adapter shim inlining (replacing `require("@alias/...")` with IIFEs) is handled
/// separately by `inline_shims()`, called from `client::run()` before submission.

use anyhow::{bail, Context, Result};
use super::adapters;
use darklua_core::{
    process, BundleConfiguration, Configuration, GeneratorParameters, Options, Resources,
};
use darklua_core::rules::bundle::BundleRequireMode;
use regex::Regex;
use std::path::{Path, PathBuf};

pub struct BundleOptions {
    pub sourcemap: Option<String>,
    pub verbose: bool,
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
    let excludes = adapters::exclude_patterns();

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

    // Pass 2: darklua bundle (inlines filesystem deps, skips adapter requires)
    let bundled_path = tmp.join("bundled.luau");

    let require_mode: BundleRequireMode = "luau".parse()
        .map_err(|e: String| anyhow::anyhow!(e))
        .context("invalid bundle require mode")?;
    // A require through an alias that doesn't resolve stays as written, and
    // only errors in Studio if it runs: code written for another runtime can
    // require that runtime's modules on a path Studio never takes.
    let mut bundle_config =
        BundleConfiguration::new(require_mode).with_keep_unresolved_aliases(true);
    for exclude in &excludes {
        bundle_config = bundle_config.with_exclude(exclude.clone());
    }

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

/// Replace `require("@alias/...")` calls with inlined adapter shim IIFEs.
///
/// Called from `client::run()` on all scripts (both file-based and inline `--source`)
/// before submission to the serve WebSocket.
///
/// Runs replacement passes to a fixpoint: a shim body may itself require
/// another shim (e.g. @lute/fs requires @lute/time for Duration timestamps),
/// and a single pass only resolves that when the inner shim's turn happens to
/// come later in iteration order. Shims must not require themselves — the
/// iteration cap turns that mistake into an error instead of unbounded
/// growth.
pub fn inline_shims(source: &str) -> Result<String> {
    let adapters = adapters::load_adapters();
    let mut result = source.to_string();

    for _pass in 0..8 {
        let before_len = result.len();
        let mut changed = false;
        for adapter in adapters {
            let escaped_alias = regex::escape(adapter.alias);
            for &(require_path, shim_src) in adapter.shims {
                let pattern_str = if require_path.is_empty() {
                    format!(r#"require\(["']{}["']\)"#, escaped_alias)
                } else {
                    format!(r#"require\(["']{}/{}["']\)"#, escaped_alias, require_path)
                };
                let re = Regex::new(&pattern_str)
                    .context("invalid regex for adapter shim")?;

                if re.is_match(&result) {
                    let comment_re = Regex::new(r"--[^\n]*\n").unwrap();
                    let clean_shim = comment_re.replace_all(shim_src, "\n");
                    let iife = format!("(function() {} end)()", clean_shim.trim());
                    result = re.replace_all(&result, iife.as_str()).to_string();
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(result);
        }
        // Growth without convergence after several passes = a shim cycle.
        if _pass == 7 && result.len() > before_len {
            bail!("adapter shim expansion did not converge (shim require cycle?)");
        }
    }
    Ok(result)
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
            let options = BundleOptions { sourcemap: None, verbose: false };
            bundle(self.0.join(entry).to_str().unwrap(), &options)
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
    fn a_hex_literal_larger_than_64_bits_bundles() {
        let scratch = Scratch::new(&[
            ("main.luau", "return require('./value')"),
            ("value.luau", "return 0x1_0000_0000_0000_0000"),
        ]);

        let bundled = scratch.bundle("main.luau").unwrap();

        assert!(bundled.contains("0x1_0000_0000_0000_0000"), "{bundled}");
    }
}

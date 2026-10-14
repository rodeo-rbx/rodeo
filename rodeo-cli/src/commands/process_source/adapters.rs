//! The `@rodeo` shims and the runtime adapters a run opts into.
//!
//! An adapter stands in for another Luau runtime's libraries on top of
//! `@rodeo` (Lune's `@lune/*`; Lute's `@lute/*`, `@std/*` and `@batteries/*`).
//! Each alias is a module tree that mirrors the runtime's own, embedded by
//! build.rs from `rodeo-adapters/<adapter>/<alias>/` (`@rodeo` from
//! `rodeo-pkg/rodeo/shims/`). Bundling writes the trees a run uses to disk and
//! points darklua's aliases at them, so they bundle like the script's own
//! modules. Adding an adapter or a module is a matter of adding files.
//!
//! Most adapter modules hand their work to the real runtime through
//! `@rodeo-bridge` (`rodeo-adapters/_bridge/`), which comes with any adapter.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct AliasTree {
    pub alias: &'static str,
    /// (path relative to the alias folder, with `/` separators; source)
    pub files: &'static [(&'static str, &'static str)],
}

pub struct Adapter {
    pub name: &'static str,
    pub aliases: &'static [AliasTree],
}

include!(concat!(env!("OUT_DIR"), "/adapters_gen.rs"));

/// The adapters a run can enable, by name (`--adapter <name>`).
pub fn names() -> Vec<&'static str> {
    ADAPTERS.iter().map(|a| a.name).collect()
}

fn find(name: &str) -> Result<&'static Adapter> {
    match ADAPTERS.iter().find(|a| a.name == name) {
        Some(adapter) => Ok(adapter),
        None => bail!("unknown adapter '{name}' — expected one of: {}", names().join(", ")),
    }
}

/// Writes `@rodeo` and the alias trees of `adapters` (with `@rodeo-bridge`)
/// under `dir`, returning each alias with the absolute folder that now holds
/// its modules.
pub fn write_alias_trees(dir: &Path, adapters: &[String]) -> Result<Vec<(&'static str, PathBuf)>> {
    let mut trees: Vec<&'static AliasTree> = vec![&RODEO];
    if !adapters.is_empty() {
        trees.push(&BRIDGE);
    }
    let mut enabled: Vec<&str> = Vec::new();
    for name in adapters {
        if enabled.contains(&name.as_str()) {
            continue;
        }
        enabled.push(name);
        for tree in find(name)?.aliases {
            if let Some(other) = trees.iter().find(|t| t.alias == tree.alias) {
                bail!("adapter '{name}' and another source both provide {}", other.alias);
            }
            trees.push(tree);
        }
    }

    let mut written = Vec::with_capacity(trees.len());
    for tree in trees {
        let root = dir.join(tree.alias.trim_start_matches('@'));
        for (rel, source) in tree.files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            std::fs::write(&path, source).with_context(|| format!("cannot write {}", path.display()))?;
        }
        let root = std::fs::canonicalize(&root)
            .with_context(|| format!("cannot resolve {}", root.display()))?;
        written.push((tree.alias, root));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rodeo_and_each_adapter_alias_are_distinct() {
        let mut aliases = vec![RODEO.alias, BRIDGE.alias];
        for adapter in ADAPTERS {
            for tree in adapter.aliases {
                assert!(!aliases.contains(&tree.alias), "{} is provided twice", tree.alias);
                aliases.push(tree.alias);
            }
        }
    }

    #[test]
    fn lune_and_lute_are_adapters() {
        assert!(names().contains(&"lune") && names().contains(&"lute"), "{:?}", names());
    }

    #[test]
    fn an_unknown_adapter_names_the_known_ones() {
        let dir = std::env::temp_dir().join(format!("rodeo-adapters-test-{}", uuid::Uuid::new_v4()));
        let err = write_alias_trees(&dir, &["nope".to_string()]).unwrap_err().to_string();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(err.contains("unknown adapter 'nope'") && err.contains("lune"), "{err}");
    }
}

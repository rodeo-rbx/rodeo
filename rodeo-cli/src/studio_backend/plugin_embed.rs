use std::path::Path;

use anyhow::{Context, Result};
use rbx_dom_weak::types::{Attributes, Variant};
use rbx_dom_weak::ustr;

/// Embedded plugin binary (pre-built .rbxm).
///
/// The bytes carry no launch-specific config. What varies per install is only
/// the pair of attributes [`plugin_bytes`] bakes onto the root instance: the
/// build id and the studio-backend port this file belongs to. A rodeo launch
/// additionally routes the plugin via the `rodeoPort`/`rodeoSession`
/// Workspace attributes its RunScript bootstrap sets.
const PLUGIN_BINARY: &[u8] = include_bytes!("../../../rodeo-plugin/build/plugin.rbxm");

/// The embedded plugin as built, with nothing baked in: what a host that is
/// not a Studio mounts (ree, under --ree, which runs the plugin's library).
pub fn embedded() -> &'static [u8] {
    PLUGIN_BINARY
}

/// Attribute names baked onto the plugin's root instance. The plugin reads
/// them as `script.Parent:GetAttribute(...)` in `init.server.luau`.
pub const BUILD_ATTR: &str = "rodeoBuild";
pub const PORT_ATTR: &str = "rodeoPort";

/// The embedded plugin with `build` and `port` baked in as attributes on its
/// root instance (the `rodeo_plugin` Folder). Deterministic for a given
/// pair, so an installed file can be byte-compared against it.
pub fn plugin_bytes(build: &str, port: u16) -> Result<Vec<u8>> {
    let mut dom = rbx_binary::from_reader(std::io::Cursor::new(PLUGIN_BINARY))
        .context("failed to parse embedded plugin")?;
    let top_level: Vec<_> = dom.root().children().to_vec();
    anyhow::ensure!(!top_level.is_empty(), "embedded plugin has no root instance");
    let key = ustr("Attributes");
    for referent in &top_level {
        let inst = dom.get_by_ref_mut(*referent).context("invalid plugin root ref")?;
        let mut attrs = match inst.properties.get(&key) {
            Some(Variant::Attributes(existing)) => existing.clone(),
            _ => Attributes::new(),
        };
        attrs.insert(BUILD_ATTR.to_string(), Variant::String(build.to_string()));
        attrs.insert(PORT_ATTR.to_string(), Variant::Float64(f64::from(port)));
        inst.properties.insert(key, Variant::Attributes(attrs));
    }
    let mut buf = Vec::new();
    rbx_binary::to_writer(&mut buf, &dom, &top_level).context("failed to serialize plugin")?;
    Ok(buf)
}

/// Whether the file at `path` is a plugin whose root carries exactly this
/// `build` and `port`. `rbx_binary` output is not byte-stable across calls
/// (property order varies), so the install compares meaning, not bytes.
pub fn installed_matches(path: &Path, build: &str, port: u16) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(dom) = rbx_binary::from_reader(std::io::Cursor::new(bytes)) else { return false };
    let key = ustr("Attributes");
    let mut roots = dom.root().children().iter();
    let Some(root) = roots.next().and_then(|r| dom.get_by_ref(*r)) else { return false };
    let Some(Variant::Attributes(attrs)) = root.properties.get(&key) else { return false };
    attr_string(attrs, BUILD_ATTR).as_deref() == Some(build)
        && attrs.get(PORT_ATTR) == Some(&Variant::Float64(f64::from(port)))
}

/// A string attribute's text. Roblox has one string attribute type; the
/// reader decodes it as `BinaryString`, so accept both spellings.
fn attr_string(attrs: &Attributes, key: &str) -> Option<String> {
    match attrs.get(key)? {
        Variant::String(s) => Some(s.clone()),
        Variant::BinaryString(b) => String::from_utf8(AsRef::<[u8]>::as_ref(b).to_vec()).ok(),
        _ => None,
    }
}

/// Write the baked plugin to `target_path`.
///
/// Idempotent: skips the write when the installed file already carries this
/// build and port. Studio reloads a plugin whenever its file changes on disk
/// — even a same-content rewrite bumps the mtime and triggers a reload — so a
/// backend returning to a port it held before must not churn Studios that
/// still have its plugin loaded (a `--detach` Studio, a hand-opened one).
pub fn write_plugin(target_path: &Path, build: &str, port: u16) -> Result<()> {
    if installed_matches(target_path, build, port) {
        return Ok(());
    }
    std::fs::write(target_path, plugin_bytes(build, port)?)
        .with_context(|| format!("failed to write plugin {}", target_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bakes_build_and_port_as_root_attributes() {
        let bytes = plugin_bytes("1.2.3+abcdef0", 46001).unwrap();
        let dom = rbx_binary::from_reader(std::io::Cursor::new(&bytes[..])).unwrap();
        let root_ref = dom.root().children()[0];
        let root = dom.get_by_ref(root_ref).unwrap();
        assert_eq!((root.class.as_str(), root.name.as_str()), ("Folder", "rodeo_plugin"));
        let Some(Variant::Attributes(attrs)) = root.properties.get(&ustr("Attributes")) else {
            panic!("root instance has no Attributes property");
        };
        assert_eq!(attr_string(attrs, BUILD_ATTR).as_deref(), Some("1.2.3+abcdef0"));
        assert_eq!(attrs.get(PORT_ATTR), Some(&Variant::Float64(46001.0)));
    }

    #[test]
    fn write_is_idempotent_by_meaning_not_bytes() {
        let dir = std::env::temp_dir().join(format!("rodeo-plugin-embed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rodeo-1.2.3+abcdef0-46001.rbxm");

        write_plugin(&path, "1.2.3+abcdef0", 46001).unwrap();
        assert!(installed_matches(&path, "1.2.3+abcdef0", 46001));
        assert!(!installed_matches(&path, "1.2.3+abcdef0", 46003));
        assert!(!installed_matches(&path, "9.9.9", 46001));

        // A second write for the same pair must leave the file alone: Studio
        // reloads on any mtime bump.
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_plugin(&path, "1.2.3+abcdef0", 46001).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);

        // A different pair rewrites it.
        write_plugin(&path, "1.2.3+abcdef0", 46003).unwrap();
        assert!(installed_matches(&path, "1.2.3+abcdef0", 46003));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

use regex::Regex;
use std::path::Path;

/// Resolve a script name to a file path.
///
/// An extensionless name resolves to `.rodeo/{name}.luau` when that file exists
/// — and `name` may include subdirectories (e.g. `benchmarks/foo` →
/// `.rodeo/benchmarks/foo.luau`), not just a shallow name. A name with an
/// extension (or a `./` / `../` prefix) is treated as a literal path and left
/// as-is. If the `.rodeo` file doesn't exist, the arg is returned unchanged.
pub fn resolve_script_path(script_arg: &str) -> String {
    if script_arg.contains('.') {
        return script_arg.to_string();
    }

    let rodeo_path = format!(".rodeo/{script_arg}.luau");
    if Path::new(&rodeo_path).is_file() {
        return rodeo_path;
    }

    script_arg.to_string()
}

/// Raw argv tokens parsed from a script's `@rodeo run …` directive header.
/// The parent splices `flag_args` into argv between the `run` subcommand and
/// user-supplied args; clap then handles validation, type conversion, and
/// override semantics. `script_args` (after the directive's `--`) are passed
/// through separately and applied only if the user didn't supply any.
pub struct DirectiveTokens {
    pub flag_args: Vec<String>,
    pub script_args: Vec<String>,
}

/// Parse `@rodeo run --flag value -- arg1 arg2` from a script header into
/// argv-style tokens. Returns None if no directive line is present.
///
/// The line splits into words as a `--ree="…"` value does
/// (`shared::words::split`): quotes keep a word with spaces together, so a
/// directive can carry `--ree="--roots game/shared;game/src"`. A line with a
/// quote left open splits on whitespace alone, as every directive did before
/// quotes meant anything (an apostrophe in a script argument).
pub fn parse_directive(content: &str) -> Option<DirectiveTokens> {
    // Match `@rodeo run [args]` on a single line. Only horizontal whitespace
    // (\t and space) between tokens — \s would let the regex jump to the next
    // line and capture script code as directive args. The args group is
    // optional so a bare `@rodeo run` directive parses to empty tokens.
    let re = Regex::new(r"@rodeo[ \t]+run[ \t]*([^\n\]]*)").ok()?;
    let captures = re.captures(content)?;
    let directive_str = captures.get(1)?.as_str().trim();
    if directive_str.is_empty() {
        return Some(DirectiveTokens { flag_args: Vec::new(), script_args: Vec::new() });
    }

    let mut flag_args = Vec::new();
    let mut script_args = Vec::new();
    let mut in_script_args = false;
    let words = crate::shared::words::split(directive_str)
        .unwrap_or_else(|_| directive_str.split_whitespace().map(str::to_string).collect());
    for tok in words {
        if !in_script_args && tok == "--" {
            in_script_args = true;
            continue;
        }
        if in_script_args {
            script_args.push(tok);
        } else {
            flag_args.push(tok);
        }
    }

    Some(DirectiveTokens { flag_args, script_args })
}

#[cfg(test)]
mod tests {
    use super::parse_directive;

    #[test]
    fn quotes_keep_a_directive_word_together() {
        let tokens = parse_directive("-- @rodeo run --context server --ree=\"--roots game/shared;game/src\" -- 'two words' three\nprint(1)").unwrap();
        assert_eq!(tokens.flag_args, ["--context", "server", "--ree=--roots game/shared;game/src"]);
        assert_eq!(tokens.script_args, ["two words", "three"]);
    }

    #[test]
    fn a_quote_left_open_splits_on_whitespace_alone() {
        let tokens = parse_directive("-- @rodeo run --show-return -- it's fine").unwrap();
        assert_eq!(tokens.flag_args, ["--show-return"]);
        assert_eq!(tokens.script_args, ["it's", "fine"]);
    }
}

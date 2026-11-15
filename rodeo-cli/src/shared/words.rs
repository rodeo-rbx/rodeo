//! A command line held in one string, split into its words: a script's
//! `@rodeo run …` directive, and the options `--ree="…"` hands to ree.

/// The words of `text`. Whitespace separates them, and `'…'` or `"…"` keeps
/// what it encloses, spaces included, in one word: the quotes are dropped and
/// a quoted part joins what touches it (`--roots='my game/src'` is one word,
/// `''` an empty one). Nothing else is special — a backslash is itself, so a
/// Windows path needs no doubling. `Err` says which quote is never closed.
pub fn split(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => word.get_or_insert_with(String::new).push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                word.get_or_insert_with(String::new);
            }
            None if c.is_whitespace() => words.extend(word.take()),
            None => word.get_or_insert_with(String::new).push(c),
        }
    }
    if let Some(open) = quote {
        return Err(format!("a {open} is never closed"));
    }
    words.extend(word);
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn whitespace_separates_words_and_quotes_keep_theirs() {
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split("  --offline\t--roots  game/shared;game/src ").unwrap(), ["--offline", "--roots", "game/shared;game/src"]);
        assert_eq!(split(r#"--roots 'my game/src' --player "A B""#).unwrap(), ["--roots", "my game/src", "--player", "A B"]);
        // A quoted part joins what touches it, and the other quote is plain inside.
        assert_eq!(split(r#"--name=it"'"s' a b'"#).unwrap(), ["--name=it's a b"]);
        assert_eq!(split("--player ''").unwrap(), ["--player", ""]);
    }

    #[test]
    fn a_backslash_is_itself() {
        assert_eq!(split(r"--project C:\games\mine 'C:\my games\x'").unwrap(), ["--project", r"C:\games\mine", r"C:\my games\x"]);
    }

    #[test]
    fn a_quote_left_open_is_an_error() {
        assert_eq!(split("--roots 'game/src").unwrap_err(), "a ' is never closed");
        assert_eq!(split(r#"--roots "game/src'"#).unwrap_err(), "a \" is never closed");
    }
}

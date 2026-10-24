//! Extra arguments the player types, split like a shell would.

/// Splits on whitespace, honouring `'…'`, `"…"` and `\` escapes.
pub fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (_, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    has = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (_, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_escapes() {
        assert_eq!(shell_split(r#"--a 1 --b "two words" 'x y' c\ d"#), ["--a", "1", "--b", "two words", "x y", "c d"]);
        assert_eq!(shell_split("  "), Vec::<String>::new());
        assert_eq!(shell_split(r#""""#), [""]);
    }
}

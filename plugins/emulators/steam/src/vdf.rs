//! Valve's text files (`.acf`, `.vdf`): quoted keys and values, and blocks in braces.
//!
//! ```text
//! "AppState"
//! {
//!     "appid"     "570"
//!     "name"      "Dota 2"
//! }
//! ```

/// A key's value: text, or a block of keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Block(Block),
}

/// The keys of a block, in the file's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block(pub Vec<(String, Value)>);

impl Block {
    /// The value of `key`, in any case: Steam writes `appid` in one file and `AppID` in another.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Text(text) => Some(text),
            Value::Block(_) => None,
        }
    }

    pub fn block(&self, key: &str) -> Option<&Block> {
        match self.get(key)? {
            Value::Block(block) => Some(block),
            Value::Text(_) => None,
        }
    }
}

/// A file's keys; `None` when it isn't one.
pub fn parse(text: &str) -> Option<Block> {
    let mut tokens = Tokens { rest: text };
    let block = read_block(&mut tokens, false)?;
    Some(block)
}

enum Token<'a> {
    Word(std::borrow::Cow<'a, str>),
    Open,
    Close,
}

struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            self.rest = self.rest.trim_start();
            // `// a comment` to the end of the line.
            if let Some(after) = self.rest.strip_prefix("//") {
                self.rest = after.split_once('\n').map_or("", |(_, rest)| rest);
                continue;
            }
            break;
        }
        let mut chars = self.rest.chars();
        match chars.next()? {
            '{' => {
                self.rest = &self.rest[1..];
                Some(Token::Open)
            }
            '}' => {
                self.rest = &self.rest[1..];
                Some(Token::Close)
            }
            '"' => {
                let body = &self.rest[1..];
                let mut word = String::new();
                let mut escaped = false;
                for (i, c) in body.char_indices() {
                    match (escaped, c) {
                        (true, 'n') => word.push('\n'),
                        (true, 't') => word.push('\t'),
                        (true, c) => word.push(c),
                        (false, '\\') => {
                            escaped = true;
                            continue;
                        }
                        (false, '"') => {
                            self.rest = &body[i + 1..];
                            return Some(Token::Word(word.into()));
                        }
                        (false, c) => word.push(c),
                    }
                    escaped = false;
                }
                // No closing quote: the file is cut short.
                self.rest = "";
                None
            }
            _ => {
                let end =
                    self.rest.find(|c: char| c.is_whitespace() || c == '{' || c == '}').unwrap_or(self.rest.len());
                let word = &self.rest[..end];
                self.rest = &self.rest[end..];
                Some(Token::Word(word.into()))
            }
        }
    }
}

/// Keys and values until the block's `}` (`inner`) or the end of the file.
fn read_block(tokens: &mut Tokens<'_>, inner: bool) -> Option<Block> {
    let mut block = Block::default();
    loop {
        let key = match tokens.next() {
            Some(Token::Word(key)) => key.into_owned(),
            Some(Token::Close) if inner => return Some(block),
            None if !inner => return Some(block),
            _ => return None,
        };
        let value = match tokens.next()? {
            Token::Word(text) => Value::Text(text.into_owned()),
            Token::Open => Value::Block(read_block(tokens, true)?),
            Token::Close => return None,
        };
        block.0.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_manifest_reads_as_its_keys() {
        let acf = r#"
"AppState"
{
	"appid"		"570"
	"name"		"Dota \"2\""
	// Steam's own note
	"StateFlags"		"4"
	"UserConfig"
	{
		"language"		"english"
	}
}
"#;
        let file = parse(acf).unwrap();
        let app = file.block("appstate").unwrap();
        assert_eq!(app.text("AppID"), Some("570"));
        assert_eq!(app.text("name"), Some(r#"Dota "2""#));
        assert_eq!(app.text("StateFlags"), Some("4"));
        assert_eq!(app.block("UserConfig").unwrap().text("language"), Some("english"));
    }

    #[test]
    fn a_file_cut_short_is_none() {
        assert_eq!(parse(r#""AppState" { "appid" "570""#), None);
        assert_eq!(parse(r#""AppState" { "name" "Dota"#), None);
    }
}

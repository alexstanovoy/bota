//! The rules version moves with the rules: a test over this crate's code as
//! its tokens, so comments and layout never count.

use std::path::{Path, PathBuf};

use crate::RULES_VERSION;

/// The rules version and the digests of the code it was set for.
#[derive(Debug, PartialEq, Eq)]
struct Pin {
    version: u32,
    /// Every number, string and character literal of the rules, in order.
    literals: u64,
    /// Every token of the rules, in order.
    code: u64,
}

const PIN: Pin = Pin {
    version: 24,
    literals: 0xa569_f75e_a3ab_af87,
    code: 0xa611_e72f_1d11_76e2,
};

/// A changed number or string is a changed rule: the version must move. Any
/// other change of the code fails until the pin is renewed, and its author
/// decides whether it changed what the rules decide.
#[test]
fn the_rules_version_moves_with_the_rules() {
    let now = digests();
    assert!(
        now.literals == PIN.literals || RULES_VERSION > PIN.version,
        "a number or a string of the rules changed: bump RULES_VERSION in fingerprint.rs, then \
         set PIN in this test to {now:x?}"
    );
    assert_eq!(
        now, PIN,
        "the rules' code changed: if it decides anything differently, bump RULES_VERSION; \
         either way set PIN to the left side"
    );
}

/// The digests of every source of the crate but its tests and the file the
/// version itself stands in, in path order.
fn digests() -> Pin {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    let (mut literals, mut code) = (Fnv::new(), Fnv::new());
    for path in files {
        let relative = path.strip_prefix(&src).expect("under src");
        let name = relative.to_str().expect("a UTF-8 path").replace('\\', "/");
        if name.starts_with("tests/") || name == "fingerprint.rs" {
            continue;
        }
        code.write(name.as_bytes());
        let text = std::fs::read_to_string(&path).expect("a source");
        for token in tokens(&text) {
            if token.literal {
                literals.write(token.text.as_bytes());
            }
            code.write(token.text.as_bytes());
        }
    }
    Pin {
        version: RULES_VERSION,
        literals: literals.0,
        code: code.0,
    }
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the crate's sources") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// FNV-1a, each written piece closed by a zero byte so tokens stay apart.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes.iter().chain(&[0]) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

struct Token {
    text: String,
    literal: bool,
}

/// Rust's tokens, without comments (doc comments too) and white space; a
/// mark of punctuation is a token of its own.
fn tokens(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let (end, literal) = token_end(&chars, at);
        let text: String = chars[at..end].iter().collect();
        if !text.trim().is_empty() && !text.starts_with("//") && !text.starts_with("/*") {
            out.push(Token { text, literal });
        }
        at = end;
    }
    out
}

/// Where the token starting at `at` ends, and whether it is a literal.
fn token_end(chars: &[char], at: usize) -> (usize, bool) {
    let c = chars[at];
    let next = chars.get(at + 1).copied();
    match c {
        _ if c.is_whitespace() => (at + 1, false),
        '/' if next == Some('/') => (find(chars, at, |c| c == '\n'), false),
        '/' if next == Some('*') => (block_comment_end(chars, at), false),
        '"' => (quoted_end(chars, at + 1, '"'), true),
        '\'' => char_or_lifetime_end(chars, at),
        _ if c.is_ascii_digit() => (number_end(chars, at), true),
        _ if c.is_alphabetic() || c == '_' => word_end(chars, at),
        _ => (at + 1, false),
    }
}

fn find(chars: &[char], from: usize, stop: impl Fn(char) -> bool) -> usize {
    (from..chars.len())
        .find(|&at| stop(chars[at]))
        .unwrap_or(chars.len())
}

fn block_comment_end(chars: &[char], mut at: usize) -> usize {
    let mut depth = 0;
    while at + 1 < chars.len() {
        match (chars[at], chars[at + 1]) {
            ('/', '*') => depth += 1,
            ('*', '/') => {
                depth -= 1;
                if depth == 0 {
                    return at + 2;
                }
            }
            _ => {
                at += 1;
                continue;
            }
        }
        at += 2;
    }
    chars.len()
}

/// The end of a quoted literal whose body starts at `at`, escapes honoured.
fn quoted_end(chars: &[char], mut at: usize, quote: char) -> usize {
    while at < chars.len() {
        match chars[at] {
            '\\' => at += 2,
            c if c == quote => return at + 1,
            _ => at += 1,
        }
    }
    chars.len()
}

/// A character literal (`'a'`, `'\n'`) or a lifetime or label (`'a`).
fn char_or_lifetime_end(chars: &[char], at: usize) -> (usize, bool) {
    if chars.get(at + 1) == Some(&'\\') || chars.get(at + 2) == Some(&'\'') {
        return (quoted_end(chars, at + 1, '\''), true);
    }
    (
        find(chars, at + 1, |c| !(c.is_alphanumeric() || c == '_')),
        false,
    )
}

fn number_end(chars: &[char], mut at: usize) -> usize {
    while at < chars.len() {
        let c = chars[at];
        let fraction = c == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit);
        if !(c.is_alphanumeric() || c == '_' || fraction) {
            break;
        }
        at += 1;
    }
    at
}

/// A word, or a prefixed literal it opens: `b"..."`, `b'.'`, `r#"..."#`.
fn word_end(chars: &[char], at: usize) -> (usize, bool) {
    let end = find(chars, at, |c| !(c.is_alphanumeric() || c == '_'));
    let word: String = chars[at..end].iter().collect();
    match (word.as_str(), chars.get(end)) {
        ("b" | "c", Some('"')) => (quoted_end(chars, end + 1, '"'), true),
        ("b", Some('\'')) => (quoted_end(chars, end + 1, '\''), true),
        ("r" | "br" | "cr", Some('"' | '#')) => (raw_end(chars, end), true),
        _ => (end, false),
    }
}

/// The end of a raw string whose hashes start at `at`.
fn raw_end(chars: &[char], at: usize) -> usize {
    let hashes = find(chars, at, |c| c != '#') - at;
    let mut close = vec!['"'];
    close.extend(std::iter::repeat_n('#', hashes));
    let body = at + hashes + 1;
    (body..chars.len())
        .find(|&end| chars[end..].starts_with(&close))
        .map_or(chars.len(), |end| end + close.len())
}

/// Layout and comments do not count; anything else does.
#[test]
fn a_comment_or_a_reformatting_leaves_the_tokens_as_they_were() {
    let texts = |source: &str| -> Vec<String> {
        tokens(source).into_iter().map(|token| token.text).collect()
    };
    let plain = texts("pub const X: [u8; 2] = [1, 0x2_0]; fn f<'a>(s: &'a str) -> char { 'x' }");
    let dressed = texts(
        "/// The doc.\npub const X : [ u8 ; 2 ] = [\n    1, // one\n    0x2_0 /* two /* deep */ */\n];\
         \nfn f < 'a > ( s : & 'a str ) -> char {\n    'x'\n}\n",
    );
    assert_eq!(plain, dressed);
    assert_ne!(
        plain,
        texts("pub const X: [u8; 2] = [1, 0x2_1]; fn f<'a>(s: &'a str) -> char { 'x' }")
    );
    assert_eq!(
        texts(r##"r#"a "quoted" // b"# "\" //""##),
        [r##"r#"a "quoted" // b"#"##, r#""\" //""#]
    );
}

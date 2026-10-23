//! What a line of chat and a shown name may hold: one policy for the server,
//! which refuses a line that breaks it and cleans a name, and for every
//! client, which checks before it sends and again on what it is told.
//!
//! Any Unicode text is welcome but for the characters that let a line pass
//! for something it is not: controls and line breaks (a forged second line),
//! bidirectional overrides (text that reads in another order than it is
//! stored), invisible and tag characters (hidden text), private-use and
//! noncharacters (no meaning shared by two clients), and stacks of combining
//! marks that spill over the lines around them.

use unicode_segmentation::UnicodeSegmentation;

use crate::rules;

/// Why a text may not be said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFault {
    Empty,
    /// It starts or ends with white space.
    Untrimmed,
    /// More bytes of UTF-8 than allowed.
    TooLong,
    /// More characters as a reader counts them (extended grapheme clusters)
    /// than allowed.
    TooManyGraphemes,
    /// It holds a character no text may hold, or a joiner outside a word.
    Forbidden(char),
    /// It stacks more than [`MAX_COMBINING_RUN`] combining diacritics.
    Stacked,
}

/// Combining diacritics allowed in a row: Vietnamese stacks two.
pub const MAX_COMBINING_RUN: usize = 2;

/// What is wrong with a chat line's text; nothing when it may be said.
pub fn chat_text_fault(text: &str) -> Option<TextFault> {
    text_fault(text, rules::MAX_CHAT_BYTES, rules::MAX_CHAT_GRAPHEMES)
}

/// What is wrong with a name a client announces or a server reserves a seat
/// under; nothing when it may stand.
pub fn name_fault(name: &str) -> Option<TextFault> {
    let most = rules::MAX_ANNOUNCED_NAME_BYTES;
    text_fault(name, most, most)
}

fn text_fault(text: &str, max_bytes: usize, max_graphemes: usize) -> Option<TextFault> {
    if text.is_empty() {
        return Some(TextFault::Empty);
    }
    if text.trim() != text {
        return Some(TextFault::Untrimmed);
    }
    if text.len() > max_bytes {
        return Some(TextFault::TooLong);
    }
    if let Some(fault) = char_fault(text) {
        return Some(fault);
    }
    (text.graphemes(true).nth(max_graphemes).is_some()).then_some(TextFault::TooManyGraphemes)
}

/// The first character of `text` that may not stand where it does.
fn char_fault(text: &str) -> Option<TextFault> {
    let chars: Vec<char> = text.chars().collect();
    let mut run = 0;
    for (at, &c) in chars.iter().enumerate() {
        if forbidden(c) || (is_joiner(c) && !joins(&chars, at)) {
            return Some(TextFault::Forbidden(c));
        }
        run = if is_combining_diacritic(c) {
            run + 1
        } else {
            0
        };
        if run > MAX_COMBINING_RUN {
            return Some(TextFault::Stacked);
        }
    }
    None
}

/// A name as it is shown: every character a name may not hold dropped,
/// combining stacks cut to [`MAX_COMBINING_RUN`], white space trimmed and
/// the rest cut to [`rules::MAX_SHOWN_NAME_BYTES`] at the last whole
/// character; `"?"` when nothing is left. A name that keeps the rules stands
/// as it is, cut.
pub fn clean_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut kept = String::new();
    let mut run = 0;
    for (at, &c) in chars.iter().enumerate() {
        if forbidden(c) || (is_joiner(c) && !joins(&chars, at)) {
            continue;
        }
        run = if is_combining_diacritic(c) {
            run + 1
        } else {
            0
        };
        if run <= MAX_COMBINING_RUN {
            kept.push(c);
        }
    }
    let mut shown = kept.trim().to_owned();
    shown.truncate(shown.floor_char_boundary(rules::MAX_SHOWN_NAME_BYTES));
    let shown = shown.trim_end().trim_end_matches(is_joiner).trim_end();
    match shown {
        "" => "?".to_owned(),
        shown => shown.to_owned(),
    }
}

/// Zero width non-joiner and joiner: needed inside words of several scripts
/// and in emoji sequences, invisible anywhere else.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Whether the joiner at `at` stands between two visible characters.
fn joins(chars: &[char], at: usize) -> bool {
    let visible = |c: &char| !c.is_whitespace() && !is_joiner(*c) && !forbidden(*c);
    at > 0 && chars.get(at - 1).is_some_and(visible) && chars.get(at + 1).is_some_and(visible)
}

/// Characters no text may hold anywhere.
fn forbidden(c: char) -> bool {
    let code = u32::from(c);
    c.is_control()
        || is_format(code)
        || matches!(code, 0x2028 | 0x2029)
        || (0xFDD0..=0xFDEF).contains(&code)
        || (code & 0xFFFE) == 0xFFFE
        || (0xE000..=0xF8FF).contains(&code)
        || (0xF_0000..=0x10_FFFF).contains(&code)
}

/// Unicode's format characters (general category Cf) but the two joiners:
/// invisible, and among them the bidirectional marks, embeddings, overrides
/// and isolates, and the tag characters.
fn is_format(code: u32) -> bool {
    matches!(
        code,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B
            | 0x200E..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// The combining diacritical mark blocks, whose marks stack over a letter.
fn is_combining_diacritic(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

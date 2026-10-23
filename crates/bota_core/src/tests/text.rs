//! The text policy: every script and emoji passes, nothing that forges,
//! hides or reorders text does. One table for the server and every client.

use crate::{TextFault, chat_text_fault, clean_name, rules};

#[test]
fn any_script_and_emoji_may_be_said() {
    for text in [
        "gg wp",
        "мид не трогай, я сам",
        "中路 下路 请支援",
        "ミッドに行く",
        "한국어 채팅",
        "مرحبا بالجميع",
        "שלום לכולם",
        "नमस्ते दोस्तों",
        "क्‍ष", // Devanagari with a joiner inside a word
        "Tiếng Việt có dấu",
        "👍 👨‍👩‍👧 🇺🇦 ❤️",       // a ZWJ family, a flag and a variation selector
        "e\u{301}\u{302} x", // two combining marks stacked
    ] {
        assert_eq!(chat_text_fault(text), None, "{text:?}");
    }
}

#[test]
fn what_forges_hides_or_reorders_a_line_is_refused() {
    use TextFault::*;
    for (text, fault) in [
        ("", Empty),
        (" gg", Untrimmed),
        ("gg\u{3000}", Untrimmed),
        ("a\nb", Forbidden('\n')),
        ("a\tb", Forbidden('\t')),
        ("a\u{85}b", Forbidden('\u{85}')),
        ("a\u{2028}b", Forbidden('\u{2028}')),
        ("abc\u{202E}def", Forbidden('\u{202E}')),
        ("a\u{2067}b", Forbidden('\u{2067}')),
        ("a\u{200F}b", Forbidden('\u{200F}')),
        ("a\u{061C}b", Forbidden('\u{061C}')),
        ("a\u{200B}b", Forbidden('\u{200B}')),
        ("a\u{FEFF}b", Forbidden('\u{FEFF}')),
        ("a\u{E0041}b", Forbidden('\u{E0041}')),
        ("a\u{E000}b", Forbidden('\u{E000}')),
        ("a\u{FDD0}b", Forbidden('\u{FDD0}')),
        ("a\u{FFFF}b", Forbidden('\u{FFFF}')),
        ("a \u{200D}b", Forbidden('\u{200D}')),
        ("\u{200C}ab", Forbidden('\u{200C}')),
        ("e\u{301}\u{302}\u{303}", Stacked),
    ] {
        assert_eq!(chat_text_fault(text), Some(fault), "{text:?}");
    }
}

#[test]
fn a_line_is_bounded_in_bytes_and_in_what_a_reader_counts() {
    let most = rules::MAX_CHAT_GRAPHEMES;
    assert_eq!(chat_text_fault(&"ж".repeat(most)), None);
    let graphemes = "ж".repeat(most + 1);
    assert_eq!(
        chat_text_fault(&graphemes),
        Some(TextFault::TooManyGraphemes)
    );
    // A family emoji is one character to a reader and eighteen bytes.
    let family = "👨‍👩‍👧".repeat(rules::MAX_CHAT_BYTES / 18 + 1);
    assert_eq!(chat_text_fault(&family), Some(TextFault::TooLong));
}

#[test]
fn a_name_is_cleaned_trimmed_and_cut_rather_than_refused() {
    assert_eq!(clean_name("Анна"), "Анна");
    assert_eq!(clean_name(" \u{202E}admin\u{202C} "), "admin");
    assert_eq!(clean_name("a\nb\u{200B}c"), "abc");
    assert_eq!(clean_name("\u{200D}\u{202E}"), "?");
    let long = clean_name(&"名".repeat(20));
    assert_eq!(long, "名".repeat(rules::MAX_SHOWN_NAME_BYTES / 3));
}

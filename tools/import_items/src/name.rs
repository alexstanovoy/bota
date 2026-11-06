//! Dota's internal names as the tool takes them from an install: each goes into the generated
//! Rust as it is, a const's name and a string literal, so only what Dota's names are made of is
//! taken. A modded or corrupt install that holds anything else fails the import.

/// The longest name taken; Dota's are a few dozen characters.
const MAX_LEN: usize = 96;

/// `text` when it is a name as Dota writes them: one to [`MAX_LEN`] ASCII letters, digits and
/// underscores.
pub fn checked(text: &str) -> Result<&str, String> {
    if text.is_empty() || text.len() > MAX_LEN {
        return Err(format!(
            "{:?} is no name of 1 to {MAX_LEN} characters",
            text.chars().take(MAX_LEN).collect::<String>()
        ));
    }
    if let Some(bad) = text
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
    {
        return Err(format!(
            "{text:?} holds {bad:?}: a name is ASCII letters, digits and underscores"
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::checked;

    /// A name that would close a string or a block of the generated Rust is refused, saying
    /// why.
    #[test]
    fn only_letters_digits_and_underscores_make_a_name() {
        assert_eq!(
            checked("special_bonus_unique_pudge_7"),
            Ok("special_bonus_unique_pudge_7")
        );
        assert_eq!(
            checked(r#"x", panic!(), ""#),
            Err(
                r#""x\", panic!(), \"" holds '"': a name is ASCII letters, digits and underscores"#
                    .to_owned()
            )
        );
        for bad in ["", "a}b", "a b", "naïve", &"n".repeat(97)] {
            assert!(checked(bad).is_err(), "{bad:?}");
        }
    }
}

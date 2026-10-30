//! Numbers as Dota's scripts write them: decimal text, read exactly.

/// The longest number read; Dota's are a handful of digits.
const MAX_LEN: usize = 16;

/// Thousandths of a number: `"0.25"` is 250. Bota's `Dec` holds the same.
pub type Milli = i64;

/// A decimal with at most three places (`"-15"`, `"0.25"`, `"7.0"`), in thousandths; anything
/// else is refused.
pub fn milli(text: &str) -> Result<Milli, String> {
    let bad = || format!("{text:?} is no number of at most three decimal places");
    if text.is_empty() || text.len() > MAX_LEN {
        return Err(bad());
    }
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((_, "")) => return Err(bad()),
        Some(parts) => parts,
        None => (digits, ""),
    };
    let is_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !is_digits(whole) || !is_digits(fraction) || fraction.len() > 3 {
        return Err(bad());
    }
    let whole: Milli = whole.parse().map_err(|_| bad())?;
    let mut thousandths: Milli = 0;
    for (place, digit) in fraction.bytes().enumerate() {
        thousandths += Milli::from(digit - b'0') * [100, 10, 1][place];
    }
    let value = whole * 1000 + thousandths;
    Ok(if negative { -value } else { value })
}

/// A whole number (`"1200"`, or `"15.0"` written with a zero fraction).
pub fn whole(text: &str) -> Result<i64, String> {
    let value = milli(text)?;
    if value % 1000 != 0 {
        return Err(format!("{text:?} is no whole number"));
    }
    Ok(value / 1000)
}

/// One number a level, as a script lists them: `"35 50 65 80"`.
pub fn levels(text: &str) -> Result<Vec<Milli>, String> {
    let levels: Vec<Milli> = text
        .split_whitespace()
        .map(milli)
        .collect::<Result<_, _>>()?;
    if levels.is_empty() || levels.len() > 8 {
        return Err(format!("{text:?} is no list of one to eight levels"));
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_number_reads_exactly_or_not_at_all() {
        assert_eq!(milli("0.25"), Ok(250));
        assert_eq!(milli("-15"), Ok(-15_000));
        assert_eq!(milli("7.0"), Ok(7000));
        assert_eq!(
            levels("35 50 65 80"),
            Ok(vec![35_000, 50_000, 65_000, 80_000])
        );
        for bad in [
            "",
            "1e3",
            "0.0001",
            "+5",
            ".5",
            "5.",
            "1.2.3",
            "=12",
            "99999999999999999",
        ] {
            assert!(milli(bad).is_err(), "{bad:?}");
        }
        assert!(whole("0.5").is_err());
    }
}

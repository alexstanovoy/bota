//! Text templates filled from the rules' own numbers: a `{key}` in a
//! template stands for a value of `bota_core`, so a blurb never states a
//! number the rules could change.

mod item;
mod keys;

use bota_core::{AbilityDef, Ratio, rules};
use bota_proto::{Fixed, ItemId};

/// What the keys of a template are read against besides the shared ones.
#[derive(Clone, Copy)]
pub(crate) enum Scope {
    /// Only the shared keys.
    Shared,
    /// An ability's own range, cost, wait and cast point.
    Ability(&'static AbilityDef),
    /// An item's own price, uses and carried bonuses.
    Item(ItemId),
}

/// The template with every `{key}` replaced by its value.
///
/// Panics on a key nothing answers to; the catalog's tests fill every
/// template, so a typo never reaches a player.
pub(crate) fn fill(template: &str, scope: Scope) -> String {
    let mut text = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        text.push_str(&rest[..open]);
        let close = rest[open..]
            .find('}')
            .map_or_else(|| panic!("unclosed key in {template:?}"), |at| open + at);
        let key = &rest[open + 1..close];
        let value = keys::value(scope, key)
            .unwrap_or_else(|| panic!("no value for {{{key}}} in {template:?}"));
        text.push_str(&value);
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    text
}

/// Hundredths as a number with no trailing zeros: 150 is "1.5", 200 is "2".
pub(crate) fn hundredths(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let (whole, part) = (value.abs() / 100, value.abs() % 100);
    match part {
        0 => format!("{sign}{whole}"),
        part if part % 10 == 0 => format!("{sign}{whole}.{}", part / 10),
        part => format!("{sign}{whole}.{part:02}"),
    }
}

/// Seconds that `ticks` last.
pub(crate) fn secs(ticks: u32) -> String {
    let rate = i64::from(rules::TICKS_PER_SECOND);
    hundredths((i64::from(ticks) * 100 + rate / 2) / rate)
}

/// Seconds that `ms` milliseconds last.
pub(crate) fn secs_ms(ms: u32) -> String {
    hundredths((i64::from(ms) + 5) / 10)
}

/// A fixed-point value to the nearest hundredth.
pub(crate) fn fixed(value: Fixed) -> String {
    hundredths((i64::from(value.raw) * 100 + (1 << (Fixed::FRAC_BITS - 1))) >> Fixed::FRAC_BITS)
}

/// A per-tick rate as the number a second gives.
pub(crate) fn per_second(per_tick: Fixed) -> String {
    fixed(per_tick * Fixed::from_int(rules::TICKS_PER_SECOND as i32))
}

/// Tenths of a point.
pub(crate) fn tenths(value: i32) -> String {
    hundredths(i64::from(value) * 10)
}

/// Per-level values joined by slashes; one value when all are the same.
pub(crate) fn slash<T: Copy + PartialEq>(values: &[T], show: impl Fn(T) -> String) -> String {
    if values.windows(2).all(|pair| pair[0] == pair[1]) {
        return values.first().map_or_else(String::new, |v| show(*v));
    }
    values
        .iter()
        .map(|v| show(*v))
        .collect::<Vec<_>>()
        .join("/")
}

/// Per-level whole numbers joined by slashes.
pub(crate) fn nums(values: &[i32]) -> String {
    slash(values, |v| v.to_string())
}

/// A share as whole percent.
pub(crate) fn percent(ratio: Ratio) -> String {
    (i32::from(ratio.num()) * 100 / i32::from(ratio.den())).to_string()
}

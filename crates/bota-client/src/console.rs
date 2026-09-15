//! The console line: a typed command turned into the order it asks for.

use bota_proto::{Cheat, ItemId, Order};

/// Turns one typed line into an order. A leading dash is allowed and
/// ignored, so `-gold 500` and `gold 500` ask the same.
///
/// Commands: `gold [amount]`, `lvlup [count]`, `refresh`, `item <id or
/// name>`.
pub fn parse(line: &str) -> Result<Order, String> {
    let mut words = line.trim().trim_start_matches('-').split_whitespace();
    let Some(word) = words.next() else {
        return Err("nothing typed".to_string());
    };
    let cheat = match word {
        "gold" => Cheat::Gold {
            amount: number(words.next(), 1000)?,
        },
        "lvlup" | "levelup" | "level" => Cheat::Levels {
            count: number(words.next(), 1)?,
        },
        "refresh" => Cheat::Refresh,
        "item" => Cheat::Item {
            item: item_named(words.next().ok_or("which item?")?)?,
        },
        other => return Err(format!("unknown command '{other}'")),
    };
    Ok(Order::Cheat { cheat })
}

/// The number typed after a command, or the usual one when none was.
fn number<T: std::str::FromStr>(word: Option<&str>, usual: T) -> Result<T, String> {
    match word {
        None => Ok(usual),
        Some(word) => word
            .parse()
            .map_err(|_| format!("'{word}' is not a number")),
    }
}

/// An item by its id or by the name the catalog shows, case aside.
fn item_named(word: &str) -> Result<ItemId, String> {
    if let Ok(id) = word.parse::<u16>() {
        return crate::catalog::item(id)
            .map(|_| ItemId(id))
            .ok_or_else(|| format!("no item {id}"));
    }
    crate::catalog::ITEMS
        .iter()
        .find(|face| face.name.eq_ignore_ascii_case(word))
        .map(|face| ItemId(face.id))
        .ok_or_else(|| format!("no item called '{word}'"))
}

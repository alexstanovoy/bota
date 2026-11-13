//! bota holds no player text: an item is known by its id and Dota's internal key, and what a
//! player reads (its name, its words) is the client's, from the player's install or its own table.

/// The item table's sources, as the build reads them.
const DATA: [(&str, &str); 2] = [
    ("data/items.rs", include_str!("../data/items.rs")),
    (
        "data/item_mechanics.rs",
        include_str!("../data/item_mechanics.rs"),
    ),
];

/// `source` without its comments and without the arguments of its `assert!` and `panic!` calls,
/// which speak to a developer whose build fails.
fn without_comments_and_assertions(source: &str) -> String {
    let lines: Vec<&str> = source
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect();
    let text = lines.join("\n");
    let mut kept = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = ["assert!(", "panic!("]
        .iter()
        .filter_map(|call| rest.find(call).map(|at| (at, call.len())))
        .min()
    {
        let (start, open) = at;
        kept.push_str(&rest[..start]);
        let mut depth = 1;
        let mut end = start + open;
        for (offset, byte) in rest[start + open..].bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = start + open + offset + 1;
                break;
            }
        }
        assert_eq!(depth, 0, "an unclosed assertion");
        rest = &rest[end..];
    }
    kept.push_str(rest);
    kept
}

/// Every string the item table holds is a Dota key or a number (`item_yasha`,
/// `movement_speed_percent_bonus`, `2.25`): no display name, recipe title or other words a player
/// would read, under whatever field. A row that names an item in English fails here; its name
/// belongs to the client's table, keyed by the item's key.
#[test]
fn the_item_table_holds_keys_and_numbers_and_no_player_text() {
    for (file, source) in DATA {
        let code = without_comments_and_assertions(source);
        let mut rest = code.as_str();
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let close = after.find('"').expect("a closed string");
            let text = &after[..close];
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)),
                "{file}: {text:?} is no Dota key or number"
            );
            rest = &after[close + 1..];
        }
    }
}

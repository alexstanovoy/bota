//! The item lines of bota's id registry (`crates/bota_proto/registry/ids.txt`).

use heatel::keyvalues::Kv;

/// One registered item.
pub struct Registered {
    pub id: u16,
    pub key: String,
    pub retired: bool,
}

/// The registry's items, in its order.
pub fn items(registry: &str) -> Result<Vec<Registered>, String> {
    registry
        .lines()
        .filter(|line| line.starts_with("item "))
        .map(|line| {
            let words: Vec<&str> = line.split(' ').collect();
            let [_, id, state, key] = words[..] else {
                return Err(format!("malformed registry line {line:?}"));
            };
            Ok(Registered {
                id: id
                    .parse()
                    .map_err(|_| format!("malformed registry line {line:?}"))?,
                key: key.to_owned(),
                retired: match state {
                    "live" => false,
                    "retired" => true,
                    _ => return Err(format!("malformed registry line {line:?}")),
                },
            })
        })
        .collect()
}

/// The registry with a line appended for each of `keys`, each an item of `items` (the
/// `DOTAAbilities` block) that holds no id yet. Lines are append-only: the next id of the space.
pub fn append(registry: &str, keys: &[String], items: &Kv) -> Result<String, String> {
    let mut registry = registry.to_owned();
    for key in keys {
        let known = self::items(&registry)?;
        if known.iter().any(|item| item.key == *key) {
            return Err(format!("{key} is registered already"));
        }
        if !key.starts_with("item_") || items.get(key).and_then(Kv::block).is_none() {
            return Err(format!("{key} is no item of items.txt"));
        }
        let next = known.iter().map(|item| item.id + 1).max().unwrap_or(0);
        let mut lines: Vec<String> = registry.lines().map(str::to_owned).collect();
        let last_item = lines
            .iter()
            .rposition(|line| line.starts_with("item "))
            .ok_or("the registry has no item line to append after")?;
        lines.insert(last_item + 1, format!("item {next} live {key}"));
        registry = lines.join("\n") + "\n";
    }
    Ok(registry)
}

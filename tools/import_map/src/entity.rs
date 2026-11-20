//! Reading one entity: its class, its name, where it stands and how it is turned.

use heatel::kv3::Value;

use crate::frame::Shifted;

/// What the map compiler puts before the names of entities that came from a prefab.
const PREFAB_MARK: &str = "[PR#]";

/// One of the map's entities.
#[derive(Clone, Copy)]
pub struct Entity<'a>(pub &'a Value);

impl<'a> Entity<'a> {
    pub fn class(&self) -> &'a str {
        self.text("classname")
    }

    /// Its `targetname`, without the prefab mark.
    pub fn name(&self) -> &'a str {
        let name = self.text("targetname");
        name.strip_prefix(PREFAB_MARK).unwrap_or(name)
    }

    /// The value of a text key, empty when absent.
    pub fn text(&self, key: &str) -> &'a str {
        self.0.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    /// The map editor's id of the entity (`hammerUniqueId`): what the generated tables cite.
    pub fn id(&self) -> &'a str {
        self.text("hammerUniqueId")
    }

    /// Where it stands, in bota's frame.
    pub fn at(&self) -> Result<[f64; 3], String> {
        Ok(self.vector("origin")?.shifted())
    }

    /// Its yaw, in degrees as the map has it.
    pub fn yaw(&self) -> Result<f64, String> {
        let angles = self.vector("angles")?;
        if angles[0] != 0.0 || angles[2] != 0.0 {
            return Err(format!("{} {}: tilted", self.class(), self.id()));
        }
        Ok(angles[1])
    }

    /// Whether it is drawn at its own size: no `scales`, or all three one.
    pub fn unscaled(&self) -> Result<bool, String> {
        match self.0.get("scales") {
            None => Ok(true),
            Some(_) => Ok(self.vector("scales")? == [1.0; 3]),
        }
    }

    fn vector(&self, key: &str) -> Result<[f64; 3], String> {
        let values = self
            .0
            .get(key)
            .and_then(Value::as_array)
            .filter(|values| values.len() == 3)
            .ok_or(format!("{} {}: no {key}", self.class(), self.id()))?;
        let mut out = [0.0; 3];
        for (out, value) in out.iter_mut().zip(values) {
            *out = value
                .as_f64()
                .filter(|v| v.is_finite() && v.abs() < 1e6)
                .ok_or(format!("{} {}: bad {key}", self.class(), self.id()))?;
        }
        Ok(out)
    }
}

/// The entities of one class, in the map's order.
pub fn of_class<'a>(entities: &'a [Value], class: &'a str) -> impl Iterator<Item = Entity<'a>> {
    entities
        .iter()
        .map(Entity)
        .filter(move |e| e.class() == class)
}

/// The one entity of a class with a name.
pub fn named<'a>(entities: &'a [Value], class: &str, name: &str) -> Result<Entity<'a>, String> {
    let mut found = entities
        .iter()
        .map(Entity)
        .filter(|e| e.class() == class && e.name() == name);
    let first = found.next().ok_or(format!("no {class} {name}"))?;
    match found.next() {
        None => Ok(first),
        Some(_) => Err(format!("two {class} {name}")),
    }
}

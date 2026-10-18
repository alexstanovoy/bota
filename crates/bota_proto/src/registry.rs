//! Every id ever assigned on the wire, as `registry/ids.txt` records it.
//!
//! Ids are append-only, like protobuf field numbers: one never changes what
//! it names and is never reused. One that leaves the game stays in the
//! registry, retired.

/// The spaces ids are drawn from, one numbering each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdSpace {
    /// [`HeroId`](crate::HeroId)s.
    Hero,
    /// [`AbilityId`](crate::AbilityId)s.
    Ability,
    /// [`ItemId`](crate::ItemId)s.
    Item,
    /// [`EffectId`](crate::EffectId)s.
    Effect,
    /// [`NeutralId`](crate::NeutralId)s.
    Neutral,
}

/// One id as the registry has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisteredId {
    /// Which numbering it belongs to.
    pub space: IdSpace,
    /// The number itself.
    pub id: u16,
    /// Whether it was taken out of the game: then nothing may name it again.
    pub retired: bool,
    /// What it names, for as long as the protocol lives.
    pub name: &'static str,
}

/// The registry as committed.
const IDS: &str = include_str!("../registry/ids.txt");

/// Every registered id of `space`, in registry order.
///
/// # Panics
///
/// On a malformed line; the crate's own tests keep the committed registry
/// well-formed.
pub fn registered(space: IdSpace) -> impl Iterator<Item = RegisteredId> {
    IDS.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| parse(line).unwrap_or_else(|| panic!("malformed id registry line {line:?}")))
        .filter(move |entry| entry.space == space)
}

fn parse(line: &'static str) -> Option<RegisteredId> {
    let mut words = line.splitn(4, ' ');
    let space = match words.next()? {
        "hero" => IdSpace::Hero,
        "ability" => IdSpace::Ability,
        "item" => IdSpace::Item,
        "effect" => IdSpace::Effect,
        "neutral" => IdSpace::Neutral,
        _ => return None,
    };
    let id = words.next()?.parse().ok()?;
    let retired = match words.next()? {
        "live" => false,
        "retired" => true,
        _ => return None,
    };
    let name = words.next().filter(|name| !name.trim().is_empty())?;
    Some(RegisteredId {
        space,
        id,
        retired,
        name,
    })
}

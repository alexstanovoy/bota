//! What a match's host decides of the map beyond its geometry: which of its
//! objectives stand and which runes spawn. Each rule is a set of kinds, one
//! bit a kind; a kind's bit is its variant index, so kinds are append-only
//! like every other wire enum, and a set has room for sixteen.

use serde::{Deserialize, Serialize};

/// A set of kinds of one rule, one bit a kind.
macro_rules! kind_set {
    ($(#[$doc:meta])* $set:ident of $kind:ident) => {
        $(#[$doc])*
        #[derive(
            Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
        )]
        pub struct $set {
            /// Bit `n` holds the kind whose variant index is `n`.
            pub bits: u16,
        }

        impl $set {
            /// No kind at all.
            pub const NONE: $set = $set { bits: 0 };

            /// Every kind there is.
            pub const ALL: $set = $set {
                bits: (1 << $kind::ALL.len()) - 1,
            };

            /// The set of one kind.
            pub const fn of(kind: $kind) -> $set {
                $set { bits: 1 << kind as u16 }
            }

            /// Whether the set holds a kind.
            pub const fn has(self, kind: $kind) -> bool {
                self.bits & $set::of(kind).bits != 0
            }

            /// The set with a kind added.
            pub const fn with(self, kind: $kind) -> $set {
                $set {
                    bits: self.bits | $set::of(kind).bits,
                }
            }

            /// The kinds both sets hold.
            pub const fn and(self, other: $set) -> $set {
                $set {
                    bits: self.bits & other.bits,
                }
            }

            /// The kinds this set holds and `other` does not, bits that name
            /// no kind included.
            pub const fn without(self, other: $set) -> $set {
                $set {
                    bits: self.bits & !other.bits,
                }
            }

            /// Whether the set holds nothing.
            pub const fn is_empty(self) -> bool {
                self.bits == 0
            }

            /// Whether every bit names a kind.
            pub const fn is_valid(self) -> bool {
                self.without($set::ALL).is_empty()
            }

            /// The kinds it holds, in variant order.
            pub fn iter(self) -> impl Iterator<Item = $kind> {
                $kind::ALL.into_iter().filter(move |kind| self.has(*kind))
            }

            /// The set named by a list of kind names separated by commas, or
            /// `all`, or `none`.
            pub fn parse(text: &str) -> Result<$set, String> {
                match text.trim() {
                    "all" => return Ok($set::ALL),
                    "none" | "" => return Ok($set::NONE),
                    _ => {}
                }
                text.split(',').try_fold($set::NONE, |set, name| {
                    let name = name.trim();
                    $kind::ALL
                        .into_iter()
                        .find(|kind| kind.name() == name)
                        .map(|kind| set.with(kind))
                        .ok_or_else(|| format!("no such kind: {name:?}"))
                })
            }
        }

        impl core::fmt::Display for $set {
            /// The names of its kinds, separated by commas; `none` for none.
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                if self.is_empty() {
                    return f.write_str("none");
                }
                for (at, kind) in self.iter().enumerate() {
                    if at > 0 {
                        f.write_str(",")?;
                    }
                    f.write_str(kind.name())?;
                }
                Ok(())
            }
        }
    };
}

/// One of the map's objectives: what stands on the map besides the lanes,
/// the bases and the jungle camps, as Dota 7.41f has them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectiveKind {
    /// Roshan in his pit, and what he drops.
    Roshan,
    /// The Tormentors in their chasms.
    Tormentor,
    /// The Watchers: lanterns a hero lights for its side's vision.
    Watchers,
    /// The Outposts: buildings a side captures and teleports to.
    Outposts,
    /// The Shrines of Wisdom: experience for the side that gathers one.
    Shrines,
}

impl ObjectiveKind {
    /// Every kind, in variant order: bit `n` of an [`ObjectiveSet`] is
    /// `ALL[n]`.
    pub const ALL: [ObjectiveKind; 5] = [
        ObjectiveKind::Roshan,
        ObjectiveKind::Tormentor,
        ObjectiveKind::Watchers,
        ObjectiveKind::Outposts,
        ObjectiveKind::Shrines,
    ];

    /// What a host calls it.
    pub const fn name(self) -> &'static str {
        match self {
            ObjectiveKind::Roshan => "roshan",
            ObjectiveKind::Tormentor => "tormentor",
            ObjectiveKind::Watchers => "watchers",
            ObjectiveKind::Outposts => "outposts",
            ObjectiveKind::Shrines => "shrines",
        }
    }
}

kind_set!(
    /// Which of the map's objectives a match stands up.
    ObjectiveSet of ObjectiveKind
);

/// One kind of rune, Dota 7.41f's.
///
/// Water and bounty runes take effect at once; the rest are power runes,
/// one of which spawns at a river spot every two minutes from 6:00.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuneKind {
    /// Mends health and mana at once; at the river spots at 2:00 and 4:00.
    Water,
    /// Gold to whoever takes it; at the jungle bounty spots every 4:00.
    Bounty,
    /// Moves at the haste speed and cannot be slowed below it.
    Haste,
    /// More attack damage and spell damage: Dota's old Double Damage.
    AmplifyDamage,
    /// Shorter cooldowns and cheaper spells.
    Arcane,
    /// Mends a share of health and mana every second until both are full.
    Regeneration,
    /// Two illusions of whoever takes it.
    Illusion,
    /// Fades out of sight until it attacks or casts.
    Invisibility,
    /// A barrier that takes damage before health does.
    Shield,
}

impl RuneKind {
    /// Every kind, in variant order: bit `n` of a [`RuneSet`] is `ALL[n]`.
    pub const ALL: [RuneKind; 9] = [
        RuneKind::Water,
        RuneKind::Bounty,
        RuneKind::Haste,
        RuneKind::AmplifyDamage,
        RuneKind::Arcane,
        RuneKind::Regeneration,
        RuneKind::Illusion,
        RuneKind::Invisibility,
        RuneKind::Shield,
    ];

    /// Whether it is a power rune: one of the two-minute river runes.
    pub const fn is_power(self) -> bool {
        !matches!(self, RuneKind::Water | RuneKind::Bounty)
    }

    /// What a host calls it.
    pub const fn name(self) -> &'static str {
        match self {
            RuneKind::Water => "water",
            RuneKind::Bounty => "bounty",
            RuneKind::Haste => "haste",
            RuneKind::AmplifyDamage => "amplify_damage",
            RuneKind::Arcane => "arcane",
            RuneKind::Regeneration => "regeneration",
            RuneKind::Illusion => "illusion",
            RuneKind::Invisibility => "invisibility",
            RuneKind::Shield => "shield",
        }
    }
}

kind_set!(
    /// Which kinds of rune a match spawns.
    RuneSet of RuneKind
);

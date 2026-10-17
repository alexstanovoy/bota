//! One module for each hero: its numbers, its body, its abilities, its
//! talents and the pure functions of them. [`HEROES`](crate::HEROES) and
//! [`ABILITIES`](crate::ABILITIES) name what these modules hold.
//!
//! # Adding a hero
//!
//! 1. Register the ids: `HeroId::X`, each `AbilityId::*` and each
//!    `EffectId::*` in `bota_proto/src/ids.rs`, and one line apiece in
//!    `bota_proto/registry/ids.txt` (append-only).
//! 2. Copy a hero directory here to `heroes/x/`: `numbers.rs` (its constants,
//!    re-exported as `rules::NAME`), `kit.rs` (its `UnitDef`, one `AbilityDef`
//!    const per ability, its talent tree if it has one, and `HERO_DEF`),
//!    `formulas.rs` for pure functions the clients read, and `mod.rs`. Name
//!    the module in this file and in `rules.rs` (`pub use ...::numbers::*`).
//! 3. Add its `HERO_DEF` to `HEROES` in `hero.rs` (bump its length) and its ability consts to
//!    `ABILITIES` in `ability.rs`, in id order, and bump `ABILITY_COUNT`.
//!    A new modifier is a `ModifierKind` variant (`modifier.rs`) with its
//!    [`effect_id`](crate::ModifierKind::effect_id) arm, and its effect id gets
//!    an [`EFFECTS`](crate::EFFECTS) row in `effect.rs` (bump `EFFECT_COUNT`).
//! 4. In `bota_server`: one `ABILITY_HOOKS` row an ability
//!    (`game/config/ability.rs`), the behaviour in `game/systems/x.rs`
//!    (named in `systems/mod.rs`), the modifier's `hash.rs` and `stats.rs`
//!    arms, and the end-to-end tests in `game/tests/x.rs`.
//! 5. Talents also need the `Talent` variants in `talent.rs` and the server
//!    sites that ask `has_talent`.
//! 6. For a front end: a short label and blurb for each ability and an
//!    `EffectFace` for each effect in `bota_client_core/src/catalog.rs`, the
//!    blurb keys of its numbers in `blurb/keys.rs`, and, in a renderer, the
//!    ability's row of its art table.

pub mod pudge;
pub mod shadow_fiend;

pub use pudge::PUDGE;
pub use shadow_fiend::{
    SHADOW_FIEND, SHADOW_FIEND_TALENTS, cast_point_ms, presence_armor, raze_burn, raze_damage,
    raze_landing, raze_stack_damage, requiem_death_lines, requiem_fear_ticks, requiem_line_damage,
    requiem_lines, requiem_width, soul_cap, soul_damage,
};

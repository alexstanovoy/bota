//! One module for each hero: its numbers, its body, its abilities, its
//! talents and the pure functions of them. [`HEROES`](crate::HEROES) and
//! [`ABILITIES`](crate::ABILITIES) name what these modules hold.
//!
//! # Adding a hero
//!
//! 1. Register the ids: one line apiece appended to
//!    `bota_proto/registry/ids.txt`, each with its Dota key
//!    (`npc_dota_hero_x`, `x_ability`, the modifier `modifier_x_...`); a
//!    `HeroId`, `AbilityId` or `EffectId` constant in `bota_proto/src/ids.rs`
//!    only where code names it. Every row below carries its `id` and `key`.
//! 2. Copy a hero directory here to `heroes/x/`: `numbers.rs` (its constants,
//!    re-exported as `rules::NAME`), `kit.rs` (its `UnitDef`, one `AbilityDef`
//!    const per ability, its talent tree if it has one, and `HERO_DEF`),
//!    `formulas.rs` for pure functions the clients read, and `mod.rs`. Name
//!    the module in this file and in `rules.rs` (`pub use ...::numbers::*`).
//! 3. Add its `HERO_DEF` to `HERO_ROWS` in `hero.rs` and its ability consts
//!    to `HERO_ABILITIES` in `ability.rs`: `HEROES` and `ABILITIES` place
//!    every row at its id. A new modifier is a `ModifierKind` variant
//!    (`modifier.rs`) with its [`effect_id`](crate::ModifierKind::effect_id)
//!    arm, and its effect id gets a row in `effect.rs`.
//! 4. In `bota_server`: one `ABILITY_HOOKS` row an ability
//!    (`game/config/ability.rs`), the behaviour in `game/systems/x.rs`
//!    (named in `systems/mod.rs`), the modifier's `hash.rs` and `stats.rs`
//!    arms, and the end-to-end tests in `game/tests/x.rs`.
//! 5. Talents also need the `Talent` variants in `talent.rs`, each with its
//!    Dota key (`special_bonus_*`), and the server sites that ask
//!    `has_talent`.
//! 6. For a front end: the words and the art of each ability and effect are
//!    the front end's own, found by the Dota key; bota carries only the ids,
//!    the keys and the numbers.

pub mod pudge;
pub mod shadow_fiend;

pub use pudge::{PUDGE, dismember_bite_damage, dismember_bite_tick};
pub use shadow_fiend::{
    SHADOW_FIEND, SHADOW_FIEND_RAZES, SHADOW_FIEND_TALENTS, cast_point_ms, feast_quickened,
    presence_armor, raze_burn, raze_damage, raze_landing, raze_stack_damage, requiem_death_lines,
    requiem_fear_ticks, requiem_line_damage, requiem_lines, requiem_width, soul_cap, soul_damage,
};

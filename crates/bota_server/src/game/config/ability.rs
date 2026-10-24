//! What every hero's and courier's ability does at each moment of its life:
//! a hook table with one row to each of their entries of
//! [`bota_core::ABILITIES`]. A neutral's ability is data its mechanics read
//! ([`World::cast_spell`](crate::game::World::cast_spell)).

use bota_core::{ABILITY_COUNT, NeutralAbility};
use bota_proto::{AbilityId, Target};

use crate::game::{Entity, World};

/// What an ability does at a moment of its life. `false`: it did not happen.
pub type CastHook = fn(&mut World, Entity, Target) -> bool;

/// What an ability does when it ends.
pub type EndHook = fn(&mut World, Entity, Target);

/// Which level of an ability its caster has learned, counted from zero.
fn level_of(world: &World, caster: Entity, id: AbilityId) -> usize {
    usize::from(world.carried_level(caster, id).max(1) - 1)
}

/// What one ability does at each moment of its life.
#[derive(Clone, Copy)]
pub struct AbilityHooks {
    /// At the cast point. `false`: nothing happened and nothing is spent.
    pub on_cast: CastHook,
    /// Every tick it runs on. `false`: it breaks off.
    pub on_during: CastHook,
    /// Broken off by an order, a stun, the caster falling, or its own
    /// `false`.
    pub on_cancel: EndHook,
    /// Run to the end of its duration.
    pub on_complete: EndHook,
}

/// Does nothing at all, so a row names only the moments that do something.
const INERT: AbilityHooks = AbilityHooks {
    on_cast: |_, _, _| false,
    on_during: |_, _, _| true,
    on_cancel: |_, _, _| {},
    on_complete: |_, _, _| {},
};

/// The hooks of every hero's and courier's ability, each by its id.
const HOOKS: [(AbilityId, AbilityHooks); 18] = [
    (
        AbilityId::MEAT_HOOK,
        AbilityHooks {
            on_cast: |world, caster, target| {
                world.cast_hook(
                    caster,
                    level_of(world, caster, AbilityId::MEAT_HOOK),
                    target,
                )
            },
            ..INERT
        },
    ),
    (
        AbilityId::ROT,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.toggle_rot(caster, level_of(world, caster, AbilityId::ROT))
            },
            ..INERT
        },
    ),
    // An innate.
    (AbilityId::FLESH_HEAP, INERT),
    (
        AbilityId::DISMEMBER,
        AbilityHooks {
            on_cast: World::dismember_takes_hold,
            on_during: World::dismember_holds,
            on_cancel: World::dismember_lets_go,
            on_complete: World::dismember_lets_go,
        },
    ),
    (
        AbilityId::MEAT_SHIELD,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.cast_meat_shield(caster, level_of(world, caster, AbilityId::MEAT_SHIELD))
            },
            ..INERT
        },
    ),
    (
        AbilityId::BURST,
        AbilityHooks {
            on_cast: |world, courier, _| world.courier_burst(courier),
            ..INERT
        },
    ),
    (
        AbilityId::RETURN_ITEMS,
        AbilityHooks {
            on_cast: |world, courier, _| world.courier_return_items(courier),
            ..INERT
        },
    ),
    (
        AbilityId::TAKE_STASH,
        AbilityHooks {
            on_cast: |world, courier, _| world.courier_take_stash(courier),
            ..INERT
        },
    ),
    (
        AbilityId::DELIVER,
        AbilityHooks {
            on_cast: |world, courier, _| world.courier_deliver(courier),
            ..INERT
        },
    ),
    (
        AbilityId::SHIELD,
        AbilityHooks {
            on_cast: |world, courier, _| world.courier_shield(courier),
            ..INERT
        },
    ),
    (
        AbilityId::RAZE_NEAR,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.cast_raze(caster, level_of(world, caster, AbilityId::RAZE_NEAR), 0)
            },
            ..INERT
        },
    ),
    (
        AbilityId::RAZE_MID,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.cast_raze(caster, level_of(world, caster, AbilityId::RAZE_MID), 1)
            },
            ..INERT
        },
    ),
    (
        AbilityId::RAZE_FAR,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.cast_raze(caster, level_of(world, caster, AbilityId::RAZE_FAR), 2)
            },
            ..INERT
        },
    ),
    (
        AbilityId::REQUIEM,
        AbilityHooks {
            on_cast: |world, caster, _| {
                let level = level_of(world, caster, AbilityId::REQUIEM).min(2);
                world.cast_requiem(caster, level)
            },
            ..INERT
        },
    ),
    // Passives.
    (AbilityId::NECROMASTERY, INERT),
    (AbilityId::PRESENCE, INERT),
    (
        AbilityId::FEAST_OF_SOULS,
        AbilityHooks {
            on_cast: |world, caster, _| {
                world.cast_feast(caster, level_of(world, caster, AbilityId::FEAST_OF_SOULS))
            },
            ..INERT
        },
    ),
    // Retired: Throw Grenade.
    (AbilityId(16), INERT),
];

/// Every ability's hooks, indexed by [`AbilityId`] like
/// [`bota_core::ABILITIES`]: a hero's or courier's from [`HOOKS`], a
/// neutral's inert.
const ABILITY_HOOKS: [AbilityHooks; ABILITY_COUNT] = {
    let mut every = [INERT; ABILITY_COUNT];
    let mut hooked = [false; ABILITY_COUNT];
    let mut at = 0;
    while at < HOOKS.len() {
        let id = HOOKS[at].0.0 as usize;
        assert!(id < ABILITY_COUNT && !hooked[id], "one row an ability");
        hooked[id] = true;
        every[id] = HOOKS[at].1;
        at += 1;
    }
    let mut id = 0;
    while id < ABILITY_COUNT {
        let neutral = NeutralAbility::of(AbilityId(id as u16)).is_some();
        assert!(
            hooked[id] != neutral,
            "a row for every ability but a neutral's"
        );
        id += 1;
    }
    every
};

/// What one ability does, or nothing if no such ability exists. A neutral's
/// goes off through its mechanics, before any hook: what is left to a hook
/// is to run on and end, which none of them does.
pub fn ability_hooks(id: AbilityId) -> Option<&'static AbilityHooks> {
    ABILITY_HOOKS.get(usize::from(id.0))
}

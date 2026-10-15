//! What every ability does at each moment of its life: a hook table with one
//! row to each entry of [`bota_core::ABILITIES`].

use bota_core::ABILITY_COUNT;
use bota_proto::{AbilityId, Target};

use crate::game::{Entity, World};

/// What an ability does at a moment of its life. `false`: it did not happen.
pub type CastHook = fn(&mut World, Entity, Target) -> bool;

/// What an ability does when it ends.
pub type EndHook = fn(&mut World, Entity, Target);

fn never(_: &mut World, _: Entity, _: Target) -> bool {
    false
}

fn goes_on(_: &mut World, _: Entity, _: Target) -> bool {
    true
}

fn nothing(_: &mut World, _: Entity, _: Target) {}

/// Which level of an ability its caster has learned, counted from zero.
fn level_of(world: &World, caster: Entity, id: AbilityId) -> usize {
    usize::from(world.carried_level(caster, id).max(1) - 1)
}

fn meat_hook(world: &mut World, caster: Entity, target: Target) -> bool {
    let level = level_of(world, caster, AbilityId::MEAT_HOOK);
    world.cast_hook(caster, level, target)
}

fn rot(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::ROT);
    world.toggle_rot(caster, level)
}

fn dismember(world: &mut World, caster: Entity, target: Target) -> bool {
    world.dismember_takes_hold(caster, target)
}

fn dismember_holds(world: &mut World, caster: Entity, target: Target) -> bool {
    world.dismember_holds(caster, target)
}

fn dismember_lets_go(world: &mut World, caster: Entity, target: Target) {
    world.dismember_lets_go(caster, target);
}

fn burst(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_burst(courier)
}

fn return_items(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_return_items(courier)
}

fn take_stash(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_take_stash(courier)
}

fn deliver(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_deliver(courier)
}

fn shield(world: &mut World, courier: Entity, _: Target) -> bool {
    world.courier_shield(courier)
}

fn raze_near(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::RAZE_NEAR);
    world.cast_raze(caster, level, 0)
}

fn raze_mid(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::RAZE_MID);
    world.cast_raze(caster, level, 1)
}

fn raze_far(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::RAZE_FAR);
    world.cast_raze(caster, level, 2)
}

fn requiem(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::REQUIEM).min(2);
    world.cast_requiem(caster, level)
}

fn feast(world: &mut World, caster: Entity, _: Target) -> bool {
    let level = level_of(world, caster, AbilityId::FEAST_OF_SOULS);
    world.cast_feast(caster, level)
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
    on_cast: never,
    on_during: goes_on,
    on_cancel: nothing,
    on_complete: nothing,
};

/// Every ability's hooks, indexed by [`AbilityId`] like
/// [`bota_core::ABILITIES`].
pub const ABILITY_HOOKS: [AbilityHooks; ABILITY_COUNT] = [
    // Meat Hook.
    AbilityHooks {
        on_cast: meat_hook,
        ..INERT
    },
    // Rot.
    AbilityHooks {
        on_cast: rot,
        ..INERT
    },
    // Flesh Heap.
    INERT,
    // Dismember.
    AbilityHooks {
        on_cast: dismember,
        on_during: dismember_holds,
        on_cancel: dismember_lets_go,
        on_complete: dismember_lets_go,
    },
    // Burst.
    AbilityHooks {
        on_cast: burst,
        ..INERT
    },
    // Return.
    AbilityHooks {
        on_cast: return_items,
        ..INERT
    },
    // Take Stash.
    AbilityHooks {
        on_cast: take_stash,
        ..INERT
    },
    // Deliver.
    AbilityHooks {
        on_cast: deliver,
        ..INERT
    },
    // Shield.
    AbilityHooks {
        on_cast: shield,
        ..INERT
    },
    // Shadowraze (Near).
    AbilityHooks {
        on_cast: raze_near,
        ..INERT
    },
    // Shadowraze (Mid).
    AbilityHooks {
        on_cast: raze_mid,
        ..INERT
    },
    // Shadowraze (Far).
    AbilityHooks {
        on_cast: raze_far,
        ..INERT
    },
    // Requiem of Souls.
    AbilityHooks {
        on_cast: requiem,
        ..INERT
    },
    // Necromastery.
    INERT,
    // Presence of the Dark Lord.
    INERT,
    // Feast of Souls.
    AbilityHooks {
        on_cast: feast,
        ..INERT
    },
    // Throw Grenade.
    INERT,
];

const _: () = assert!(ABILITY_HOOKS.len() == bota_core::ABILITIES.len());

/// What one ability does, or nothing if no such ability exists.
pub fn ability_hooks(id: AbilityId) -> Option<&'static AbilityHooks> {
    ABILITY_HOOKS.get(usize::from(id.0))
}

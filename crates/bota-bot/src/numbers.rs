//! Everything the bot weighs that a snapshot does not state.
//!
//! Two kinds of number live here. Some are the server's own and simply never
//! cross the wire — the raze radius, the wind-up before a swing lands, the
//! reach of a shop. The rest are taste: how low is low, how far is far, how
//! long a want stands before it is worth repeating.

use bota_proto::{AbilityId, EffectId, HeroId, ItemId};

// Rules the wire leaves out.

/// Divisor turning armor into the fraction of a physical blow that lands.
pub const ARMOR_SCALE: i32 = 6;

/// How wide a raze burns around where it lands, in world units.
pub const RAZE_RADIUS: i32 = 250;

/// How far from a fountain an order to buy or to sell is taken.
pub const SHOP_RANGE: i32 = 1000;

/// How far off a swing may be looking when it begins, in brads.
pub const ATTACK_ANGLE: u16 = 2094;

/// How far a hero turns in one tick, in brads.
pub const TURN_RATE: u16 = 5795;

/// How far a hero reaches to take an item off the ground.
pub const TAKE_ITEM_RANGE: i32 = 150;

/// The effect a Shadow Fiend's gathered souls show up as.
pub const SOULS: EffectId = EffectId(11);

/// What one raze burns for, by the level it is cast at.
pub const RAZE_DAMAGE: [i32; 4] = [90, 160, 230, 300];

/// What one soul is worth to a requiem, by the level it is cast at.
pub const REQUIEM_DAMAGE_PER_SOUL: [i32; 3] = [8, 11, 14];

/// How far a tower reaches.
pub const TOWER_ATTACK_RANGE: i32 = 700;

/// Sylla, who crits, hastens, bounces a bolt and looses a volley.
pub const SYLLA: HeroId = HeroId(0);

/// Pudge, who hooks.
pub const PUDGE: HeroId = HeroId(1);

/// Shadow Fiend, who razes and gathers souls.
pub const SHADOW_FIEND: HeroId = HeroId(2);

/// What a hero's swing takes to leave and to arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swing {
    /// Ticks between a swing starting and the blow leaving.
    pub point: u32,
    /// World units a missile covers in a second. Zero for a hero that
    /// strikes by hand.
    pub missile_speed: i32,
}

/// What one hero's swing is made of.
pub fn swing_of(hero: HeroId) -> Swing {
    match hero {
        SHADOW_FIEND => Swing {
            point: 15,
            missile_speed: 1200,
        },
        PUDGE => Swing {
            point: 17,
            missile_speed: 0,
        },
        _ => Swing {
            point: 9,
            missile_speed: 900,
        },
    }
}

// The abilities, by the numbers the wire names them with.

/// Sylla's critical strike.
pub const CRIT: AbilityId = AbilityId(0);
/// Sylla's frenzy.
pub const FRENZY: AbilityId = AbilityId(1);
/// Sylla's bouncing bolt.
pub const BOUNCE: AbilityId = AbilityId(2);
/// Sylla's volley.
pub const VOLLEY: AbilityId = AbilityId(3);
/// Pudge's hook.
pub const MEAT_HOOK: AbilityId = AbilityId(4);
/// Pudge's rot.
pub const ROT: AbilityId = AbilityId(5);
/// Pudge's flesh heap.
pub const FLESH_HEAP: AbilityId = AbilityId(6);
/// Pudge's dismember.
pub const DISMEMBER: AbilityId = AbilityId(7);
/// A courier's burst of speed.
pub const BURST: AbilityId = AbilityId(8);
/// A courier putting back what it carries.
pub const RETURN_ITEMS: AbilityId = AbilityId(9);
/// A courier taking what waits in the stash.
pub const TAKE_STASH: AbilityId = AbilityId(10);
/// A courier handing over what it carries.
pub const DELIVER: AbilityId = AbilityId(11);
/// A courier's shield.
pub const SHIELD: AbilityId = AbilityId(12);
/// Shadow Fiend's nearest raze.
pub const RAZE_NEAR: AbilityId = AbilityId(13);
/// Shadow Fiend's middle raze.
pub const RAZE_MID: AbilityId = AbilityId(14);
/// Shadow Fiend's farthest raze.
pub const RAZE_FAR: AbilityId = AbilityId(15);
/// Shadow Fiend's requiem.
pub const REQUIEM: AbilityId = AbilityId(16);
/// Shadow Fiend's necromastery.
pub const NECROMASTERY: AbilityId = AbilityId(17);
/// Shadow Fiend's presence.
pub const PRESENCE: AbilityId = AbilityId(18);

/// The three razes, nearest first.
pub const RAZES: [AbilityId; 3] = [RAZE_NEAR, RAZE_MID, RAZE_FAR];

// The shop, by the numbers the wire names items with.

/// Boots of Speed.
pub const BOOTS: ItemId = ItemId(0);
/// Clarity.
pub const CLARITY: ItemId = ItemId(1);
/// Healing Salve.
pub const SALVE: ItemId = ItemId(2);
/// Iron Branch.
pub const BRANCH: ItemId = ItemId(3);
/// Observer Ward.
pub const OBSERVER_WARD: ItemId = ItemId(4);
/// Quelling Blade.
pub const QUELLING_BLADE: ItemId = ItemId(5);
/// Sentry Ward.
pub const SENTRY_WARD: ItemId = ItemId(6);
/// Tango.
pub const TANGO: ItemId = ItemId(7);
/// Town Portal Scroll.
pub const SCROLL: ItemId = ItemId(8);
/// Circlet.
pub const CIRCLET: ItemId = ItemId(9);
/// Gauntlets of Strength.
pub const GAUNTLETS: ItemId = ItemId(10);
/// Slippers of Agility.
pub const SLIPPERS: ItemId = ItemId(11);
/// Mantle of Intelligence.
pub const MANTLE: ItemId = ItemId(12);
/// Belt of Strength.
pub const BELT: ItemId = ItemId(13);
/// Band of Elvenskin.
pub const BAND: ItemId = ItemId(14);
/// Robe of the Magi.
pub const ROBE: ItemId = ItemId(15);
/// Ogre Axe.
pub const OGRE_AXE: ItemId = ItemId(16);
/// Blade of Alacrity.
pub const ALACRITY: ItemId = ItemId(17);
/// Staff of Wizardry.
pub const WIZARDRY: ItemId = ItemId(18);
/// Gloves of Haste.
pub const GLOVES: ItemId = ItemId(19);
/// Blades of Attack.
pub const BLADES: ItemId = ItemId(20);
/// Broadsword.
pub const BROADSWORD: ItemId = ItemId(21);
/// Quarterstaff.
pub const QUARTERSTAFF: ItemId = ItemId(22);
/// Ring of Protection.
pub const RING_OF_PROTECTION: ItemId = ItemId(23);
/// Chainmail.
pub const CHAINMAIL: ItemId = ItemId(24);
/// Ring of Regeneration.
pub const RING_OF_REGEN: ItemId = ItemId(25);
/// Sage's Mask.
pub const SAGES_MASK: ItemId = ItemId(26);
/// Vitality Booster.
pub const VITALITY_BOOSTER: ItemId = ItemId(27);
/// Energy Booster.
pub const ENERGY_BOOSTER: ItemId = ItemId(28);
/// Power Treads.
pub const POWER_TREADS: ItemId = ItemId(29);
/// Phase Boots.
pub const PHASE_BOOTS: ItemId = ItemId(30);
/// Blink Dagger.
pub const BLINK_DAGGER: ItemId = ItemId(31);
/// Bracer.
pub const BRACER: ItemId = ItemId(32);
/// Wraith Band.
pub const WRAITH_BAND: ItemId = ItemId(33);
/// Null Talisman.
pub const NULL_TALISMAN: ItemId = ItemId(34);
/// Magic Stick.
pub const MAGIC_STICK: ItemId = ItemId(35);
/// Magic Wand.
pub const MAGIC_WAND: ItemId = ItemId(36);
/// The recipe Phase Boots are finished with.
pub const RECIPE_PHASE_BOOTS: ItemId = ItemId(37);
/// The recipe a Bracer is finished with.
pub const RECIPE_BRACER: ItemId = ItemId(38);
/// The recipe a Wraith Band is finished with.
pub const RECIPE_WRAITH_BAND: ItemId = ItemId(39);
/// The recipe a Null Talisman is finished with.
pub const RECIPE_NULL_TALISMAN: ItemId = ItemId(40);
/// The recipe a Magic Wand is finished with.
pub const RECIPE_MAGIC_WAND: ItemId = ItemId(41);

/// Attack damage a Quelling Blade adds against anything that is not a hero.
///
/// Carried damage that answers to what is being struck, so it rides in no
/// stat a view shows.
pub const QUELLING_BONUS: i32 = 18;

// Taste.

/// Health, as a part of the whole, below which the hero pulls out of the
/// lane.
pub const RETREAT_HEALTH: f32 = 0.32;

/// Health, as a part of the whole, at which the hero goes back to the lane.
pub const RETURN_HEALTH: f32 = 0.8;

/// The same, for a hero a salve or a tango is already mending.
pub const MENDED_RETURN: f32 = 0.55;

/// Health, as a part of the whole, below which a salve is worth drinking.
pub const SALVE_HEALTH: f32 = 0.55;

/// Health, as a part of the whole, below which a tango is worth eating.
pub const TANGO_HEALTH: f32 = 0.7;

/// Mana, as a part of the whole, below which a clarity is worth drinking.
pub const CLARITY_MANA: f32 = 0.5;

/// How far a hero will walk to reach a tree worth eating.
///
/// A tango is paid for by a tree standing within reach of it, and the trees
/// beside a lane are cleared away from its centre, so the tango a hero is
/// carrying is of no use where it is standing.
pub const TANGO_WALK: f32 = 1300.0;

/// Health one salve gives back over the whole of it.
pub const SALVE_HEALS: i32 = 400;

/// Health one charge of a tango gives back over the whole of it.
pub const TANGO_HEALS: i32 = 115;

/// Health or mana one charge of a wand or a stick gives back.
pub const RESTORE_PER_CHARGE: i32 = 15;

/// Charges a wand or a stick must hold to be worth pressing.
pub const RESTORE_CHARGES: u8 = 5;

/// Working slots that must be free over and above the one a consumable would
/// take for it to be bought at all.
///
/// Nought, so a consumable is bought whenever any working slot is free. A
/// higher figure stops the drink being restocked at all once the goods fill
/// the bag, which is exactly when a hero has furthest to walk home.
pub const SPARE_SLOTS: usize = 0;

/// How far from an enemy hero counts as being in a fight.
pub const FIGHT_RANGE: i32 = 1000;

/// How far from an enemy tower a hero keeps while its own creeps are not
/// there to take the blows.
pub const TOWER_KEEP_OUT: i32 = 850;

/// How far behind its own creep line the hero stands while it waits.
pub const STAND_BEHIND: i32 = 250;

/// Ticks the same want stands before it is worth sending again.
pub const RESEND_TICKS: u32 = 8;

/// How far two walks may aim apart and still count as the same want.
pub const RESEND_DRIFT: f32 = 120.0;

/// Ticks an errand already under way is left alone before it is repeated.
pub const ERRAND_TICKS: u32 = 150;

/// Items that must pile up in the stash before the courier is sent for them.
pub const COURIER_BATCH: usize = 2;

/// Ticks the first item may wait in the stash before the courier is sent
/// whatever has piled up.
pub const COURIER_PATIENCE: u32 = 300;

/// Ticks of health a creep's fall is forecast over, past which the forecast
/// is not trusted.
pub const FORECAST_TICKS: u32 = 60;

/// Ticks of health kept for working out how fast a body is falling.
pub const HISTORY_TICKS: u32 = 10;

/// Ticks a blow taken is remembered for, when working out what is chewing on
/// the hero.
///
/// Longer than a creep's interval between swings, so a creep that is set on
/// the hero is still counted between its blows.
pub const BITTEN_TICKS: u32 = 60;

/// Creeps that must be chewing on the hero for it to be worth shaking them
/// off.
pub const SHAKE_CREEPS: usize = 2;

/// How far a lane creep looks for something to take on.
///
/// Also how near the hero a creep must stand to hear an order it gave.
pub const CREEP_ACQUISITION: i32 = 500;

/// Ticks a creep called on by an order stays called on.
pub const AGGRO_HOLD: u32 = 70;

/// Ticks before an order may call the same creep on again.
pub const AGGRO_COOLDOWN: u32 = 90;

/// How far past the spot the waves should meet the other side's creeps must
/// have been pushed for it to be worth calling them back.
pub const PULL_DRIFT: f32 = 700.0;

/// Health, as a part of the whole, below which the hero does not take a wave
/// onto itself to hold the lane.
pub const PULL_HEALTH: f32 = 0.6;

/// Ticks between one shake and the next.
///
/// An order at one of your own costs the creeps nothing and puts nothing on
/// the clock, so the only reason to wait is that the tick it takes is a tick
/// the hero did not swing in.
pub const SHAKE_TICKS: u32 = 45;

/// How much of a swing's worth is allowed to go to waste on a last hit.
///
/// A blow forecast to land on a creep already below this share of what the
/// swing is worth is a blow that would have fallen anyway.
pub const LAST_HIT_SLACK: f32 = 0.15;

/// How far an enemy hero may stand and still be worth swinging at.
pub const HARASS_RANGE: i32 = 700;

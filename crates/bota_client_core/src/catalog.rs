//! What the client knows about abilities, items, effects and heroes.
//!
//! Names and blurb templates, one entry to a thing, found by the id the wire
//! carries. The numbers in a blurb are the rules': a template names them and
//! `bota_core` gives them, and the views tell what a unit holds.

use bota_core::{
    EFFECT_COUNT, EFFECTS as EFFECT_DEFS, FIRST_LATE_ABILITY, FIRST_NEUTRAL_ABILITY,
    ITEMS as ITEM_DEFS, NEUTRAL_ABILITIES, Talent, ability_def, agility_armor,
    agility_attack_speed, attribute_regen, attributes_at_level, effect_def, hero_def,
    intelligence_magic_resist_bp, talent_floor, white_damage_least,
};
use bota_proto::{AbilityId, EffectId, Fixed, HeroId, ItemId, TalentSlot, Talents};

use crate::blurb::{Scope, fill};

/// One ability, as the client shows and aims it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the panel.
    pub name: &'static str,
    /// What it does, for the hover popup, with the rules' numbers as keys;
    /// read it filled through [`AbilityFace::blurb`].
    pub template: &'static str,
}

impl AbilityFace {
    /// What it does, with the numbers the rules give it.
    pub fn blurb(&self) -> String {
        let scope = ability_def(AbilityId(self.id)).map_or(Scope::Shared, Scope::Ability);
        fill(self.template, scope)
    }
}

/// One item, as the client shows and aims it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// Short name for the panel and the shop.
    pub name: &'static str,
    /// Its one-line stats, for the shop row, as a template.
    pub stats_template: &'static str,
    /// What it does, for the hover popup, as a template.
    pub template: &'static str,
}

impl ItemFace {
    /// An item that does nothing but add what it carries.
    const fn plain(id: u16, name: &'static str) -> ItemFace {
        ItemFace::with(id, name, "{short}", "{carried}.")
    }

    const fn with(
        id: u16,
        name: &'static str,
        stats_template: &'static str,
        template: &'static str,
    ) -> ItemFace {
        ItemFace {
            id,
            name,
            stats_template,
            template,
        }
    }

    /// Its one-line stats, for the shop row.
    pub fn stats(&self) -> String {
        fill(self.stats_template, Scope::Item(ItemId(self.id)))
    }

    /// What it does, with the numbers the rules give it.
    pub fn blurb(&self) -> String {
        fill(self.template, Scope::Item(ItemId(self.id)))
    }
}

/// One timed effect, as the client shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectFace {
    /// The id it answers to on the wire.
    pub id: u16,
    /// The chip's name where it is not the rules'
    /// [`EffectDef::name`](bota_core::EffectDef::name).
    pub label: Option<&'static str>,
    /// What it does, for the hover popup, as a template.
    pub template: &'static str,
    /// Whether a front end lists it among a unit's buffs and debuffs. Dota
    /// shows how long a ward or a summoned unit has left on its health bar,
    /// not as a buff.
    pub listed: bool,
}

impl EffectFace {
    const fn of(id: u16, label: Option<&'static str>, template: &'static str) -> EffectFace {
        EffectFace {
            id,
            label,
            template,
            listed: true,
        }
    }

    /// Short name for the chip; empty for a retired id.
    pub fn name(&self) -> &'static str {
        self.label
            .or_else(|| effect_def(EffectId(self.id)).map(|def| def.name))
            .unwrap_or_default()
    }

    /// What it does, with the numbers the rules give it.
    pub fn blurb(&self) -> String {
        fill(self.template, Scope::Shared)
    }
}

/// What each of the three razes does.
const RAZE: &str = "No target, {point} s cast point. Burns every enemy but a building within {raze_radius} of a spot {range} ahead of where you face for {raze_damage} magic damage, plus {raze_soul} per soul held and {raze_stack} per prior same-caster stack. Each damaging hit refreshes all stacks for {raze_secs} s. One point levels all three razes. {mana} mana, {wait} s wait.";

/// Every ability, in id order.
pub const ABILITIES: [AbilityFace; FIRST_NEUTRAL_ABILITY as usize] = [
    AbilityFace {
        id: AbilityId::MEAT_HOOK.0,
        name: "Hook",
        template: "Point target, range {range}, {point} s cast point. Catches the first unit in its way, drags it back and deals {hook_damage} pure damage.",
    },
    AbilityFace {
        id: AbilityId::ROT.0,
        name: "Rot",
        template: "Toggle. Burns every enemy within {range} for {rot_dps} a second and slows it {rot_slow}%. It burns its owner for the same, but never kills its owner.",
    },
    AbilityFace {
        id: AbilityId::FLESH_HEAP.0,
        name: "Heap",
        template: "Innate. +{heap_strength} strength for every enemy hero that dies within {range} of you, whoever brought it down. Stacks are kept through your own death.",
    },
    AbilityFace {
        id: AbilityId::DISMEMBER.0,
        name: "Dismem",
        template: "Ultimate, enemy target, range {range}, {point} s cast point. Holds it stunned for {duration} s and pulls it {dismember_pull} a second to {dismember_limit} from you. Bites {dismember_bites} times, the first as it takes hold, for {dismember_damage} magic damage a second plus {dismember_strength} per point of your strength, and heals you for what each bite takes. {mana} mana, {wait} s wait.",
    },
    AbilityFace {
        id: AbilityId::BURST.0,
        name: "Burst",
        template: "No target. The courier moves {burst_pct}% faster for {burst_secs} s. {wait} s wait.",
    },
    AbilityFace {
        id: AbilityId::RETURN_ITEMS.0,
        name: "Return",
        template: "No target. The courier puts what it holds back in the stash, then goes home.",
    },
    AbilityFace {
        id: AbilityId::TAKE_STASH.0,
        name: "Stash",
        template: "No target. The courier takes what waits in your stash and carries it to you.",
    },
    AbilityFace {
        id: AbilityId::DELIVER.0,
        name: "Give",
        template: "No target. The courier carries what it holds to you, then goes home.",
    },
    AbilityFace {
        id: AbilityId::SHIELD.0,
        name: "Shield",
        template: "No target. Nothing gets through to the courier for {shield_secs} s. {wait} s wait.",
    },
    AbilityFace {
        id: AbilityId::RAZE_NEAR.0,
        name: "Raze 1",
        template: RAZE,
    },
    AbilityFace {
        id: AbilityId::RAZE_MID.0,
        name: "Raze 2",
        template: RAZE,
    },
    AbilityFace {
        id: AbilityId::RAZE_FAR.0,
        name: "Raze 3",
        template: RAZE,
    },
    AbilityFace {
        id: AbilityId::REQUIEM.0,
        name: "Requiem",
        template: "Ultimate, no target, {point} s cast point. Lets a line fly out for every soul held, up to {requiem_lines}, out to {range}. Each line burns every enemy but a building it crosses for {requiem_damage} magic damage, and every hit adds {requiem_fear} s of fear, a {requiem_slow}% slow and {requiem_break}% less magic resistance, up to {requiem_hold} s. What stands close is crossed by many lines. The souls are kept. On your death, {requiem_death_pct}% of the souls you keep fly out as lines that only burn.",
    },
    AbilityFace {
        id: AbilityId::NECROMASTERY.0,
        name: "Necro",
        template: "Innate. A soul is kept of everything you bring down, {souls_hero} for a hero, each worth {soul_damage} attack damage and {soul_gain} more every {soul_levels} levels. Holds up to {soul_cap}; a death lets {soul_loss}% of them go.",
    },
    AbilityFace {
        id: AbilityId::PRESENCE.0,
        name: "Presence",
        template: "Passive. Enemies within {range} that see you wear {presence_armor} less armor.",
    },
    AbilityFace {
        id: AbilityId::FEAST_OF_SOULS.0,
        name: "Feast",
        template: "No target, goes off beside whatever you are doing. For {feast_secs} s: +{feast_speed} attack speed and +{feast_move}% movement speed, and every {feast_period} s souls from {feast_per_period} enemies within {range} you have not fed on yet, heroes first: {feast_hero} a hero, {feast_unit} a creep, from up to {feast_targets} enemies. Then the souls of those still standing go; the rest stay {feast_linger} s past the cap. {mana} mana, {wait} s wait.",
    },
    // Retired: what named a Blood Grenade in flight, which names its item now.
    AbilityFace {
        id: 16,
        name: "Retired",
        template: "No longer in the game.",
    },
];

/// Every item the shop sells, in id order.
pub const ITEMS: [ItemFace; ITEM_DEFS.len()] = [
    ItemFace::plain(ItemId::BOOTS.0, "Boots"),
    ItemFace::with(
        ItemId::CLARITY.0,
        "Clarity",
        "{mend_total}{mend_pool_short}/{mend_secs}s",
        "Consumable. Restores {mend_total} {mend_pool} over {mend_secs} s. Damage from an enemy hero breaks it.",
    ),
    ItemFace::with(
        ItemId::HEALING_SALVE.0,
        "Salve",
        "{mend_total}{mend_pool_short}/{mend_secs}s",
        "Consumable. Restores {mend_total} {mend_pool} over {mend_secs} s. Damage from an enemy hero breaks it.",
    ),
    ItemFace::plain(ItemId::IRON_BRANCH.0, "Branch"),
    ItemFace::with(
        ItemId::OBSERVER_WARD.0,
        "Obs",
        "Vision",
        "Consumable. Stands a ward that sees {ward_vision} and that the enemy cannot see.",
    ),
    ItemFace::with(
        ItemId::QUELLING_BLADE.0,
        "Quell",
        "{short}",
        "{carried}. Fells the tree you point at.",
    ),
    ItemFace::with(
        ItemId::SENTRY_WARD.0,
        "Sentry",
        "True sight",
        "Consumable. Stands a ward that gives true sight, revealing enemy wards.",
    ),
    ItemFace::with(
        ItemId::TANGO.0,
        "Tango",
        "{mend_total}HP x{charges}",
        "{charges} charges. Eats a tree to restore {mend_total} health over {mend_secs} s.",
    ),
    ItemFace::with(
        ItemId::TOWN_PORTAL_SCROLL.0,
        "TP",
        "Teleport",
        "Consumable. Channels, then carries you to an allied building.",
    ),
    ItemFace::plain(ItemId::CIRCLET.0, "Circlet"),
    ItemFace::plain(ItemId::GAUNTLETS.0, "Gauntlet"),
    ItemFace::plain(ItemId::SLIPPERS.0, "Slippers"),
    ItemFace::plain(ItemId::MANTLE.0, "Mantle"),
    ItemFace::plain(ItemId::BELT.0, "Belt"),
    ItemFace::plain(ItemId::BAND.0, "Band"),
    ItemFace::plain(ItemId::ROBE.0, "Robe"),
    ItemFace::plain(ItemId::OGRE_AXE.0, "Ogre Axe"),
    ItemFace::plain(ItemId::BLADE_OF_ALACRITY.0, "Alacrity"),
    ItemFace::plain(ItemId::STAFF_OF_WIZARDRY.0, "Wizardry"),
    ItemFace::plain(ItemId::GLOVES.0, "Gloves"),
    ItemFace::plain(ItemId::BLADES_OF_ATTACK.0, "Blades"),
    ItemFace::plain(ItemId::BROADSWORD.0, "Broad"),
    ItemFace::plain(ItemId::QUARTERSTAFF.0, "Qstaff"),
    ItemFace::plain(ItemId::RING_OF_PROTECTION.0, "Ring Pro"),
    ItemFace::plain(ItemId::CHAINMAIL.0, "Chain"),
    ItemFace::plain(ItemId::RING_OF_REGEN.0, "Regen"),
    ItemFace::plain(ItemId::SAGES_MASK.0, "Sage"),
    ItemFace::plain(ItemId::VITALITY_BOOSTER.0, "Vitality"),
    ItemFace::plain(ItemId::ENERGY_BOOSTER.0, "Energy"),
    ItemFace::with(
        ItemId::POWER_TREADS.0,
        "Treads",
        "{short}",
        "{carried}, +{mode_bonus} to the attribute it is set to. Built of Boots, Gloves and a Belt, Robe or Band, which sets it. Using them sets them to the next one.",
    ),
    ItemFace::with(
        ItemId::PHASE_BOOTS.0,
        "Phase",
        "{short}",
        "{carried}. Walks {phase_pct}% faster ({phase_ranged_pct}% for a ranged hero) and through bodies for {phase_secs} s. {wait} s wait.",
    ),
    ItemFace::with(
        ItemId::BLINK_DAGGER.0,
        "Blink",
        "{blink_range} jump",
        "Carries you to a point up to {blink_range} away, or {blink_overshoot} toward one further. {wait} s wait, and damage from an enemy hero leaves it at least {break_secs} s from ready.",
    ),
    ItemFace::plain(ItemId::BRACER.0, "Bracer"),
    ItemFace::plain(ItemId::WRAITH_BAND.0, "Wraith"),
    ItemFace::plain(ItemId::NULL_TALISMAN.0, "Null"),
    ItemFace::with(
        ItemId::MAGIC_STICK.0,
        "Stick",
        "{cast_charges} charges",
        "Gains a charge from every enemy cast within {magic_range}. Spends them all to restore {restore} health and mana each. {wait} s wait.",
    ),
    ItemFace::with(
        ItemId::MAGIC_WAND.0,
        "Wand",
        "{short}",
        "{carried}. Gains a charge from every enemy cast within {magic_range}, up to {cast_charges}. {wait} s wait.",
    ),
    // Retired: the recipe Phase Boots were built with.
    ItemFace::with(37, "Retired", "retired", "No longer in the game."),
    ItemFace::with(ItemId::RECIPE_BRACER.0, "Rcp Bracer", "recipe", "{builds}"),
    ItemFace::with(
        ItemId::RECIPE_WRAITH_BAND.0,
        "Rcp Wraith",
        "recipe",
        "{builds}",
    ),
    ItemFace::with(
        ItemId::RECIPE_NULL_TALISMAN.0,
        "Rcp Null",
        "recipe",
        "{builds}",
    ),
    ItemFace::with(
        ItemId::RECIPE_MAGIC_WAND.0,
        "Rcp Wand",
        "recipe",
        "{builds}",
    ),
    ItemFace::with(
        ItemId::MANGO.0,
        "Mango",
        "{short} each",
        "One charge per purchase, stacks up to {stack_limit}. Each charge adds {hp_regen} health regeneration per second. Self-use consumes one charge to restore up to {replenish} mana instantly. Requires missing mana.",
    ),
    ItemFace::plain(ItemId::EAGLESONG.0, "Eaglesong"),
    ItemFace::plain(ItemId::CLAYMORE.0, "Claymore"),
    ItemFace::with(
        ItemId::TALISMAN_OF_EVASION.0,
        "Talisman",
        "{evasion}% evasion",
        "{evasion}% of attacks at the carrier miss. Evasions carried stack: each lets through its share of what the others do.",
    ),
    ItemFace::with(
        ItemId::BUTTERFLY.0,
        "Butterfly",
        "{short}",
        "{carried}. {evasion}% of attacks at the carrier miss. Evasions carried stack: each lets through its share of what the others do.",
    ),
    ItemFace::with(
        ItemId::JAVELIN.0,
        "Javelin",
        "{pierce}% pierce",
        "{pierce}% of attacks pierce: they cannot miss and deal {pierce_damage} bonus magical damage. Not against buildings. Each pierce carried rolls its own.",
    ),
    ItemFace::plain(ItemId::DEMON_EDGE.0, "Demon Edge"),
    ItemFace::plain(ItemId::BLITZ_KNUCKLES.0, "Blitz"),
    ItemFace::with(
        ItemId::RECIPE_MONKEY_KING_BAR.0,
        "Rcp MKB",
        "recipe",
        "{builds}",
    ),
    ItemFace::with(
        ItemId::MONKEY_KING_BAR.0,
        "MKB",
        "{short}",
        "{carried}. {pierce}% of attacks pierce: they cannot miss and deal {pierce_damage} bonus magical damage. Not against buildings. Each pierce carried rolls its own, several Monkey King Bars as one.",
    ),
    ItemFace::with(
        ItemId::WIND_LACE.0,
        "Lace",
        "{short}",
        "{carried}, on top of boots. Of several laces carried, only one counts.",
    ),
    ItemFace::with(
        ItemId::BLOOD_GRENADE.0,
        "Grenade",
        "{short}",
        "{carried}. Thrown at a spot within {range}, {point} s cast point, costs {health_cost} health but never the last point: enemy heroes and creeps within {grenade_radius} of where it lands take {grenade_impact} magic damage, then {grenade_dps} every second for {grenade_secs} s, slowed {grenade_slow}%. Stacks {stack_limit} to a slot. The shop holds {stock_max} at most, {stock_initial} at the start, one more every {stock_minutes} min. {wait} s wait.",
    ),
];

/// Every timed effect, indexed by [`EffectId`]; the slot of a retired one
/// holds nothing.
pub const EFFECTS: [EffectFace; FIRST_SPELL_EFFECT] = [
    EffectFace::of(0, None, ""),
    EffectFace::of(EffectId::MENDING.0, None, "Regenerating health."),
    EffectFace::of(EffectId::CLARITY.0, None, "Regenerating mana."),
    EffectFace::of(
        EffectId::FOUNTAIN.0,
        None,
        "Regenerating health and mana for standing in the fountain.",
    ),
    EffectFace::of(
        EffectId::STUNNED.0,
        Some("Held"),
        "Cannot move, attack or cast.",
    ),
    EffectFace::of(EffectId::SLOWED.0, None, "Movement speed reduced."),
    EffectFace::of(EffectId::BURNING.0, None, "Losing health over time."),
    EffectFace::of(EffectId::HASTENED.0, None, "Movement speed increased."),
    EffectFace::of(EffectId::SHIELDED.0, None, "Nothing gets through."),
    EffectFace::of(
        EffectId::PHASED.0,
        None,
        "Walking through the bodies in the way.",
    ),
    EffectFace::of(
        EffectId::FLESH_HEAP.0,
        Some("Heap"),
        "Strength kept from every enemy hero that died nearby.",
    ),
    EffectFace::of(
        EffectId::SOULS.0,
        None,
        "Attack damage from every unit brought down.",
    ),
    EffectFace::of(
        EffectId::ARMOR_BROKEN.0,
        Some("Presence"),
        "Armor worn down by a dark presence nearby.",
    ),
    EffectFace::of(
        EffectId::GUARDED.0,
        None,
        "Armor and health mended, for standing by a tower of your own.",
    ),
    EffectFace::of(
        EffectId::INSPIRED.0,
        None,
        "Health mended and magic shrugged off, for marching beside the one carrying the flag.",
    ),
    EffectFace::of(
        EffectId::SHADOWRAZE.0,
        Some("Razed"),
        "Shadowrazes from the same caster deal {raze_stack} more magic damage per stack. Each damaging hit refreshes all stacks for {raze_secs} s; up to {raze_max_stacks} stacks.",
    ),
    EffectFace::of(
        EffectId::ROT.0,
        None,
        "In a rot: {rot_dps} damage a second and slowed {rot_slow}%. Its owner burns as well, but never to death, and is not slowed.",
    ),
    EffectFace::of(
        EffectId::FEARED.0,
        None,
        "Running from whoever put the fear on. Cannot attack or cast.",
    ),
    EffectFace::of(
        EffectId::MAGIC_BROKEN.0,
        Some("Torn"),
        "Magic resistance torn by a requiem.",
    ),
    EffectFace::of(
        EffectId::FEASTING.0,
        Some("Feast"),
        "Attacking and moving faster while taking souls from enemies nearby; counts the souls taken.",
    ),
    EffectFace::of(
        EffectId::FEAST_LINGER.0,
        Some("Fed"),
        "Souls a feast took from enemies since fallen, kept past the cap until it runs out.",
    ),
    EffectFace::of(
        EffectId::BLOOD_GRENADE.0,
        Some("Grenade"),
        "Slowed {grenade_slow}% and losing {grenade_dps} health to magic every second, from a Blood Grenade.",
    ),
];

/// The abilities added after the neutrals', from [`FIRST_LATE_ABILITY`].
const LATE_FACES: [AbilityFace; 1] = [AbilityFace {
    id: AbilityId::MEAT_SHIELD.0,
    name: "Shield",
    template: "No target. Every blow that reaches you loses {meat_block} damage, after armor and resistance, for {meat_secs} s. {mana} mana, {wait} s wait.",
}];

/// The ability of that id, or nothing for one the catalog does not hold.
pub fn ability(id: u16) -> Option<&'static AbilityFace> {
    ABILITIES
        .get(usize::from(id))
        .or_else(|| {
            let at = id.checked_sub(FIRST_NEUTRAL_ABILITY)?;
            NEUTRAL_FACES.get(usize::from(at))
        })
        .or_else(|| {
            let at = id.checked_sub(FIRST_LATE_ABILITY)?;
            LATE_FACES.get(usize::from(at))
        })
}

/// Every neutral's ability, as its rules name it.
const NEUTRAL_FACES: [AbilityFace; NEUTRAL_ABILITIES.len()] = {
    let mut faces = [AbilityFace {
        id: 0,
        name: "",
        template: "",
    }; NEUTRAL_ABILITIES.len()];
    let mut at = 0;
    while at < faces.len() {
        faces[at] = AbilityFace {
            id: FIRST_NEUTRAL_ABILITY + at as u16,
            name: NEUTRAL_ABILITIES[at].def.name,
            template: "",
        };
        at += 1;
    }
    faces
};

/// The first effect a neutral's spell leaves: from it on, every effect is
/// shown by its rules name.
const FIRST_SPELL_EFFECT: usize = EffectId::ROOTED.0 as usize;

/// Every effect from [`FIRST_SPELL_EFFECT`] on.
const SPELL_EFFECT_FACES: [EffectFace; EFFECT_COUNT - FIRST_SPELL_EFFECT] = {
    let mut faces = [EffectFace::of(0, None, ""); EFFECT_COUNT - FIRST_SPELL_EFFECT];
    let mut at = 0;
    while at < faces.len() {
        assert!(!EFFECT_DEFS[FIRST_SPELL_EFFECT + at].retired);
        faces[at] = EffectFace::of((FIRST_SPELL_EFFECT + at) as u16, None, "");
        at += 1;
    }
    // The heroes' effects added after the neutrals' have their own words.
    faces[EffectId::MEAT_SHIELD.0 as usize - FIRST_SPELL_EFFECT] = EffectFace::of(
        EffectId::MEAT_SHIELD.0,
        Some("Meat Shield"),
        "Every blow loses the damage shown, after armor and resistance.",
    );
    faces[EffectId::DISMEMBERED.0 as usize - FIRST_SPELL_EFFECT] = EffectFace::of(
        EffectId::DISMEMBERED.0,
        Some("Held"),
        "Held stunned and bitten by a dismember, pulled towards whoever holds it.",
    );
    faces[EffectId::ILLUSION.0 as usize - FIRST_SPELL_EFFECT] = EffectFace::of(
        EffectId::ILLUSION.0,
        None,
        "An illusion of the hero. Only its own side can tell it from the hero.",
    );
    faces[EffectId::TIMED_LIFE.0 as usize - FIRST_SPELL_EFFECT] = EffectFace {
        listed: false,
        ..EffectFace::of(
            EffectId::TIMED_LIFE.0,
            Some("Time left"),
            "Gone when its time runs out.",
        )
    };
    faces[EffectId::FOUNTAIN_INVULNERABLE.0 as usize - FIRST_SPELL_EFFECT] = EffectFace::of(
        EffectId::FOUNTAIN_INVULNERABLE.0,
        Some("Untouchable"),
        "Risen at its fountain: invulnerable and untargetable, and attacking nobody of its own accord, until given a command or until the fountain's mending wears off.",
    );
    faces
};

/// The item of that id, or nothing for one the catalog does not hold.
pub fn item(id: u16) -> Option<&'static ItemFace> {
    ITEMS.get(usize::from(id))
}

/// The effect of that id, or nothing for one the catalog does not hold.
pub fn effect(id: u16) -> Option<&'static EffectFace> {
    effect_def(EffectId(id))?;
    EFFECTS.get(usize::from(id)).or_else(|| {
        let at = usize::from(id).checked_sub(FIRST_SPELL_EFFECT)?;
        SPELL_EFFECT_FACES.get(at)
    })
}

/// One talent of a hero's tree, as the talent panel shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalentFace {
    /// What it gives, short, as Dota's tree writes it.
    pub name: String,
    /// What it does, for the hover popup.
    pub blurb: String,
}

/// Hero levels at which the talent in a slot opens: the first of its tier
/// to be taken, and the other once its tier has one.
pub fn talent_levels(slot: TalentSlot) -> (u8, u8) {
    let none = Talents::default();
    (
        talent_floor(slot, none),
        talent_floor(slot, none.with(slot.other())),
    )
}

/// What a talent is called and does, with the rules' numbers in.
fn talent_face(talent: Talent) -> TalentFace {
    let (name, blurb) = match talent {
        Talent::FeastAttackSpeed => (
            "+{talent_feast_speed} Feast of Souls Attack Speed",
            "A feast gives {talent_feast_speed} more attack speed.",
        ),
        Talent::RazeStackDamage => (
            "+{talent_raze_stack} Shadowraze Stack Damage",
            "Each stack of a raze's mark adds {talent_raze_stack} more magic damage.",
        ),
        Talent::FeastSoulsPerHero => (
            "+{talent_feast_souls} Feast of Souls Souls Per Hero",
            "A feast takes {talent_feast_souls} more souls from every enemy hero.",
        ),
        Talent::PresenceArmor => (
            "+{talent_presence} Presence Armor Reduction",
            "The presence takes {talent_presence} more armor from enemies near you.",
        ),
        Talent::RequiemFear => (
            "+{talent_fear}s Requiem Fear per Line (max +{talent_fear_max}s)",
            "Every requiem line fears, slows and tears {talent_fear} s longer, up to {talent_fear_max} s more in all.",
        ),
        Talent::NecroSoulCap => (
            "+{talent_souls} Necromastery Max Souls",
            "Hold {talent_souls} more souls from kills.",
        ),
        Talent::RazeAttackDamage => (
            "Shadowraze Applies Attack Damage",
            "A raze also deals your attack damage, as physical damage.",
        ),
        Talent::FeastCastSpeed => (
            "Feast of Souls Grants +{talent_cast_speed}% Cast Speed",
            "While a feast runs, cast points last {talent_cast_pct}% of their length.",
        ),
    };
    TalentFace {
        name: fill(name, Scope::Shared),
        blurb: fill(blurb, Scope::Shared),
    }
}

/// Whether a hero has a talent tree at all.
pub fn has_talents(hero: u16) -> bool {
    hero_def(HeroId(hero)).is_some_and(|def| def.talents.is_some())
}

/// A hero's talent tree by talent slot, if it has one.
pub fn talents(hero: u16) -> Option<Vec<TalentFace>> {
    let tree = hero_def(HeroId(hero))?.talents?;
    Some(tree.iter().copied().map(talent_face).collect())
}

/// A unit's attack damage the way Dota's panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRange {
    /// The least and the most a swing deals without bonuses: the base
    /// range plus the primary attribute.
    pub min: i32,
    pub max: i32,
    /// What items and effects add on top of that, shown apart.
    pub bonus: i32,
}

/// Splits the range a swing draws from, as the view tells it, into the base
/// range and the bonus on top.
pub fn damage_range(unit: &bota_proto::UnitView) -> DamageRange {
    // White, as Dota draws it: the hero's base range and what its primary
    // attribute pays for every point it holds, from items too; green: flat
    // bonus damage alone.
    let base_least = unit
        .hero
        .and_then(hero_def)
        .map(|hero| white_damage_least(hero.unit, unit.level, sane(unit.attributes)));
    // Saturating: the view is a peer's word.
    let bonus = base_least.map_or(0, |base| unit.attack_damage_min.saturating_sub(base).max(0));
    DamageRange {
        min: unit.attack_damage_min.saturating_sub(bonus),
        max: unit.attack_damage_max.saturating_sub(bonus),
        bonus,
    }
}

/// One stat as Dota's panel splits it: white, what the unit's kind and level
/// and every attribute point it holds are worth; green, what items, buffs,
/// auras and talents add on top (negative for a loss).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split<T> {
    pub base: T,
    pub bonus: T,
}

impl Split<i32> {
    /// Saturating: the view is a peer's word.
    fn whole(base: i32, total: i32) -> Split<i32> {
        Split {
            base,
            bonus: total.saturating_sub(base),
        }
    }
}

impl Split<Fixed> {
    /// Saturating, on the same grounds.
    fn fixed(base: Fixed, total: Fixed) -> Split<Fixed> {
        Split {
            base,
            bonus: Fixed {
                raw: total.raw.saturating_sub(base.raw),
            },
        }
    }
}

/// Most of an attribute a view may claim that the client works with: far
/// past any hero's, so that a peer's absurd number cannot overflow what is
/// worked out of it.
const MOST_ATTRIBUTE: i32 = 10_000;

/// A fraction from the view as a percent, held to the range of a percent
/// either way.
fn percent(fraction: Fixed) -> Fixed {
    fraction.clamp(-Fixed::ONE, Fixed::ONE) * Fixed::from_int(100)
}

/// A Q16.16 number, widened, to the nearest whole, held to `i32`.
fn rounded(raw: i64) -> i32 {
    let half = 1i64 << (Fixed::FRAC_BITS - 1);
    ((raw + half) >> Fixed::FRAC_BITS).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// A view's attributes held between none and [`MOST_ATTRIBUTE`].
fn sane(has: bota_proto::Attributes) -> bota_proto::Attributes {
    let held = |f: Fixed| f.clamp(Fixed::ZERO, Fixed::from_int(MOST_ATTRIBUTE));
    bota_proto::Attributes {
        strength: held(has.strength),
        agility: held(has.agility),
        intelligence: held(has.intelligence),
    }
}

/// A hero's stats split as Dota's panel and its stats tooltip split them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatSplit {
    pub armor: Split<Fixed>,
    pub attack_speed: Split<i32>,
    pub move_speed: Split<i32>,
    pub attack_range: Split<i32>,
    /// Health and mana mended a second.
    pub hp_regen: Split<Fixed>,
    pub mana_regen: Split<Fixed>,
    /// Magic resistance, percent: white the hero's base with what its
    /// intelligence adds to it, as Dota's Base Magic Resistance.
    pub magic_resist: Split<Fixed>,
    /// Status resistance and evasion, percent: a hero has none of its own,
    /// so all of either is green.
    pub status_resist: Split<Fixed>,
    pub evasion: Split<Fixed>,
    /// Strength, agility, intelligence in whole points: white what the
    /// hero's level gives, green what anything else does.
    pub attributes: [Split<i32>; 3],
}

/// Splits a hero's stats, as the view tells them, into Dota's white and
/// green. None for anything but a hero.
pub fn stat_split(unit: &bota_proto::UnitView) -> Option<StatSplit> {
    let kind = hero_def(unit.hero?)?.unit;
    let levels = i32::from(unit.level.max(1)) - 1;
    let has = sane(unit.attributes);
    let a_second = Fixed::from_int(bota_core::rules::TICKS_PER_SECOND as i32);
    let paid = attribute_regen(has);
    let level = base_attributes(unit)?;
    let total = [has.strength, has.agility, has.intelligence].map(Fixed::to_int);
    let whole = |f: Fixed| rounded(i64::from(f.raw));
    Some(StatSplit {
        armor: Split::fixed(Fixed::from_int(kind.armor) + agility_armor(has), unit.armor),
        attack_speed: Split::whole(
            kind.attack_speed + kind.per_level.attack_speed * levels + agility_attack_speed(has),
            unit.attack_speed,
        ),
        move_speed: Split::whole(kind.move_speed, whole(unit.move_speed)),
        attack_range: Split::whole(kind.attack_range, whole(unit.attack_range)),
        hp_regen: Split::fixed((kind.hp_regen + paid.hp) * a_second, unit.hp_regen),
        mana_regen: Split::fixed((kind.mana_regen + paid.mana) * a_second, unit.mana_regen),
        // Worked out as the server's view is, so that none is told green.
        magic_resist: Split::fixed(
            percent(Fixed::from_ratio(
                kind.magic_resist_pct * 100 + intelligence_magic_resist_bp(has),
                bota_core::rules::NOMINAL_BP,
            )),
            percent(unit.magic_resist),
        ),
        status_resist: Split::fixed(Fixed::ZERO, percent(unit.status_resist)),
        evasion: Split::fixed(Fixed::ZERO, percent(unit.evasion)),
        attributes: [0, 1, 2].map(|k| Split::whole(level[k].min(total[k]), total[k])),
    })
}

/// A hero's strength, agility and intelligence of its level alone, before
/// items, souls, buffs and talents: whole points, as Dota counts them.
pub fn base_attributes(unit: &bota_proto::UnitView) -> Option<[i32; 3]> {
    let has = attributes_at_level(hero_def(unit.hero?)?.unit, unit.level);
    Some([has.strength, has.agility, has.intelligence].map(Fixed::to_int))
}

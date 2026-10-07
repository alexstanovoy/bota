use bota_proto::{Fixed, HeroId, ItemId, SlotId, Target, Team, Vec2};

use crate::game::{BAG_SLOTS, Entity, Inventory, ItemStack, Seat, World, fountain_pos, rules};

pub(super) const MANGO: ItemId = ItemId(42);
pub(super) const OWNER: SlotId = SlotId(0);
pub(super) const AWAY: Vec2 = Vec2::from_ints(8000, 8000);

pub(super) fn fixture() -> (World, Entity) {
    let mut world = World::new();
    let home = fountain_pos(world.map, Team::Radiant);
    let hero = world.spawn_hero(Team::Radiant, home, OWNER, HeroId(2));
    let mut seat = Seat::new(OWNER, Team::Radiant, HeroId(2), 1000, rules::STASH_SLOTS);
    seat.unit = Some(hero);
    world.seats.push(seat);
    world.settle();
    world.stats.get_mut(hero).unwrap().max_mana = Fixed::from_int(500);
    world.mana.get_mut(hero).unwrap().mana = Fixed::ZERO;
    assert!(world.alive(hero));
    assert!(world.at_shop(hero));
    (world, hero)
}

pub(super) fn mango(charges: u8) -> ItemStack {
    assert!(charges > 0);
    assert!(charges <= 3);
    ItemStack {
        id: MANGO,
        charges,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: OWNER,
        for_sale: false,
    }
}

pub(super) fn put(world: &mut World, unit: Entity, at: usize, stack: ItemStack) {
    let bag = world.inventory.get_mut(unit).unwrap();
    assert!(at < bag.slots.len());
    assert!(bag.slots[at].is_none());
    bag.slots[at] = Some(stack);
}

pub(super) fn held(world: &World, unit: Entity, at: usize) -> ItemStack {
    world.inventory.get(unit).unwrap().slots[at].unwrap()
}

pub(super) fn fill_storage(world: &mut World, hero: Entity) {
    let mut filler = mango(1);
    filler.id = ItemId(37);
    filler.charges = 0;
    world
        .inventory
        .get_mut(hero)
        .unwrap()
        .slots
        .fill(Some(filler));
    world.seats[0].stash.slots.fill(Some(filler));
    assert_eq!(world.inventory.get(hero).unwrap().held().count(), BAG_SLOTS);
    assert_eq!(world.seats[0].stash.held().count(), rules::STASH_SLOTS);
}

pub(super) fn reject_use(world: &mut World, hero: Entity, at: usize, target: Target) {
    let bag = world.inventory.get(hero).cloned();
    let mana = world.mana.get(hero).copied();
    let health = world.health.get(hero).copied();
    let hash = world.hash();
    let mut events = Vec::new();
    assert!(!world.use_item(hero, at, target, &mut events));
    assert_eq!(world.inventory.get(hero), bag.as_ref());
    assert_eq!(world.mana.get(hero).copied(), mana);
    assert_eq!(world.health.get(hero).copied(), health);
    assert_eq!(world.hash(), hash);
    assert!(events.is_empty());
}

pub(super) fn reject_buy(world: &mut World, hero: Entity) {
    let bag = world.inventory.get(hero).unwrap().clone();
    let stash = world.seats[0].stash.clone();
    let gold = world.seats[0].gold;
    let hash = world.hash();
    let mut events = Vec::new();
    assert!(!world.buy(OWNER, MANGO, &mut events));
    assert_eq!(world.inventory.get(hero), Some(&bag));
    assert_eq!(world.seats[0].stash, stash);
    assert_eq!(world.seats[0].gold, gold);
    assert_eq!(world.hash(), hash);
    assert!(events.is_empty());
}

pub(super) fn charges_in(bag: &Inventory) -> u32 {
    bag.held()
        .filter(|stack| stack.id == MANGO)
        .map(|stack| u32::from(stack.charges))
        .sum()
}

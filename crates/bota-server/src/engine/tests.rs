//! Entity allocation and component storage.

use crate::engine::{Entity, EntityAllocator, Table};

#[test]
fn an_allocator_counts_what_is_live() {
    let mut entities = EntityAllocator::new();
    assert!(entities.is_empty());
    let first = entities.alloc();
    let second = entities.alloc();
    assert_eq!(entities.len(), 2);
    assert!(entities.contains(first) && entities.contains(second));
    assert!(entities.free(first));
    assert_eq!(entities.len(), 1);
    assert!(!entities.is_empty());
}

#[test]
fn a_handle_kept_past_a_death_names_nobody() {
    let mut entities = EntityAllocator::new();
    let dead = entities.alloc();
    assert!(entities.free(dead));
    assert!(!entities.contains(dead));
    assert!(!entities.free(dead), "freeing twice has to be refused");
}

#[test]
fn a_slot_handed_out_again_carries_a_raised_generation() {
    let mut entities = EntityAllocator::new();
    let first = entities.alloc();
    assert!(entities.free(first));
    let second = entities.alloc();
    assert_eq!(first.index(), second.index(), "the free slot is taken back");
    assert_ne!(first.generation(), second.generation());
    assert!(!entities.contains(first) && entities.contains(second));
}

#[test]
fn live_entities_come_out_in_slot_order() {
    let mut entities = EntityAllocator::new();
    let all: Vec<Entity> = (0..4).map(|_| entities.alloc()).collect();
    assert!(entities.free(all[1]));
    let live: Vec<Entity> = entities.iter().collect();
    assert_eq!(live, vec![all[0], all[2], all[3]]);
    let fresh = entities.alloc();
    let live: Vec<Entity> = entities.iter().collect();
    assert_eq!(
        live,
        vec![all[0], fresh, all[2], all[3]],
        "a reused slot is walked where it sits, not where it was made"
    );
}

#[test]
fn a_table_holds_a_component_for_the_entity_that_owns_it() {
    let mut entities = EntityAllocator::new();
    let mine = entities.alloc();
    let theirs = entities.alloc();
    let mut table: Table<i32> = Table::new();
    assert_eq!(table.insert(mine, 7), None);
    assert_eq!(table.get(mine), Some(&7));
    assert!(!table.contains(theirs));
    assert_eq!(table.insert(mine, 9), Some(7), "the old value comes back");
    *table.get_mut(mine).expect("just written") += 1;
    assert_eq!(table.get(mine), Some(&10));
    assert_eq!(table.remove(mine), Some(10));
    assert!(!table.contains(mine));
    assert_eq!(table.remove(mine), None);
}

#[test]
fn what_a_dead_entity_left_is_never_the_new_tenants() {
    let mut entities = EntityAllocator::new();
    let first = entities.alloc();
    let mut table: Table<i32> = Table::new();
    table.insert(first, 7);
    assert!(entities.free(first));
    let second = entities.alloc();
    assert_eq!(first.index(), second.index());
    assert_eq!(table.get(second), None, "the slot came empty");
    assert_eq!(
        table.insert(second, 3),
        None,
        "nothing of its own to return"
    );
    assert_eq!(table.get(second), Some(&3));
    assert_eq!(table.get(first), None, "the old handle reads nothing");
}

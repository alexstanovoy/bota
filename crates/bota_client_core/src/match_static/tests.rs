use bota_proto::{HeroId, ItemId, MapId, MatchInfo, Pick, ShopEntry, SlotId, Team, TickMode, Vec2};

use crate::{MatchStatic, SessionError};

fn info(shop: Vec<ShopEntry>) -> MatchInfo {
    MatchInfo {
        match_id: 1,
        map: MapId(0),
        tick_rate: 30,
        pregame_ticks: 0,
        trees: Vec::new(),
        terrain_cells: 128,
        terrain_rle: vec![(16384, 0x80)],
        opaque_cells: Vec::new(),
        mode: TickMode::Lockstep,
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::PUDGE,
        }],
        shop,
        fountains: [Vec2::ZERO; 2],
        shop_range: 0,
        secret_shops: Vec::new(),
        universal_shop: false,
        cheats: false,
        objectives: bota_proto::ObjectiveSet::NONE,
        runes: bota_proto::RuneSet::NONE,
        catalogue: bota_core::Catalogue::FULL.prefix(),
    }
}

#[test]
fn shop_context_rejects_home_ranges_that_cannot_be_predicted_without_overflow() {
    for range in [i32::MIN, -1, 32768, i32::MAX] {
        let mut frame = info(Vec::new());
        frame.shop_range = range;
        assert_eq!(
            MatchStatic::new(&frame).err(),
            Some(SessionError::Stream(
                "home shop range does not fit nonnegative fixed-point coordinates"
            ))
        );
    }
}

#[test]
fn a_secret_shop_rejects_a_crossing_footprint_even_when_each_corner_turns_left() {
    let points = [(200, 64), (329, 158), (280, 310), (120, 310), (71, 158)];
    let mut frame = info(Vec::new());
    frame.secret_shops = vec![bota_proto::ShopZoneView {
        pos: Vec2::from_ints(200, 200),
        corners: [0, 2, 4, 1, 3]
            .map(|index| Vec2::from_ints(points[index].0, points[index].1))
            .to_vec(),
    }];
    assert_eq!(
        MatchStatic::new(&frame),
        Err(SessionError::Stream(
            "shop zone must be a convex counter-clockwise polygon"
        ))
    );
}

#[test]
fn a_secret_shop_rejects_an_out_of_map_shopkeeper() {
    let mut frame = info(Vec::new());
    frame.secret_shops = vec![bota_proto::ShopZoneView {
        pos: Vec2::from_ints(9000, 128),
        corners: vec![
            Vec2::from_ints(64, 64),
            Vec2::from_ints(256, 64),
            Vec2::from_ints(256, 256),
            Vec2::from_ints(64, 256),
        ],
    }];
    let Err(SessionError::PositionOutOfMap { field, .. }) = MatchStatic::new(&frame) else {
        panic!("out-of-map shopkeeper rejected")
    };
    assert_eq!(field, "ShopZoneView.pos");
}

/// Each item of a chain made of sixteen of the next: laid out whole, the
/// first would be 16^39 items, which froze a client asked what it still
/// lacks of it.
#[test]
fn a_shop_whose_recipes_multiply_out_is_refused() {
    let chain = 40u16;
    let shop = (0..chain)
        .map(|at| ShopEntry {
            id: ItemId(at),
            cost: 1,
            components: if at + 1 < chain {
                vec![ItemId(at + 1); 16]
            } else {
                Vec::new()
            },
            alternatives: Vec::new(),
            secret_shop: false,
        })
        .collect();
    assert_eq!(
        MatchStatic::new(&info(shop)),
        Err(SessionError::InvalidShop("has a recipe tree too large"))
    );
}

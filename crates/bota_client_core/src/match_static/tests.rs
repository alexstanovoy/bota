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
        cheats: false,
        objectives: bota_proto::ObjectiveSet::NONE,
        runes: bota_proto::RuneSet::NONE,
        catalogue: bota_core::Catalogue::FULL.prefix(),
    }
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
        })
        .collect();
    assert_eq!(
        MatchStatic::new(&info(shop)),
        Err(SessionError::InvalidShop("has a recipe tree too large"))
    );
}

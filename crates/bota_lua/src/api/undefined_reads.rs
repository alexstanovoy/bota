//! Optional diagnostics for missing API names. A missing lookup still returns nil.
use super::Shared;
use crate::BindingError;
use luagee::Lua;
use std::rc::Rc;

const PREFIXES: &[&[u8]] = &[
    b"Get",
    b"Is",
    b"Has",
    b"Can",
    b"Was",
    b"TimeSince",
    b"Action",
    b"Num",
    b"Distance",
    b"Random",
    b"DotaTime",
    b"GameTime",
    b"RealTime",
    b"Glyph",
    b"Upgrade",
    b"Toggle",
    b"Set",
    b"Find",
    b"UnitTo",
    b"Create",
    b"BOT_",
    b"UNIT_",
    b"ABILITY_",
    b"ACTIVITY_",
    b"COURIER_",
    b"DAMAGE_",
    b"ATTRIBUTE_",
    b"TEAM_",
    b"LANE_",
    b"RUNE_",
    b"SHOP_",
    b"ITEM_",
    b"GAME_",
    b"GAMEMODE_",
    b"HEROPICK_",
    b"HERO_PICK_",
    b"PURCHASE_",
    b"DIFFICULTY_",
    b"SHRINE_",
    b"TOWER_",
    b"BARRACKS_",
    b"CDOTA",
    b"DOTA_",
    b"MODIFIER_",
    b"FIND_",
];
fn looks_like_api(name: &[u8]) -> bool {
    PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}
pub(crate) fn install(lua: &mut Lua, state: Shared) -> Result<(), BindingError> {
    let Some(stats) = state.borrow().statistics.clone() else {
        return Ok(());
    };
    let tables = [
        Some(lua.globals()),
        state.borrow().unit_methods,
        state.borrow().ability_methods,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    lua.vm().observe_missing_reads(
        &tables,
        Some(Rc::new(move |_, name| {
            if looks_like_api(name) {
                stats.borrow_mut().undefined_api_reads.record(name);
            }
        })),
    )?;
    Ok(())
}

//! Dota's Area of Effect bonus: world units a carrier's Chasm Stone adds to every radius its
//! script marks `affected_by_aoe_increase`; one not marked (a cast range, a hook's width) is
//! left as it is.

/// A radius of `radius` world units widened by an AoE bonus of `bonus`: Dota adds the bonus
/// ("Effective radius increased from 350 to 400 due to item's built-in Area of Effect bonus",
/// 7.41b Gleipnir).
pub const fn widened(radius: i32, bonus: i32) -> i32 {
    radius + bonus
}

/// The values of bota's heroes' abilities that an AoE bonus widens, by ability and value key:
/// those their scripts mark `affected_by_aoe_increase`. An install-gated test of `import_items`
/// holds this list to the install's; each is read through [`widened`] where bota uses it. The
/// items' marks come with their rows ([`ItemData::aoe`](crate::ItemData::aoe)).
pub const AOE_ABILITY_VALUES: [(&str, &str); 9] = [
    ("nevermore_shadowraze1", "shadowraze_radius"),
    ("nevermore_shadowraze2", "shadowraze_radius"),
    ("nevermore_shadowraze3", "shadowraze_radius"),
    ("nevermore_dark_lord", "presence_radius"),
    ("nevermore_requiem", "requiem_radius"),
    ("nevermore_requiem", "requiem_line_width_start"),
    ("nevermore_requiem", "requiem_line_width_end"),
    ("pudge_rot", "rot_radius"),
    ("pudge_innate_graft_flesh", "flesh_heap_range"),
];

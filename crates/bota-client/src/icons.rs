//! The art, turned from drawings into something the screen can take.
//!
//! Each file draws its icon inside one fixed frame of a wider canvas; what is
//! rasterised is that frame alone, once, the first time it is asked for.

use macroquad::prelude::*;

/// The frame the icon is drawn in, inside the canvas of the file.
const FRAME: (f32, f32, f32, f32) = (220.0, 50.0, 240.0, 160.0);

/// How many pixels one unit of that frame is rasterised to.
const SCALE: f32 = 0.8;

/// The texture of one drawing, rasterised on the first call and cached.
pub fn icon(art: &'static [u8]) -> Option<Texture2D> {
    thread_local! {
        static DRAWN: std::cell::RefCell<Vec<(usize, Option<Texture2D>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = art.as_ptr() as usize;
    DRAWN.with(|slot| {
        let mut drawn = slot.borrow_mut();
        if let Some((_, texture)) = drawn.iter().find(|(seen, _)| *seen == key) {
            return texture.clone();
        }
        let texture = rasterise(art);
        drawn.push((key, texture.clone()));
        texture
    })
}

/// Turns one drawing into a texture of its icon frame.
fn rasterise(art: &[u8]) -> Option<Texture2D> {
    let (w, h, bytes) = pixels(art)?;
    let image = Image {
        bytes,
        width: w as u16,
        height: h as u16,
    };
    let texture = Texture2D::from_image(&image);
    texture.set_filter(FilterMode::Linear);
    Some(texture)
}

/// The icon frame of one drawing as width, height and straight-alpha RGBA.
pub fn pixels(art: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let tree = resvg::usvg::Tree::from_data(art, &resvg::usvg::Options::default()).ok()?;
    let (w, h) = ((FRAME.2 * SCALE) as u32, (FRAME.3 * SCALE) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    // The frame is moved to the corner and scaled, so what lands on the pixmap
    // is the icon and none of the canvas around it.
    let put = resvg::tiny_skia::Transform::from_row(
        SCALE,
        0.0,
        0.0,
        SCALE,
        -FRAME.0 * SCALE,
        -FRAME.1 * SCALE,
    );
    resvg::render(&tree, put, &mut pixmap.as_mut());
    let mut bytes = pixmap.data().to_vec();
    // What comes out is multiplied by its own alpha; what goes to the screen
    // is not.
    for pixel in bytes.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in pixel.iter_mut().take(3) {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    Some((w, h, bytes))
}

/// The drawing of an item, an SVG file, by the item's id.
pub fn item_art(id: u16) -> Option<&'static [u8]> {
    Some(match id {
        0 => include_bytes!("../assets/items/boots.svg"),
        1 => include_bytes!("../assets/items/clarity.svg"),
        2 => include_bytes!("../assets/items/healing_salve.svg"),
        3 => include_bytes!("../assets/items/iron_branch.svg"),
        4 => include_bytes!("../assets/items/observer_ward.svg"),
        5 => include_bytes!("../assets/items/quelling_blade.svg"),
        6 => include_bytes!("../assets/items/sentry_ward.svg"),
        7 => include_bytes!("../assets/items/tango.svg"),
        8 => include_bytes!("../assets/items/town_portal_scroll.svg"),
        9 => include_bytes!("../assets/items/circlet.svg"),
        10 => include_bytes!("../assets/items/gauntlets.svg"),
        11 => include_bytes!("../assets/items/slippers.svg"),
        12 => include_bytes!("../assets/items/mantle.svg"),
        13 => include_bytes!("../assets/items/belt.svg"),
        14 => include_bytes!("../assets/items/band.svg"),
        15 => include_bytes!("../assets/items/robe.svg"),
        16 => include_bytes!("../assets/items/ogre_axe.svg"),
        17 => include_bytes!("../assets/items/blade_of_alacrity.svg"),
        18 => include_bytes!("../assets/items/staff_of_wizardry.svg"),
        19 => include_bytes!("../assets/items/gloves.svg"),
        20 => include_bytes!("../assets/items/blades_of_attack.svg"),
        21 => include_bytes!("../assets/items/broadsword.svg"),
        22 => include_bytes!("../assets/items/quarterstaff.svg"),
        23 => include_bytes!("../assets/items/ring_of_protection.svg"),
        24 => include_bytes!("../assets/items/chainmail.svg"),
        25 => include_bytes!("../assets/items/ring_of_regen.svg"),
        26 => include_bytes!("../assets/items/sages_mask.svg"),
        27 => include_bytes!("../assets/items/vitality_booster.svg"),
        28 => include_bytes!("../assets/items/energy_booster.svg"),
        29 => include_bytes!("../assets/items/power_treads.svg"),
        30 => include_bytes!("../assets/items/phase_boots.svg"),
        31 => include_bytes!("../assets/items/blink_dagger.svg"),
        32 => include_bytes!("../assets/items/bracer.svg"),
        33 => include_bytes!("../assets/items/wraith_band.svg"),
        34 => include_bytes!("../assets/items/null_talisman.svg"),
        35 => include_bytes!("../assets/items/magic_stick.svg"),
        36 => include_bytes!("../assets/items/magic_wand.svg"),
        37 => include_bytes!("../assets/items/recipe.svg"),
        38 => include_bytes!("../assets/items/recipe.svg"),
        39 => include_bytes!("../assets/items/recipe.svg"),
        40 => include_bytes!("../assets/items/recipe.svg"),
        41 => include_bytes!("../assets/items/recipe.svg"),
        42 => include_bytes!("../assets/items/mango.svg"),
        43 => include_bytes!("../assets/items/eaglesong.svg"),
        44 => include_bytes!("../assets/items/claymore.svg"),
        45 => include_bytes!("../assets/items/talisman_of_evasion.svg"),
        46 => include_bytes!("../assets/items/butterfly.svg"),
        47 => include_bytes!("../assets/items/javelin.svg"),
        48 => include_bytes!("../assets/items/demon_edge.svg"),
        49 => include_bytes!("../assets/items/blitz_knuckles.svg"),
        50 => include_bytes!("../assets/items/recipe.svg"),
        51 => include_bytes!("../assets/items/monkey_king_bar.svg"),
        52 => include_bytes!("../assets/items/wind_lace.svg"),
        53 => include_bytes!("../assets/items/blood_grenade.svg"),
        _ => return None,
    })
}

/// Draws an item's icon to fill a box, or reports that there is none to draw.
pub fn draw_item_icon(id: u16, x: f32, y: f32, w: f32, h: f32) -> bool {
    let Some(texture) = item_art(id).and_then(icon) else {
        return false;
    };
    draw_texture_ex(
        &texture,
        x,
        y,
        WHITE,
        DrawTextureParams {
            dest_size: Some(vec2(w, h)),
            ..Default::default()
        },
    );
    true
}

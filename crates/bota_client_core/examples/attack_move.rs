//! The shortest bot: it attack-moves at the enemy fountain every tick.
//!
//! ```text
//! cargo run --release -p bota_server -- --map 2 --mode lockstep --dummy-seat
//! cargo run --release -p bota_client_core --example attack_move [ADDRESS]
//! ```

use bota_client_core::{IssuedOrder, Link, PlayError};
use bota_core::fountain_pos;
use bota_proto::{HeroId, Order, Target};

fn main() -> Result<(), PlayError> {
    let address = std::env::args().nth(1).unwrap_or("127.0.0.1:4455".into());
    let outcome = Link::play(&address, "attack-move", HeroId::SHADOW_FIEND, |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        let attack = IssuedOrder::hero(Order::Attack {
            target: Target::Pos(enemy),
        });
        session.own_hero().map(|_| attack) // a dead hero takes no orders
    })?;
    println!("{outcome:?}");
    Ok(())
}

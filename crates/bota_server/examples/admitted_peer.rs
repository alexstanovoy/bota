//! A client to play a server of another build with: it takes a seat at `ADDRESS`, buys the item
//! `ITEM` once the match shows it a snapshot, and stops when it is told of the purchase, saying
//! what it saw: `catalogue N tick T bought ITEM`. Exit 1 with the reason otherwise.
//!
//! ```text
//! cargo run --release -p bota_server --example admitted_peer -- ADDRESS ITEM
//! ```
//!
//! `tools/admitted.sh build` builds it into every admitted wire's build, for
//! `tests/admitted_builds.rs`: a client of that build against a server of this one.

#[path = "../tests/admitted/peer.rs"]
mod peer;

use bota_proto::ItemId;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(address), Some(item), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: admitted_peer ADDRESS ITEM");
        std::process::exit(2);
    };
    let Ok(item) = item.parse().map(ItemId) else {
        eprintln!("admitted_peer: {item:?} is not an item id");
        std::process::exit(2);
    };
    match peer::play(&address, item) {
        Ok(seen) => println!("{seen}"),
        Err(error) => {
            eprintln!("admitted_peer: {error}");
            std::process::exit(1);
        }
    }
}

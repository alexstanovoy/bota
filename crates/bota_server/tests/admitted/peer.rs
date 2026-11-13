//! A client that takes a seat at a server, buys one item and tells what it saw: the program of
//! `examples/admitted_peer.rs`, built into every admitted wire's build by `tools/admitted.sh
//! build`, and this build's own client in `tests/admitted_builds.rs`. It takes only what every
//! build since the first admitted wire has.

use std::time::Duration;

use bota_client_core::{Link, Transport};
use bota_proto::{ClientMsg, EventKind, HeroId, ItemId, Order, ServerMsg};

/// Messages read before giving up on the purchase.
const MOST_MESSAGES: usize = 20_000;

/// Takes a seat at `address` as a lone Shadow Fiend, buys `item` once the match shows a snapshot
/// and returns, once told of the purchase, `catalogue N tick T bought ITEM`: the items the match
/// plays with, the tick of the purchase and the item.
pub fn play(address: &str, item: ItemId) -> std::io::Result<String> {
    let (mut link, seated) = Link::join_with_timeout(
        address,
        "admitted peer",
        HeroId::SHADOW_FIEND,
        None,
        Duration::from_secs(30),
    )?;
    let mut catalogue = None;
    let mut asked = false;
    for _ in 0..MOST_MESSAGES {
        let message = link
            .recv()?
            .ok_or_else(|| std::io::Error::other("the server closed the connection"))?;
        match message {
            ServerMsg::MatchStart { info } => catalogue = Some(info.catalogue.items),
            ServerMsg::Snapshot { view } if !asked => {
                asked = true;
                link.send(&ClientMsg::Order {
                    seq: 1,
                    unit: None,
                    order: Order::Buy { item },
                    seen: Some(view.tick),
                    queue: false,
                })?;
                link.send(&ClientMsg::Ack { tick: view.tick })?;
            }
            ServerMsg::Snapshot { view } => link.send(&ClientMsg::Ack { tick: view.tick })?,
            ServerMsg::Events { tick, events } => {
                let bought = events.iter().any(|event| {
                    matches!(event, EventKind::ItemBought { slot, item: had }
                        if *slot == seated.slot && *had == item)
                });
                if bought {
                    let catalogue = catalogue.unwrap_or_default();
                    return Ok(format!(
                        "catalogue {catalogue} tick {tick} bought {}",
                        item.0
                    ));
                }
            }
            ServerMsg::OrderRejected { reason, .. } => {
                return Err(std::io::Error::other(format!(
                    "the purchase was refused: {reason:?}"
                )));
            }
            _ => {}
        }
    }
    Err(std::io::Error::other(format!(
        "{MOST_MESSAGES} messages without the purchase"
    )))
}

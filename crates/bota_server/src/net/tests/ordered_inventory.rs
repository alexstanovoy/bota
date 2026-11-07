//! Application-time refusals reach the socket without erasing admitted attempts from replay.

use super::*;
use bota_proto::{ItemId, SlotOrder, TickMode};

fn replay_directory() -> std::path::PathBuf {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).expect("temporary directory entropy");
    let directory = std::env::temp_dir().join(format!(
        "bota-net-purchases-{:032x}",
        u128::from_le_bytes(nonce)
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .recursive(false)
        .create(&directory)
        .expect("exclusive private replay directory");
    directory
}

fn replay_orders(path: &std::path::Path, tick: u32) -> Vec<SlotOrder> {
    let bytes = read_replay(path).expect("finished replay");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    while let Some(record) = reader.next_message::<ReplayRecord>().expect("valid replay") {
        if let ReplayRecord::Orders { tick: at, orders } = record
            && at == tick
        {
            return orders;
        }
    }
    panic!("the replay records tick {tick}");
}

#[test]
fn same_tick_purchases_refuse_the_second_on_the_socket_and_record_both_attempts() {
    let directory = replay_directory();
    let path = directory.join("attempts.brp");
    let mut opts = realtime(41);
    opts.players = 1;
    opts.mode = TickMode::Lockstep;
    opts.ack_timeout_ticks = 600;
    opts.empty_after = Some(Duration::ZERO);
    opts.replay = Some(path.clone());
    opts.replay_max_bytes = Some(1 << 20);
    let server = start_ending_server(opts);
    let (mut client, slot) = join_as_bot(server.addr, "buyer");
    let slot = slot.expect("player seat");
    let first = client.recv_until(snapshot);
    let gold = first.player(slot).expect("player").gold.expect("own gold");
    let cost = bota_core::item_def(ItemId::BOOTS).expect("boots").cost;
    assert!(gold >= cost, "enough gold for one pair");
    assert!(gold < 2 * cost, "not enough gold for two pairs");
    let buy = Order::Buy {
        item: ItemId::BOOTS,
    };
    let mut frames = Vec::new();
    for seq in [10, 11] {
        frames.extend(
            encode_frame_to_vec(&ClientMsg::Order {
                seq,
                unit: None,
                order: buy,
                seen: Some(first.tick),
                queue: false,
            })
            .expect("order frame"),
        );
    }
    client
        .stream
        .write_all(&frames)
        .expect("both orders in one write");
    // The echo proves both preceding frames reached the waiting lockstep tick.
    client.say_all_and_hear("purchase batch arrived");
    client.send(&ClientMsg::Ack { tick: first.tick });
    let told = client.collect_through_tick(first.tick + 1);
    let refusals: Vec<_> = told
        .iter()
        .filter_map(|message| match message {
            ServerMsg::OrderRejected { seq, reason } => Some((*seq, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(refusals, vec![(11, RejectReason::NotEnoughGold)]);
    let applied = told
        .into_iter()
        .find_map(snapshot)
        .expect("result snapshot");
    assert_eq!(applied.tick, first.tick + 1);
    let player = applied.player(slot).expect("player");
    assert_eq!(player.gold, Some(gold - cost));
    let hero = applied
        .unit(player.unit.expect("hero"))
        .expect("visible hero");
    assert_eq!(
        hero.items
            .iter()
            .flatten()
            .filter(|i| i.id == ItemId::BOOTS)
            .count(),
        1
    );
    drop(client);
    server.end();
    let attempts = replay_orders(&path, applied.tick);
    std::fs::remove_file(path).expect("remove replay fixture");
    std::fs::remove_dir(directory).expect("remove replay directory");
    assert_eq!(
        attempts.len(),
        2,
        "the refused admitted attempt stays on the tape"
    );
    assert!(
        attempts
            .iter()
            .all(|order| order.slot == slot && order.order == buy)
    );
}

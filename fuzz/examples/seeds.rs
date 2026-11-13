//! Writes seed inputs for `server_session`, whose pieces are client messages:
//! `cargo run --manifest-path fuzz/Cargo.toml --example seeds`.

use std::path::Path;

use bota_proto::{
    ClientMsg, EntityId, Fixed, HeroId, Order, Role, SeatKey, SlotId, Target, Vec2,
    encode_frame_to_vec,
};

/// One piece: the connection, then the message's payload.
fn piece(conn: u8, msg: &ClientMsg) -> Vec<u8> {
    let body = &encode_frame_to_vec(msg).expect("encodes")[4..];
    let mut out = u16::try_from(body.len() + 1)
        .expect("small")
        .to_le_bytes()
        .to_vec();
    out.push(conn);
    out.extend_from_slice(body);
    out
}

fn order(seq: u32, order: Order) -> ClientMsg {
    ClientMsg::Order {
        seq,
        unit: None,
        order,
        seen: Some(seq),
        // Every other order queued, so the corpus reaches the queues.
        queue: seq % 2 == 0,
    }
}

fn main() {
    profile_seeds();
    let at = |x: i32, y: i32| {
        Target::Pos(Vec2 {
            x: Fixed { raw: x << 16 },
            y: Fixed { raw: y << 16 },
        })
    };
    let script = [
        (
            0,
            ClientMsg::hello(Role::Player, "one", None, bota_core::WIRE_FINGERPRINT),
        ),
        (
            1,
            ClientMsg::hello(Role::Bot, "two", None, bota_core::WIRE_FINGERPRINT),
        ),
        (
            2,
            ClientMsg::hello(Role::Spectator, "eyes", None, bota_core::WIRE_FINGERPRINT),
        ),
        (
            3,
            ClientMsg::hello(
                Role::Referee,
                "ref",
                Some(SeatKey([7; 32])),
                bota_core::WIRE_FINGERPRINT,
            ),
        ),
        (
            0,
            ClientMsg::PickHero {
                hero: HeroId::SHADOW_FIEND,
            },
        ),
        (
            1,
            ClientMsg::PickHero {
                hero: HeroId::PUDGE,
            },
        ),
        (0, ClientMsg::SetReady(true)),
        (1, ClientMsg::SetReady(true)),
        (
            2,
            ClientMsg::ViewAs {
                seat: Some(SlotId(1)),
            },
        ),
        (
            0,
            order(
                1,
                Order::Move {
                    target: at(8000, 8000),
                },
            ),
        ),
        (
            1,
            order(
                1,
                Order::Attack {
                    target: at(100, 100),
                },
            ),
        ),
        (0, ClientMsg::Ack { tick: 3 }),
        (
            1,
            order(
                2,
                Order::Attack {
                    target: Target::Unit(EntityId {
                        idx: 1,
                        generation: 1,
                    }),
                },
            ),
        ),
        (0, ClientMsg::Pong),
    ];
    let input: Vec<u8> = script
        .iter()
        .flat_map(|(conn, msg)| piece(*conn, msg))
        .collect();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/server_session");
    std::fs::create_dir_all(&dir).expect("corpus");
    std::fs::write(dir.join("seed-match"), &input).expect("seed");
    std::fs::write(dir.join("seed-lobby"), &input[..input.len() / 2]).expect("seed");
}

fn profile_seeds() {
    use bota_replay::meta::{MetaHeader, MetaRecord};
    let header = MetaRecord::Header(MetaHeader {
        magic: MetaHeader::MAGIC,
        schema: 1,
        wire_fingerprint: 7,
        rules_version: 0,
        match_id: 42,
        map: 2,
        tick_rate: 30,
        pregame_ticks: 0,
        catalogue_items: 0,
        catalogue_digest: 0,
    });
    let head = encode_frame_to_vec(&header).expect("header");
    let profile = encode_frame_to_vec(&MetaRecord::Profiles {
        matchmaker: Some("wss://match.example/control".into()),
        seats: vec![(0, Some(42)), (1, None)],
    })
    .expect("profile");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/replay_meta");
    std::fs::create_dir_all(&dir).expect("corpus");
    for (name, tail) in [
        ("profiles", profile.clone()),
        (
            "profiles_duplicate",
            [profile.clone(), profile.clone()].concat(),
        ),
        ("profiles_broken", [profile.clone(), vec![4, 0]].concat()),
        (
            "profiles_future",
            [
                profile,
                encode_frame_to_vec(&(200_u32, 17_u64)).expect("future"),
            ]
            .concat(),
        ),
        (
            "profiles_local",
            encode_frame_to_vec(&MetaRecord::Profiles {
                matchmaker: None,
                seats: vec![],
            })
            .expect("local"),
        ),
        (
            "profiles_zero",
            encode_frame_to_vec(&(
                13_u8,
                Some("wss://match.example/control"),
                vec![(0_u8, Some(0_u64))],
            ))
            .expect("zero"),
        ),
        (
            "profiles_private",
            encode_frame_to_vec(&(
                13_u8,
                Some("ws://192.168.1.5:7000"),
                vec![(0_u8, Some(42_u64))],
            ))
            .expect("private"),
        ),
        (
            "profiles_oversize",
            encode_frame_to_vec(&(13_u8, Some("x".repeat(513)), vec![(0_u8, Some(42_u64))]))
                .expect("oversize"),
        ),
    ] {
        std::fs::write(dir.join(name), [vec![0], head.clone(), tail].concat()).expect("seed");
    }
}

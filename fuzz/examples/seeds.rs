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
    }
}

fn main() {
    let at = |x: i32, y: i32| {
        Target::Pos(Vec2 {
            x: Fixed { raw: x << 16 },
            y: Fixed { raw: y << 16 },
        })
    };
    let script = [
        (0, ClientMsg::hello(Role::Player, "one", None)),
        (1, ClientMsg::hello(Role::Bot, "two", None)),
        (2, ClientMsg::hello(Role::Spectator, "eyes", None)),
        (
            3,
            ClientMsg::hello(Role::Referee, "ref", Some(SeatKey([7; 32]))),
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

use super::NativeTeam;
use crate::{HookActivity, TeamTurn};
use bota_client_core::IssuedOrder;
use bota_client_core::{ClientSession, Ingested, Outcome, Seat, Seated, SessionOptions, Transport};
use bota_proto::{ClientMsg, Order, ServerMsg, Target};
use std::io;

#[derive(Debug)]
pub struct TeamOutcome {
    pub hook_activity: HookActivity,
    pub outcomes: Vec<Outcome>,
    pub orders_sent: u64,
    pub script_failures: u64,
    pub retired: bool,
}
struct Connection<T> {
    transport: T,
    seat: Seat,
    seated: Seated,
    outcome: Option<Outcome>,
    rejections: u32,
    unsent: u32,
}

/// Drives all joined seats of one team directly over their transports; no relay or simulation access.
pub fn play_team<T: Transport>(
    connections: Vec<(T, Seated)>,
    worker: &mut NativeTeam,
) -> io::Result<TeamOutcome> {
    play_team_recorded(connections, worker, None)
}

/// Optionally records the first seat's already-filtered stream in the existing replay framing.
pub fn play_team_recorded<T: Transport>(
    connections: Vec<(T, Seated)>,
    worker: &mut NativeTeam,
    recording: Option<&mut dyn io::Write>,
) -> io::Result<TeamOutcome> {
    let mut recording = recording.map(Recording::new);
    if let Some(writer) = recording.as_mut() {
        writer.record(&bota_proto::ReplayRecord::Header(
            bota_proto::ReplayHeader::new(bota_core::WIRE_FINGERPRINT),
        ))?;
    }
    let mut connections = prepare_connections(connections, worker)?;
    let mut result = TeamOutcome {
        hook_activity: HookActivity::default(),
        outcomes: Vec::new(),
        orders_sent: 0,
        script_failures: 0,
        retired: false,
    };
    let mut last = None;
    let mut warned = false;
    for _ in 0..u32::MAX {
        for (index, connection) in connections.iter_mut().enumerate() {
            if connection.outcome.is_none() {
                if index == 0 {
                    collect_tick(connection, &mut recording)?;
                } else {
                    collect_tick(connection, &mut None)?;
                }
            }
        }
        if connections
            .iter()
            .all(|connection| connection.outcome.is_some())
        {
            result.outcomes = connections
                .into_iter()
                .map(|connection| connection.outcome.expect("all seats ended"))
                .collect();
            result.retired = worker.retired();
            return Ok(result);
        }
        if connections
            .iter()
            .any(|connection| connection.outcome.is_some())
        {
            continue;
        }
        let newest = newest_session(&connections)?;
        let tick = newest.current().expect("a complete tick").tick;
        let turn = if last.is_none_or(|last| tick > last) {
            last = Some(tick);
            decide(worker, newest, &mut result, &mut warned)
        } else {
            None
        };
        for connection in &mut connections {
            send_turn(connection, turn.as_ref(), &mut result, worker.retired())?;
        }
    }
    Err(io::Error::other("match exceeded the protocol tick range"))
}

fn prepare_connections<T: Transport>(
    connections: Vec<(T, Seated)>,
    worker: &NativeTeam,
) -> io::Result<Vec<Connection<T>>> {
    if connections.len() != worker.seats().len() || connections.is_empty() {
        return Err(io::Error::other("connections do not match Lua seats"));
    }
    let connections: Vec<_> = connections
        .into_iter()
        .map(|(transport, seated)| Connection {
            transport,
            seat: Seat::new(seated, SessionOptions::default()),
            seated,
            outcome: None,
            rejections: 0,
            unsent: 0,
        })
        .collect();
    for (index, connection) in connections.iter().enumerate() {
        if !worker
            .seats()
            .iter()
            .any(|seat| seat.slot == connection.seated.slot)
            || connections[..index]
                .iter()
                .any(|other| other.seated.slot == connection.seated.slot)
        {
            return Err(io::Error::other(
                "connections name duplicate or foreign seats",
            ));
        }
    }
    Ok(connections)
}

fn newest_session<T>(connections: &[Connection<T>]) -> io::Result<&ClientSession> {
    connections
        .iter()
        .filter_map(|connection| connection.seat.session())
        .max_by_key(|session| session.current().map_or(0, |view| view.tick))
        .ok_or_else(|| io::Error::other("tick without a session"))
}

fn decide(
    worker: &mut NativeTeam,
    session: &ClientSession,
    result: &mut TeamOutcome,
    warned: &mut bool,
) -> Option<TeamTurn> {
    let tick = session.current().expect("a complete tick").tick;
    match worker.think(session) {
        Ok(turn) => {
            result.hook_activity += turn.hook_activity;
            result.script_failures = result
                .script_failures
                .saturating_add(turn.failures.len() as u64)
                .saturating_add(turn.hook_activity.suppressed_team_errors);
            for failure in &turn.failures {
                if failure.report {
                    eprintln!(
                        "WARN pack=lua slot={} tick={} suppressed_team_errors={} error={:?}",
                        failure.slot.0,
                        tick,
                        turn.hook_activity.suppressed_team_errors,
                        failure.error.to_string()
                    );
                }
            }
            Some(turn)
        }
        Err(error) => {
            if !*warned {
                eprintln!("WARN pack=lua tick={tick} error={error:?}");
                *warned = true;
            }
            None
        }
    }
}

fn send_turn<T: Transport>(
    connection: &mut Connection<T>,
    turn: Option<&TeamTurn>,
    result: &mut TeamOutcome,
    retired: bool,
) -> io::Result<()> {
    let session = connection.seat.session_mut().expect("a complete tick");
    if retired {
        stand(&mut connection.transport, session, result)?;
    }
    if let Some(bot) = turn.and_then(|turn| {
        turn.bots
            .iter()
            .find(|bot| bot.slot == connection.seated.slot)
    }) {
        for line in &bot.chat {connection.transport.send(&ClientMsg::Say(line.clone()))?;}
        for &order in &bot.orders {
            if session.own_hero().is_none()
                && order.unit.is_none()
                && bota_core::order_needs_body(order.order)
            {
                connection.unsent = connection.unsent.saturating_add(1);
                continue;
            }
            if session.ledger().repeats(order) {
                continue;
            }
            let outgoing = session
                .issue(order)
                .map_err(|error| io::Error::other(error.to_string()))?;
            connection.transport.send(&outgoing.message())?;
            result.orders_sent += 1;
        }
    }
    if let Some(tick) = session.end_turn() {
        connection.transport.send(&ClientMsg::Ack { tick })?;
    }
    Ok(())
}

fn collect_tick<T: Transport>(
    connection: &mut Connection<T>,
    recording: &mut Option<Recording<'_>>,
) -> io::Result<()> {
    for _ in 0..65536 {
        let message = connection
            .transport
            .recv()?
            .ok_or_else(|| io::Error::other("server closed before match end"))?;
        if (connection.seat.session().is_some() || matches!(message, ServerMsg::MatchStart { .. }))
            && let Some(writer) = recording.as_mut()
        {
            writer.record(&bota_proto::ReplayRecord::Msg(message.clone()))?;
        }
        match connection
            .seat
            .ingest(message, &mut ())
            .map_err(|error| io::Error::other(error.to_string()))?
        {
            Ingested::TickComplete { .. } => return Ok(()),
            Ingested::Rejected { .. } => {
                connection.rejections = connection.rejections.saturating_add(1)
            }
            Ingested::Passed(message) if matches!(*message, ServerMsg::Ping) => {
                connection.transport.send(&ClientMsg::Pong)?
            }
            Ingested::Resumed { resend } => {
                for order in resend {
                    connection.transport.send(&order.message())?;
                }
            }
            Ingested::Over { winner, .. } => {
                let session = connection.seat.session().expect("end after start");
                connection.outcome = Some(Outcome {
                    slot: connection.seated.slot,
                    team: session.team(),
                    winner,
                    ticks: session.current().map_or(0, |view| view.tick),
                    rejections: connection.rejections,
                    unsent_while_dead: connection.unsent,
                });
                return Ok(());
            }
            _ => {}
        }
    }
    Err(io::Error::other(
        "server sent too many messages without a complete tick",
    ))
}

// Retirement is a host decision, so it uses only ordinary stop orders for the controlled bodies.
fn stand(
    transport: &mut impl Transport,
    session: &mut ClientSession,
    result: &mut TeamOutcome,
) -> io::Result<()> {
    let stop = Order::Move {
        target: Target::None,
    };
    let hero = session.own_hero().map(|_| IssuedOrder::hero(stop));
    let courier = session
        .own_courier()
        .map(|unit| IssuedOrder::courier(unit.id, stop));
    for order in [hero, courier].into_iter().flatten() {
        if session.ledger().repeats(order) {
            continue;
        }
        let outgoing = session
            .issue(order)
            .map_err(|error| io::Error::other(error.to_string()))?;
        transport.send(&outgoing.message())?;
        result.orders_sent += 1;
    }
    Ok(())
}

const MAX_RECORDING_BYTES: u64 = 1 << 30;
const MAX_RECORDING_FRAMES: u64 = 1 << 20;
struct Recording<'a> {
    writer: &'a mut dyn io::Write,
    bytes: u64,
    frames: u64,
    max_bytes: u64,
    max_frames: u64,
}
impl<'a> Recording<'a> {
    fn new(writer: &'a mut dyn io::Write) -> Self {
        Self {
            writer,
            bytes: 0,
            frames: 0,
            max_bytes: MAX_RECORDING_BYTES,
            max_frames: MAX_RECORDING_FRAMES,
        }
    }
    fn record(&mut self, record: &bota_proto::ReplayRecord) -> io::Result<()> {
        if self.frames >= self.max_frames {
            return Err(io::Error::other(
                "recording frame cap reached; recording was not published",
            ));
        }
        let frame = bota_proto::encode_frame_to_vec(record).map_err(io::Error::other)?;
        let bytes = self
            .bytes
            .checked_add(frame.len() as u64)
            .filter(|&bytes| bytes <= self.max_bytes)
            .ok_or_else(|| {
                io::Error::other("recording byte cap reached; recording was not published")
            })?;
        self.writer.write_all(&frame)?;
        self.bytes = bytes;
        self.frames += 1;
        Ok(())
    }
}
#[cfg(test)]
mod recording_tests {
    use super::*;
    #[test]
    fn recording_limits_stop_before_writing_an_excess_frame_or_byte() {
        let record = bota_proto::ReplayRecord::Header(bota_proto::ReplayHeader::new(
            bota_core::WIRE_FINGERPRINT,
        ));
        for byte_cap in [false, true] {
            let mut bytes = Vec::new();
            let size = {
                let mut writer = Recording::new(&mut bytes);
                if byte_cap {
                    writer.max_bytes =
                        bota_proto::encode_frame_to_vec(&record).unwrap().len() as u64;
                } else {
                    writer.max_frames = 1;
                }
                writer.record(&record).unwrap();
                let size = writer.bytes;
                let error = writer.record(&record).unwrap_err();
                assert!(error.to_string().contains(if byte_cap {
                    "byte cap"
                } else {
                    "frame cap"
                }));
                assert_eq!(writer.frames, 1);
                assert_eq!(writer.bytes, size);
                size
            };
            assert_eq!(bytes.len() as u64, size);
        }
    }
}

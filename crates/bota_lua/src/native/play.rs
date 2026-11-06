use std::io;
use bota_client_core::{ClientSession, Ingested, Outcome, Seat, Seated, SessionOptions, Transport};
use bota_proto::{ClientMsg, ServerMsg, Order, Target};
use bota_client_core::IssuedOrder;
use super::NativeTeam;
use crate::TeamTurn;

#[derive(Debug)]
pub struct TeamOutcome { pub outcomes: Vec<Outcome>, pub orders_sent: u64, pub script_failures: u64, pub retired: bool }
struct Connection<T> { transport: T, seat: Seat, seated: Seated, outcome: Option<Outcome>, rejections: u32, unsent: u32 }

/// Drives all joined seats of one team directly over their transports; no relay or simulation access.
pub fn play_team<T: Transport>(connections: Vec<(T, Seated)>, worker: &mut NativeTeam) -> io::Result<TeamOutcome> {
    let mut connections = prepare_connections(connections, worker)?;
    let mut result = TeamOutcome { outcomes: Vec::new(), orders_sent: 0, script_failures: 0, retired: false };
    let mut last = None;
    let mut warned = false;
    for _ in 0..u32::MAX {
        for connection in &mut connections { if connection.outcome.is_none() { collect_tick(connection)?; } }
        if connections.iter().all(|connection| connection.outcome.is_some()) {
            result.outcomes = connections.into_iter().map(|connection| connection.outcome.expect("all seats ended")).collect();
            result.retired = worker.retired();
            return Ok(result);
        }
        if connections.iter().any(|connection| connection.outcome.is_some()) { continue; }
        let newest = newest_session(&connections)?;
        let tick = newest.current().expect("a complete tick").tick;
        let turn = if last.is_none_or(|last| tick > last) {
            last = Some(tick);
            decide(worker, newest, &mut result, &mut warned)
        } else { None };
        for connection in &mut connections { send_turn(connection, turn.as_ref(), &mut result, worker.retired())?; }
    }
    Err(io::Error::other("match exceeded the protocol tick range"))
}

fn prepare_connections<T: Transport>(connections: Vec<(T, Seated)>, worker: &NativeTeam) -> io::Result<Vec<Connection<T>>> {
    if connections.len() != worker.seats().len() || connections.is_empty() { return Err(io::Error::other("connections do not match Lua seats")); }
    let connections: Vec<_> = connections.into_iter().map(|(transport, seated)| Connection { transport, seat: Seat::new(seated, SessionOptions::default()), seated, outcome: None, rejections: 0, unsent: 0 }).collect();
    for (index, connection) in connections.iter().enumerate() {
        if !worker.seats().iter().any(|seat| seat.slot == connection.seated.slot) || connections[..index].iter().any(|other| other.seated.slot == connection.seated.slot) {
            return Err(io::Error::other("connections name duplicate or foreign seats"));
        }
    }
    Ok(connections)
}

fn newest_session<T>(connections: &[Connection<T>]) -> io::Result<&ClientSession> {
    connections.iter().filter_map(|connection| connection.seat.session()).max_by_key(|session| session.current().map_or(0, |view| view.tick)).ok_or_else(|| io::Error::other("tick without a session"))
}

fn decide(worker: &mut NativeTeam, session: &ClientSession, result: &mut TeamOutcome, warned: &mut bool) -> Option<TeamTurn> {
    let tick = session.current().expect("a complete tick").tick;
    match worker.think(session) {
        Ok(turn) => {
            result.script_failures += turn.failures.len() as u64;
            for failure in &turn.failures {
                if failure.report { eprintln!("WARN pack=lua slot={} tick={} error={:?}", failure.slot.0, tick, failure.error.to_string()); }
            }
            Some(turn)
        }
        Err(error) => {
            if !*warned { eprintln!("WARN pack=lua tick={tick} error={error:?}"); *warned = true; }
            None
        }
    }
}

fn send_turn<T: Transport>(connection: &mut Connection<T>, turn: Option<&TeamTurn>, result: &mut TeamOutcome, retired: bool) -> io::Result<()> {
    let session = connection.seat.session_mut().expect("a complete tick");
    if retired { stand(&mut connection.transport, session, result)?; }
    if let Some(bot) = turn.and_then(|turn| turn.bots.iter().find(|bot| bot.slot == connection.seated.slot)) {
        for &order in &bot.orders {
            if session.own_hero().is_none() && order.unit.is_none() && bota_core::order_needs_body(order.order) { connection.unsent = connection.unsent.saturating_add(1); continue; }
            if session.ledger().repeats(order) { continue; }
            let outgoing = session.issue(order).map_err(|error| io::Error::other(error.to_string()))?;
            connection.transport.send(&outgoing.message())?;
            result.orders_sent += 1;
        }
    }
    if let Some(tick) = session.end_turn() { connection.transport.send(&ClientMsg::Ack { tick })?; }
    Ok(())
}

fn collect_tick<T: Transport>(connection: &mut Connection<T>) -> io::Result<()> {
    for _ in 0..65536 {
        let message = connection.transport.recv()?.ok_or_else(|| io::Error::other("server closed before match end"))?;
        match connection.seat.ingest(message, &mut ()).map_err(|error| io::Error::other(error.to_string()))? {
            Ingested::TickComplete { .. } => return Ok(()),
            Ingested::Rejected { .. } => connection.rejections = connection.rejections.saturating_add(1),
            Ingested::Passed(message) if matches!(*message, ServerMsg::Ping) => connection.transport.send(&ClientMsg::Pong)?,
            Ingested::Resumed { resend } => { for order in resend { connection.transport.send(&order.message())?; } }
            Ingested::Over { winner, .. } => {
                let session = connection.seat.session().expect("end after start");
                connection.outcome = Some(Outcome { slot: connection.seated.slot, team: session.team(), winner, ticks: session.current().map_or(0, |view| view.tick), rejections: connection.rejections, unsent_while_dead: connection.unsent });
                return Ok(());
            }
            _ => {}
        }
    }
    Err(io::Error::other("server sent too many messages without a complete tick"))
}

// Retirement is a host decision, so it uses only ordinary stop orders for the controlled bodies.
fn stand(transport: &mut impl Transport, session: &mut ClientSession, result: &mut TeamOutcome) -> io::Result<()> {
    let stop=Order::Move { target: Target::None };
    let hero=session.own_hero().map(|_|IssuedOrder::hero(stop));
    let courier=session.own_courier().map(|unit|IssuedOrder::courier(unit.id,stop));
    for order in [hero,courier].into_iter().flatten() {
        if session.ledger().repeats(order) {continue;}
        let outgoing=session.issue(order).map_err(|error|io::Error::other(error.to_string()))?;
        transport.send(&outgoing.message())?;
        result.orders_sent+=1;
    }
    Ok(())
}

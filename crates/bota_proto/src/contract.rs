//! The protocol contract: what a client may rely on beyond the types.
//!
//! The types say what crosses the wire; this says what the server does with
//! it and what it never tells.
//!
//! # Hidden state
//!
//! A client is told nothing from which a future outcome can be derived: not
//! the match key or the positions of its random streams, not the hidden order
//! of a chance block (below), not which roster a camp fills with next. The
//! types of this crate cannot carry any of it, and a replay holds none of it
//! either: it is a recording, never simulated again. What a side does not see
//! it is not told, so a client draws its own fog from the terrain and its own
//! units; the server alone decides which units a view holds.
//!
//! Two further channels are closed by rule, since a bot that optimises a
//! score finds any leak a human would shrug off:
//!
//! - **A reject reason does not depend on hidden state.** An order aimed at a
//!   unit the side cannot see is refused with
//!   [`UnknownTarget`](crate::RejectReason::UnknownTarget), exactly as one
//!   aimed at a unit that has died or never was, so probing the fog with a
//!   stale handle reveals nothing.
//! - **A unit never acts on what its side cannot see.** A standing
//!   [`Order::Attack`](crate::Order::Attack) at a
//!   [`Target::Unit`](crate::Target::Unit) whose target slips into the fog
//!   becomes an attack-move to the spot where it was last seen, and a
//!   [`Order::Move`](crate::Order::Move) at a unit becomes a walk to that spot.
//!   The target is dropped: the order does not take it up again when it
//!   reappears, so the unit's own path gives nothing away.
//!
//! # Map 2 (mid-only)
//!
//! Map 2 is the full Dota geometry with creep waves on the mid lane only. The
//! side lanes' tier-one towers are invulnerable and nothing behind them opens
//! (`bota_core::SOLO_MID_PROTECTION`), so only the mid towers can fall. A side
//! loses on the tick it loses any tower or its seats' heroes have died
//! `MAP2_DEATH_LIMIT` (two) times together. When both sides would lose on one
//! tick, the match is a draw. The match is also a draw at `MAP2_TICK_CAP`, the
//! 900 pregame ticks plus twenty game minutes (36 900 ticks at the default 30
//! per second): a loss on that very tick is a draw too. Every other map ends
//! by its own rules (the Dota map when an Ancient falls).
//!
//! # Orders
//!
//! At most one order per seat takes effect on a tick, the last one accepted
//! for that tick, whichever unit it names: a later one replaces an earlier one
//! without an answer. An order is checked when it arrives and again on the
//! tick it takes effect; failing either is answered with
//! [`ServerMsg::OrderRejected`](crate::ServerMsg::OrderRejected) carrying its
//! `seq`. A seat whose hero is dead is refused every order for the hero with
//! [`HeroDead`](crate::RejectReason::HeroDead); its courier still takes
//! orders.
//!
//! # Order delay and the `seen` tick
//!
//! An order given on the view of tick `T` takes effect on tick `T + 1` at the
//! earliest. A server may hold every order of a seat back a fixed number of
//! ticks and add a jitter drawn per order from a hidden stream, together at
//! most `bota_core::rules::MAX_ORDER_DELAY_TICKS` (thirty, a second), in
//! either tick mode. Orders never overtake one another: one drawn earlier
//! than the order before it waits for it, as on one TCP stream.
//!
//! A client says which view an order was given on in
//! [`ClientMsg::Order::seen`](crate::ClientMsg::Order). Each side is told, in
//! [`WorldView::order_delays`](crate::WorldView::order_delays), how many ticks
//! past `T + 1` the latest such order of each of its seats took effect: in
//! lockstep the configured delay, in realtime whatever the line and the
//! client's own thinking cost. An order without `seen` measures nothing.
//!
//! # Tick modes and slow receivers
//!
//! In [`TickMode::Realtime`](crate::TickMode::Realtime) ticks follow the wall
//! clock, and an order that misses its tick takes effect on a later one. In
//! [`TickMode::Lockstep`](crate::TickMode::Lockstep) the server waits until
//! every connected seat has sent [`ClientMsg::Ack`](crate::ClientMsg::Ack)
//! for the tick, or until its acknowledgement timeout runs out, and then
//! advances without the straggler; a bot may think as long as it needs.
//!
//! Every tick a connection is sent [`ServerMsg::Snapshot`] and then
//! [`ServerMsg::Events`] for the same tick, an empty batch included. An event
//! may name a unit the snapshot no longer holds: it fell on that tick, and
//! the previous snapshot is the one to read it against.
//!
//! A slow receiver never stalls the match. A tick waiting to be sent is
//! replaced by the next one, and the events of every tick replaced ride along
//! in the newer snapshot's
//! [`WorldView::skipped_ticks`](crate::WorldView::skipped_ticks), each with
//! its tick. A slow client therefore jumps from state to state but is told
//! every tick and every event it may know, in order. A connection more than
//! 900 ticks behind (thirty seconds at 30 a second), or whose queue of other
//! messages overflows, is dropped as dead (below).
//!
//! # Connections: liveness and coming back
//!
//! The server sends [`ServerMsg::Ping`] on every connection about once a
//! second, whatever else it sends, so a client that hears nothing for a few
//! seconds knows its connection is dead, even in a quiet lobby. A client
//! answers each with [`ClientMsg::Pong`](crate::ClientMsg::Pong); any message
//! counts. A connection that sends nothing, or takes in nothing of what it is
//! sent, for the server's idle timeout (five seconds by default) is dropped
//! as dead, and so is one whose peer closed it or broke it: the seat it held
//! is free at once. A connection dropped as dead is reset rather than closed:
//! a clean close means the server ended the connection on purpose (the match
//! is over, the `Hello` was refused), and a client does not dial again after
//! one. A connection that has not said `Hello` is told nothing, and one that
//! has not said it within ten seconds is closed.
//!
//! A connection is a TCP stream of frames, or a WebSocket (plain or over TLS)
//! on a port of its own, where every binary message is exactly one frame,
//! length prefix included; everything above is the same on both. A server
//! closes a connection that sends what is no frame (a text message, a
//! message whose prefix disagrees with its size), a frame whose payload is
//! over [`MAX_CLIENT_PAYLOAD_LEN`](crate::MAX_CLIENT_PAYLOAD_LEN), a payload
//! that decodes to no message, or more messages than its allowance: a rate
//! the server sets (a thousand a second by default, two seconds' worth of
//! burst) and four more for every tick it is sent. A seat's key lets at most
//! a bounded number of spectators (seven by default) watch through it at
//! once; one more is refused as
//! [`NotAdmitted`](crate::RejectReason::NotAdmitted).
//!
//! A `Hello` that admits to nothing is answered with
//! [`ServerMsg::Refused`] and the connection closed:
//! [`NotAdmitted`](crate::RejectReason::NotAdmitted) for a missing or wrong
//! key, no open seat, or an unreserved seat asked for mid-match without the
//! key that claimed it, and [`SeatHeld`](crate::RejectReason::SeatHeld) for
//! the right key to a seat a live connection holds: a seat never changes
//! hands while its connection lives.
//!
//! A seat's player may connect again at any moment while the match runs and
//! take the seat back, once its old connection has ended or been found dead,
//! with its hero and everything the simulation holds for it: with reserved
//! seats by the seat's key, and without them by the key its first `Hello`
//! came with, which claimed the seat. A `Hello` without a key takes an
//! unreserved seat in the lobby only. The new connection is sent `Welcome`
//! and `MatchStart` again, then every tick from the next one; the ticks
//! between the two connections are not told.
//!
//! A seat nobody holds is absent: its hero is given no orders (the last one
//! it was given runs its course), and the match neither waits for it nor ends
//! because of it. In lockstep the server waits for the seats held; a seat
//! taken back is waited for from the tick after the one it came back on. A
//! match whose seats are all empty ends early only when none of them may be
//! taken back.
//!
//! # Chances
//!
//! Evasion and pierce must hit at an exact rate and still be unpredictable. A
//! chance is a fraction `num / den` with `den` at most 64, dealt in blocks of
//! `den` attempts: exactly `num` of each block hit, at places drawn from a
//! hidden stream of the source's own, and each source starts at a hidden
//! place inside its first block. Seeing past hits says nothing about the
//! next block. A changed chance (an item picked up) takes effect at the next
//! block.
//!
//! The residual leak is inside a block: an opponent who counts may know that
//! the rest of a block is clean (three pierces seen in six swings of a three
//! in ten). Every scheme with an exact rate has it; the alternative is giving
//! the exact rate up. The uphill miss follows Dota's pseudo-random
//! distribution for 25%, and gold bounties and damage ranges are plain draws
//! from hidden streams.
//!
//! # Day and night
//!
//! The time of day crosses no wire: it is a pure function of the tick
//! (`bota_core::time_of_day`), the same for every viewer and every match.
//! The pregame is night, the first day breaks at the horn and each half of
//! the day lasts five minutes. What a unit sees by at the moment rides in its
//! [`UnitView::vision_radius`](crate::UnitView::vision_radius), and a client
//! draws its fog from that.
//!
//! # Names
//!
//! The name in [`ClientMsg::Hello`](crate::ClientMsg::Hello), and a name a
//! server reserves a seat under, is shown in
//! [`ServerMsg::LobbyState`](crate::ServerMsg::LobbyState) cut to
//! `bota_core::rules::MAX_SHOWN_NAME_BYTES` (32) bytes of UTF-8, at the last
//! whole character within them. The server does not refuse a longer hello
//! name. The two input bounds are
//! `bota_core::rules::MAX_ANNOUNCED_NAME_BYTES` (64): the client library
//! refuses to announce a longer name, and the server refuses a seat
//! reservation under one.
//!
//! # Wire fingerprint
//!
//! A client sends [`WIRE_FINGERPRINT`](crate::WIRE_FINGERPRINT) in its
//! [`ClientMsg::Hello`](crate::ClientMsg::Hello). A server built from other
//! registries answers [`ServerMsg::WireMismatch`] with its own fingerprint and
//! closes the connection, before anything else could be misread.
//!
//! [`ServerMsg::Snapshot`]: crate::ServerMsg::Snapshot
//! [`ServerMsg::Events`]: crate::ServerMsg::Events
//! [`ServerMsg::WireMismatch`]: crate::ServerMsg::WireMismatch
//! [`ServerMsg::Ping`]: crate::ServerMsg::Ping
//! [`ServerMsg::Refused`]: crate::ServerMsg::Refused

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
//! units; the server alone decides which units a view holds. Who sees what
//! is worked out once a tick, after its movement; units act on it within
//! the tick. A view shows the tick as it ended through those eyes: a unit
//! that moved after them (blinked, was carried, flew) or came into the world
//! after them (a missile let go) is shown when they reach where it now
//! stands, so a unit that leaves a side's sight on a tick is not shown where
//! it went, and what arrives in sight is. Whatever a side's view shows, an
//! order given on that view may name, and an event about a unit is told to
//! a side whose view would show the unit.
//!
//! Three further channels are closed by rule, since a bot that optimises a
//! score finds any leak a human would shrug off:
//!
//! - **An [`EntityId`](crate::EntityId) a side is told is that side's own
//!   name.** The simulation's handles come from one allocator for the whole
//!   world, so their slots and generations would count what the other side
//!   spawns in the fog. A view through a side's eyes, the events and the
//!   orders it is told name each entity by a handle given the first time the
//!   side is told of it: `idx` counts up from zero, `generation` is zero, and
//!   a handle is never given twice. Of the handles first given on one tick,
//!   those of what the match was set up with (the map's buildings, the picks'
//!   heroes and couriers) come first in the setup's own order, the rest in an
//!   order drawn from a hidden stream; a hero with illusions of it standing
//!   counts among the rest (see [Illusions](#illusions)). The side's orders
//!   name units the same way. The other side knows the same unit by another handle; a
//!   view of everything, and the replay, carry the simulation's own handles,
//!   whose generation is never zero.
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
//! # What a side is told of what happens
//!
//! As in Dota, a side learns of what befalls a unit when it sees that unit,
//! not when it sees the ground the unit stands on: a ward hidden from it is
//! hidden in the events too. An event is told to the side of the unit it
//! befalls ([`Damaged`], [`Missed`] and [`Healed`]: the target; [`Died`]:
//! the unit; [`AbilityCast`]: the caster; [`LevelUp`]: the hero) and to
//! every side that sees that unit as it happens; a [`Died`] also to the side
//! of its killer. Who dealt it, the `source` or the `killer`, is told only
//! to a side that sees the dealer as it happens, and is `None` to the
//! others.
//!
//! Dota's kill feed and announcer make exceptions, told to both sides
//! wherever they happen: the fall of a hero (not of an illusion), a tower, a
//! barracks, an Ancient or a courier; and of a hero, a tower or a barracks
//! the killer too when a hero struck last (the feed names a player, and of
//! any other killer only its side). [`StructureDestroyed`] is told to both
//! sides, and [`ItemBought`] to the buyer's side alone.
//!
//! [`Damaged`]: crate::EventKind::Damaged
//! [`Missed`]: crate::EventKind::Missed
//! [`Healed`]: crate::EventKind::Healed
//! [`Died`]: crate::EventKind::Died
//! [`AbilityCast`]: crate::EventKind::AbilityCast
//! [`LevelUp`]: crate::EventKind::LevelUp
//! [`StructureDestroyed`]: crate::EventKind::StructureDestroyed
//! [`ItemBought`]: crate::EventKind::ItemBought
//!
//! # What a side is told of the other side's seats
//!
//! As Dota's scoreboard: of a seat of the other side,
//! [`PlayerView`](crate::PlayerView) tells the hero's level and experience
//! as they stood when the viewing side last saw the hero (level one before
//! it ever did), its kills, deaths, assists and respawn wait as they are,
//! and neither its gold, stash and fallen hero's kit (absent) nor its last
//! hits and denies (zero). A hero the side sees shows its level in its
//! [`UnitView`](crate::UnitView) as it is.
//!
//! Of a unit of the other side it sees, a side is told what Dota shows of
//! it (abilities with their levels and cooldowns, items, effects) but not
//! what is its owner's alone: [`AbilityView::can_level`] and
//! [`ItemView::for_sale`] are false. [`AggroView`] tells nothing of a unit
//! the view does not carry: neither whom a unit is set on or swings at, nor
//! how long it holds or chases such a target.
//!
//! [`AbilityView::can_level`]: crate::AbilityView::can_level
//! [`ItemView::for_sale`]: crate::ItemView::for_sale
//! [`AggroView`]: crate::AggroView
//!
//! # What a side is told of the forest
//!
//! Since 2014 Dota shows no tree falling, growing back or being put up where
//! a side has no sight. Each side keeps the forest as it last saw it:
//! [`WorldView::felled_trees`](crate::WorldView::felled_trees) and
//! [`WorldView::planted_trees`](crate::WorldView::planted_trees) change for
//! a tree only when the side sees the ground it stands on (one of the
//! cells a map tree covers, the cell a planted one stands in), and then
//! show it as it is. A tango eaten in the fog leaves the tree standing in
//! the other side's view until that side looks.
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
//! A seat orders any unit it drives ([`UnitView::owner`](crate::UnitView)):
//! its hero, its courier, and later its summons and illusions, each by its
//! handle, or the hero by naming none. Every unit takes orders of its own:
//! an order to the courier and one to the hero on one tick both take effect.
//!
//! Each unit keeps a queue, as Dota's Shift-queue. An order given queued
//! ([`ClientMsg::Order::queue`](crate::ClientMsg::Order)) waits its turn:
//! the unit takes it up once it is done with the order in hand and with
//! every order queued before it, and a unit with nothing in hand and nothing
//! queued takes it up at once. An order given unqueued replaces the order in
//! hand and empties the queue. What [`Order::queues`](crate::Order::queues)
//! says is carried out at once (learning, talents, the shop, the bag,
//! cheats, the Glyph) is never queued and leaves the queue be, and so does an
//! unqueued cast or use of what goes off beside whatever the body does
//! (Dota's immediate abilities and items); queued, such a cast or use waits
//! its turn like any other. A unit is done with
//!
//! - a move once it stands at the end of its walk, an attack-move once it
//!   does with nobody left to fight, a stop at once;
//! - an attack or a follow when its target falls, and then takes up what it
//!   has queued; with nothing queued it carries on to where the target fell;
//! - a cast or a use once it went off and its channel and backswing are
//!   over, or it was dropped; laying an item down or taking one up once that
//!   is done; a courier's errand once it is over.
//!
//! A hold, and an attack or a follow whose target stands, are never done:
//! what is queued behind them waits for an unqueued order. A stun or a fear
//! keeps the queue; death empties it. A queue holds at most
//! `bota_core::rules::MAX_QUEUED_ORDERS`; one more is refused as
//! [`QueueFull`](crate::RejectReason::QueueFull). Each unit's queue is told to
//! its own side in [`UnitView::queue`](crate::UnitView), never to the other.
//!
//! On one tick a unit takes at most one unqueued order, the last one accepted
//! for that tick: it replaces, without an answer, the orders for that unit
//! given before it for the same tick, queued or not. The queued orders given
//! after it follow it in the order given. A seat's orders that take effect on
//! one tick, at most `bota_core::rules::MAX_ORDERS_PER_TICK`, are applied
//! seat by seat in slot order, and each seat's in the order it gave them; the
//! Glyph, which needs no body, is one more order of the seat, a later Glyph
//! replacing an earlier one.
//!
//! An order is checked when it arrives and again on the tick it takes
//! effect, and a queued one again when its unit takes it up. Failing any is
//! answered with [`ServerMsg::OrderRejected`](crate::ServerMsg::OrderRejected)
//! carrying its `seq`; a queued order refused when it comes up is dropped and
//! the next one taken up. A seat whose hero is dead is refused every order for
//! the hero with [`HeroDead`](crate::RejectReason::HeroDead), as in Dota but
//! for the shop: it may buy (into the stash; what the hero carried when it
//! fell counts towards a recipe) and sell what it bought since the hero fell
//! (from the stash), and quick-buy is buying. Its courier still takes orders
//! and carries the stash as ever. What a body or a courier carried when it
//! fell sells back at the full price no more.
//!
//! # Order delay and the `seen` tick
//!
//! An order given on the view of tick `T` takes effect on tick `T + 1` at the
//! earliest. A server may hold every order of a seat back a fixed number of
//! ticks and add a jitter drawn from a hidden stream, together at most
//! `bota_core::rules::MAX_ORDER_DELAY_TICKS` (thirty, a second), in either
//! tick mode. The jitter is drawn once for all the orders a seat gives while
//! the world stands on one tick, so a burst of them (a hero's and a
//! courier's, a queue given at once) takes effect together. Orders never
//! overtake one another: one drawn earlier than the order before it waits
//! for it, as on one TCP stream.
//!
//! A client says which view an order was given on in
//! [`ClientMsg::Order::seen`](crate::ClientMsg::Order). Each side is told, in
//! [`WorldView::order_delays`](crate::WorldView::order_delays), how many ticks
//! past `T + 1` the latest such order of each of its seats took effect: in
//! lockstep the configured delay, in realtime whatever the line and the
//! client's own thinking cost. An order without `seen` measures nothing.
//!
//! # What a side is told of the other's Glyph
//!
//! Dota shows a Glyph of Fortification on every building and creep one sees,
//! and its announcer tells both teams when one is used. So a unit in a view
//! wears the [`EffectId::FORTIFIED`](crate::EffectId::FORTIFIED) buff, with
//! the ticks its cover has left, whatever its side, only for units the view
//! holds (what the fog hides is not told), and
//! [`WorldView::glyphs`](crate::WorldView::glyphs) holds both sides'
//! entries. The buff is put on the structures and lane creeps that stand
//! when the Glyph is used, each with its own time; a creep that spawns later
//! never wears it, as in Dota. The other side's entry has `ready_in` zero always: that its Glyph
//! is active is public, when it may be used again is not.
//!
//! # Illusions
//!
//! A side is told which of its own units are illusions: an illusion wears
//! [`EffectId::ILLUSION`](crate::EffectId::ILLUSION), with the time it has
//! left and its full length, and is gone when that runs out. The other side
//! cannot tell a hero from its illusions by anything the protocol says:
//!
//! - [`PlayerView::unit`](crate::PlayerView::unit) of the other side's seat
//!   is absent while illusions of its hero stand. Each of them is a unit of
//!   kind [`Hero`](crate::UnitKind::Hero) whose `owner` is the seat, as the
//!   hero is.
//! - An illusion shows the other side its hero's sheet: kind, hero, owner,
//!   level, attributes, every stat, abilities, items, mana and regeneration
//!   are the hero's on that tick. Its body is its own: where it stands and
//!   faces, its health and maximum health, statuses, effects (less the
//!   marker, which is told to its own side alone), cast and aggro. What the
//!   bodies do (the damage an illusion takes and deals, the buffs on it) may
//!   give it away, as in Dota.
//! - Handles: once an illusion of a hero is made, the other side's names for
//!   the hero and for its illusions made before name nothing any more, and
//!   they are all named afresh in one batch, so no name the hero was known
//!   by follows it. While illusions of it stand, names given on one tick to
//!   the hero and to them are ordered by the hidden stream, never by the
//!   setup. Once the last of them is gone, the hero is named afresh again
//!   before `PlayerView::unit` names it, so that name says nothing of which
//!   body it was.
//! - The other side's seat's [`TalentView`](crate::TalentView) is told while
//!   the hero or any illusion of it is in the view, and a
//!   [`LevelUp`](crate::EventKind::LevelUp) of a hero with illusions is told
//!   to the other side once for the hero and once for every illusion of it,
//!   in the order of their names.
//! - An illusion's death is told to everyone, as Dota shows it.
//!
//! A view of everything, and the replay, tell all of it as it is. Whatever
//! makes an illusion also calls off the other side's orders, queued ones
//! and attacks aimed at the hero, as Dota's illusion makers do, so none of
//! them follows the hero under its new name.
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
//! burst) and four more for every tick it is sent.
//!
//! # Spectators and picks
//!
//! A spectator that comes with a seat's key (reserved, or the key that
//! claimed an unreserved seat) watches live through that seat's eyes and is
//! told its orders ([`ServerMsg::Orders`]); at most a bounded number (seven
//! by default) at once per seat, one more is refused as
//! [`NotAdmitted`](crate::RejectReason::NotAdmitted). Nobody else is told an
//! order live. A spectator without a key is refused unless the server lets
//! anyone watch: live (the whole map, or a side's fog through
//! [`ViewAs`](crate::ClientMsg::ViewAs), never an order), or a set number of
//! seconds of game time late, where the whole map comes with every seat's
//! orders, as late. When the match is over a late spectator is shown its
//! last tick at once, then the result.
//!
//! A view of the whole map (a replay, a fogless spectator) says of every
//! unit, missile and ground item which sides' views hold it
//! ([`SeenBy`](crate::SeenBy)): Dota's spectator vision toggle, exact,
//! with no client working out the fog itself. A side's view never says it:
//! a side is not told whether the other sees its units.
//!
//! As in Dota, players are not told about spectators: a spectator's or a
//! referee's arrival and departure reach no seat (no
//! [`ParticipantLeft`](crate::ServerMsg::ParticipantLeft), no lobby sent
//! again), only the spectators are told a watcher left, and the
//! [`PlayerId`](crate::PlayerId) a connection is welcomed with counts seats
//! and watchers apart. That the lobby shows every seat's pick to everyone is
//! Solo Mid's blind pick; draft modes will tell picks by side.
//!
//! A seat picks only among the heroes the server allows (every hero unless
//! it says otherwise); a seat without a pick when the lobby runs out of time
//! is given one of them. A pick is final once the seat was ready with it,
//! even if it withdraws its readiness: nobody waits for the others' picks to
//! change their own.
//!
//! # Before the match
//!
//! Once every seat is held the selection clock runs (`--pick-secs`, Dota's
//! 60 s): [`LobbyState::pick`](crate::ServerMsg::LobbyState) says how many
//! milliseconds are left when it is sent, and is sent again whenever the
//! lobby changes, to a reconnecting client too, which counts down from the
//! time it was told. A seat that has picked and been ready with its pick is
//! locked in; when all are, or the time is out, every pick is final. Past the
//! time a seat without a hero loses two starting gold for every whole second
//! it keeps the others waiting, and after thirty seconds is given a hero drawn
//! from the match seed among the allowed ones that are free (the drawing is
//! the server's, so replays agree). A hero some seat holds is refused to the
//! others when `LobbyState::same_heroes` is false; Dota's 1v1 Solo Mid and
//! Mid Only allow it, so it is true by default.
//!
//! Then comes the tactical phase (Dota's strategy time, `--tactical-secs`,
//! 30 s): the heroes are known and the seats plan their start; the match
//! begins when it ends. The kind of player decides whether it is held: a
//! player is a human ([`Role::Player`](crate::Role::Player)) or a bot
//! ([`Role::Bot`](crate::Role::Bot)), declared in `Hello` and shown to
//! everyone in [`LobbySlot::role`](crate::LobbySlot); when every seat is
//! held by a bot there is no tactical phase and the match begins as the last
//! pick is final. A reserved seat may name its kind (`BOTA_SEATS`'s last
//! field): the reservation knows what the account is, so a `Hello` of the
//! other role is refused as
//! [`NotAdmitted`](crate::RejectReason::NotAdmitted) rather than believed,
//! which keeps a human from passing as a bot to skip the phase, and the
//! other way round. Without a named kind the declared role stands.
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
//! match whose seats are all empty ends early, abandoned and without a
//! result, only when none of them may be taken back, or when the server was
//! told to give up on a match that stood empty a set time; a server may also
//! abandon a match whose lobby ran out of time with no seat held.
//!
//! # Chances
//!
//! Every chance of an attack rolls as Dota rolls it, with its pseudo-random
//! distribution: the `n`-th attempt since a source's last hit hits with `n`
//! times a step worked out from the chance, so runs of misses are short and
//! over many attempts the source hits at its chance. Each source draws from
//! a hidden stream of its own and keeps its run of misses when its chance
//! changes.
//!
//! What sources there are and how they stack follows Dota as well. A unit's
//! evasions (its items, and the uphill miss of a ranged attack thrown up at
//! it) stack diminishingly, each letting through its share of what the
//! others do: `1 - (1 - a)(1 - b)`, rolled at once on the target's sequence.
//! Accuracy stacks the same way and rolls on the attacker's. Each pierce an
//! attacker carries rolls its own, and several may land on one attack, but
//! several Monkey King Bars pierce as one. Critical strikes roll from the
//! strongest down and stop at the first that hits; one that did not roll
//! keeps its run. An opponent who counts attempts learns what Dota players
//! learn, how long a run has gone. Gold bounties and damage ranges are plain
//! draws from hidden streams.
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
//! [`ServerMsg::LobbyState`](crate::ServerMsg::LobbyState) (and a
//! spectator's in its chat lines) cleaned: every character a chat line may
//! not hold dropped (see Chat below), stacks of combining marks cut,
//! trimmed, and cut to `bota_core::rules::MAX_SHOWN_NAME_BYTES` (32) bytes of
//! UTF-8 at the last whole character within them; `?` when nothing is left
//! (`bota_core::clean_name`). The server does not refuse a hello for its
//! name. The two input bounds are `bota_core::rules::MAX_ANNOUNCED_NAME_BYTES`
//! (64) with the chat's character rules (`bota_core::name_fault`): the client
//! library refuses to announce another name, and the server refuses a seat
//! reservation under one.
//!
//! # Wire fingerprint
//!
//! A client sends its build's wire fingerprint in its
//! [`ClientMsg::Hello`](crate::ClientMsg::Hello): `bota_core::WIRE_FINGERPRINT`,
//! which folds two numbers into one. The
//! [`REGISTRY_DIGEST`](crate::REGISTRY_DIGEST) names the wire layout and the
//! ids; `bota_core::RULES_VERSION` names the rules a client checks its orders
//! against and predicts with (the tables and the integer functions of
//! `bota_core`), bumped with every change to what they decide and held to
//! that by a test over their code. A server built with another
//! fingerprint answers [`ServerMsg::WireMismatch`] with its own and closes the
//! connection, before anything else could be misread: a server whose
//! balance moved (a new patch's numbers, a changed table) and a client
//! built before it part at the hello instead of the client failing in
//! mid-match on an order its stale rules thought legal.
//!
//! The registries only grow, but postcard reads by position: a build reads
//! only what a build of its own fingerprint wrote. So what builds of
//! different wires must still read of each other never grows: the hello with
//! its [`Role`](crate::Role), the refusal, and a replay's
//! [`ReplayHeader`](crate::ReplayHeader). A replay starts with that header,
//! which names the fingerprint it was recorded with, and a player refuses a
//! replay of another one up front, naming both fingerprints.
//!
//! # Compression
//!
//! A replay file is one Zstandard stream (RFC 8878) of its frames, written at
//! level 3 with a 2 MiB window; a player also takes the frames as they are,
//! and tells the two apart by the stream's magic number, which no frame
//! prefix of a replay begins with. A player refuses a stream that asks for a
//! window above 8 MiB, or that unpacks to more than 1024 times the bytes it
//! took from the file (one frame aside): a server's replay unpacks some 87
//! times.
//!
//! A connection may ask for its stream to be compressed with
//! [`ClientMsg::Compress`](crate::ClientMsg::Compress), at any time once it
//! said `Hello` (a client that sends it right behind the hello has everything
//! after the welcome compressed). From then on the server sends
//! everything but chat as [`ServerMsg::Compressed`] parts: one DEFLATE
//! stream (RFC 1951, raw, its 32 KiB window) per connection whose bytes are
//! the frames the connection would carry plainly, flushed after each message
//! or tick, so every part ends on a frame's end and unpacks at once. Chat
//! ([`ServerMsg::Heard`]) is never compressed: a line an enemy chose must not
//! share a window with a side's own state, or the size of what follows
//! would tell whether it guessed some of it. A client refuses a part that
//! unpacks to more than
//! [`MAX_UNPACKED_PART_LEN`](crate::MAX_UNPACKED_PART_LEN), a part inside a
//! part, a plain message inside a frame the parts began, and a part that
//! breaks the stream; the connection is then useless and closed. A
//! connection that dials again starts a new stream and asks again.
//!
//! # Chat
//!
//! Chat is beside the match: a line never reaches the simulation, the order
//! line, the acknowledgements or a tick's hash, and no bot needs it. A
//! [`ClientMsg::Say`](crate::ClientMsg::Say) is passed on to its audience as
//! [`ServerMsg::Heard`], the speaker included, or refused to the speaker
//! alone with [`ServerMsg::SayRefused`]. Who hears what, as in Dota:
//!
//! - A seat (alive or dead) speaks to
//!   [`All`](crate::Audience::All): every seat and every spectator; or to
//!   [`Team`](crate::Audience::Team): its side's seats and the spectators
//!   that came with one of its side's seat keys. A spectator without a key
//!   never hears a team line, and nothing tells the other side one was said.
//! - A ping ([`Body::Ping`](crate::Body::Ping)) goes to the speaker's side
//!   alone, at a spot on the map or at a unit named in the side's own names
//!   that its view holds at that moment; a ping at a name the side does not
//!   hold, or that its view does not hold now, is refused as
//!   [`NotAllowed`](crate::SayRefusal::NotAllowed), exactly as one at a unit
//!   that died, so a refusal says nothing of the fog. The replay keeps it in
//!   the simulation's handles.
//! - A spectator speaks to [`Spectators`](crate::Audience::Spectators) only,
//!   and no seat ever hears it. A referee says nothing.
//! - A server may close chat to the seats, keep them to their own side, or
//!   close it to spectators (a ladder of bots closes it: chat would be a
//!   channel between opponents); what it closes is refused as
//!   [`NotAllowed`](crate::SayRefusal::NotAllowed).
//! - A line is stamped with the tick its speaker was watching, and each hearer
//!   hears it once its own feed has reached that tick: a late spectator hears
//!   a seat's line with the tick it was said on, never before.
//! - Text is one line of Unicode: trimmed, at most
//!   `bota_core::rules::MAX_CHAT_BYTES` bytes and
//!   `bota_core::rules::MAX_CHAT_GRAPHEMES` characters as a reader counts
//!   them, and free of control, line-break, bidirectional, invisible, tag,
//!   private-use and noncharacters and of stacks of more than two combining
//!   marks (`bota_core::chat_text_fault`). A line that breaks this is
//!   refused as [`Unfit`](crate::SayRefusal::Unfit), never cleaned. Names
//!   follow the same character rules but are cleaned rather than refused.
//! - A connection says five lines in a burst and one a second after that, and
//!   all spectators together ten in a burst and two a second; a line over the
//!   rate is refused as [`TooFast`](crate::SayRefusal::TooFast), and the
//!   connection stays. Every line still counts towards the connection's
//!   message allowance.
//! - The replay records every line a seat said, in the lobby and the match,
//!   as [`ReplayRecord::Heard`](crate::ReplayRecord::Heard), where the server
//!   took it in; team lines too unless the server was told not to. A
//!   replay of the whole map thus shows both sides' team lines; a player
//!   showing it through one side's eyes shows everyone's lines and that
//!   side's own. Spectators' lines are not recorded.
//! - A client checks every line again and drops, without failing, one that
//!   could not have been said to it. Chat counts towards none of the bounds
//!   on messages before the match start or between snapshots.
//!
//! [`ServerMsg::Snapshot`]: crate::ServerMsg::Snapshot
//! [`ServerMsg::Events`]: crate::ServerMsg::Events
//! [`ServerMsg::WireMismatch`]: crate::ServerMsg::WireMismatch
//! [`ServerMsg::Ping`]: crate::ServerMsg::Ping
//! [`ServerMsg::Refused`]: crate::ServerMsg::Refused
//! [`ServerMsg::Compressed`]: crate::ServerMsg::Compressed
//! [`ServerMsg::Heard`]: crate::ServerMsg::Heard
//! [`ServerMsg::SayRefused`]: crate::ServerMsg::SayRefused
//! [`ServerMsg::Orders`]: crate::ServerMsg::Orders

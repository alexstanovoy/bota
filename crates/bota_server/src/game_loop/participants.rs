//! What a connection is called on the wire, and who is told it came or left.
//!
//! As in Dota, players are not told about spectators: a spectator's arrival
//! or departure reaches no seat, not even as a lobby sent again, and the
//! numbers connections are told count seats and watchers apart, so a seat's
//! own number never says how many came to watch.

use bota_proto::{PlayerId, ReplayRecord, Role, ServerMsg, SlotId};

use super::Server;

/// The numbers watchers (spectators, referees) are told start here, apart
/// from the seats'.
const WATCHERS_FROM: u32 = 1 << 31;

/// The numbers handed out so far.
#[derive(Default)]
pub(super) struct Told {
    /// Each greeted connection's number on the wire.
    ids: Vec<(PlayerId, PlayerId)>,
    seats: u32,
    watchers: u32,
}

impl Told {
    /// Hands connection `id` its number: the next seat's, or the next
    /// watcher's.
    pub(super) fn give(&mut self, id: PlayerId, seated: bool) -> Option<PlayerId> {
        let counter = if seated {
            &mut self.seats
        } else {
            &mut self.watchers
        };
        let next = counter
            .checked_add(1)
            .filter(|next| *next < WATCHERS_FROM)?;
        *counter = next;
        let told = PlayerId(if seated { next } else { WATCHERS_FROM + next });
        self.ids.retain(|(own, _)| *own != id);
        self.ids.push((id, told));
        Some(told)
    }

    pub(super) fn get(&self, id: PlayerId) -> Option<PlayerId> {
        self.ids
            .iter()
            .find(|(own, _)| *own == id)
            .map(|(_, told)| *told)
    }

    /// Takes connection `id`'s number back; none for one never greeted.
    pub(super) fn take(&mut self, id: PlayerId) -> Option<PlayerId> {
        let at = self.ids.iter().position(|(own, _)| *own == id)?;
        Some(self.ids.swap_remove(at).1)
    }
}

impl Server {
    /// Keeps only the public identities of held seats, not the observer roster or credentials.
    pub(super) fn record_seat_names(&self, replay: &mut crate::replay::ReplayWriter) {
        for seat in &self.roster.seats {
            if seat.player.is_some()
                && !seat.name.is_empty()
                && let Some(role @ (Role::Player | Role::Bot)) = seat.role
            {
                replay.record(&ReplayRecord::SeatName {
                    slot: seat.slot,
                    name: seat.name.clone(),
                    bot: role == Role::Bot,
                });
            }
        }
    }

    /// Tells of a greeted connection that ended: every connection when it
    /// held a seat, the spectators alone when it watched. In the lobby the
    /// seats are told the lobby as it now stands instead, when a seat
    /// emptied.
    pub(super) fn tell_departure(&self, told: PlayerId, slot: Option<SlotId>, in_match: bool) {
        match (in_match, slot) {
            (false, Some(_)) => self.broadcast_lobby(),
            (false, None) => {}
            (true, Some(_)) => self.broadcast(&ServerMsg::ParticipantLeft {
                player_id: told,
                slot,
            }),
            (true, None) => {
                let left = ServerMsg::ParticipantLeft {
                    player_id: told,
                    slot,
                };
                for watcher in &self.spectators {
                    if let Some(conn) = self.conn(watcher.id) {
                        conn.send(&left);
                    }
                }
            }
        }
    }

    /// The lobby after a hello in it: everyone is told it when a seat was
    /// taken, a watcher alone is told it otherwise.
    pub(super) fn lobby_after_hello(&self, id: PlayerId) {
        if self.roster.seat_of(id).is_some() {
            self.broadcast_lobby();
        } else if let Some(conn) = self.conn(id) {
            conn.send(&self.lobby_state());
        }
    }
}

#[cfg(test)]
mod settlement_tests {
    use super::*;

    #[test]
    fn a_seated_connection_identity_never_enters_the_watcher_namespace() {
        let mut told = Told {
            seats: WATCHERS_FROM - 1,
            ..Told::default()
        };
        let next: Option<PlayerId> = told.give(PlayerId(1), true);
        assert!(
            next.is_none(),
            "exhaustion must not reuse a connection namespace"
        );
        assert!(told.ids.is_empty());
    }

    #[test]
    fn a_watcher_connection_identity_never_wraps_or_panics() {
        let mut told = Told {
            watchers: WATCHERS_FROM - 1,
            ..Told::default()
        };
        let next: Option<PlayerId> = told.give(PlayerId(1), false);
        assert!(next.is_none());
        assert!(told.ids.is_empty());
    }
}

//! What a connection is called on the wire, and who is told it came or left.
//!
//! As in Dota, players are not told about spectators: a spectator's arrival
//! or departure reaches no seat, not even as a lobby sent again, and the
//! numbers connections are told count seats and watchers apart, so a seat's
//! own number never says how many came to watch.

use bota_proto::{PlayerId, ServerMsg, SlotId};

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
    pub(super) fn give(&mut self, id: PlayerId, seated: bool) -> PlayerId {
        let told = if seated {
            self.seats += 1;
            PlayerId(self.seats)
        } else {
            self.watchers += 1;
            PlayerId(WATCHERS_FROM + self.watchers)
        };
        self.ids.retain(|(own, _)| *own != id);
        self.ids.push((id, told));
        told
    }

    /// Takes connection `id`'s number back; none for one never greeted.
    pub(super) fn take(&mut self, id: PlayerId) -> Option<PlayerId> {
        let at = self.ids.iter().position(|(own, _)| *own == id)?;
        Some(self.ids.swap_remove(at).1)
    }
}

impl Server {
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

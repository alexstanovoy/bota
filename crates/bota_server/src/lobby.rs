//! Seats, picks, readiness, and the mapping between wire and match identity.

use bota_core::rules;
use bota_proto::{HeroId, LobbySlot, PlayerId, Role, SeatKey, SlotId, Team};

/// The hero a dummy seat plays: Shadow Fiend.
pub const DUMMY_HERO: HeroId = HeroId::SHADOW_FIEND;

/// One seat of the match and who currently holds it.
#[derive(Clone, Debug)]
pub struct RosterSeat {
    pub slot: SlotId,
    pub team: Team,
    /// The connection holding it. Absent while open.
    pub player: Option<PlayerId>,
    /// Display name of the holder. Empty while open.
    pub name: String,
    /// Absent while open.
    pub role: Option<Role>,
    /// Absent until picked.
    pub hero: Option<HeroId>,
    pub ready: bool,
    /// Whether the pick is final: it was made, or stood, while the seat was
    /// ready. It stays so when readiness is withdrawn or the holder changes,
    /// so the last to ready cannot wait for the others' picks and switch.
    pub pick_locked: bool,
    /// Whole seconds the seat picked after the selection time ran out; it
    /// starts the match with that much penalty gold less.
    pub late_secs: u32,
    /// A seat nobody holds: ready from the start, its hero stays put and never acts.
    pub dummy: bool,
    /// The key the holder of an unreserved seat came with, which takes the
    /// seat back after a lost connection, also mid-match.
    pub claim: Option<SeatKey>,
}

impl RosterSeat {
    /// Hands the seat to a connection, not yet ready; its pick stays. The
    /// name is cut to [`rules::MAX_SHOWN_NAME_BYTES`].
    pub fn hold(&mut self, player: PlayerId, mut name: String, role: Role) {
        name.truncate(name.floor_char_boundary(rules::MAX_SHOWN_NAME_BYTES));
        self.player = Some(player);
        self.name = name;
        self.role = Some(role);
        self.ready = false;
    }
}

/// A seat held back for whoever presents its key.
#[derive(Clone, Debug)]
pub struct ReservedSeat {
    /// What entitles a connection to the seat.
    pub key: SeatKey,
    /// The name whoever holds the seat is shown under.
    pub name: String,
    /// The side the seat plays on.
    pub team: Team,
    /// The kind of player the seat is for: [`Role::Bot`] or [`Role::Player`]
    /// (a human). A `Hello` of the other role is refused. Absent admits both.
    pub kind: Option<Role>,
}

/// All seats, and the `PlayerId` to `SlotId` mapping.
///
/// The simulation never sees a `PlayerId`; this is where network identity
/// stops.
#[derive(Clone, Debug)]
pub struct Roster {
    /// Every seat, sorted by slot.
    pub seats: Vec<RosterSeat>,
}

impl Roster {
    /// An empty roster: even slots are Radiant, odd are Dire. With `dummy`
    /// the last seat is a ready dummy playing [`DUMMY_HERO`].
    ///
    /// [`Roster::reserved`] seats each reservation on its own side instead.
    pub fn new(players: u8, dummy: bool) -> Roster {
        let mut roster = Roster {
            seats: (0..players)
                .map(|i| RosterSeat {
                    slot: SlotId(i),
                    team: if i.is_multiple_of(2) {
                        Team::Radiant
                    } else {
                        Team::Dire
                    },
                    player: None,
                    name: String::new(),
                    role: None,
                    hero: None,
                    ready: false,
                    pick_locked: false,
                    late_secs: 0,
                    dummy: false,
                    claim: None,
                })
                .collect(),
        };
        if dummy && let Some(seat) = roster.seats.last_mut() {
            seat.dummy = true;
            seat.name = "dummy".to_string();
            seat.role = Some(Role::Bot);
            seat.hero = Some(DUMMY_HERO);
            seat.ready = true;
            seat.pick_locked = true;
        }
        roster
    }

    /// An empty roster of reserved seats, each on its reservation's side.
    pub fn reserved(reserved: &[ReservedSeat]) -> Roster {
        let players = u8::try_from(reserved.len()).expect("at most 255 reserved seats");
        let mut roster = Roster::new(players, false);
        for (seat, reservation) in roster.seats.iter_mut().zip(reserved) {
            seat.team = reservation.team;
        }
        roster
    }

    /// Whether any seat is held by a connection.
    pub fn anyone_seated(&self) -> bool {
        self.seats.iter().any(|s| s.player.is_some())
    }

    /// The seat a key is reserved for. Every reservation is compared, so the time
    /// taken does not tell which one matched.
    pub fn seat_for_key(&self, reserved: &[ReservedSeat], key: &SeatKey) -> Option<SlotId> {
        let mut found = None;
        for (seat, reservation) in self.seats.iter().zip(reserved) {
            if reservation.key == *key {
                found = Some(seat.slot);
            }
        }
        found
    }

    /// The unreserved seat a key claimed. Every claim is compared, so the
    /// time taken does not tell which one matched.
    pub fn seat_for_claim(&self, key: &SeatKey) -> Option<SlotId> {
        let mut found = None;
        for seat in &self.seats {
            if seat.claim.as_ref() == Some(key) {
                found = Some(seat.slot);
            }
        }
        found
    }

    /// The seat a connection holds.
    pub fn seat_of(&self, player: PlayerId) -> Option<&RosterSeat> {
        self.seats.iter().find(|s| s.player == Some(player))
    }

    /// The seat a connection holds, for updating.
    pub fn seat_of_mut(&mut self, player: PlayerId) -> Option<&mut RosterSeat> {
        self.seats.iter_mut().find(|s| s.player == Some(player))
    }

    /// The first open seat, if any.
    pub fn free_seat_mut(&mut self) -> Option<&mut RosterSeat> {
        self.seats
            .iter_mut()
            .find(|s| s.player.is_none() && !s.dummy)
    }

    /// Opens whatever seat a connection held; its pick stays.
    pub fn release(&mut self, player: PlayerId) {
        if let Some(seat) = self.seat_of_mut(player) {
            seat.player = None;
            seat.name.clear();
            seat.role = None;
            seat.ready = false;
        }
    }

    /// Whether every seat is held (a dummy needs no holder).
    pub fn all_held(&self) -> bool {
        self.seats.iter().all(|s| s.dummy || s.player.is_some())
    }

    /// Whether every seat has a hero.
    pub fn all_picked(&self) -> bool {
        self.seats.iter().all(|s| s.hero.is_some())
    }

    /// Whether any seat holds this hero.
    pub fn holds(&self, hero: HeroId) -> bool {
        self.seats.iter().any(|s| s.hero == Some(hero))
    }

    /// Whether another seat than `player`'s holds this hero.
    pub fn taken_by_other(&self, player: PlayerId, hero: HeroId) -> bool {
        self.seats
            .iter()
            .any(|s| s.hero == Some(hero) && s.player != Some(player))
    }

    /// Whether every seat is held, picked and ready with its pick.
    pub fn all_locked_in(&self) -> bool {
        self.all_held() && self.all_picked() && self.seats.iter().all(|s| s.ready)
    }

    /// Whether every seat is a dummy or held by a bot: a game of bots alone
    /// has no tactical phase.
    pub fn only_bots(&self) -> bool {
        self.seats
            .iter()
            .all(|s| s.dummy || s.role == Some(Role::Bot))
    }

    /// The lobby as the wire sees it.
    pub fn lobby_slots(&self) -> Vec<LobbySlot> {
        self.seats
            .iter()
            .map(|s| LobbySlot {
                slot: s.slot,
                team: s.team,
                name: s.name.clone(),
                role: s.role,
                hero: s.hero,
                ready: s.ready,
            })
            .collect()
    }

    /// The picks. Panics unless every seat has picked.
    pub fn picks(&self) -> Vec<bota_proto::Pick> {
        self.seats
            .iter()
            .map(|s| bota_proto::Pick {
                slot: s.slot,
                team: s.team,
                hero: s.hero.expect("picks are read only when all are ready"),
            })
            .collect()
    }
}

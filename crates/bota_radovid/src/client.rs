//! The control connection, with bota's fingerprint, the folded view and the
//! automatic answers.

use radovid_client::proto::{
    AccountInfo, AccountKind, ClientInfo, ClientMsg, Event, Limits, MatchId, ProposalId,
    QueueEntryId, Reply, RulesetId, RulesetInfo, Secret, SessionToken,
};
use radovid_client::{ClientError, Incoming};

use crate::game::{GAME, fingerprint};
use crate::search::Searches;
use crate::tls::Tls;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
type Backend = radovid_client::WebClient;
#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
type Backend = radovid_client::Background;

/// Requests whose answers the client files; more unanswered ones than this
/// mean the service is not answering.
const MAX_FILED: usize = 1024;

/// What the answer to a request is filed under.
enum Kind {
    Limits(Option<RulesetId>),
    Accept(ProposalId),
    /// Nothing is filed; the answer only tells whether it worked.
    Plain,
}

struct Filed {
    seq: u32,
    kind: Kind,
    /// The client asked for its own sake: the answer is not handed on.
    own: bool,
}

/// The control connection of a bota client. Nothing blocks except
/// [`Client::wait`]; everything the service says comes out of
/// [`Client::poll`], after the client has noted it in its view.
pub struct Client {
    /// The link; none once [`Client::hang_up`] closed it.
    backend: Option<Backend>,
    /// What the service said about itself and the account, kept past a hang-up.
    kept: Option<(Vec<RulesetInfo>, Option<AccountInfo>)>,
    searches: Searches,
    limits: Vec<(Option<RulesetId>, Limits)>,
    filed: Vec<Filed>,
    session: Option<SessionToken>,
    auto_accept: Option<bool>,
}

impl Client {
    /// Starts connecting to `url` (`wss://host:port`, with or without
    /// `/control`) and queues the Hello that names bota's wire fingerprint.
    /// The Welcome arrives as the reply to sequence number 1.
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    pub fn open(url: &str, tls: &Tls) -> Result<Client, ClientError> {
        Ok(Self::with(Backend::open(url, &tls.control, fingerprint())?))
    }

    /// As on the desktop, over the browser's WebSocket, which brings its own
    /// trust.
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    pub fn open(url: &str, _tls: &Tls) -> Result<Client, ClientError> {
        Ok(Self::with(Backend::open(url, fingerprint())?))
    }

    fn with(backend: Backend) -> Client {
        Client {
            backend: Some(backend),
            kept: None,
            searches: Searches::default(),
            limits: Vec::new(),
            filed: Vec::new(),
            session: None,
            auto_accept: None,
        }
    }

    /// Sends a request; its answer comes out of [`Client::poll`] as a reply
    /// with the sequence number returned. Lobbies, ladders and the rest of the
    /// protocol go through here.
    pub fn request(&mut self, msg: ClientMsg) -> Result<u32, ClientError> {
        self.send(msg, false)
    }

    fn send(&mut self, msg: ClientMsg, own: bool) -> Result<u32, ClientError> {
        let kind = match &msg {
            ClientMsg::Limits { ruleset } => Kind::Limits(ruleset.clone()),
            ClientMsg::MatchAccept { proposal } => Kind::Accept(*proposal),
            _ => Kind::Plain,
        };
        let filing = own || !matches!(kind, Kind::Plain);
        if filing && self.filed.len() >= MAX_FILED {
            return Err(ClientError::Protocol(
                "too many unanswered requests".to_owned(),
            ));
        }
        let seq = self
            .backend
            .as_mut()
            .ok_or(ClientError::Closed)?
            .request(msg)?;
        if filing {
            self.filed.push(Filed { seq, kind, own });
        }
        Ok(seq)
    }

    pub fn register(
        &mut self,
        login: &str,
        password: &str,
        kind: AccountKind,
        bot_version: Option<String>,
    ) -> Result<u32, ClientError> {
        self.request(ClientMsg::Register {
            login: login.to_owned(),
            password: Secret::new(password),
            kind,
            client: ClientInfo { bot_version },
        })
    }

    pub fn login(
        &mut self,
        login: &str,
        password: &str,
        bot_version: Option<String>,
    ) -> Result<u32, ClientError> {
        self.request(ClientMsg::Login {
            login: login.to_owned(),
            password: Secret::new(password),
            client: ClientInfo { bot_version },
        })
    }

    /// Logs in again with the token of an earlier login.
    pub fn resume(
        &mut self,
        session: SessionToken,
        bot_version: Option<String>,
    ) -> Result<u32, ClientError> {
        self.request(ClientMsg::Resume {
            session,
            client: ClientInfo { bot_version },
        })
    }

    /// Adds a queue entry for `ruleset`; the entry shows up in
    /// [`Client::searches`] with the reply.
    pub fn queue(&mut self, ruleset: &RulesetId) -> Result<u32, ClientError> {
        self.request(ClientMsg::QueueJoin {
            ruleset: ruleset.clone(),
        })
    }

    pub fn leave_queue(&mut self, entry: QueueEntryId) -> Result<u32, ClientError> {
        self.request(ClientMsg::QueueLeave { entry })
    }

    pub fn accept(&mut self, proposal: ProposalId) -> Result<u32, ClientError> {
        self.request(ClientMsg::MatchAccept { proposal })
    }

    /// Declines `proposal`: the entry leaves the queue and the account waits
    /// out the decline cooldown. The others go back to their places.
    pub fn decline(&mut self, proposal: ProposalId) -> Result<u32, ClientError> {
        self.request(ClientMsg::MatchDecline { proposal })
    }

    /// Asks for the handoff of a match of the account that has not finished:
    /// to take the seat again after a break, or to watch from another device.
    pub fn handoff(&mut self, match_id: MatchId) -> Result<u32, ClientError> {
        self.request(ClientMsg::MatchHandoff { match_id })
    }

    /// Asks for the limits as they apply to the account, in general or for
    /// one ruleset. The client already asked for the general ones on
    /// connecting and on logging in; [`Client::limits`] has the answers.
    pub fn request_limits(&mut self, ruleset: Option<RulesetId>) -> Result<u32, ClientError> {
        self.request(ClientMsg::Limits { ruleset })
    }

    /// The next message from the service, if any. The client has noted it
    /// before it comes out: the view and the limits are already up to date,
    /// and a proposal has already been accepted if that is automatic.
    pub fn poll(&mut self) -> Result<Option<Incoming>, ClientError> {
        while let Some(incoming) = match self.backend.as_mut() {
            Some(backend) => backend.poll()?,
            None => None,
        } {
            if let Some(shown) = self.note(incoming)? {
                return Ok(Some(shown));
            }
        }
        Ok(None)
    }

    /// As [`Client::poll`], waiting up to `timeout` for a message. Not for
    /// the browser, where nothing may block.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(&mut self, timeout: std::time::Duration) -> Result<Option<Incoming>, ClientError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(incoming) = self.poll()? {
                return Ok(Some(incoming));
            }
            if std::time::Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn note(&mut self, incoming: Incoming) -> Result<Option<Incoming>, ClientError> {
        match &incoming {
            Incoming::Reply { seq, result } => {
                let filed = self
                    .filed
                    .iter()
                    .position(|f| f.seq == *seq)
                    .map(|at| self.filed.swap_remove(at));
                if let Ok(reply) = result {
                    self.note_reply(reply, filed.as_ref().map(|f| &f.kind))?;
                }
                if filed.is_some_and(|f| f.own) {
                    return Ok(None);
                }
            }
            Incoming::Event(event) => self.note_event(event)?,
        }
        Ok(Some(incoming))
    }

    fn note_reply(&mut self, reply: &Reply, filed: Option<&Kind>) -> Result<(), ClientError> {
        self.searches.fold_reply(reply);
        match reply {
            Reply::Welcome { .. } => {
                self.send(ClientMsg::Limits { ruleset: None }, true)?;
            }
            Reply::LoggedIn {
                session,
                activities,
                ..
            } => {
                self.session = Some(*session);
                self.searches.start(activities);
                self.send(ClientMsg::Rulesets, true)?;
                self.send(ClientMsg::Limits { ruleset: None }, true)?;
            }
            Reply::Limits { limits } => {
                if let Some(Kind::Limits(ruleset)) = filed {
                    match self.limits.iter_mut().find(|(r, _)| r == ruleset) {
                        Some(known) => known.1 = *limits,
                        None => self.limits.push((ruleset.clone(), *limits)),
                    }
                }
            }
            Reply::Done => {
                if let Some(Kind::Accept(proposal)) = filed {
                    self.searches.answered(*proposal);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn note_event(&mut self, event: &Event) -> Result<(), ClientError> {
        self.searches.fold_event(event);
        if let Event::MatchProposed { proposal } = event
            && self.auto_accepting()
        {
            self.send(
                ClientMsg::MatchAccept {
                    proposal: proposal.proposal,
                },
                true,
            )?;
        }
        Ok(())
    }

    fn auto_accepting(&self) -> bool {
        self.auto_accept
            .unwrap_or_else(|| self.me().is_some_and(|me| me.kind == AccountKind::Bot))
    }

    /// Whether proposals are accepted at once: on for a bot account unless
    /// told otherwise, off for a human one (a menu shows the prompt).
    pub fn auto_accept(&self) -> bool {
        self.auto_accepting()
    }

    pub fn set_auto_accept(&mut self, on: bool) {
        self.auto_accept = Some(on);
    }

    /// The searches in flight, as the service last told them.
    pub fn searches(&self) -> &Searches {
        &self.searches
    }

    /// The limits that apply to the account, in general (`None`) or for a
    /// ruleset, once the service has told them. Per-account concurrency
    /// (`games_per_account`), the accept timeout, the decline cooldown and the
    /// capacities.
    pub fn limits(&self, ruleset: Option<&RulesetId>) -> Option<Limits> {
        self.limits
            .iter()
            .find(|(r, _)| r.as_ref() == ruleset)
            .map(|(_, limits)| *limits)
    }

    /// The rulesets this build can play: those of bota's game and wire.
    pub fn rulesets(&self) -> impl Iterator<Item = &RulesetInfo> {
        let known = match (&self.backend, &self.kept) {
            (Some(backend), _) => backend.control().rulesets(),
            (None, Some((rulesets, _))) => rulesets,
            (None, None) => &[],
        };
        known
            .iter()
            .filter(|r| r.game == GAME && r.game_fingerprint == fingerprint())
    }

    /// The account, once logged in.
    pub fn me(&self) -> Option<&AccountInfo> {
        match (&self.backend, &self.kept) {
            (Some(backend), _) => backend.control().me(),
            (None, Some((_, me))) => me.as_ref(),
            (None, None) => None,
        }
    }

    /// Closes the link and keeps the view: the searches, limits, rulesets and account stay
    /// readable, requests fail with [`ClientError::Closed`] and nothing more is received. For a
    /// client that has nothing live to hear and would only hold a socket idle.
    pub fn hang_up(&mut self) {
        if let Some(backend) = self.backend.take() {
            let control = backend.control();
            self.kept = Some((control.rulesets().to_vec(), control.me().cloned()));
        }
    }

    /// Whether [`Client::hang_up`] closed the link.
    pub fn hung_up(&self) -> bool {
        self.backend.is_none()
    }

    /// The token of the last login, to [`Client::resume`] with.
    pub fn session(&self) -> Option<SessionToken> {
        self.session
    }
}

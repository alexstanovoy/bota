//! The round trip of a connection, timed where the bytes leave and arrive.
//!
//! The meter sends a [`ClientMsg::Probe`] a second and the server's network side answers each
//! with a [`ServerMsg::Echo`](bota_proto::ServerMsg::Echo) as it reads it. The send and the
//! receipt are stamped by the socket layer on a monotonic clock the caller supplies (a duration
//! since any fixed start), never by a frame or a tick, so the ping is the line's, whatever the
//! tick rate. It feeds nothing into the simulation or a session: it is for the player's eyes.

use std::time::Duration;

use bota_proto::ClientMsg;

/// How often a probe goes out.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// Each sample moves the smoothed ping this fraction of the way to it: a running average that
/// follows a change in a few seconds yet does not jump with one slow packet, as the ping Dota
/// shows.
const SMOOTHING: u32 = 4;

/// The probe in flight, the smoothed ping so far.
#[derive(Clone, Debug, Default)]
pub struct PingMeter {
    next_id: u32,
    /// The probe awaiting its echo: its id and when it was sent.
    in_flight: Option<(u32, Duration)>,
    /// When the last probe went out, answered or not.
    last_sent: Option<Duration>,
    smoothed: Option<Duration>,
}

impl PingMeter {
    /// The probe to send now, stamped as sent at `now`; none while the last went out less than
    /// [`PROBE_INTERVAL`] ago. A probe never answered is given up on when the next goes out.
    pub fn probe(&mut self, now: Duration) -> Option<ClientMsg> {
        if self
            .last_sent
            .is_some_and(|last| now.saturating_sub(last) < PROBE_INTERVAL)
        {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.in_flight = Some((id, now));
        self.last_sent = Some(now);
        Some(ClientMsg::Probe { id })
    }

    /// Takes the echo of probe `id` as received at `now`; one that answers no probe in flight
    /// (a late echo of a probe given up on) is ignored.
    pub fn echoed(&mut self, id: u32, now: Duration) {
        let Some((sent_id, sent)) = self.in_flight else {
            return;
        };
        if sent_id != id {
            return;
        }
        self.in_flight = None;
        let sample = now.saturating_sub(sent);
        self.smoothed = Some(match self.smoothed {
            None => sample,
            Some(average) if sample >= average => average + (sample - average) / SMOOTHING,
            Some(average) => average - (average - sample) / SMOOTHING,
        });
    }

    /// The smoothed round trip, once an echo has come back.
    pub const fn ping(&self) -> Option<Duration> {
        self.smoothed
    }
}

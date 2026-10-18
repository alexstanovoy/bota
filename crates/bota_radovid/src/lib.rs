//! The bota side of a radovid client.
//!
//! radovid knows no game: it keeps accounts, queues, lobbies and ratings, and
//! hands a match's players a URL and a ticket. This crate adds what is bota's:
//!
//! - [`Client`], the control connection (a thread natively, the browser's
//!   WebSocket on wasm32) that says bota's wire fingerprint in its Hello, folds
//!   the service's news into a [`Searches`] view a menu only has to draw,
//!   answers proposals by itself when [`Client::set_auto_accept`] says so, and
//!   knows the account's [`Limits`];
//! - the way into the game a [`Handoff`] names: [`take_seat`], [`spectate`],
//!   [`connect`] and [`connect_net`] natively, [`web_link`] and [`seat_hello`]
//!   in the browser, all with the handoff's ticket as the seat key;
//! - [`decode_report`], the picks and numbers of a finished match.
//!
//! One account may hold several games at once (see [`Limits`]): a client keeps
//! one control connection and one game link per match.

mod client;
mod clock;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
mod game;
mod report;
mod search;
mod tls;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web;

pub use client::Client;
#[cfg(not(target_arch = "wasm32"))]
pub use connect::{connect, connect_net, spectate, take_seat};
pub use game::{GAME, check_handoff, fingerprint, seat_hello, seat_key, seat_name};
pub use radovid_client::proto::{self, *};
pub use radovid_client::{ClientError, Incoming};
pub use report::decode_report;
pub use search::{Offer, Search, Searches, SlotWait};
pub use tls::Tls;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web::web_link;

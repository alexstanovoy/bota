//! What a match leaves to look back on, outside the wire.
//!
//! The replay is the build-locked recording of a match: only a build of its own wire plays it.
//! The match metadata file ([`meta`]) is its durable companion: a small log of facts (what each
//! seat held and had earned every game minute, the kills, the purchases, the levels, the
//! buildings that fell, the final items and the end) in a format of its own, which every later
//! build reads and which only grows. The [`Recorder`](meta::Recorder) writes it by watching the
//! records a replay holds, in the server while the match runs, or in any tool over a replay of
//! the same wire; the [`trailer`] carries it inside a replay without a record of its own.
//!
//! Nothing here is wording, layout or a derived number (a rate, a curve, a highlight): those are
//! a client's.

pub mod limits;
pub mod meta;
pub mod trailer;

#[cfg(test)]
mod tests;

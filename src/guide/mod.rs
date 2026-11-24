// SPDX-License-Identifier: MIT

//! The guide: how Avestruz is put together, and why. Documentation only.
//!
//! Every other module says what its own piece does, where the piece is.
//! These pages say how the pieces go together: what talks to what, what
//! happens in which order, which relay is asked what, and what was decided
//! along the way. There is no code in them. They are built with the rest
//! of the documentation (`just doc-open`), where their diagrams are drawn.
//!
//! | Page | What it answers |
//! |---|---|
//! | [`architecture`] | What the parts are, what each may know of the others, where each runs, and where things are kept |
//! | [`flows`] | What happens, step by step: signing in, filling the feed, publishing, bookmarking, searching, zapping |
//! | [`relays`] | Which relays there are, which of them is asked or told what, and whose word is taken for where a relay is |
//! | [`protocol`] | Which NIPs are spoken and which kinds are read and written, and where in the code each is |
//! | [`decisions`] | What was decided and why, the rules everything else keeps to, and what is not there yet |
//! | [`testing`] | How it is tested, what is not, and every test by name |
//!
//! # Where to start
//!
//! ```mermaid
//! flowchart LR
//!     new(["New here"]) --> architecture
//!     architecture --> flows
//!     flows --> relays
//!     change(["About to change something"]) --> decisions
//!     decisions --> testing
//!     nostr(["Wondering what is spoken"]) --> protocol
//!     protocol --> relays
//! ```
//!
//! # How these pages are kept true
//!
//! They are doc comments, so they are read by whoever reads the code, and
//! built by `just check`, which fails on a link that leads nowhere: a
//! function that is renamed or removed takes the build of the page that
//! names it down with it. Where a page gives a number (a timeout, a limit)
//! it links the constant, and the constant is what counts. The diagrams
//! are checked to parse by `just mermaid-check`. What a diagram *says* is
//! checked by nobody but its reader: if one disagrees with the code, the
//! code is right and the diagram wants fixing.

pub mod architecture;
pub mod decisions;
pub mod flows;
pub mod protocol;
pub mod relays;
pub mod testing;

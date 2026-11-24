// SPDX-License-Identifier: MIT

//! Everything that speaks Nostr: the relay client and its local event store,
//! the signed-in account and its key, and the models the views are built from.
//!
//! Nothing here knows of the toolkit: no module names a widget, a
//! message or the model. Three modules do the talking, four more ask a
//! web server one thing each, and the rest are pure: an event goes in
//! and something for a view comes out, or what someone meant goes in and
//! an unsigned event comes out.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph talk["What talks"]
//!         client["client: relays and the store"]
//!         account["account: the keyring, a remote signer"]
//!         private["private_relay: the relay it runs itself"]
//!     end
//!     subgraph web["What asks a web server one thing"]
//!         nip05
//!         zap
//!         relay_info
//!         blossom
//!     end
//!     subgraph read["Events read: what a view shows"]
//!         models
//!         content
//!         thread
//!         feed
//!         engagement
//!         search
//!     end
//!     subgraph write["Events made: unsigned, for the signer"]
//!         compose
//!         contacts
//!         bookmarks
//!         relay_lists
//!         kept_drafts
//!     end
//!     subgraph choose["What is decided"]
//!         outbox["outbox: which relays to ask"]
//!         sweep["sweep: what the store lets go of"]
//!         sightings["sightings: which relay sent what"]
//!     end
//!     client -->|"reads into"| read
//!     client -->|"signs and sends"| write
//!     client -->|"goes by"| choose
//!     client --> account
//!     client --> private
//! ```
//!
//! | Module | What it is |
//! |---|---|
//! | [`account`] | Parsing an `nsec`, an `npub` or a bunker address, keeping what is secret in the keyring, signing, encrypting |
//! | [`blossom`] | The account's media servers, and sending a file to them for a note to link to |
//! | [`bookmarks`] | The account's bookmark list, and what it keeps to itself |
//! | [`client`] | The relay pools and the LMDB event store behind them |
//! | [`compose`] | What someone wrote, made into an event ready to be signed |
//! | [`contacts`] | A contact list, changed by one person |
//! | [`content`] | A note's text taken apart: links, mentions, hashtags, pictures, quotes |
//! | [`engagement`] | What others made of a note: replies, reposts, likes and zaps |
//! | [`feed`] | The notes of a feed, newest first, and the ones that wait beside it |
//! | [`kept_drafts`] | Drafts as events, as the relay Avestruz runs itself keeps them |
//! | [`models`] | Notes and profiles, as the views need them |
//! | [`nip05`] | Names at a domain, and whether the domain agrees |
//! | [`notifications`] | What others did that concerns the account: replies, mentions, reactions, reposts and zaps |
//! | [`outbox`] | People's relay lists, and which of the relays they name to ask |
//! | [`private_relay`] | The relay Avestruz runs itself, and the way to it |
//! | [`relay_info`] | What a relay says of itself |
//! | [`relay_lists`] | The relays an account says are its own, by what they are for |
//! | [`search`] | Searching for people and notes, and what is typed that is no search |
//! | [`share`] | A link to a note that anyone can open |
//! | [`sightings`] | Which relays each note was seen on |
//! | [`sweep`] | What the event store lets go of, and what it keeps however old |
//! | [`thread`] | A conversation: a note, what it answers, and what answers it |
//! | [`zap`] | Asking someone's Lightning address for an invoice that zaps a note |
//!
//! Which NIP and which kind each of these speaks is in
//! [`crate::guide::protocol`], and what is decided about relays in
//! [`crate::guide::relays`].

pub mod account;
pub mod blossom;
pub mod bookmarks;
pub mod client;
pub mod compose;
pub mod contacts;
pub mod content;
pub mod engagement;
pub mod feed;
pub mod kept_drafts;
pub mod models;
pub mod nip05;
pub mod notifications;
pub mod outbox;
pub mod private_relay;
pub mod relay_info;
pub mod relay_lists;
pub mod search;
pub mod share;
pub mod sightings;
pub mod sweep;
pub mod thread;
pub mod zap;

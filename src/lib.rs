// SPDX-License-Identifier: MIT

//! Avestruz: a Nostr client that lives in the COSMIC™ desktop panel.
//!
//! A button in the panel opens a popup, and the popup is the whole client:
//! the notes of the people an account follows, their threads and profiles,
//! writing and answering, likes, reposts, zaps, bookmarks, search, and the
//! account's relays.
//!
//! This is the library the binary is a thin `main` over. It is not meant
//! to be used by anything else: it is a library so that its modules can be
//! tested and documented, and what is `pub` here is public to the crate's
//! own tests and docs.
//!
//! # Start here
//!
//! The [`guide`] is the way in: how the parts go together, what happens
//! in which order, which relay is asked what, what is spoken, what was
//! decided, and how it is tested. Each module below then says what its own
//! piece does.
//!
//! ```mermaid
//! flowchart TB
//!     panel["The panel button: a process of its own"] -- "toggle" --> popup["The popup"]
//!     popup -- "a message" --> update["app: update"]
//!     update --> handlers["handlers"]
//!     handlers -- "change" --> state[("state: AppModel")]
//!     state -- "is drawn by" --> views["views"]
//!     views --> popup
//!     handlers -- "ask, tell" --> nostr["nostr"]
//!     nostr <--> store[("The event store")]
//!     nostr <--> relays(["Relays"])
//!     handlers -- "fetch" --> web(["The web: pictures, previews, names, invoices"])
//!     nostr -- "sign" --> keyring[("The keyring, or a remote signer")]
//! ```
//!
//! | To find out | Read |
//! |---|---|
//! | What the parts are and how they talk | [`guide::architecture`] |
//! | What happens when something is done | [`guide::flows`] |
//! | Which relay is asked or told what | [`guide::relays`] |
//! | Which NIPs and kinds are spoken, and where | [`guide::protocol`] |
//! | What must not be undone by accident | [`guide::decisions`] |
//! | How it is tested, and every test by name | [`guide::testing`] |
//!
//! # The modules
//!
//! | Module | What it is |
//! |---|---|
//! | [`app`] | The application: init, the subscriptions, and `update` |
//! | [`state`] | The model: everything that outlives the popup |
//! | [`messages`] | Every message that changes the model |
//! | [`handlers`] | What happens on each message |
//! | [`views`] | What each view looks like, and the components they share |
//! | [`nostr`] | Everything that speaks Nostr: relays, the store, the account, events |
//! | [`config`] | The settings |
//! | [`http`], [`images`], [`previews`], [`requests`] | Everything fetched from the web |
//! | [`emoji`] | The emoji to react with: the ones used most, and all there are |
//! | [`fonts`] | Which fonts emoji are drawn with: none whose colour glyphs can't be drawn, and the ones for emoji before any other |
//! | [`video`] | The videos notes link to, played in their rows |
//! | [`youtube`] | Which links are YouTube videos, and asking yt-dlp what there is to play of one |
//! | [`drafts`] | What is being written |
//! | [`media`] | A file to be attached to a note: what it is, and a picture without what it says beside the picture |
//! | [`screen`], [`links`], [`logging`], [`helpers`], [`i18n`] | The rest |
//!
//! # How much of it is tested
//!
#![doc = include_str!(concat!(env!("OUT_DIR"), "/test_summary.md"))]
//! They are listed by name, as the sentences they were written as, in
//! [`guide::testing`].
//!
//! # Building these pages
//!
//! ```sh
//! just doc-open
//! ```
//!
//! That builds the documentation with private items, which is most of an
//! application, and with its diagrams drawn. Plain `cargo doc` builds the
//! same pages with each diagram as the text it was written as.

pub mod app;
pub mod button;
pub mod config;
pub mod drafts;
pub mod emoji;
pub mod fonts;
pub mod guide;
pub mod handlers;
pub mod helpers;
pub mod http;
pub mod i18n;
pub mod images;
pub mod links;
pub mod logging;
pub mod media;
pub mod messages;
pub mod nostr;
pub mod previews;
pub mod requests;
pub mod screen;
pub mod state;
pub mod video;
pub mod views;
pub mod youtube;

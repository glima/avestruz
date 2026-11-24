// SPDX-License-Identifier: MIT

//! View rendering modules for Avestruz.
//!
//! A view is a method on [`AppModel`](crate::state::AppModel) that takes
//! `&self` and returns widgets. It can't change the model: what it wants
//! done, it says in the messages its widgets send. It is built again
//! whenever the model changed, and from nothing whenever the popup opens.
//!
//! ```mermaid
//! flowchart LR
//!     panel["the button in the panel: another process"] -->|"pressed"| popup["popup: an error banner, over the view on show"]
//!     popup --> state{"ViewState"}
//!     state -->|"Login"| login
//!     state -->|"Feed"| feed
//!     state -->|"Page"| page{"The last page opened"}
//!     state -->|"Compose"| compose
//!     state -->|"Zap"| zap
//!     state -->|"Settings"| settings
//!     state -->|"Relays"| relays
//!     state -->|"Media"| media
//!     page --> thread
//!     page --> profile
//!     page --> bookmarks
//!     page --> search
//!     page --> notifications
//!     feed --> rows["components: a VirtualList of note rows"]
//!     thread --> rows
//!     profile --> rows
//!     bookmarks --> rows
//!     search --> rows
//!     notifications --> rows
//! ```
//!
//! | Module | What it draws |
//! |---|---|
//! | [`popup`] | The popup: the view on show, under an error banner when something went wrong |
//! | [`login`] | Signing in |
//! | [`feed`] | The feed, in a scroller |
//! | [`bookmarks`] | The notes the account has bookmarked |
//! | [`notifications`] | What others did that concerns the account |
//! | [`compose`] | The composer: a note, a reply or a quote being written |
//! | [`thread`] | A note's thread: the conversation it is part of |
//! | [`profile`] | Someone's profile: who they are, and their notes |
//! | [`relays`] | The account's relays, by what they are for |
//! | [`media`] | The account's media servers, where what it attaches to notes is kept |
//! | [`search`] | A search: what is typed, and the people and notes found |
//! | [`settings`] | The settings: the account, signing out, text size, how long notes are kept, logging |
//! | [`keys`] | What the keys do |
//! | [`zap`] | Zapping a note: how much, and the invoice to pay |
//! | [`components`] | What the views share: the virtual list, the note row, the cards under a note |
//!
//! Why nothing that matters is kept in a widget is in
//! [`crate::guide::architecture`], and what keeps a list from moving
//! under its reader in [`crate::guide::decisions`].

pub mod bookmarks;
pub mod components;
pub mod compose;
pub mod feed;
pub mod keys;
pub mod login;
pub mod media;
pub mod notifications;
pub mod popup;
pub mod profile;
pub mod relays;
pub mod search;
pub mod settings;
pub mod thread;
pub mod zap;

// SPDX-License-Identifier: MIT

//! Message handlers for Avestruz.
//!
//! Every handler is a method on [`AppModel`](crate::state::AppModel),
//! called from the one `match` in [`crate::app`]. A handler changes the
//! model at once, and returns a `Task` for whatever takes time, whose
//! answer comes back as another message. Nothing else changes the model.
//!
//! ```mermaid
//! flowchart LR
//!     message["A Message"] --> update["app: update"]
//!     update --> handler["One handler"]
//!     handler -->|"at once"| model[("The model")]
//!     handler -->|"returns"| task["A Task: the store, a relay, the signer, the web"]
//!     task -->|"its answer, as a Message"| update
//! ```
//!
//! They are split by what happens:
//!
//! | Module | What it handles |
//! |---|---|
//! | [`auth`] | Signing in with a key, signing out, restoring the account at startup |
//! | [`bookmarks`] | What the account has bookmarked, and bookmarking a note |
//! | [`compose`] | Writing a note, a reply or a quote, keeping drafts, and publishing |
//! | [`engagement`] | What was made of the notes on show, and liking and reposting them |
//! | [`feed`] | Loading the feed and other lists of notes, paging into the past, and authors' profiles |
//! | [`images`] | Fetching the pictures views ask for |
//! | [`keys`] | The keyboard, on a list: stepping through its rows, opening the one it is at, showing the notes that wait, going back |
//! | [`media`] | The account's media servers, and attaching files to what is being written |
//! | [`navigation`] | The popup, which view it shows, the pages opened over the feed, and showing a long note in full where it is |
//! | [`notifications`] | What others did that concerns the account, and how much of it was looked at |
//! | [`previews`] | Asking the web pages notes link to what they say of themselves |
//! | [`profile`] | Opening someone's profile, and following them |
//! | [`relays`] | The account's relays: reading its lists, changing them, and saving them |
//! | [`search`] | Searching for people and notes, and opening what is typed by its address |
//! | [`settings`] | Changing settings, and taking changes made elsewhere |
//! | [`share`] | Copying a link to a note that anyone can open |
//! | [`store`] | Having the event store let go of what is old, once a day |
//! | [`thread`] | Opening a note's thread, and gathering its conversation |
//! | [`video`] | Playing a video a note links to, and finding what there is to play of a YouTube one |
//! | [`zap`] | Zapping a note: asking for the invoice, and waiting for its receipt |
//!
//! What several of them do together, in order, is in
//! [`crate::guide::flows`].

pub mod auth;
pub mod bookmarks;
pub mod compose;
pub mod engagement;
pub mod feed;
pub mod images;
pub mod keys;
pub mod media;
pub mod navigation;
pub mod notifications;
pub mod previews;
pub mod profile;
pub mod relays;
pub mod search;
pub mod settings;
pub mod share;
pub mod store;
pub mod thread;
pub mod video;
pub mod zap;

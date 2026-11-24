// SPDX-License-Identifier: MIT

//! Application state for Avestruz.
//!
//! This module defines the main application model and view state types.
//!
//! The popup closes whenever something else is clicked, and its widget tree
//! goes with it. Anything that should still be there when it opens again —
//! the view, what was typed, the feed and the pages opened over it, how their
//! rows measure and how far they are scrolled — lives here, not in a widget.
//!
//! ```mermaid
//! flowchart LR
//!     model["AppModel"]
//!     model --> feed["following: FeedState. The feed"]
//!     model --> pages["pages: a stack of Page"]
//!     pages --> thread["Thread: ThreadState"]
//!     pages --> profile["Profile: ProfileState, with a FeedState of its notes"]
//!     pages --> bookmarks["Bookmarks: BookmarksPage"]
//!     pages --> search["Search: SearchPage"]
//!     model --> composer["composer, drafts: what is being written"]
//!     model --> zap["zap: ZapState"]
//!     model --> relays["relays: RelaysState"]
//!     model --> media["media: MediaState. Media servers, and files on their way to them"]
//!     model --> people["profiles, nip05: everyone, by public key"]
//!     model --> marks["engagement, bookmarks: what was made of notes"]
//!     model --> stores["images, previews: what was fetched"]
//! ```
//!
//! Every list of notes keeps the same three things beside its notes: how
//! tall each row measured ([`RowHeights`]), how far it is scrolled (a
//! `Cell<f32>` a [`ScrollPosition`](crate::views::components::ScrollPosition)
//! writes to), and what is on its way. That is what lets a list come back
//! exactly as it was left.
//!
//! The whole of what is in the model, by what it is for, is in
//! [`crate::guide::architecture`].

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use cosmic::iced::task;
use cosmic::iced::window::Id;
use cosmic::widget::image;
use cosmic::widget::{segmented_button, text_editor};
use nostr_sdk::prelude::{Event, EventId, PublicKey, RelayUrl, Tag, Url};

use crate::config::Config;
use crate::drafts::Drafts;
use crate::emoji::Group;
use crate::images::{ImageRequest, ImageStore};
use crate::nostr::account::{Account, CredentialError, Signer};
use crate::nostr::blossom::{self, Sent};
use crate::nostr::bookmarks::Bookmarks;
use crate::nostr::client::{NostrService, RelayUse};
use crate::nostr::compose::Target;
use crate::nostr::engagement::{Doer, Engagement, Left, Mark};
use crate::nostr::feed::{Feed, Pending};
use crate::nostr::models::{Note, Profile};
use crate::nostr::notifications::Notifications;
use crate::nostr::relay_info::RelayInfo;
use crate::nostr::relay_lists::{RelayLists, RelaySets, Section};
use crate::nostr::search::SearchFor;
use crate::nostr::thread::Thread;
use crate::nostr::zap::Invoice;
use crate::previews::PreviewStore;
use crate::requests::Inbox;
use crate::screen::Screen;
use crate::video::Videos;
use crate::views::components::RowHeights;

/// Which way the list on show is stepped through from the keyboard, a row
/// at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// To the row after the one at the top of what is in view.
    Next,
    /// To the one before it.
    Previous,
    /// To the very top of the list, and of whatever is above it.
    Top,
}

/// Which view the popup shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewState {
    /// Signing in: nobody is.
    Login,
    /// The feed: notes from the people the account follows.
    Feed,
    /// A page opened over the feed, a thread or a profile: the last one
    /// opened.
    Page,
    /// The composer: a note or a reply being written.
    Compose,
    /// Zapping a note: choosing how much, or the invoice to pay.
    Zap,
    /// The settings: the account, and signing out.
    Settings,
    /// What the keys do.
    Keys,
    /// The account's relays: where it writes, where it reads, where it
    /// searches.
    Relays,
    /// The account's media servers: where what it attaches to notes is
    /// kept.
    Media,
}

/// How far a sign-in has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginProgress {
    /// Nobody is signing in.
    Idle,
    /// The keyring is being asked to keep a secret key.
    Storing,
    /// A remote signer is being waited for. This can be called off.
    Connecting,
}

/// A remote signer turned a sign-in down, or could not be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BunkerRefusal {
    /// What it said, or what went wrong.
    pub reason: String,
    /// The key the signer knows this client by, or would have to: what to
    /// authorize there. `None` if it never came to having one.
    pub client: Option<PublicKey>,
    /// The address had a secret in it, which may have been used up.
    pub with_secret: bool,
    /// That key was copied to the clipboard.
    pub copied: bool,
}

/// How many older pages in a row may bring nothing to show before a list
/// stops looking further back on its own.
pub const BARREN_PAGES_MAX: u8 = 5;

/// A list of someone's notes, as the model keeps it: the feed, or the notes
/// on a profile.
pub struct FeedState {
    /// Which list this is, of all there have been. What is asked of the
    /// store and the relays for it comes back with this, and finds the list
    /// or finds it gone: a feed that started over, a profile that was closed.
    pub id: u64,
    /// Its notes, newest first.
    pub feed: Feed,
    /// How tall each of its rows has measured, by note.
    pub heights: RowHeights<EventId>,
    /// Which of its long notes show in full.
    pub expanded: Expanded,
    /// The newest notes are being fetched.
    pub refreshing: bool,
    /// An older page is being fetched.
    pub loading_older: bool,
    /// The relays had nothing older than the feed's last note.
    pub end_reached: bool,
    /// How many older pages in a row brought nothing to show: replies
    /// only, to a list read without them. Looking further back stops at
    /// [`BARREN_PAGES_MAX`] until it is asked for.
    pub barren_pages: u8,
    /// How far its list is scrolled, kept by the view: where to scroll back
    /// to when it shows again after something else did, or the popup opens
    /// again.
    pub position: Cell<f32>,
    /// The relays answered for its newest notes once, or failed to: it has
    /// been filled, and is its reader's from here on.
    pub settled: bool,
    /// The notes that turned up since, waiting to be asked for.
    pub pending: Pending,
    /// The newest notes on their way were asked for by hand: they are
    /// shown when they are here, with what waited.
    pub show_when_fetched: bool,
    /// The row the keyboard was last at, if it was at any: the one
    /// marked (see [`crate::handlers::keys`]).
    pub marked: Option<EventId>,
}

impl FeedState {
    /// Whether the list stopped looking further back because page after
    /// page had nothing to show, and waits to be asked.
    pub fn paused(&self) -> bool {
        self.barren_pages >= BARREN_PAGES_MAX && !self.end_reached && !self.loading_older
    }

    /// An empty list, the `id`th there has been.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            feed: Feed::default(),
            heights: RowHeights::default(),
            expanded: Expanded::default(),
            refreshing: false,
            loading_older: false,
            end_reached: false,
            barren_pages: 0,
            position: Cell::new(0.0),
            settled: false,
            pending: Pending::default(),
            show_when_fetched: false,
            marked: None,
        }
    }
}

/// One open thread, as the model keeps it.
pub struct ThreadState {
    /// The conversation, as far as it is known.
    pub thread: Thread,
    /// How tall each of its rows has measured, by note.
    pub heights: RowHeights<EventId>,
    /// Which of its long notes show in full, beside the one it was opened
    /// for, which always does.
    pub expanded: Expanded,
    /// The relays are being asked about it.
    pub loading: bool,
    /// How far its list is scrolled, kept by the view: where to scroll back
    /// to when a thread opened from this one is closed.
    pub position: Cell<f32>,
    /// The row the keyboard was last at, if it was at any: the one
    /// marked (see [`crate::handlers::keys`]).
    pub marked: Option<EventId>,
}

impl ThreadState {
    /// `thread`, just opened: the relays are about to be asked.
    pub fn new(thread: Thread) -> Self {
        Self {
            thread,
            heights: RowHeights::default(),
            expanded: Expanded::default(),
            loading: true,
            position: Cell::new(0.0),
            marked: None,
        }
    }
}

/// One open profile, as the model keeps it. What its person says of
/// themselves is with everyone else's, in [`AppModel::profiles`].
pub struct ProfileState {
    /// Whose it is.
    pub public_key: PublicKey,
    /// Their notes.
    pub notes: FeedState,
    /// Their `npub` was copied to the clipboard.
    pub npub_copied: bool,
    /// Why following them did not happen, the last time it was tried.
    pub follow_problem: Option<FollowProblem>,
    /// How far down the page their notes start, under who they are: kept
    /// by the view. How far the page is scrolled when their first note is
    /// at the top of what is in view.
    pub notes_top: Cell<f32>,
}

impl ProfileState {
    /// The profile of `public_key`, just opened, with `notes` to fill.
    pub fn new(public_key: PublicKey, notes: FeedState) -> Self {
        Self { public_key, notes, npub_copied: false, follow_problem: None, notes_top: Cell::new(0.0) }
    }
}

/// The notes the account has bookmarked, as the bookmarks page keeps them.
#[derive(Default)]
pub struct BookmarkedNotes {
    /// The notes bookmarked, the one bookmarked last first, as the
    /// account's list had them when they were last taken from it.
    pub ids: Vec<EventId>,
    /// How many times they were taken from the list: none before the
    /// bookmarks are known. What was asked about them comes back with
    /// this, and finds them as they were or finds them taken anew.
    pub taken: u64,
    /// Those of `ids` that were found, in that order.
    pub notes: Vec<Note>,
    /// How many of `ids`, from the first, have been asked about.
    pub asked: usize,
    /// How many of those asked about are not shown: no relay has them, or
    /// they are no notes a row shows.
    pub missing: usize,
    /// How many things that are not notes are bookmarked: articles,
    /// hashtags and links, which other clients keep there.
    pub others: usize,
    /// The next of them are being looked up.
    pub loading: bool,
    /// How tall each of the rows has measured, by note.
    pub heights: RowHeights<EventId>,
    /// Which of the long ones show in full.
    pub expanded: Expanded,
    /// How far the list is scrolled, kept by the view.
    pub position: Cell<f32>,
    /// The row the keyboard was last at, if it was at any: the one
    /// marked (see [`crate::handlers::keys`]).
    pub marked: Option<EventId>,
}

impl BookmarkedNotes {
    /// Whether every note bookmarked has been asked about.
    pub fn all_asked(&self) -> bool {
        self.asked >= self.ids.len()
    }

    /// How many bookmarks are not shown, as far as they were asked about.
    pub fn hidden(&self) -> usize {
        self.missing + self.others
    }

    /// Start over with the notes `bookmarks` has, from the top. Returns
    /// whether that is another list of notes than was here: if it is the
    /// same, what was found of it stays.
    pub fn take(&mut self, bookmarks: &Bookmarks) -> bool {
        let ids = bookmarks.notes();
        self.others = bookmarks.others();
        if self.taken > 0 && ids == self.ids {
            return false;
        }
        *self = Self { ids, taken: self.taken + 1, others: self.others, ..Self::default() };
        true
    }
}

/// The account's bookmarks, as a page shows them.
pub struct BookmarksPage {
    /// Which page this is, as lists of notes are told apart (see
    /// [`FeedState::id`]).
    pub id: u64,
    /// The notes bookmarked.
    pub notes: BookmarkedNotes,
}

impl BookmarksPage {
    /// The page `id`, just opened.
    pub fn new(id: u64) -> Self {
        Self { id, notes: BookmarkedNotes::default() }
    }
}

/// Why following or unfollowing someone did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowProblem {
    /// The relays have no contact list of the account's, and a first one
    /// is only written when the account says so.
    NoList,
    /// Something went wrong: what, in the user's language.
    Failed(String),
}

/// How far a search for people, or for notes, has got.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SearchProgress {
    /// Nothing was searched for.
    #[default]
    Idle,
    /// The relays are being asked.
    Searching,
    /// They answered: what they found is on the page.
    Found,
    /// They didn't, and why: in the user's language.
    Failed(String),
}

/// A search for people and notes, as its page keeps it. None of it is
/// written anywhere: what was searched for is gone when the page is.
pub struct SearchPage {
    /// Which page this is, as lists of notes are told apart (see
    /// [`FeedState::id`]).
    pub id: u64,
    /// What is typed where the search is written.
    pub typed: String,
    /// The search the page shows the answers to: which one, counted, and
    /// its words. Answers to an earlier one are dropped.
    pub serial: u64,
    pub words: Option<String>,
    /// The relay whose latest notes are on show, if it was a relay's
    /// address that was typed, and what the relay says of itself, once it
    /// has.
    pub relay: Option<RelayUrl>,
    pub relay_info: Option<RelayInfo>,
    /// The name at a domain that is being asked of its domain.
    pub naming: Option<String>,
    /// Whether the people found are on show, or the notes.
    pub what: SearchFor,
    /// How far the search for people got, and who was found, the best
    /// match first. Their profiles are with everyone else's.
    pub people_progress: SearchProgress,
    pub people: Vec<PublicKey>,
    /// How far the search for notes got, and the notes found, the best
    /// match first.
    pub notes_progress: SearchProgress,
    pub notes: Vec<Note>,
    /// How tall the rows of the notes measured.
    pub heights: RowHeights<EventId>,
    /// Which of the long notes show in full.
    pub expanded: Expanded,
    /// How far the people are scrolled, and how far the notes.
    pub people_position: Cell<f32>,
    pub notes_position: Cell<f32>,
    /// The row the keyboard was last at, if it was at any: the one
    /// marked, of the notes found (see [`crate::handlers::keys`]).
    pub marked: Option<EventId>,
}

impl SearchPage {
    /// A search page with nothing searched for.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            typed: String::new(),
            serial: 0,
            words: None,
            relay: None,
            relay_info: None,
            naming: None,
            what: SearchFor::default(),
            people_progress: SearchProgress::default(),
            people: Vec::new(),
            notes_progress: SearchProgress::default(),
            notes: Vec::new(),
            heights: RowHeights::default(),
            expanded: Expanded::default(),
            people_position: Cell::new(0.0),
            notes_position: Cell::new(0.0),
            marked: None,
        }
    }

    /// How far the list on show is scrolled.
    pub fn position(&self) -> &Cell<f32> {
        match self.what {
            SearchFor::People => &self.people_position,
            SearchFor::Notes => &self.notes_position,
        }
    }

    /// How far the search for `what` got.
    pub fn progress(&self, what: SearchFor) -> &SearchProgress {
        match what {
            SearchFor::People => &self.people_progress,
            SearchFor::Notes => &self.notes_progress,
        }
    }
}

/// The account's relays, as the relays page shows and changes them.
#[derive(Debug, Default)]
pub struct RelaysState {
    /// The account's relay lists as they were last read. `None` until
    /// they were.
    pub lists: Option<RelayLists>,
    /// Whether `lists` is what the relays have, and not only what the
    /// store had: nothing is changed until it is.
    pub from_relays: bool,
    /// The relays as the page has them: the lists', with what was changed
    /// here and not saved yet.
    pub edited: RelaySets,
    /// What is typed where a relay is added, for each set of relays.
    pub typed: HashMap<Section, String>,
    /// How the relays this is connected to are doing, as of when the
    /// page last asked.
    pub in_use: HashMap<RelayUrl, RelayUse>,
    /// What the relays that were asked what they support said of
    /// themselves: `None` for one that hasn't said, or didn't.
    pub said: HashMap<RelayUrl, Option<RelayInfo>>,
    /// The lists whose private part is to be read when they are asked
    /// for: more of them as more are needed (see
    /// [`crate::handlers::relays`]).
    pub open: HashSet<Section>,
    /// The lists are being asked of the relays.
    pub loading: bool,
    /// The changed lists are on their way.
    pub saving: bool,
    /// The relays had no list of the account's when its relays were last
    /// saved, and the account was told: saving again writes the first.
    pub first_lists: bool,
}

impl RelaysState {
    /// Whether the page has other relays than the lists do.
    pub fn changed(&self) -> bool {
        self.lists.as_ref().is_some_and(|lists| !lists.sets().same_as(&self.edited))
    }

    /// What is typed where a relay is added to `section`.
    pub fn typed(&self, section: Section) -> &str {
        self.typed.get(&section).map_or("", String::as_str)
    }
}

/// A file on its way to a media server, to be attached to what is being
/// written.
#[derive(Debug)]
pub struct Upload {
    /// Which one it is, of those there have been.
    pub id: u64,
    /// What it is attached to: where its address goes when it has one.
    pub target: Target,
    /// What the file is called.
    pub name: String,
    /// How large it is as it is sent, in bytes: nothing until it was read
    /// and made ready.
    pub size: Sent,
    /// How much of it has been handed over to be sent.
    pub sent: Sent,
    /// The way to call it off.
    pub handle: task::Handle,
}

impl Upload {
    /// How far it has got, in hundredths, once there is something to
    /// send. All of it handed over is not all of it taken: the server has
    /// yet to say so, and the others to fetch it.
    pub fn percent(&self) -> Option<usize> {
        let size = self.size.load(std::sync::atomic::Ordering::Relaxed);
        let sent = self.sent.load(std::sync::atomic::Ordering::Relaxed);
        (size > 0).then(|| (sent.min(size) * 100 / size).min(99))
    }
}

/// The account's media servers, as the media servers page shows and
/// changes them, and what is on its way to them.
#[derive(Debug, Default)]
pub struct MediaState {
    /// The account's server list as it was last read: `None` if it has
    /// none, or was not read yet.
    pub list: Option<Event>,
    /// Whether the list was read at all, from the store or the relays.
    pub read: bool,
    /// Whether `list` is what the relays have, and not only what the
    /// store had: nothing is changed until it is.
    pub from_relays: bool,
    /// The servers as the page has them: the list's, with what was
    /// changed here and not saved yet. Files go to the first.
    pub edited: Vec<Url>,
    /// What is typed where a server is added.
    pub typed: String,
    /// The list is being asked of the relays.
    pub loading: bool,
    /// The changed list is on its way.
    pub saving: bool,
    /// The relays had no server list of the account's when its servers
    /// were last saved, and the account was told: saving again writes
    /// the first.
    pub first_list: bool,
    /// The desktop's file chooser is open, for what is to be attached.
    pub picking: bool,
    /// The files on their way, in the order they were attached.
    pub uploads: Vec<Upload>,
    /// How many files have been attached since the start: what the next
    /// is told apart by.
    pub attached: u64,
    /// What a note says of each file that was sent, by its address
    /// (NIP-92): said when a note that links to it goes out.
    pub said: HashMap<String, Tag>,
}

impl MediaState {
    /// The servers the account's list names, the one files go to first
    /// at the head.
    pub fn servers(&self) -> Vec<Url> {
        blossom::servers(self.list.as_ref())
    }

    /// Whether the page has other servers than the list does.
    pub fn changed(&self) -> bool {
        self.read && self.servers() != self.edited
    }

    /// What a note that says `text` says of the files it links to that
    /// were sent from here, in the order it links to them.
    pub fn said_in(&self, text: &str) -> Vec<Tag> {
        let mut linked: Vec<(usize, &Tag)> =
            self.said.iter().filter_map(|(url, tag)| Some((text.find(url.as_str())?, tag))).collect();
        linked.sort_by_key(|(at, _)| *at);
        linked.into_iter().map(|(_, tag)| tag.clone()).collect()
    }
}

/// How far the account's bookmarks are known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BookmarkState {
    /// Not asked yet, or not answered.
    #[default]
    Unknown,
    /// As the account's newest bookmark list has them. Boxed: a list is
    /// many times the size of having none.
    Known(Box<Bookmarks>),
    /// The list keeps something to itself that could not be read: why, in
    /// the user's language. Nothing is bookmarked until it can be, since a
    /// list written without it would lose it.
    Unreadable(String),
}

/// How far finding out whether a name is someone's has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip05State {
    /// The domain is being asked.
    Checking,
    /// The domain says the name is theirs.
    Verified,
    /// The domain didn't say so: it says otherwise, or didn't answer.
    Unverified,
}

/// The name at a domain someone says is theirs (NIP-05), and whether it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip05Check {
    /// The name, as their profile gave it when the domain was asked.
    pub name: String,
    pub state: Nip05State,
}

/// Something opened over the feed. Pages stack up, each opened from the one
/// before, and each keeps what it shows and its place.
pub enum Page {
    /// A note's conversation. Boxed, as the others are: what a list of
    /// notes keeps of its rows is more than a page should weigh in a stack.
    Thread(Box<ThreadState>),
    /// Someone's profile, and their notes. Boxed: a list of notes, with
    /// the ones that wait for it, is several times the size of a thread.
    Profile(Box<ProfileState>),
    /// The notes the account has bookmarked. Boxed, as a profile is.
    Bookmarks(Box<BookmarksPage>),
    /// A search for people and notes. Boxed, as a profile is.
    Search(Box<SearchPage>),
    /// What the account is told of. Boxed, as a profile is.
    Notifications(Box<NotificationsPage>),
}

impl Page {
    /// How far the page's list is scrolled.
    pub fn position(&self) -> f32 {
        match self {
            Self::Thread(state) => state.position.get(),
            Self::Profile(state) => state.notes.position.get(),
            Self::Bookmarks(page) => page.notes.position.get(),
            Self::Search(page) => page.position().get(),
            Self::Notifications(page) => page.position.get(),
        }
    }
}

/// A note, a reply or a quote being written.
pub struct Composer {
    /// What it is written to.
    pub target: Target,
    /// The note replied to or quoted, as the composer shows it above the
    /// text.
    pub about: Option<Note>,
    /// The text, and where in it the cursor is.
    pub content: text_editor::Content,
    /// It is being signed and sent.
    pub publishing: bool,
    /// Why it did not go out, the last time it was tried.
    pub error: Option<String>,
    /// Whether what was written is kept as a draft is being asked:
    /// Backspace was pressed to leave it, with something written. The
    /// text is not written to while it is.
    pub leaving: bool,
    /// When the text was last changed, or Backspace last had nothing
    /// left to rub out: a Backspace held down to rub out all of it is
    /// not one that leaves.
    pub rubbed: Option<Instant>,
    /// What is written as it would show once it is out, while that is
    /// looked at in place of the text. The text is not written to
    /// meanwhile.
    pub preview: Option<Preview>,
}

/// What is being written, as a row would show it if it went out as it is.
pub struct Preview {
    /// It, as a note: one with no id of its own yet.
    pub note: Note,
    /// Nothing, ever: a row is built with the notes its list shows in
    /// full, and this is no list. What is written shows whole as it is.
    pub expanded: Expanded,
}

impl Preview {
    /// The preview of `note`.
    pub fn of(note: Note) -> Self {
        Self { note, expanded: Expanded::default() }
    }
}

/// How far the zap of a note has got.
pub enum ZapStage {
    /// How much, and what to say, is being chosen.
    Choosing,
    /// The author's Lightning address is being asked for an invoice.
    Requesting,
    /// There is an invoice to pay.
    Invoice {
        invoice: Invoice,
        /// The invoice as a QR code. `None` if it is too long for one, which
        /// leaves copying it.
        qr: Option<image::Handle>,
        /// The invoice was copied to the clipboard.
        copied: bool,
        /// The wait for the receipt that says it was paid. Dropping it ends
        /// the wait; `None` when there is nothing to wait for, or no more
        /// time.
        waiting: Option<task::Handle>,
    },
    /// The invoice was paid: the receipt said so.
    Paid { sats: u64 },
}

/// A note being zapped.
pub struct ZapState {
    /// The note.
    pub note: EventId,
    /// Its author, who is paid.
    pub recipient: PublicKey,
    /// The author's Lightning address.
    pub address: String,
    /// How much, in sats, as typed: digits only.
    pub amount: String,
    /// What is said with it.
    pub comment: String,
    pub stage: ZapStage,
    /// Why the last request for an invoice failed.
    pub error: Option<String>,
}

impl ZapState {
    /// The amount typed, in sats, if it is one that can be paid.
    pub fn sats(&self) -> Option<u64> {
        self.amount.parse().ok().filter(|sats| *sats > 0)
    }
}

/// How many emoji are offered first to react with, beside the like and
/// the way to all the others.
pub(crate) const REACTIONS_OFFERED: usize = 8;

/// A way to pass a note on: what its repost action offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passing {
    /// Repost it as it is.
    Repost,
    /// Quote it in a note of one's own.
    Quote,
}

impl Passing {
    /// The ways there are to pass on a note, in the order they are
    /// offered: one that the account has reposted (`reposted`) is not
    /// reposted again.
    pub fn open_to(reposted: bool) -> &'static [Passing] {
        if reposted { &[Passing::Quote] } else { &[Passing::Repost, Passing::Quote] }
    }
}

/// The ways to pass a note on, on offer under its repost action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepostMenu {
    /// The note it is for.
    pub note: EventId,
    /// Which of the ways the keyboard is at, once the keyboard was used
    /// on the menu: it is shown as one the pointer is over, and Enter
    /// takes it.
    pub at: Option<Passing>,
}

impl RepostMenu {
    /// The menu for `note`, as a click opens it: the keyboard at nothing.
    pub fn of(note: EventId) -> Self {
        Self { note, at: None }
    }
}

/// How many of those who left something on a note are shown, at the
/// most: more than anyone reads through, and few enough to ask after.
pub const WHO_SHOWN: usize = 100;

/// Who left something on a note, on show under the count of them: who
/// reposted it, who reacted to it, or who zapped it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoMenu {
    /// The note it is for.
    pub note: EventId,
    /// Which of its counts.
    pub what: Left,
    /// Who they are, once the store has been read.
    pub people: Option<Vec<Doer>>,
}

/// What to react to a note with, on offer under its like: the emoji used
/// most, or, once more were asked for, all there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionMenu {
    /// The note it is for.
    pub note: EventId,
    /// Every emoji there is, to pick from, once asked for.
    pub picker: Option<EmojiPicker>,
}

/// Every emoji there is, to pick one from: a group of them at a time, or
/// those found by what is typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiPicker {
    /// What is typed to find an emoji by.
    pub typed: String,
    /// The group on show while nothing is typed.
    pub group: Group,
}

/// The notes whose rows show all they say, where a long note is cut until
/// its reader asks for the rest: each with how far the top of its row is
/// above the top of what is in view, which the row keeps as it is shown
/// (see [`ScrollPosition`](crate::views::components::ScrollPosition)).
/// Showing less of a note brings its row back into view by that much.
///
/// Each list has its own: a note shown in full in the feed is cut on its
/// author's profile until it is asked for the rest there. A row that
/// grew in a list that is not on show would move what that list shows
/// when it is come back to.
#[derive(Debug, Default)]
pub struct Expanded {
    notes: HashMap<EventId, Cell<f32>>,
}

impl Expanded {
    /// Whether the note `id` shows in full.
    pub fn has(&self, id: &EventId) -> bool {
        self.notes.contains_key(id)
    }

    /// Where the row of the note `id` keeps how far its top is above what
    /// is in view, if the note shows in full.
    pub fn above(&self, id: &EventId) -> Option<&Cell<f32>> {
        self.notes.get(id)
    }

    /// Show the note `id` in full from now on.
    pub fn expand(&mut self, id: EventId) {
        self.notes.entry(id).or_default();
    }

    /// Show the note `id` cut again. How far the top of its row was above
    /// what is in view, if it showed in full: nothing, if its top was in
    /// view.
    pub fn collapse(&mut self, id: &EventId) -> Option<f32> {
        self.notes.remove(id).map(|above| above.get())
    }
}

/// What the account is told of, and how far finding it out has got.
/// The model's, not a page's: it is kept up while no page shows it, for
/// the count of what is new.
#[derive(Debug, Default)]
pub struct NotificationsState {
    /// What others did that concerns the account.
    pub list: Notifications,
    /// The relays are being asked for the newest.
    pub loading: bool,
    /// They are being asked for the page before the oldest.
    pub loading_older: bool,
    /// They answered once: from then on they are asked only for what is
    /// newer, or older, than what is known.
    pub settled: bool,
    /// They could not be asked, the one time they were: an empty list is
    /// not known to be empty.
    pub failed: bool,
    /// When the oldest event the relays sent was written: where the page
    /// before it ends.
    pub oldest: Option<u64>,
    /// The relays had nothing older.
    pub end_reached: bool,
    /// The account's Lightning address was asked, or is being asked,
    /// which key signs its zap receipts.
    pub zapper_asked: bool,
    /// The notifications are being looked at: their page is on show, in
    /// an open popup.
    pub in_view: bool,
    /// When they were last looked at before the visit under way: what was
    /// written after is picked out for as long as the visit lasts.
    pub seen_before: u64,
}

/// The page that shows what the account is told of: what its list keeps
/// of its rows. What it shows is the model's (see [`NotificationsState`]).
#[derive(Default)]
pub struct NotificationsPage {
    /// How tall each row measured, by the event it tells of.
    pub heights: RowHeights<EventId>,
    /// Which of the long notes among them show in full.
    pub expanded: Expanded,
    /// How far the list is scrolled, kept by the view.
    pub position: Cell<f32>,
    /// The row the keyboard was last at, if it was at any: the one
    /// marked (see [`crate::handlers::keys`]).
    pub marked: Option<EventId>,
}

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    pub core: cosmic::Core,
    /// Configuration data that persists between application runs.
    pub config: Config,
    /// The popup id.
    pub popup: Option<Id>,
    /// The surface that is there for as long as the window runs, and shows
    /// nothing: what is copied is kept for as long as it is (see
    /// `keep_what_is_copied`).
    pub keeper: Id,
    /// When the popup last closed because something outside it was
    /// clicked: a press of the panel button is such a click, and is heard
    /// of a moment later.
    pub popup_unfocused_at: Option<Instant>,
    /// What the button in the panel was last told: whether there is
    /// something the account has not looked at.
    pub unread_told: bool,
    /// Which view the popup shows.
    pub view_state: ViewState,
    /// The screen the popup opens on: how tall it is, which the popup's
    /// own height is a share of, and how many of its pixels a point is,
    /// which pictures are kept by. `None` until that is known.
    pub screen: Option<Screen>,

    /// The relay client and event store, once the store is open.
    pub nostr: Option<NostrService>,

    /// The signed-in account. `None` when signed out, and at startup until
    /// the account the config names has been looked up.
    pub account: Option<Account>,
    /// What is typed in the sign-in field. May be a secret key: it is cleared
    /// as soon as it has been used.
    pub login_input: String,
    /// Whether the sign-in field hides what is typed.
    pub login_input_hidden: bool,
    /// Why the last sign-in attempt was refused.
    pub login_error: Option<CredentialError>,
    /// The last sign-in with a bunker address was turned down. Shown under
    /// the field until what is in the field changes.
    pub login_refusal: Option<BunkerRefusal>,
    /// How far the sign-in under way has got.
    pub login_progress: LoginProgress,
    /// What signs for the account: its key from the keyring, or its remote
    /// signer.
    pub signer: Signer,

    /// The feed: notes from the people the account follows.
    pub following: FeedState,
    /// Who the account follows, by its contact list. `None` until that is
    /// known; empty when it follows nobody.
    pub follows: Option<Vec<PublicKey>>,
    /// How many lists of notes there have been: the next one's
    /// [`FeedState::id`] comes after.
    pub feeds_made: u64,
    /// When the feed was last looked at for new notes.
    pub feed_checked_at: Option<Instant>,
    /// The choice between the ways a list of notes is read, as the control
    /// that offers it keeps it: each choice carries its
    /// [`FeedMode`](crate::nostr::feed::FeedMode), and
    /// the one chosen is [`Config::feed_mode`].
    pub feed_modes: segmented_button::SingleSelectModel,
    /// What the account is told of.
    pub notifications: NotificationsState,
    /// Which of it shows: everything, or one kind.
    pub notification_tabs: segmented_button::SingleSelectModel,

    /// What was opened over the feed, one page from another, and not closed
    /// yet. The last is the one on show, and Back closes it.
    pub pages: Vec<Page>,

    /// The note or reply being written, while the composer is open.
    pub composer: Option<Composer>,
    /// The choice between writing it and looking at how it would show, as
    /// the control that offers it keeps it: each choice carries whether it
    /// is the look, and the one chosen is whether [`Composer::preview`] is
    /// there.
    pub compose_tabs: segmented_button::SingleSelectModel,
    /// What was written and left unpublished, by what it was written to.
    pub drafts: Drafts,
    /// Where the draft of a new note is kept between runs.
    pub draft_path: PathBuf,
    /// The drafts as the relay of this machine has them, for those it
    /// has: what is sent there is what differs from this.
    pub drafts_kept: HashMap<Target, String>,
    /// Whether that relay was asked for the account's drafts.
    pub drafts_asked: bool,

    /// The note being zapped, while the zap view is open.
    pub zap: Option<ZapState>,

    /// What was made of the notes seen: replies, reposts, likes and zaps.
    pub engagement: HashMap<EventId, Engagement>,
    /// When each note's engagement was last asked for, so that it isn't
    /// asked again right away.
    pub engagement_asked: HashMap<EventId, Instant>,
    /// The relays each note was seen on, for the notes seen on any.
    pub seen_on: HashMap<EventId, Vec<RelayUrl>>,
    /// The likes and reposts that are on their way to the relays, by note.
    pub pending_marks: HashSet<(EventId, Mark)>,
    /// The repost menu that is open, if one is: repost a note, or quote it.
    pub repost_menu: Option<RepostMenu>,
    /// What to react with is on offer, for one note.
    pub reaction_menu: Option<ReactionMenu>,
    /// Who left something on a note is on show, for one of its counts.
    pub who_menu: Option<WhoMenu>,
    /// The emoji offered first to react with: the ones used most (see
    /// [`crate::emoji::suggested`]), as the settings have them now.
    pub reaction_suggestions: Vec<String>,
    /// The note a share link to which was copied last, while the popup
    /// that was done in is open: its row says so.
    pub share_copied: Option<EventId>,
    /// The picture that was copied just now, if one was: where it is.
    pub picture_copied: Option<String>,

    /// What the account has bookmarked.
    pub bookmarks: BookmarkState,
    /// When that was last asked for.
    pub bookmarks_asked_at: Option<Instant>,
    /// The note being bookmarked (`true`) or having its bookmark taken
    /// off, while the changed bookmark list is on its way. One at a time:
    /// each change is made to the list the last one left.
    pub bookmark_pending: Option<(EventId, bool)>,
    /// The relays had no bookmark list of the account's when a note was
    /// last bookmarked, and the account was told: bookmarking again writes
    /// the first.
    pub bookmark_first_list: bool,
    /// When the event store was last told to let go of what is old.
    pub swept_at: Option<Instant>,
    /// The menu behind the cog above the feed is open.
    pub menu_open: bool,
    /// The account's relays, as the relays page has them.
    pub relays: RelaysState,
    /// The account's media servers, as their page has them, and the files
    /// on their way to them.
    pub media: MediaState,

    /// What is known about note authors.
    pub profiles: HashMap<PublicKey, Profile>,
    /// The authors whose profile has been asked for, found or not, so that
    /// nobody is asked about twice.
    pub profiles_requested: HashSet<PublicKey>,
    /// Whether the names people give as theirs are, for those whose
    /// profile was opened: nobody else's domain is asked.
    pub nip05: HashMap<PublicKey, Nip05Check>,
    /// The person being followed or unfollowed, while the changed contact
    /// list is on its way. One at a time: each change is made to the list
    /// the last one left.
    pub follow_pending: Option<PublicKey>,

    /// Avatars and the images in notes, as far as they have loaded.
    pub images: ImageStore,
    /// The pictures views have asked `images` for, waiting to be turned into
    /// messages by a subscription.
    pub image_requests: Inbox<ImageRequest>,
    /// What the web pages notes link to say of themselves, as far as they
    /// were asked.
    pub previews: PreviewStore,
    /// The pages views have asked `previews` about, waiting likewise.
    pub preview_requests: Inbox<String>,
    /// The video that plays, or did last, in the row of the note that
    /// links to it.
    pub videos: Videos,
    /// The HTTP client everything from the web is fetched with.
    pub http: reqwest::Client,

    /// Something went wrong that the user should know about.
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Expanded;
    use nostr_sdk::prelude::EventId;

    #[test]
    fn a_note_shows_in_full_from_when_it_is_asked_to_until_it_is_asked_not_to() {
        let (one, other) = (EventId::from_byte_array([1; 32]), EventId::from_byte_array([2; 32]));
        let mut expanded = Expanded::default();
        assert!(!expanded.has(&one) && expanded.above(&one).is_none());
        assert_eq!(expanded.collapse(&one), None, "it was not in full");

        expanded.expand(one);
        assert!(expanded.has(&one) && !expanded.has(&other));
        // Its row says how far its top is above what is in view, and
        // asking for it in full again forgets none of that.
        expanded.above(&one).unwrap().set(240.0);
        expanded.expand(one);
        assert_eq!(expanded.collapse(&one), Some(240.0));
        assert!(!expanded.has(&one));
    }

    #[test]
    fn a_note_says_of_the_files_it_links_to_in_the_order_it_links_to_them_and_of_no_others() {
        use super::MediaState;
        use nostr_sdk::prelude::Tag;

        let said = |url: &str| Tag::parse(["imeta", &format!("url {url}")]).unwrap();
        let (first, second, gone) =
            ("https://cdn.example.com/a.jpg", "https://cdn.example.com/b.png", "https://cdn.example.com/c.mp4");
        let mut media = MediaState::default();
        for url in [first, second, gone] {
            media.said.insert(url.to_string(), said(url));
        }

        let text = format!("look {second}\nand {first}");
        assert_eq!(media.said_in(&text), [said(second), said(first)]);
        assert!(media.said_in("nothing attached").is_empty(), "an address taken out of the text takes its file out of the note");
    }
}

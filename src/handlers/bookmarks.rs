// SPDX-License-Identifier: MIT

//! Bookmark message handlers for Avestruz: what the account has bookmarked,
//! and bookmarking a note or taking its bookmark off.
//!
//! The bookmarks are the account's bookmark list (see
//! [`crate::nostr::bookmarks`]), and part of it is encrypted: reading it
//! takes the account's key. They are asked for when the popup opens, not
//! when the applet starts, so that a remote signer is not reached for, nor
//! whoever keeps it asked anything, before the account is looked at; and
//! again each time it opens after a while, for what was bookmarked
//! elsewhere since. The store answers first and the relays after. A list
//! that was read once is not decrypted again.
//!
//! A bookmark shows at once and goes out behind it: the newest list, asked
//! of the relays just then, written again with that one change. One at a
//! time, since each change is made to the list the last one left. If it
//! does not go out, the note shows as it was, with the reason.
//!
//! An account whose relays have no list of its own is told so the first
//! time, and nothing is written: a first list replaces any list kept on
//! relays that are not asked here. Bookmarking again writes it.
//!
//! The bookmarks page shows the notes bookmarked, and looks them up by id
//! a page at a time, in the store and then on the relays, as its list is
//! scrolled. Which notes it shows is taken from the bookmarks when it
//! opens, and again when they turn out to be others than they were, if
//! its list is at its top: further down it is being read, and stays as it
//! is. A bookmark put on or taken off here changes nothing on a page that
//! is open, for the same reason.

use std::time::{Duration, Instant};

use cosmic::prelude::*;
use nostr_sdk::prelude::{EventId, PublicKey};

use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::handlers::compose::publish_error_text;
use crate::messages::Message;
use crate::nostr::bookmarks::Bookmarks;
use crate::nostr::client::{FetchError, ListError, Source};
use crate::nostr::models::Note;
use crate::nostr::outbox::Whereabouts;
use crate::state::{AppModel, BookmarkState, BookmarkedNotes, BookmarksPage, Page};

/// How long the bookmarks are taken as known, before the popup opening
/// has them asked for again.
const BOOKMARKS_FRESH: Duration = Duration::from_secs(60);

/// How many bookmarked notes the bookmarks page looks up at a time.
const BOOKMARKS_PAGE: usize = 20;

impl AppModel {
    /// Forget the bookmarks: the account changed, and they were its own.
    pub(crate) fn reset_bookmarks(&mut self) {
        self.bookmarks = BookmarkState::Unknown;
        self.bookmarks_asked_at = None;
        self.bookmark_pending = None;
        self.bookmark_first_list = false;
    }

    /// The account that keeps bookmarks: the one signed in, if it has its
    /// key.
    fn bookmarking_account(&self) -> Option<PublicKey> {
        self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key)
    }

    /// Find out what the account has bookmarked, while the popup is open
    /// and unless that was asked a moment ago. Called whenever the popup
    /// opens, and when the account or the store arrive.
    pub(crate) fn load_bookmarks(&mut self) -> Task<cosmic::Action<Message>> {
        if self.popup.is_none() || self.bookmark_pending.is_some() {
            return Task::none();
        }
        let now = Instant::now();
        if self.bookmarks_asked_at.is_some_and(|at| now.duration_since(at) < BOOKMARKS_FRESH) {
            return Task::none();
        }
        let asked = self.ask_bookmarks(Source::Store);
        if asked.is_some() {
            self.bookmarks_asked_at = Some(now);
        }
        asked.unwrap_or_else(Task::none)
    }

    /// Ask `source` what the account has bookmarked, if there is an
    /// account that keeps bookmarks, and a store.
    fn ask_bookmarks(&self, source: Source) -> Option<Task<cosmic::Action<Message>>> {
        let (service, account) = (self.nostr.clone()?, self.bookmarking_account()?);
        let signer = self.signer.clone();
        let known = self.known_bookmarks().cloned();
        Some(Task::perform(async move { service.bookmarks(source, &signer, account, known).await }, move |result| {
            cosmic::Action::App(Message::BookmarksLoaded { account, source, result })
        }))
    }

    /// The bookmarks, if they are known.
    fn known_bookmarks(&self) -> Option<&Bookmarks> {
        match &self.bookmarks {
            BookmarkState::Known(bookmarks) => Some(bookmarks),
            BookmarkState::Unknown | BookmarkState::Unreadable(_) => None,
        }
    }

    /// The account's bookmarks are `bookmarks`, unless what is known is
    /// from a newer list: answers come from the store and from the relays,
    /// in no order that can be counted on. An open bookmarks page takes
    /// its notes from them.
    fn take_bookmarks(&mut self, bookmarks: Bookmarks) -> Task<cosmic::Action<Message>> {
        let written = |bookmarks: &Bookmarks| bookmarks.list().map(|list| list.created_at);
        if self.known_bookmarks().is_some_and(|known| written(known) > written(&bookmarks)) {
            return Task::none();
        }
        if bookmarks.list().is_some() {
            self.bookmark_first_list = false;
        }
        self.bookmarks = BookmarkState::Known(Box::new(bookmarks));
        // What stays in the event store is known now.
        Task::batch([self.fill_bookmarks_pages(), self.sweep_store()])
    }

    /// What the account has bookmarked came from the store or from the
    /// relays, or didn't. After the store, the relays are asked.
    pub fn handle_bookmarks_loaded(
        &mut self,
        account: PublicKey,
        source: Source,
        result: Result<Bookmarks, ListError>,
    ) -> Task<cosmic::Action<Message>> {
        // Whoever signed out meanwhile took their bookmarks with them.
        if self.bookmarking_account() != Some(account) {
            return Task::none();
        }
        let taken = match result {
            // A change on its way is made to the newest list, and says
            // what the bookmarks are when it is done.
            Ok(_) if self.bookmark_pending.is_some() => Task::none(),
            Ok(bookmarks) => {
                tracing::debug!("{} note(s) bookmarked, by {source:?}", bookmarks.count());
                self.take_bookmarks(bookmarks)
            }
            Err(ListError::Unreadable(e)) => {
                tracing::error!("the bookmarks could not be read: {e:?}");
                self.bookmarks = BookmarkState::Unreadable(sign_error_text(e));
                Task::none()
            }
            Err(e) => {
                tracing::warn!("the bookmarks could not be asked of {source:?}: {e:?}");
                Task::none()
            }
        };
        let asked = match source {
            Source::Store => self.ask_bookmarks(Source::Relays).unwrap_or_else(Task::none),
            Source::Relays => Task::none(),
        };
        Task::batch([taken, asked])
    }

    // =========================================================================
    // The bookmarks page
    // =========================================================================

    /// The notes of the bookmarks page `id`, if that page is still open.
    fn bookmarked_notes_mut(&mut self, id: u64) -> Option<&mut BookmarkedNotes> {
        self.pages.iter_mut().find_map(|page| match page {
            Page::Bookmarks(page) if page.id == id => Some(&mut page.notes),
            _ => None,
        })
    }

    /// Have every open bookmarks page take its notes from the bookmarks,
    /// which are known now or are others than they were, if it was never
    /// filled or is at its top. Then look up the first of them.
    fn fill_bookmarks_pages(&mut self) -> Task<cosmic::Action<Message>> {
        let Self { bookmarks: BookmarkState::Known(bookmarks), pages, .. } = self else {
            return Task::none();
        };
        let mut to_load = Vec::new();
        for page in pages.iter_mut() {
            let Page::Bookmarks(page) = page else { continue };
            if page.notes.taken == 0 || page.notes.position.get() == 0.0 {
                page.notes.take(bookmarks);
            }
            if page.notes.asked == 0 {
                to_load.push(page.id);
            }
        }
        Task::batch(to_load.into_iter().map(|page| self.handle_load_bookmarked_notes(page)))
    }

    /// Show the notes the account has bookmarked, over whatever is on show
    /// now. The bookmarks are asked for again if that was a while ago.
    pub fn handle_open_bookmarks(&mut self) -> Task<cosmic::Action<Message>> {
        self.menu_open = false;
        if self.bookmarking_account().is_none() {
            return Task::none();
        }
        self.feeds_made += 1;
        let page = BookmarksPage::new(self.feeds_made);
        let opened = self.open_page(Page::Bookmarks(Box::new(page)));
        Task::batch([opened, self.fill_bookmarks_pages(), self.load_bookmarks()])
    }

    /// Look up the next notes of the bookmarks page `page`, unless they
    /// are being looked up or there are no more.
    pub fn handle_load_bookmarked_notes(&mut self, page: u64) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        let Some(listed) = self.bookmarked_notes_mut(page) else {
            return Task::none();
        };
        if listed.loading || listed.all_asked() {
            return Task::none();
        }
        let from = listed.asked;
        let ids: Vec<EventId> = listed.ids.iter().skip(from).take(BOOKMARKS_PAGE).copied().collect();
        let (taken, to) = (listed.taken, from + ids.len());
        listed.loading = true;
        // With where the list says each is, for those the account's own
        // relays don't have.
        let said = |id: &EventId| match &self.bookmarks {
            BookmarkState::Known(bookmarks) => bookmarks.whereabouts(id),
            _ => Whereabouts::default(),
        };
        let listed: Vec<(EventId, Whereabouts)> = ids.into_iter().map(|id| (id, said(&id))).collect();
        Task::perform(async move { service.notes_listed(listed).await }, move |notes| {
            cosmic::Action::App(Message::BookmarkedNotesLoaded { page, taken, from, to, notes })
        })
    }

    /// The notes bookmarked `from`th to `to`th were looked up for the page
    /// `page`, as it had them the `taken`th time: `notes` are those that
    /// were found, which go under the ones already there. A page that
    /// brought none is followed by the next at once: nothing was added to
    /// the list that scrolling could reach the end of.
    pub fn handle_bookmarked_notes_loaded(
        &mut self,
        page: u64,
        taken: u64,
        from: usize,
        to: usize,
        notes: Vec<Note>,
    ) -> Task<cosmic::Action<Message>> {
        let Some(listed) = self.bookmarked_notes_mut(page) else {
            return Task::none();
        };
        // Asked of a list that was taken anew since.
        if listed.taken != taken || listed.asked != from {
            return Task::none();
        }
        listed.loading = false;
        listed.asked = to;
        listed.missing += to.saturating_sub(from).saturating_sub(notes.len());
        tracing::debug!("bookmarks: {} of {} note(s) found", notes.len(), to.saturating_sub(from));
        let people: Vec<PublicKey> = notes.iter().flat_map(Note::people).collect();
        let ids: Vec<EventId> = notes.iter().map(|note| note.id).collect();
        listed.notes.extend(notes);
        let next = if ids.is_empty() { self.handle_load_bookmarked_notes(page) } else { Task::none() };
        Task::batch([self.look_up_people(people), self.look_up_engagement(ids), next])
    }

    /// Bookmark the note `id`, or take its bookmark off if it has one.
    /// Not while another change is on its way, nor before the bookmarks
    /// are known; and if they could not be read, that is said again.
    pub fn handle_bookmark(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        self.repost_menu = None;
        let (Some(service), Some(account)) = (self.nostr.clone(), self.bookmarking_account()) else {
            return Task::none();
        };
        let known = match &self.bookmarks {
            BookmarkState::Known(bookmarks) => bookmarks.as_ref().clone(),
            BookmarkState::Unreadable(why) => {
                self.error_message = Some(fl!("bookmark-unreadable", error = why.as_str()));
                return Task::none();
            }
            BookmarkState::Unknown => return Task::none(),
        };
        if self.bookmark_pending.is_some() {
            return Task::none();
        }
        let bookmark = !known.has(&id);
        self.bookmark_pending = Some((id, bookmark));
        let (signer, first_list) = (self.signer.clone(), self.bookmark_first_list);
        Task::perform(
            async move { service.set_bookmarked(&signer, account, id, bookmark, first_list, Some(known)).await },
            move |result| cosmic::Action::App(Message::Bookmarked { account, note: id, result }),
        )
    }

    /// The bookmark list of `account` was changed for the note `id`, or
    /// wasn't.
    pub fn handle_bookmarked(&mut self, account: PublicKey, id: EventId, result: Result<Bookmarks, ListError>) {
        if self.bookmark_pending.is_some_and(|(pending, _)| pending == id) {
            self.bookmark_pending = None;
        }
        // Whoever signed out meanwhile took their bookmarks with them.
        if self.bookmarking_account() != Some(account) {
            return;
        }
        self.error_message = match result {
            Ok(bookmarks) => {
                tracing::info!("the bookmark list changed: {} note(s) bookmarked", bookmarks.count());
                // What said there was no list has been answered.
                let answered = std::mem::take(&mut self.bookmark_first_list);
                self.bookmarks = BookmarkState::Known(Box::new(bookmarks));
                if answered { None } else { self.error_message.take() }
            }
            Err(ListError::NoList) => {
                self.bookmark_first_list = true;
                Some(fl!("bookmark-no-list"))
            }
            Err(ListError::Fetch(e)) => {
                tracing::error!("the bookmark list could not be asked for: {e:?}");
                let why = match e {
                    FetchError::NoRelayReachable => fl!("bookmark-unreachable"),
                    FetchError::Incomplete => fl!("error-relays-incomplete"),
                    FetchError::Failed(error) => error,
                };
                Some(fl!("bookmark-error", error = why))
            }
            Err(ListError::Unreadable(e)) => {
                tracing::error!("the bookmarks could not be read: {e:?}");
                let why = sign_error_text(e);
                self.bookmarks = BookmarkState::Unreadable(why.clone());
                Some(fl!("bookmark-unreadable", error = why))
            }
            Err(ListError::Publish(e)) => {
                tracing::error!("the bookmark list did not go out: {e:?}");
                Some(fl!("bookmark-error", error = publish_error_text(e)))
            }
        };
    }
}

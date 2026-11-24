// SPDX-License-Identifier: MIT

//! Feed message handlers for Avestruz.
//!
//! The feed is the notes of the people the account follows. A profile has a
//! list of its own, the notes of one person, and both are kept and filled
//! the same way, as a [`FeedState`]. What the event store already holds of a
//! list shows at once; the relays are then asked for the newest page, and
//! for an older page each time the list is scrolled near its end. Nothing
//! is fetched unasked after that: Refresh pulls the feed's latest.
//!
//! What a list shows is its reader's: it only changes when they say so.
//! The feed is looked at again every so often, and notes that turned up
//! since wait beside it ([`Pending`](crate::nostr::feed::Pending)), offered
//! by a button over the list; pressing it, or Refresh, shows them and goes
//! to the top. Until then nothing is put in above what is being read, and
//! nothing moves.
//!
//! A list is read with replies or without ([`FeedMode`]), by a choice made
//! above it and kept in the config. Read without, a page may bring nothing
//! to show: the list then looks further back on its own, a few pages at
//! most, and after that when asked.
//!
//! Every list has an id of its own, and whatever is asked for it comes back
//! with that id. An answer whose list is gone by then is dropped: a profile
//! that was closed, a feed that started over.
//!
//! Who the account follows is its contact list (kind 3), read from the store
//! first and then asked of the relays. Whenever that answer changes — another
//! account, or a newer list — the feed starts over, as a new list.

use std::time::{Duration, Instant};

use cosmic::prelude::*;
use cosmic::widget::segmented_button;
use nostr_sdk::prelude::{EventId, PublicKey};

use crate::fl;
use crate::handlers::navigation::scroll_list_to;
use crate::handlers::relays::ALWAYS_READ;
use crate::messages::Message;
use crate::nostr::client::{FEED_PAGE, FetchError, NostrService, Source};
use crate::nostr::feed::{Feed, FeedMode, FeedPage};
use crate::nostr::models::{Note, Profile};
use crate::state::{AppModel, FeedState, Page, ViewState};
use crate::views::components::RowHeights;

/// How long after the feed was looked at for new notes it is not looked at
/// again for the popup opening.
const NEW_NOTES_FRESH: Duration = Duration::from_secs(30);

/// Ask the relays for the profiles of `people`: the account's own, and,
/// for those these don't have, the relays those people write to, which
/// take longer. Each answers when it has one.
pub(crate) fn fetch_profiles(service: NostrService, people: Vec<PublicKey>) -> Task<cosmic::Action<Message>> {
    let arrived = |found| cosmic::Action::App(Message::ProfilesFetched(found));
    let own = {
        let (service, people) = (service.clone(), people.clone());
        Task::perform(async move { service.fetch_profiles(people).await }, arrived)
    };
    let theirs = Task::perform(async move { service.fetch_profiles_where_written(people).await }, arrived);
    Task::batch([own, theirs])
}

impl AppModel {
    /// A new, empty list of notes, with an id no list had before.
    pub(crate) fn new_feed(&mut self) -> FeedState {
        self.feeds_made += 1;
        FeedState::new(self.feeds_made)
    }

    /// The list of notes `id`, if it is still there: the feed, or the notes
    /// of an open profile.
    fn feed_mut(&mut self, id: u64) -> Option<&mut FeedState> {
        if self.following.id == id {
            return Some(&mut self.following);
        }
        self.pages.iter_mut().find_map(|page| match page {
            Page::Profile(profile) if profile.notes.id == id => Some(&mut profile.notes),
            _ => None,
        })
    }

    /// Whose notes the list `id` is made of. `None` if the list is gone,
    /// and for the feed while who is followed isn't known.
    fn feed_authors(&self, id: u64) -> Option<Vec<PublicKey>> {
        if self.following.id == id {
            return self.follows.clone();
        }
        self.pages.iter().find_map(|page| match page {
            Page::Profile(profile) if profile.notes.id == id => Some(vec![profile.public_key]),
            _ => None,
        })
    }

    /// The event store opened (or didn't). Find out about the account, if
    /// someone is signed in already: who they follow fills the feed.
    pub fn handle_nostr_ready(&mut self, result: Result<NostrService, String>) -> Task<cosmic::Action<Message>> {
        match result {
            Ok(service) => {
                self.nostr = Some(service);
                self.load_account()
            }
            Err(e) => {
                tracing::error!("event store could not be opened: {e}");
                self.error_message = Some(fl!("error-storage", error = e));
                Task::none()
            }
        }
    }

    /// Fill the list `id`: what the store holds of it now, and the newest
    /// page from the relays after.
    pub(crate) fn load_feed(&mut self, id: u64) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(authors)) = (self.nostr.clone(), self.feed_authors(id)) else {
            return Task::none();
        };
        let cached = Task::perform(async move { service.cached_feed(&authors, FEED_PAGE).await }, move |result| {
            cosmic::Action::App(Message::FeedCached(id, result))
        });
        Task::batch([cached, self.fetch_feed(id, FeedPage::Newest)])
    }

    /// Ask the relays for a page of the list `id`, unless that page is
    /// already on its way (or, for an older page, there is nothing older).
    pub(crate) fn fetch_feed(&mut self, id: u64, page: FeedPage) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(authors)) = (self.nostr.clone(), self.feed_authors(id)) else {
            return Task::none();
        };
        let Some(state) = self.feed_mut(id) else {
            return Task::none();
        };
        let until = match page {
            FeedPage::Newest => {
                if state.refreshing {
                    return Task::none();
                }
                state.refreshing = true;
                None
            }
            FeedPage::Older => {
                if state.loading_older || state.end_reached {
                    return Task::none();
                }
                let Some(oldest) = state.feed.oldest() else {
                    return Task::none();
                };
                state.loading_older = true;
                // `until` includes its own second, so notes sharing the
                // oldest note's second come again and are skipped as known.
                Some(oldest)
            }
        };
        Task::perform(async move { service.fetch_feed(&authors, until, FEED_PAGE).await }, move |result| {
            cosmic::Action::App(Message::FeedFetched { list: id, page, until, result })
        })
    }

    /// The notes of the list `id` already in the event store were read:
    /// what it shows until the relays answer. Once they have, the list is
    /// what it is, and the store has nothing to add to it, unless they
    /// answered with nothing.
    pub fn handle_feed_cached(&mut self, id: u64, result: Result<Vec<Note>, String>) -> Task<cosmic::Action<Message>> {
        match result {
            Ok(notes) => {
                tracing::info!("list {id}: {} note(s) from the event store", notes.len());
                match self.feed_mut(id) {
                    Some(state) if !state.settled || state.feed.is_empty() => self.merge_notes(id, notes),
                    _ => Task::none(),
                }
            }
            Err(e) => {
                tracing::warn!("list {id} could not be read from the event store: {e}");
                Task::none()
            }
        }
    }

    /// A page of the list `id` arrived from the relays (or didn't): the
    /// newest there is, or the page before the note written at `until`.
    pub fn handle_feed_fetched(
        &mut self,
        id: u64,
        page: FeedPage,
        until: Option<u64>,
        result: Result<Vec<Note>, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        match page {
            FeedPage::Newest => self.handle_newest_fetched(id, result),
            FeedPage::Older => self.handle_older_fetched(id, until, result),
        }
    }

    /// The newest notes of the list `id` arrived from the relays (or
    /// didn't).
    ///
    /// They go into the list itself only while nobody can have been
    /// reading it: when it is empty, or being filled for the first time and
    /// still at its top. After that they wait to be asked for, and what is
    /// on show stays as it is, unless this page was asked for by hand.
    fn handle_newest_fetched(&mut self, id: u64, result: Result<Vec<Note>, FetchError>) -> Task<cosmic::Action<Message>> {
        let mode = self.config.feed_mode;
        let Some(state) = self.feed_mut(id) else {
            return Task::none();
        };
        state.refreshing = false;
        let asked_for = std::mem::take(&mut state.show_when_fetched);
        let first = !std::mem::replace(&mut state.settled, true);
        match result {
            Ok(notes) => {
                let received = notes.len();
                let unread = state.feed.is_empty() || (first && state.position.get() <= 0.0);
                // What the store had to show came without the quoted notes
                // only the relays have: while nobody can have been
                // reading, it is shown with them.
                let mut people = if unread { state.feed.take_quotes_from(&notes) } else { Vec::new() };
                people.extend(state.pending.offer(&state.feed, notes, received >= FEED_PAGE));
                tracing::info!(
                    "list {id}: {received} note(s) from the relays for Newest, {} not shown yet",
                    state.pending.notes(mode).len()
                );
                // The relays answered: they may have older notes again, if
                // they had none before.
                state.end_reached = false;
                let looked_up = self.look_up_people(people);
                let shown = if asked_for || unread { self.show_new_notes(id) } else { Task::none() };
                Task::batch([looked_up, shown, self.look_back_if_nothing_to_show(id)])
            }
            Err(e) => {
                // A look in passing that came to nothing is not worth a
                // banner: the next one is a minute away.
                if asked_for || first {
                    tracing::error!("list {id} could not be fetched for Newest: {e:?}");
                    self.error_message = Some(fetch_error_text(e));
                } else {
                    tracing::warn!("list {id}: no new notes could be looked for: {e:?}");
                }
                Task::none()
            }
        }
    }

    /// The page of the list `id` before the note written at `until`
    /// arrived from the relays (or didn't).
    fn handle_older_fetched(
        &mut self,
        id: u64,
        until: Option<u64>,
        result: Result<Vec<Note>, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        let mode = self.config.feed_mode;
        let Some(state) = self.feed_mut(id) else {
            return Task::none();
        };
        state.loading_older = false;
        // The list started over since this was asked for: the page is from
        // before where it now ends, and would leave a hole.
        if state.feed.oldest() != until {
            return Task::none();
        }
        let (before, shown_before) = (state.feed.len(), state.feed.notes(mode).len());
        match result {
            Ok(notes) => {
                let received = notes.len();
                let task = self.merge_notes(id, notes);
                let Some(state) = self.feed_mut(id) else {
                    return task;
                };
                let added = state.feed.len() - before;
                let shown = state.feed.notes(mode).len() - shown_before;
                tracing::info!("list {id}: {received} note(s) from the relays for Older, {added} new, {shown} to show");
                state.end_reached = added == 0;
                // Nothing to show in the page: whoever scrolled here has
                // nothing new under them to scroll to, so the list goes on
                // by itself, for a while.
                let further = if shown > 0 || added == 0 {
                    state.barren_pages = 0;
                    Task::none()
                } else {
                    state.barren_pages = state.barren_pages.saturating_add(1);
                    if state.paused() { Task::none() } else { self.fetch_feed(id, FeedPage::Older) }
                };
                Task::batch([task, further])
            }
            Err(e) => {
                tracing::error!("list {id} could not be fetched for Older: {e:?}");
                self.error_message = Some(fetch_error_text(e));
                Task::none()
            }
        }
    }

    /// Whether the list `id` is what the popup shows, or would if it were
    /// open.
    fn list_on_show(&self, id: u64) -> bool {
        match self.view_state {
            ViewState::Feed => self.following.id == id,
            ViewState::Page => self.profile_on_top().is_some_and(|profile| profile.notes.id == id),
            _ => false,
        }
    }

    /// Show the notes that wait for the list `id`, which its reader asked
    /// for, and bring the list to its top, where they are.
    ///
    /// Notes that reach down to the list's newest go in above it. Notes
    /// that may not start the list over: it is then those, and what is
    /// older comes back as the list is scrolled to its end.
    pub fn show_new_notes(&mut self, id: u64) -> Task<cosmic::Action<Message>> {
        let Some(state) = self.feed_mut(id) else {
            return Task::none();
        };
        let (notes, detached) = state.pending.take();
        if notes.is_empty() {
            return Task::none();
        }
        if detached {
            state.feed = Feed::default();
            state.heights = RowHeights::default();
            state.end_reached = false;
            state.barren_pages = 0;
        }
        // Where it is read from when it next shows, if it isn't showing.
        state.position.set(0.0);
        let to_top = if self.list_on_show(id) { scroll_list_to(0.0) } else { Task::none() };
        Task::batch([self.merge_notes(id, notes), to_top, self.look_back_if_nothing_to_show(id)])
    }

    /// It is time to look for new notes for the feed. Whatever there is
    /// waits to be asked for.
    pub fn handle_check_for_new_notes(&mut self) -> Task<cosmic::Action<Message>> {
        // Not before the feed was filled once: that is under way, or has
        // nobody to fill it for.
        if !self.following.settled {
            return Task::none();
        }
        self.feed_checked_at = Some(Instant::now());
        // What is new for the account is looked for at the same time.
        Task::batch([self.fetch_feed(self.following.id, FeedPage::Newest), self.check_notifications()])
    }

    /// Look for new notes for the feed now, if that was not done just
    /// now: the popup opened, and whoever opened it is about to look.
    pub(crate) fn check_for_new_notes_if_stale(&mut self) -> Task<cosmic::Action<Message>> {
        match self.feed_checked_at {
            Some(checked) if checked.elapsed() < NEW_NOTES_FRESH => Task::none(),
            _ => self.handle_check_for_new_notes(),
        }
    }

    /// The list `id` was scrolled near its end, or asked to look further
    /// back: the page before its oldest note, however many pages before
    /// brought nothing to show.
    pub fn handle_load_older_notes(&mut self, id: u64) -> Task<cosmic::Action<Message>> {
        if let Some(state) = self.feed_mut(id) {
            state.barren_pages = 0;
        }
        self.fetch_feed(id, FeedPage::Older)
    }

    /// Look further back for the list `id` if it has notes and none of
    /// them to show: nothing is there to scroll to the end of, which is
    /// what otherwise asks for the next page.
    fn look_back_if_nothing_to_show(&mut self, id: u64) -> Task<cosmic::Action<Message>> {
        let mode = self.config.feed_mode;
        match self.feed_mut(id) {
            Some(state) if state.feed.notes(mode).is_empty() && !state.feed.is_empty() && !state.paused() => {
                self.fetch_feed(id, FeedPage::Older)
            }
            _ => Task::none(),
        }
    }

    /// Which notes the lists show was chosen, above one of them: `chosen`
    /// is that choice in the control. Kept, and shown from now on.
    pub fn handle_select_feed_mode(&mut self, chosen: segmented_button::Entity) -> Task<cosmic::Action<Message>> {
        let Some(mode) = self.feed_modes.data::<FeedMode>(chosen).copied() else {
            return Task::none();
        };
        self.feed_modes.activate(chosen);
        if mode == self.config.feed_mode {
            return Task::none();
        }
        self.config.feed_mode = mode;
        self.save_config("feed mode");
        self.show_feed_mode()
    }

    /// Show every list as [`crate::config::Config::feed_mode`] has it,
    /// which just changed: the control says so, and each list starts
    /// counting pages with nothing to show over again, looking back at
    /// once if it has nothing. The feed, if it is on show, is read from
    /// its top.
    pub(crate) fn show_feed_mode(&mut self) -> Task<cosmic::Action<Message>> {
        let mode = self.config.feed_mode;
        if let Some(position) = FeedMode::ALL.iter().position(|offered| *offered == mode) {
            self.feed_modes.activate_position(position as u16);
        }
        let mut lists = vec![self.following.id];
        lists.extend(self.profiles_mut().map(|profile| profile.notes.id));
        let mut tasks = Vec::new();
        for id in lists {
            if let Some(state) = self.feed_mut(id) {
                state.barren_pages = 0;
            }
            tasks.push(self.look_back_if_nothing_to_show(id));
        }
        if self.view_state == ViewState::Feed {
            tasks.push(scroll_list_to(0.0));
        }
        Task::batch(tasks)
    }

    /// Refresh was pressed: the newest notes of the feed, shown as soon as
    /// they are here, with whatever waited already. While who is followed
    /// isn't known, that is asked first.
    pub fn handle_refresh_feed(&mut self) -> Task<cosmic::Action<Message>> {
        if self.follows.is_none() {
            return self.load_contacts();
        }
        self.following.show_when_fetched = true;
        self.feed_checked_at = Some(Instant::now());
        // One may be on its way already, asked for in passing: it will do.
        self.fetch_feed(self.following.id, FeedPage::Newest)
    }

    /// Scroll the feed to where it was left: the scroller has the place of
    /// whatever list it showed last.
    pub(crate) fn restore_feed_position(&self) -> Task<cosmic::Action<Message>> {
        scroll_list_to(self.following.position.get())
    }

    /// Add `notes` to the list `id`, if it is still there, and look up the
    /// people they are shown with (authors, and those mentioned) and what
    /// was made of them.
    pub(crate) fn merge_notes(&mut self, id: u64, notes: Vec<Note>) -> Task<cosmic::Action<Message>> {
        let Some(state) = self.feed_mut(id) else {
            return Task::none();
        };
        let ids: Vec<EventId> = notes.iter().map(|note| note.id).collect();
        let people = state.feed.merge(notes);
        Task::batch([self.look_up_people(people), self.look_up_engagement(ids)])
    }

    /// Look up the profiles of those of `people` who were not asked about
    /// yet: first in the event store, which answers at once.
    pub(crate) fn look_up_people(&mut self, people: Vec<PublicKey>) -> Task<cosmic::Action<Message>> {
        // Without the store nobody is asked about, and nobody counts as
        // asked about either.
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        let people: Vec<PublicKey> = people.into_iter().filter(|person| self.profiles_requested.insert(*person)).collect();
        if people.is_empty() {
            return Task::none();
        }
        Task::perform(
            async move {
                let found = service.cached_profiles(people.clone()).await;
                (people, found)
            },
            |(requested, found)| cosmic::Action::App(Message::ProfilesCached { requested, found }),
        )
    }

    /// Profiles were read from the event store. Ask the relays for the
    /// authors it didn't know.
    pub fn handle_profiles_cached(
        &mut self,
        requested: Vec<PublicKey>,
        found: Vec<(PublicKey, Profile)>,
    ) -> Task<cosmic::Action<Message>> {
        let checked = self.keep_profiles(found);
        let missing: Vec<PublicKey> = requested.into_iter().filter(|author| !self.profiles.contains_key(author)).collect();
        let Some(service) = self.nostr.clone() else {
            return checked;
        };
        if missing.is_empty() {
            return checked;
        }
        Task::batch([checked, fetch_profiles(service, missing)])
    }

    /// Profiles arrived, as they stand now.
    pub fn handle_profiles_fetched(&mut self, found: Vec<(PublicKey, Profile)>) -> Task<cosmic::Action<Message>> {
        tracing::debug!("profiles: {} arrived", found.len());
        self.keep_profiles(found)
    }

    /// Keep `found` as what is known of those people. Those of them whose
    /// profile is open may have a name to check that they hadn't before.
    fn keep_profiles(&mut self, found: Vec<(PublicKey, Profile)>) -> Task<cosmic::Action<Message>> {
        let people: Vec<PublicKey> = found.iter().map(|(person, _)| *person).collect();
        self.profiles.extend(found);
        Task::batch(people.into_iter().map(|person| self.check_nip05(person)))
    }

    // =========================================================================
    // Who the account follows
    // =========================================================================

    /// Forget who is followed and the feed made of their notes: the account
    /// changed. Anything still on its way for the old one is dropped.
    pub(crate) fn reset_following(&mut self) {
        self.follows = None;
        self.following = self.new_feed();
    }

    /// Find out about the account: who it follows, which fills the feed,
    /// what it says of itself, which is its picture above the feed, and,
    /// if the popup is open, what it has bookmarked. Does nothing until
    /// both the account and the store are there; it is called when either
    /// arrives.
    pub(crate) fn load_account(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(account) = self.account.as_ref().map(|account| account.public_key) else {
            return Task::none();
        };
        Task::batch([
            self.use_account_relays(),
            self.load_contacts(),
            self.look_up_people(vec![account]),
            self.load_bookmarks(),
            self.load_relay_lists(&ALWAYS_READ),
            self.load_drafts(),
            self.load_notifications(),
        ])
    }

    /// Have the relays the account's relay list says it writes to join
    /// its own: where its lists are, and where what it writes goes. Nothing
    /// waits for it: until they have joined, the relays the config names
    /// are the account's own.
    fn use_account_relays(&self) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        Task::future(async move {
            service.use_relays_of(Some(account)).await;
        })
        .discard()
    }

    /// Find out who the account follows: from the store now, from the relays
    /// after.
    fn load_contacts(&mut self) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        let feed = self.following.id;
        Task::perform(async move { service.cached_contacts(account).await }, move |follows| {
            cosmic::Action::App(Message::ContactsLoaded { feed, source: Source::Store, result: Ok(follows) })
        })
    }

    /// The account's contact list was read from the store, or came back
    /// from the relays, asked for the feed `feed`: if that is not the feed
    /// any more, neither is the answer.
    pub fn handle_contacts_loaded(
        &mut self,
        feed: u64,
        source: Source,
        result: Result<Option<Vec<PublicKey>>, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        if feed != self.following.id {
            return Task::none();
        }
        match (source, result) {
            // What the store knows shows at once; the relays are asked
            // whether there is a newer list either way.
            (Source::Store, result) => {
                let shown = match result {
                    Ok(Some(follows)) => self.set_follows(follows),
                    _ => Task::none(),
                };
                let (Some(service), Some(account)) =
                    (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key))
                else {
                    return shown;
                };
                let feed = self.following.id;
                let asked = Task::perform(async move { service.fetch_contacts(account).await }, move |result| {
                    cosmic::Action::App(Message::ContactsLoaded { feed, source: Source::Relays, result })
                });
                Task::batch([shown, asked])
            }
            (Source::Relays, Ok(Some(follows))) => self.set_follows(follows),
            // No list anywhere: the account follows nobody.
            (Source::Relays, Ok(None)) => match self.follows {
                None => self.set_follows(Vec::new()),
                Some(_) => Task::none(),
            },
            (Source::Relays, Err(e)) => {
                tracing::error!("the contact list could not be fetched: {e:?}");
                // With a list from the store there is still a feed to show.
                if self.follows.is_none() {
                    self.error_message = Some(fetch_error_text(e));
                }
                Task::none()
            }
        }
    }

    /// The account follows `follows`. If that is news, the feed starts over
    /// with them.
    pub(crate) fn set_follows(&mut self, follows: Vec<PublicKey>) -> Task<cosmic::Action<Message>> {
        if self.follows.as_ref() == Some(&follows) {
            return Task::none();
        }
        tracing::info!("following {} account(s)", follows.len());
        self.following = self.new_feed();
        self.follows = Some(follows);
        self.load_feed(self.following.id)
    }
}

/// Why a request to the relays failed, in the user's language.
pub(crate) fn fetch_error_text(error: FetchError) -> String {
    match error {
        FetchError::NoRelayReachable => fl!("error-no-relay"),
        FetchError::Failed(error) => fl!("error-feed", error = error),
        FetchError::Incomplete => fl!("error-feed", error = fl!("error-relays-incomplete")),
    }
}

// SPDX-License-Identifier: MIT

//! Searching for people and notes.
//!
//! A search page is a page like a thread or a profile: what was typed and
//! what was found stay with it while the popup is closed, and while a
//! thread or a profile opened from it is on show, and are gone when it
//! is left. Nothing of a search is written anywhere: there are no recent
//! searches, and the words are not logged.
//!
//! The relays asked are the account's search relays, as its lists have
//! them (see [`crate::handlers::relays`]): there are no others, and with
//! none set the page says where to set them. People and notes are
//! searched for at once, and the page shows one or the other.
//!
//! What is typed that names a person or a note by its address is opened
//! rather than searched for, and no relay is told. A secret key typed
//! here is cleared, and goes nowhere.

use std::collections::HashSet;

use cosmic::prelude::*;
use nostr_sdk::prelude::{EventId, PublicKey, RelayUrl};

use crate::fl;
use crate::handlers::feed::fetch_error_text;
use crate::handlers::navigation::scroll_list_to;
use crate::messages::Message;
use crate::nostr::client::FetchError;
use crate::nostr::models::Note;
use crate::nostr::nip05;
use crate::nostr::relay_info::{self, RelayInfo};
use crate::nostr::relay_lists::{Kept, Section};
use crate::nostr::search::{self, Found, SearchFor, Typed};
use crate::state::{AppModel, Page, SearchPage, SearchProgress};
use crate::views::components::RowHeights;
use crate::views::search::search_input_id;

impl AppModel {
    /// The search page `id`, if that page is still open.
    fn search_page_mut(&mut self, id: u64) -> Option<&mut SearchPage> {
        self.pages.iter_mut().find_map(|page| match page {
            Page::Search(page) if page.id == id => Some(page.as_mut()),
            _ => None,
        })
    }

    /// The account's search relays, as its lists were last read. `None`
    /// until they were.
    pub(crate) fn search_relays(&self) -> Option<Vec<RelayUrl>> {
        let lists = self.relays.lists.as_ref()?;
        let search = lists.of(Section::Search);
        // Not before the private part was looked at: most keep them there.
        (*search.kept() != Kept::Unread).then(|| search.relays())
    }

    /// Show a page to search on, over whatever is on show now, with the
    /// place to type in ready to be typed in. The account's lists are
    /// asked for, its search relays read in full, unless they were
    /// already: they say which relays search.
    pub fn handle_open_search(&mut self) -> Task<cosmic::Action<Message>> {
        self.menu_open = false;
        if self.account.is_none() {
            return Task::none();
        }
        self.feeds_made += 1;
        let opened = self.open_page(Page::Search(Box::new(SearchPage::new(self.feeds_made))));
        let lists = self.load_relay_lists(&[Section::Search]);
        Task::batch([opened, lists, cosmic::widget::text_input::focus(search_input_id())])
    }

    /// What is typed on the search page `id` changed.
    pub fn handle_search_typed(&mut self, id: u64, typed: String) {
        if let Some(page) = self.search_page_mut(id) {
            page.typed = typed;
        }
    }

    /// Search for what is typed on the search page `id`; or, if that is
    /// the address of a person or a note, open it.
    pub fn handle_search(&mut self, id: u64) -> Task<cosmic::Action<Message>> {
        let Some(page) = self.search_page_mut(id) else {
            return Task::none();
        };
        match search::typed(&page.typed) {
            Typed::Nothing => Task::none(),
            Typed::Secret => {
                // Not left on the screen either.
                page.typed.clear();
                self.error_message = Some(fl!("search-secret"));
                Task::none()
            }
            Typed::Person(person) => self.handle_open_profile(person),
            Typed::Note(note, whereabouts) => self.handle_open_thread(note, whereabouts),
            Typed::Name(name) => {
                page.naming = Some(name.clone());
                let http = self.http.clone();
                Task::perform(
                    async move {
                        let person = nip05::resolve(&http, &name).await;
                        (name, person)
                    },
                    move |(name, person)| cosmic::Action::App(Message::SearchNamed { page: id, name, person }),
                )
            }
            Typed::Relay(relay) => self.show_relay(id, relay),
            Typed::Words(words) => self.search_for(id, words),
        }
    }

    /// The domain of `name`, typed on the search page `id`, said whose it
    /// is: their profile opens. Or it didn't, and that is said.
    pub fn handle_search_named(&mut self, id: u64, name: &str, person: Option<PublicKey>) -> Task<cosmic::Action<Message>> {
        let Some(page) = self.search_page_mut(id).filter(|page| page.naming.as_deref() == Some(name)) else {
            return Task::none();
        };
        page.naming = None;
        match person {
            Some(person) => self.handle_open_profile(person),
            None => {
                self.error_message = Some(fl!("search-no-such-name", name = name));
                Task::none()
            }
        }
    }

    /// Start the search page `id` over for another search: what the last
    /// one found goes, and the answers that come are to this one. Returns
    /// which search this is.
    fn start_search(page: &mut SearchPage) -> u64 {
        page.serial += 1;
        page.words = None;
        page.relay = None;
        page.relay_info = None;
        page.naming = None;
        page.people.clear();
        page.notes.clear();
        page.heights = RowHeights::default();
        page.people_position.set(0.0);
        page.notes_position.set(0.0);
        page.people_progress = SearchProgress::Idle;
        page.notes_progress = SearchProgress::Idle;
        page.serial
    }

    /// Show what `relay`, whose address was typed on the search page `id`,
    /// has: the notes it got last, and what it says of itself.
    fn show_relay(&mut self, id: u64, relay: RelayUrl) -> Task<cosmic::Action<Message>> {
        let (Some(service), http) = (self.nostr.clone(), self.http.clone()) else {
            return Task::none();
        };
        let Some(page) = self.search_page_mut(id) else {
            return Task::none();
        };
        let serial = Self::start_search(page);
        page.relay = Some(relay.clone());
        page.what = SearchFor::Notes;
        page.notes_progress = SearchProgress::Searching;

        let notes = {
            let relay = relay.clone();
            Task::perform(async move { service.relay_notes(&relay).await.map(Found::Notes) }, move |result| {
                cosmic::Action::App(Message::SearchFound { page: id, serial, what: SearchFor::Notes, result })
            })
        };
        let info = Task::perform(async move { relay_info::fetch(&http, &relay).await }, move |info| {
            cosmic::Action::App(Message::RelayInfoLoaded { page: id, serial, info })
        });
        Task::batch([notes, info, scroll_list_to(0.0)])
    }

    /// The relay the search page `id` shows said what it is, or didn't.
    pub fn handle_relay_info_loaded(&mut self, id: u64, serial: u64, info: Option<RelayInfo>) {
        if let Some(page) = self.search_page_mut(id).filter(|page| page.serial == serial) {
            page.relay_info = info;
        }
    }

    /// Ask the account's search relays what `words` find, of people and
    /// of notes, for the search page `id`. With no search relays there is
    /// nobody to ask, and the page says so.
    fn search_for(&mut self, id: u64, words: String) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        let Some(relays) = self.search_relays().filter(|relays| !relays.is_empty()) else {
            return Task::none();
        };
        let Some(page) = self.search_page_mut(id) else {
            return Task::none();
        };
        let serial = Self::start_search(page);
        page.words = Some(words.clone());
        page.people_progress = SearchProgress::Searching;
        page.notes_progress = SearchProgress::Searching;
        tracing::debug!("searching {} relay(s)", relays.len());

        let ask = |what: SearchFor| {
            let (service, relays, words) = (service.clone(), relays.clone(), words.clone());
            Task::perform(async move { service.search(&relays, &words, what).await }, move |result| {
                cosmic::Action::App(Message::SearchFound { page: id, serial, what, result })
            })
        };
        Task::batch([ask(SearchFor::People), ask(SearchFor::Notes), scroll_list_to(0.0)])
    }

    /// Show the people the search page `id` found, or the notes, where
    /// that list was left.
    pub fn handle_search_show(&mut self, id: u64, what: SearchFor) -> Task<cosmic::Action<Message>> {
        let Some(page) = self.search_page_mut(id) else {
            return Task::none();
        };
        if page.what == what {
            return Task::none();
        }
        page.what = what;
        let position = page.position().get();
        if self.search_on_top().is_some_and(|on_top| on_top.id == id) { scroll_list_to(position) } else { Task::none() }
    }

    /// The search relays answered the search page `id`, for the search it
    /// numbered `serial`, with the people or the notes they found; or
    /// didn't. An answer to a search that another has replaced is dropped.
    pub fn handle_search_found(
        &mut self,
        id: u64,
        serial: u64,
        what: SearchFor,
        result: Result<Found, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        let followed: HashSet<PublicKey> = self.follows.iter().flatten().copied().collect();
        let Some(page) = self.search_page_mut(id).filter(|page| page.serial == serial) else {
            return Task::none();
        };
        match result {
            Ok(Found::People(people)) => {
                tracing::debug!("search: {} people found", people.len());
                // Whoever the account follows first: a name is anyone's to take.
                page.people = search::followed_first(people.iter().map(|(person, _)| *person).collect(), &followed);
                page.people_progress = SearchProgress::Found;
                self.handle_profiles_fetched(people)
            }
            Ok(Found::Notes(notes)) => {
                tracing::debug!("search: {} note(s) found", notes.len());
                let people: Vec<PublicKey> = notes.iter().flat_map(Note::people).collect();
                let ids: Vec<EventId> = notes.iter().map(|note| note.id).collect();
                page.notes = notes;
                page.notes_progress = SearchProgress::Found;
                Task::batch([self.look_up_people(people), self.look_up_engagement(ids)])
            }
            Err(e) => {
                tracing::warn!("the search for {what:?} did not go through: {e:?}");
                let failed = SearchProgress::Failed(fetch_error_text(e));
                match what {
                    SearchFor::People => page.people_progress = failed,
                    SearchFor::Notes => page.notes_progress = failed,
                }
                Task::none()
            }
        }
    }
}

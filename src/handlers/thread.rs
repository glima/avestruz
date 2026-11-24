// SPDX-License-Identifier: MIT

//! Thread message handlers for Avestruz.
//!
//! A thread opens for one note: from a row of a feed, from a row of another
//! thread, or from a reference to the note in what another says. It shows at
//! once with whatever of the conversation is at hand, which is the note
//! itself when it came from a row. The event store is asked for the rest
//! and answers within the frame; the relays are asked at the same time and
//! answer when they do.
//!
//! A thread is a page: it opens over whatever is on show, and Back closes
//! it (see [`crate::handlers::navigation`]).

use cosmic::prelude::*;
use nostr_sdk::prelude::EventId;

use crate::handlers::feed::fetch_error_text;
use crate::handlers::navigation::scroll_list_to;
use crate::messages::Message;
use crate::nostr::client::{FetchError, Source};
use crate::nostr::models::Note;
use crate::nostr::outbox::Whereabouts;
use crate::nostr::thread::Thread;
use crate::state::{AppModel, Page, ThreadState, ViewState};
use crate::views::components::RowSizes;
use crate::views::thread::estimated_row_height;

/// How far down `state`'s list the focused note starts, its rows set in
/// `sizes`: nowhere, until it is known.
fn focused_offset(state: &ThreadState, sizes: RowSizes) -> f32 {
    let rows = state.thread.rows();
    let above = state.thread.focused_row().and_then(|row| rows.get(..row)).unwrap_or_default();
    state.heights.total(above, |row| row.note.id, |row, width| estimated_row_height(row, sizes, width, &state.expanded))
}

impl AppModel {
    /// The note `id`, if it is at hand: in the thread, the profile, the
    /// bookmarks or the search on show, or in the feed.
    pub(crate) fn note_at_hand(&self, id: EventId) -> Option<Note> {
        let in_thread = self.thread_on_top().into_iter().flat_map(|state| state.thread.rows()).map(|row| &row.note);
        let in_profile = self.profile_on_top().into_iter().flat_map(|state| state.notes.feed.all());
        let in_bookmarks = self.bookmarks_on_top().into_iter().flat_map(|page| &page.notes.notes);
        let in_search = self.search_on_top().into_iter().flat_map(|page| &page.notes);
        let told = self.notifications.list.notes();
        let at_hand =
            in_thread.chain(in_profile).chain(in_bookmarks).chain(in_search).chain(told).chain(self.following.feed.all());
        at_hand.into_iter().find(|note| note.id == id).cloned()
    }

    /// Whether the thread of the note `focused` is the one on show.
    fn thread_on_show(&self, focused: EventId) -> bool {
        self.view_state == ViewState::Page && self.thread_on_top().is_some_and(|state| state.thread.focused() == focused)
    }

    /// Show the thread of the note `id`, over whatever is on show now.
    /// `whereabouts` is where the note may be if the account's own relays
    /// don't have it, as whatever named it said.
    pub fn handle_open_thread(&mut self, id: EventId, whereabouts: Whereabouts) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        if self.thread_on_show(id) {
            return Task::none();
        }
        let thread = Thread::new(id, self.note_at_hand(id));
        let opened = self.open_page(Page::Thread(Box::new(ThreadState::new(thread))));

        let ask = |source: Source| {
            let (service, whereabouts) = (service.clone(), whereabouts.clone());
            Task::perform(async move { service.thread(source, id, &whereabouts).await }, move |result| {
                cosmic::Action::App(Message::ThreadLoaded { focused: id, source, result })
            })
        };
        Task::batch([opened, ask(Source::Store), ask(Source::Relays)])
    }

    /// The conversation around a thread's note came from the store or from
    /// the relays (or didn't). It goes to every open thread of that note.
    pub fn handle_thread_loaded(
        &mut self,
        focused: EventId,
        source: Source,
        result: Result<Vec<Note>, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        let on_show = self.thread_on_show(focused);
        let row_before = self.thread_on_top().and_then(|state| state.thread.focused_row());

        let notes = match result {
            Ok(notes) => {
                tracing::info!("thread: {} note(s) from {source:?}", notes.len());
                notes
            }
            Err(e) => {
                tracing::error!("thread could not be fetched from {source:?}: {e:?}");
                self.error_message = Some(fetch_error_text(e));
                Vec::new()
            }
        };
        let mut people = Vec::new();
        for state in self.threads_mut().filter(|state| state.thread.focused() == focused) {
            people.extend(state.thread.merge(notes.iter().cloned()));
            if source == Source::Relays {
                state.loading = false;
            }
        }
        let ids: Vec<EventId> = notes.iter().map(|note| note.id).collect();
        let looked_up = Task::batch([self.look_up_people(people), self.look_up_engagement(ids)]);

        // Notes that came in above the focused one pushed it down, perhaps
        // out of view: bring it back to the top. When nothing above it
        // changed, the list stays where the reader has it.
        let row_after = self.thread_on_top().and_then(|state| state.thread.focused_row());
        let anchored = match self.thread_on_top() {
            Some(state) if on_show && row_after != row_before => {
                scroll_list_to(focused_offset(state, RowSizes::new(self.config.note_text_size())))
            }
            _ => Task::none(),
        };
        Task::batch([looked_up, anchored])
    }
}

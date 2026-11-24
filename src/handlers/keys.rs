// SPDX-License-Identifier: MIT

//! The keyboard, on a list: stepping through its rows, opening the one
//! the keyboard is at, showing the notes that wait, and going back.
//!
//! | Key | What it does |
//! |---|---|
//! | `j` | The next row: marked, and put at the top of what is in view |
//! | `k` | The one before |
//! | Home | The very top, and no row marked |
//! | `l`, Enter | Open the marked row: a note's thread; for something told of with no note to it, whose it is |
//! | `y` | Copy a link to the marked note that anyone can open, as the chain link under it does |
//! | Space | The first thing the marked note links to, pictures apart: a video is played, or held if it plays; any other link is opened in the browser |
//! | `h`, Backspace | Back: what the arrow at the top left of the view does |
//! | Backspace | Out of the composer too, where `h` is a letter: with something written, it asks first whether to keep it as a draft (see [`crate::handlers::compose`]) |
//! | `r` | Reply to the marked note, as the arrow under it does |
//! | `R` | Offer the ways to pass the marked note on, as its repost action does: see below |
//! | `z` | Zap the marked note, as the bolt under it does |
//! | `b` | Bookmark the marked note, or take its bookmark off, as the ribbon under it does |
//! | `B` | Show the bookmarks, over the list on show |
//! | `.` | Show the notes that wait, as the button over the list does |
//! | `?` | Show what the keys do: a view of its own, which the menu above the feed opens too |
//!
//! The keys are a list's where no widget had a use for them: a text being
//! typed takes them for itself (see [`crate::app`]). They work on
//! whichever list is on show: the feed, a thread, the notes of a
//! profile, the bookmarks, the notes a search found, what the account is
//! told of.
//!
//! A row can be put at the top of what is in view only while there is
//! enough under it: the last rows of a list can't be, and in a thread
//! that fits the popup none can. So where the keyboard is does not go by
//! where the list is scrolled to alone. The row it is at is **marked**,
//! with a line around it, and each list keeps its own mark: coming back
//! from a thread, the note it was opened from is marked still, and `j`
//! goes on from there.
//!
//! ```mermaid
//! flowchart TD
//!     key["j or k"] --> live{"Is a row marked, and about where the view is?"}
//!     live -->|"yes"| beside["The row after it, or before"]
//!     live -->|"no: nothing marked, or the list was scrolled away from it"| place["The first row below the top of the view, or the last above it"]
//!     beside --> mark["That row is marked, and the list scrolled to put it at the top, as far as it goes"]
//!     place --> mark
//!     open["l, Enter, y, Space or b"] --> which{"Is a row marked, and about where the view is?"}
//!     which -->|"yes"| opened["It is that row: opened, its link copied, its video played or its link followed, bookmarked"]
//!     which -->|"no"| top["The row at the top of the view is marked, and it is that one"]
//! ```
//!
//! While the menu `R` opens is open, the keys are the menu's, and the
//! keyboard is at one of the ways it offers, which is shown as one the
//! pointer is over: the first that can be taken, to begin with.
//!
//! ```mermaid
//! flowchart TD
//!     r["R, on a list"] --> menu["The marked note's repost menu opens, the keyboard at the first way that can be taken"]
//!     menu --> arrows["Arrow down or j, arrow up or k: the next way, or the one before"]
//!     arrows --> menu
//!     menu --> enter["Enter or l: that way is taken, and the menu is put away"]
//!     menu --> away["R again, or Escape: the menu is put away, and nothing done"]
//! ```
//!
//! A mark the list was scrolled away from is not gone back to: a key
//! then goes by what is in view, as it does with nothing marked.

use cosmic::prelude::*;
use nostr_sdk::prelude::{EventId, PublicKey};

use crate::handlers::navigation::scroll_list_to;
use crate::messages::Message;
use crate::nostr::engagement::Mark;
use crate::nostr::models::Note;
use crate::nostr::outbox::Whereabouts;
use crate::nostr::search::SearchFor;
use crate::nostr::thread::ThreadRow;
use crate::previews::PreviewState;
use crate::state::{AppModel, Expanded, FeedState, Page, Passing, RepostMenu, Step, ViewState};
use crate::views::components::{RowHeights, RowSizes, estimated_note_height, estimated_notification_height};
use crate::views::thread::estimated_row_height;

/// How near, in points, the top of a row may be to the top of what is in
/// view and be the row that is there: where a list is scrolled to is not
/// kept to the last fraction.
const AT_THE_TOP: f32 = 1.0;

/// How much of a row has to show at the top of what is in view, in
/// points, for it to be the row that is there, when none is marked: a
/// row of which only the last line is left is on its way out, and the one
/// under it is what is being looked at.
const SHOWING: f32 = 40.0;

/// What opening a row leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leads {
    /// The thread of this note.
    Thread(EventId),
    /// The profile of this person.
    Profile(PublicKey),
}

/// A row of the list on show, or something above its rows that is a
/// place to step to as a row is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Row {
    /// What the row is told apart by, and marked by. `None` for what is
    /// no row of the list.
    key: Option<EventId>,
    /// How far down what the scroller holds it starts.
    start: f32,
    /// What opening it leads to, if anything.
    leads: Option<Leads>,
    /// The note it shows, with its actions under it, if it shows one.
    note: Option<EventId>,
}

impl Row {
    /// The row of a note that starts at `start`, which opens its thread.
    fn of_note(note: EventId, start: f32) -> Self {
        Self { key: Some(note), start, leads: Some(Leads::Thread(note)), note: Some(note) }
    }
}

/// The list on show, as the keyboard goes by it.
struct Rows {
    /// Its rows, from the top.
    rows: Vec<Row>,
    /// How far it is scrolled.
    here: f32,
    /// The row the keyboard was last at, if it was at any.
    marked: Option<EventId>,
}

/// Where the keyboard is among `rows`: the row `marked`, if it is one of
/// them and about where the list, scrolled to `here`, shows `view` points
/// of itself. `None` with nothing marked, and with a mark the list was
/// scrolled away from: one whose row starts above what is in view, or
/// below it.
fn marked_row(rows: &[Row], marked: Option<EventId>, here: f32, view: f32) -> Option<usize> {
    let marked = marked?;
    let at = rows.iter().position(|row| row.key == Some(marked))?;
    let start = rows.get(at)?.start;
    (start >= here - AT_THE_TOP && start <= here + view).then_some(at)
}

/// The row at the top of what is in view, in a list scrolled to `here`:
/// the last that starts above where [`SHOWING`] of it would still show.
fn row_at_top(rows: &[Row], here: f32) -> Option<usize> {
    rows.iter().rposition(|row| row.start <= here + SHOWING)
}

/// Which of `rows` a step lands on, and where the list is scrolled to to
/// have it at the top of what is in view. `None` if there is nowhere to
/// go: the keyboard is at the first row, or the last.
///
/// From the row the keyboard is `at`, it is the one after it or before.
/// With no such row it goes by where the list is scrolled to, `here`: the
/// first row below the top of what is in view, or the last one above it.
/// To the top is to where the list starts, with nothing marked.
fn stepped(rows: &[Row], here: f32, at: Option<usize>, step: Step) -> Option<(Option<usize>, f32)> {
    let to = match (step, at) {
        (Step::Top, _) => return (here > AT_THE_TOP || at.is_some()).then_some((None, 0.0)),
        (Step::Next, Some(at)) => at + 1,
        (Step::Next, None) => rows.iter().position(|row| row.start > here + AT_THE_TOP)?,
        (Step::Previous, Some(at)) => at.checked_sub(1)?,
        (Step::Previous, None) => rows.iter().rposition(|row| row.start < here - AT_THE_TOP)?,
    };
    rows.get(to).map(|row| (Some(to), row.start))
}

/// Which of `ways`, the ways a menu offers that can be taken, the
/// keyboard is at after a step from `at`: the next, or the one before,
/// and no further than the first or the last. From none of them, the
/// first, or the last if the step is back.
fn stepped_way(ways: &[Passing], at: Option<Passing>, step: Step) -> Option<Passing> {
    let place = at.and_then(|at| ways.iter().position(|way| *way == at));
    let to = match (place, step) {
        (_, Step::Top) | (None, Step::Next) => 0,
        (None, Step::Previous) => ways.len().saturating_sub(1),
        (Some(place), Step::Next) => (place + 1).min(ways.len().saturating_sub(1)),
        (Some(place), Step::Previous) => place.saturating_sub(1),
    };
    ways.get(to).copied()
}

/// What Space does to a note.
#[derive(Debug, PartialEq, Eq)]
enum Pressed {
    /// Play the video at this address, or hold it if it plays.
    Play(String),
    /// Open this address in the browser.
    Open(String),
}

/// What Space does to `note`. It is for the first thing the note links
/// to that is not a picture, which is shown where it is and has no use
/// for a key. A video is played, and so is a page that `is_video` says
/// turned out to be one; anything else is opened in the browser, as a
/// click on the link does. Nothing, of a note that links to nothing.
fn pressed(note: &Note, is_video: impl Fn(&str) -> bool) -> Option<Pressed> {
    let first = note.content.first_link.as_ref()?;
    let video = note.content.videos.iter().any(|video| video.url == *first) || is_video(first);
    Some(if video { Pressed::Play(first.clone()) } else { Pressed::Open(first.clone()) })
}

/// The rows of a page whose list is `top` down the page, under something
/// else: that, at the very top, is a place to step to as a row is, though
/// it is none to mark or open, and the rows of the list follow it.
fn under(top: f32, rows: Vec<Row>) -> Vec<Row> {
    let above = Row { key: None, start: 0.0, leads: None, note: None };
    std::iter::once(above).chain(rows.into_iter().map(|row| Row { start: top + row.start, ..row })).collect()
}

impl AppModel {
    /// Whether the keys are a list's now: the popup is open, and no menu
    /// is open over what it shows.
    fn keys_are_the_lists(&self) -> bool {
        let over_the_list =
            self.menu_open || self.repost_menu.is_some() || self.reaction_menu.is_some() || self.who_menu.is_some();
        self.popup.is_some() && !over_the_list
    }

    /// The list on show, as the keyboard goes by it: where its rows
    /// start, from the top of what its scroller holds, what each leads
    /// to, how far it is scrolled, and which row is marked. `None` where
    /// what is on show is no list.
    ///
    /// A row is as tall as it measured, or as its estimate until it has
    /// been in view: the same heights, by the same estimate, as the list
    /// is laid out with (see [`RowHeights::starts`]).
    fn rows_at_the_keys(&self) -> Option<Rows> {
        let sizes = RowSizes::new(self.config.note_text_size());
        let notes = |notes: &[Note], heights: &RowHeights<EventId>, expanded: &Expanded| -> Vec<Row> {
            let starts = heights.starts(
                notes,
                |note: &Note| note.id,
                |note, width| estimated_note_height(note, sizes, width, expanded.has(&note.id)),
            );
            notes.iter().zip(starts).map(|(note, start)| Row::of_note(note.id, start)).collect()
        };
        let feed = |state: &FeedState| notes(state.feed.notes(self.config.feed_mode), &state.heights, &state.expanded);
        match self.view_state {
            ViewState::Feed => {
                let state = &self.following;
                Some(Rows { rows: feed(state), here: state.position.get(), marked: state.marked })
            }
            ViewState::Page => Some(match self.pages.last()? {
                Page::Thread(state) => {
                    let listed = state.thread.rows();
                    let starts = state.heights.starts(
                        listed,
                        |row: &ThreadRow| row.note.id,
                        |row, width| estimated_row_height(row, sizes, width, &state.expanded),
                    );
                    // The note the thread is for leads nowhere: it is here.
                    let focused = state.thread.focused();
                    let leads = |note: EventId| (note != focused).then_some(Leads::Thread(note));
                    let rows = listed.iter().zip(starts);
                    let rows = rows.map(|(row, start)| Row { leads: leads(row.note.id), ..Row::of_note(row.note.id, start) });
                    Rows { rows: rows.collect(), here: state.position.get(), marked: state.marked }
                }
                // Who they are is above their notes, in the same scroller.
                Page::Profile(state) => Rows {
                    rows: under(state.notes_top.get(), feed(&state.notes)),
                    here: state.notes.position.get(),
                    marked: state.notes.marked,
                },
                Page::Bookmarks(page) => {
                    let listed = &page.notes;
                    Rows {
                        rows: notes(&listed.notes, &listed.heights, &listed.expanded),
                        here: listed.position.get(),
                        marked: listed.marked,
                    }
                }
                Page::Search(page) => match page.what {
                    SearchFor::Notes => Rows {
                        rows: notes(&page.notes, &page.heights, &page.expanded),
                        here: page.notes_position.get(),
                        marked: page.marked,
                    },
                    // The people found are not rows that were measured:
                    // there is the top to go to, and no more.
                    SearchFor::People => Rows { rows: Vec::new(), here: page.people_position.get(), marked: None },
                },
                Page::Notifications(page) => {
                    let told = &self.notifications.list;
                    let listed = told.listed(self.notifications_tab());
                    let starts = page.heights.starts(
                        listed,
                        // A place that holds nothing has no row, as in the view.
                        |place: &usize| {
                            told.get(*place).map_or(EventId::from_byte_array([0; 32]), |notification| notification.id)
                        },
                        |place, width| {
                            told.get(*place).map_or(0.0, |notification| {
                                estimated_notification_height(notification, sizes, width, page.expanded.has(&notification.id))
                            })
                        },
                    );
                    let rows = listed.iter().zip(starts).map(|(place, start)| match told.get(*place) {
                        // As a click on it leads: to the note it is
                        // about, or to whose doing it is.
                        Some(notification) => Row {
                            key: Some(notification.id),
                            start,
                            leads: Some(notification.about.map_or(Leads::Profile(notification.who), Leads::Thread)),
                            note: notification.note.as_ref().map(|note| note.id),
                        },
                        None => Row { key: None, start, leads: None, note: None },
                    });
                    Rows { rows: rows.collect(), here: page.position.get(), marked: page.marked }
                }
            }),
            _ => None,
        }
    }

    /// Where the list on show keeps which of its rows is marked. `None`
    /// where what is on show is no list.
    fn mark_on_show(&mut self) -> Option<&mut Option<EventId>> {
        match self.view_state {
            ViewState::Feed => Some(&mut self.following.marked),
            ViewState::Page => Some(match self.pages.last_mut()? {
                Page::Thread(state) => &mut state.marked,
                Page::Profile(state) => &mut state.notes.marked,
                Page::Bookmarks(page) => &mut page.notes.marked,
                Page::Search(page) => &mut page.marked,
                Page::Notifications(page) => &mut page.marked,
            }),
            _ => None,
        }
    }

    /// `j`, `k` or Home was pressed: with a list on show, and nothing
    /// over it, the next row, the one before, or the very top goes to the
    /// top of what is in view, as far as the list scrolls, and the row is
    /// marked (see the [module](self)). Anywhere else the keys mean
    /// nothing.
    ///
    /// On a profile, who they are is the first place: `k` from their
    /// first note goes there, and `j` from there to their first note.
    ///
    /// A row below is where the heights of those above it put it, and
    /// those have all been in view, coming down from the top: it lands
    /// exactly. After a jump over rows never shown, a row above may be
    /// off by what its estimate was, as scrolling up to it would be.
    pub fn handle_step_note(&mut self, step: Step) -> Task<cosmic::Action<Message>> {
        // With the repost menu open, the keys are the menu's.
        if self.repost_menu.is_some() {
            return self.handle_step_choice(step);
        }
        if !self.keys_are_the_lists() {
            return Task::none();
        }
        let Some(Rows { rows, here, marked }) = self.rows_at_the_keys() else {
            return Task::none();
        };
        let at = marked_row(&rows, marked, here, self.popup_max_height());
        let Some((to, start)) = stepped(&rows, here, at, step) else {
            return Task::none();
        };
        if let Some(mark) = self.mark_on_show() {
            *mark = to.and_then(|to| rows.get(to)).and_then(|row| row.key);
        }
        scroll_list_to(start)
    }

    /// The row the keyboard is at, for a key that does something to it.
    /// With no row marked where the view is, that is the row at the top
    /// of what is in view, which is marked: it is where the keyboard is
    /// from then on, and when the list is come back to. `None` where the
    /// keys are not a list's, or the list has no row there.
    fn row_at_the_keys(&mut self) -> Option<Row> {
        if !self.keys_are_the_lists() {
            return None;
        }
        let Rows { rows, here, marked } = self.rows_at_the_keys()?;
        let at = marked_row(&rows, marked, here, self.popup_max_height()).or_else(|| row_at_top(&rows, here))?;
        let row = *rows.get(at)?;
        if let (Some(mark), Some(key)) = (self.mark_on_show(), row.key) {
            *mark = Some(key);
        }
        Some(row)
    }

    /// `l` or Enter was pressed: open the row the keyboard is at (see
    /// [`Self::row_at_the_keys`]), as a click on it does.
    pub fn handle_open_marked(&mut self) -> Task<cosmic::Action<Message>> {
        // With the repost menu open, the key takes what the keyboard is
        // at in it.
        if let Some(menu) = self.repost_menu.filter(|_| self.popup.is_some()) {
            return match menu.at {
                Some(Passing::Repost) => self.handle_mark(menu.note, Mark::Repost),
                Some(Passing::Quote) => self.handle_compose_quote(menu.note),
                None => Task::none(),
            };
        }
        // Asked whether what was written is kept as a draft, the key
        // says yes: the composer is left as its arrow leaves it.
        let asked = self.view_state == ViewState::Compose && self.composer.as_ref().is_some_and(|composer| composer.leaving);
        if asked && self.popup.is_some() {
            return self.handle_close_composer();
        }
        match self.row_at_the_keys().and_then(|row| row.leads) {
            Some(Leads::Thread(note)) => self.handle_open_thread(note, Whereabouts::default()),
            Some(Leads::Profile(person)) => self.handle_open_profile(person),
            None => Task::none(),
        }
    }

    /// `y` was pressed: copy a link to the note the keyboard is at that
    /// anyone can open, as the chain link under it does.
    pub fn handle_share_marked(&mut self) -> Task<cosmic::Action<Message>> {
        match self.row_at_the_keys().and_then(|row| row.note) {
            Some(note) => self.handle_copy_share_link(note),
            None => Task::none(),
        }
    }

    /// Space was pressed: it is for the first thing the note the keyboard
    /// is at links to, pictures apart (see [`pressed`]). A video is
    /// played, or held if it plays, as a press of its play button does;
    /// any other link is opened in the browser, as a click on it does.
    pub fn handle_play_marked(&mut self) -> Task<cosmic::Action<Message>> {
        let note = self.row_at_the_keys().and_then(|row| row.note).and_then(|note| self.note_at_hand(note));
        let pressed = note.as_ref().and_then(|note| {
            pressed(note, |page| self.videos.opened.contains(page) || matches!(self.previews.get(page), PreviewState::Video))
        });
        match pressed {
            Some(Pressed::Play(url)) => self.handle_video_press(url),
            Some(Pressed::Open(url)) => {
                self.handle_open_url(&url);
                Task::none()
            }
            None => Task::none(),
        }
    }

    /// `r` was pressed: reply to the note the keyboard is at, as the
    /// arrow under it does. Not by an account that can't publish.
    pub fn handle_reply_marked(&mut self) -> Task<cosmic::Action<Message>> {
        match self.row_at_the_keys().and_then(|row| row.note) {
            Some(note) => self.handle_compose_reply(note),
            None => Task::none(),
        }
    }

    /// `z` was pressed: zap the note the keyboard is at, as the bolt
    /// under it does. Nothing, if its author has nowhere to be zapped.
    pub fn handle_zap_marked(&mut self) -> Task<cosmic::Action<Message>> {
        match self.row_at_the_keys().and_then(|row| row.note) {
            Some(note) => self.handle_zap_note(note),
            None => Task::none(),
        }
    }

    /// `R` was pressed: offer the ways to pass on the note the keyboard
    /// is at, as its repost action does, with the keyboard at the first
    /// that can be taken; or put the offer away, if it is showing. Not
    /// to an account that can't publish.
    pub fn handle_repost_marked(&mut self) -> Task<cosmic::Action<Message>> {
        if self.repost_menu.take().is_some() || !self.can_publish() {
            return Task::none();
        }
        if let Some(note) = self.row_at_the_keys().and_then(|row| row.note) {
            let reposted = self.engagement.get(&note).is_some_and(|engagement| engagement.reposted);
            self.reaction_menu = None;
            self.repost_menu = Some(RepostMenu { note, at: Passing::open_to(reposted).first().copied() });
        }
        Task::none()
    }

    /// An arrow up or down was pressed, or `j` or `k` with the repost
    /// menu open: the keyboard goes to the next of the ways the menu
    /// offers, or the one before. Nothing, with no such menu open.
    pub fn handle_step_choice(&mut self, step: Step) -> Task<cosmic::Action<Message>> {
        if let Some(menu) = self.repost_menu.filter(|_| self.popup.is_some()) {
            let reposted = self.engagement.get(&menu.note).is_some_and(|engagement| engagement.reposted);
            self.repost_menu = Some(RepostMenu { at: stepped_way(Passing::open_to(reposted), menu.at, step), ..menu });
        }
        Task::none()
    }

    /// `b` was pressed: bookmark the note the keyboard is at, or take its
    /// bookmark off if it has one, as the ribbon under it does.
    pub fn handle_bookmark_marked(&mut self) -> Task<cosmic::Action<Message>> {
        match self.row_at_the_keys().and_then(|row| row.note) {
            Some(note) => self.handle_bookmark(note),
            None => Task::none(),
        }
    }

    /// Whether a list is on show for the keys that go elsewhere from one:
    /// the feed, or a page opened over it. Not the composer or a zap,
    /// which a stray key would put away, nor where nobody is signed in.
    fn keys_go_elsewhere(&self) -> bool {
        self.keys_are_the_lists() && matches!(self.view_state, ViewState::Feed | ViewState::Page)
    }

    /// `B` was pressed: show the bookmarks, as the menu does. Not over
    /// the bookmarks themselves: one way back is enough.
    pub fn handle_go_to_bookmarks(&mut self) -> Task<cosmic::Action<Message>> {
        let there = self.view_state == ViewState::Page && matches!(self.pages.last(), Some(Page::Bookmarks(_)));
        if !self.keys_go_elsewhere() || there {
            return Task::none();
        }
        self.handle_open_bookmarks()
    }

    /// `?` was pressed: show what the keys do, in the view that says so.
    pub fn handle_show_keys(&mut self) -> Task<cosmic::Action<Message>> {
        if self.keys_go_elsewhere() {
            self.handle_open_keys();
        }
        Task::none()
    }

    /// `.` was pressed: show the notes that wait for the list on show, as
    /// the button over it does. They are at its top, which is where it
    /// goes, and no row is marked.
    pub fn handle_show_waiting(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.keys_are_the_lists() {
            return Task::none();
        }
        let state = match self.view_state {
            ViewState::Feed => Some(&mut self.following),
            ViewState::Page => match self.pages.last_mut() {
                Some(Page::Profile(state)) => Some(&mut state.notes),
                _ => None,
            },
            _ => None,
        };
        let Some(state) = state.filter(|state| !state.pending.is_empty()) else {
            return Task::none();
        };
        state.marked = None;
        let id = state.id;
        self.show_new_notes(id)
    }

    /// Backspace was pressed, or is `held` down: go back, as `h` does;
    /// and out of the composer too, which `h` is not, since Backspace
    /// asks before anything written is lost (see
    /// [`Self::handle_abandon_composer`]). Not out of the composer while
    /// it is held: one press, one answer.
    pub fn handle_backspace(&mut self, held: bool) -> Task<cosmic::Action<Message>> {
        match self.view_state {
            ViewState::Compose if !self.keys_are_the_lists() || held => Task::none(),
            ViewState::Compose => self.handle_abandon_composer(),
            _ => self.handle_go_back(),
        }
    }

    /// `h` was pressed, or Backspace: go back, as the arrow at the top
    /// left of the view on show does. Not out of the composer: an `h`
    /// that was meant for its text, pressed when the keyboard was not in
    /// it, would put it away. Backspace leaves it, in a way of its own
    /// (see [`Self::handle_backspace`]).
    pub fn handle_go_back(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.keys_are_the_lists() {
            return Task::none();
        }
        match self.view_state {
            ViewState::Page => self.handle_close_page(),
            ViewState::Settings | ViewState::Relays | ViewState::Keys => self.show_list_view(),
            ViewState::Media => self.handle_close_media(),
            ViewState::Zap => self.handle_close_zap(),
            ViewState::Feed | ViewState::Login | ViewState::Compose => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Leads, Pressed, Row, marked_row, pressed, row_at_top, stepped, stepped_way, under};
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::models::Note;
    use crate::state::{Passing, Step};
    use nostr_sdk::prelude::{EventId, Keys, Kind};

    /// A note that says `said`.
    fn note(said: &str) -> Note {
        Note {
            id: id(7),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse(said, FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        }
    }

    #[test]
    fn the_keyboard_goes_through_the_ways_a_repost_menu_offers_and_no_further() {
        use Passing::{Quote, Repost};
        let both = Passing::open_to(false);
        assert_eq!(both, [Repost, Quote]);
        // It opens at the first; down and up go between them, and stop
        // at either end.
        assert_eq!(stepped_way(both, Some(Repost), Step::Next), Some(Quote));
        assert_eq!(stepped_way(both, Some(Quote), Step::Next), Some(Quote));
        assert_eq!(stepped_way(both, Some(Quote), Step::Previous), Some(Repost));
        assert_eq!(stepped_way(both, Some(Repost), Step::Previous), Some(Repost));
        assert_eq!(stepped_way(both, Some(Quote), Step::Top), Some(Repost));
        // A menu a click opened has the keyboard at nothing: down comes
        // to the first, and up to the last.
        assert_eq!(stepped_way(both, None, Step::Next), Some(Repost));
        assert_eq!(stepped_way(both, None, Step::Previous), Some(Quote));

        // A note that was reposted can only be quoted: the keyboard is
        // never at what can't be taken.
        let quote_only = Passing::open_to(true);
        assert_eq!(quote_only, [Quote]);
        assert_eq!(stepped_way(quote_only, Some(Quote), Step::Previous), Some(Quote));
        assert_eq!(stepped_way(quote_only, Some(Repost), Step::Next), Some(Quote));
        assert_eq!(stepped_way(&[], None, Step::Next), None);
    }

    #[test]
    fn space_plays_a_note_whose_first_link_is_a_video_and_opens_any_other_first_link() {
        let (video, other) = ("https://cdn.example.com/one.mp4", "https://cdn.example.com/two.mp4");
        let (page, picture) = ("https://example.com/watch/this", "https://example.com/a/picture.jpg");
        let play = |url: &str| Some(Pressed::Play(url.to_string()));
        let open = |url: &str| Some(Pressed::Open(url.to_string()));

        // The first of its videos, whatever comes after it.
        assert_eq!(pressed(&note(&format!("watch {video} and {other} and {page}")), |_| false), play(video));
        // A link first: it is followed, though a video comes after.
        assert_eq!(pressed(&note(&format!("read {page} then watch {video}")), |_| false), open(page));
        // A picture is not what the key is for, wherever it stands.
        assert_eq!(pressed(&note(&format!("{picture} then watch {video}")), |_| false), play(video));
        assert_eq!(pressed(&note(&format!("{picture} and read {page}")), |_| false), open(page));

        // A link that does not say what it is: played, if it turned out
        // to be a video, and followed if not.
        assert_eq!(pressed(&note(&format!("see {page}")), |_| true), play(page));
        assert_eq!(pressed(&note(&format!("see {page}")), |_| false), open(page));
        // A YouTube link is a video from the start.
        let youtube = "https://youtu.be/aqz-KE-bpKQ";
        assert_eq!(pressed(&note(&format!("{youtube} and {page}")), |_| false), play(youtube));

        // Nothing linked to but pictures, or nothing at all: nothing done.
        assert_eq!(pressed(&note(&format!("only words, and {picture}")), |_| true), None);
        assert_eq!(pressed(&note("only words"), |_| true), None);
    }

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    /// Rows of notes that start at `starts`, the first of them note 1.
    fn rows(starts: &[f32]) -> Vec<Row> {
        starts.iter().enumerate().map(|(nth, start)| Row::of_note(id(nth as u8 + 1), *start)).collect()
    }

    /// Where a step from `here` leads with no row marked: where the list
    /// is scrolled to.
    fn scrolled(rows: &[Row], here: f32, step: Step) -> Option<f32> {
        stepped(rows, here, None, step).map(|(_, start)| start)
    }

    #[test]
    fn a_step_puts_the_next_note_or_the_one_before_at_the_top_of_what_is_in_view() {
        // Four rows, of 100, 250, 80 and 300 points.
        let rows = rows(&[0.0, 100.0, 350.0, 430.0]);

        // From the top, down a note at a time, and no further than the last.
        assert_eq!(scrolled(&rows, 0.0, Step::Next), Some(100.0));
        assert_eq!(scrolled(&rows, 100.0, Step::Next), Some(350.0));
        assert_eq!(scrolled(&rows, 350.0, Step::Next), Some(430.0));
        assert_eq!(scrolled(&rows, 430.0, Step::Next), None);
        assert_eq!(scrolled(&rows, 600.0, Step::Next), None, "part of the way down the last note");

        // And back up, no further than the first.
        assert_eq!(scrolled(&rows, 430.0, Step::Previous), Some(350.0));
        assert_eq!(scrolled(&rows, 100.0, Step::Previous), Some(0.0));
        assert_eq!(scrolled(&rows, 0.0, Step::Previous), None);

        // Part of the way down a note, the next is the one under it, and
        // the one before is the top of the note itself.
        assert_eq!(scrolled(&rows, 180.0, Step::Next), Some(350.0));
        assert_eq!(scrolled(&rows, 180.0, Step::Previous), Some(100.0));

        // Where the list is scrolled to is not kept to the last fraction.
        assert_eq!(scrolled(&rows, 99.6, Step::Next), Some(350.0));
        assert_eq!(scrolled(&rows, 100.4, Step::Previous), Some(0.0));

        assert_eq!(scrolled(&[], 0.0, Step::Next), None);
        assert_eq!(scrolled(&[], 0.0, Step::Previous), None);
    }

    #[test]
    fn the_row_stepped_to_is_the_one_that_is_marked() {
        let rows = rows(&[0.0, 100.0, 350.0, 430.0]);
        assert_eq!(stepped(&rows, 0.0, None, Step::Next), Some((Some(1), 100.0)));
        assert_eq!(stepped(&rows, 430.0, None, Step::Previous), Some((Some(2), 350.0)));
    }

    #[test]
    fn from_a_marked_row_a_step_is_to_the_row_beside_it_wherever_the_list_can_scroll_to() {
        // A thread that fits the popup: it is not scrolled at all, and
        // none of its rows but the first is ever at the top.
        let rows = rows(&[0.0, 100.0, 350.0, 430.0]);
        assert_eq!(stepped(&rows, 0.0, Some(1), Step::Next), Some((Some(2), 350.0)));
        assert_eq!(stepped(&rows, 0.0, Some(2), Step::Next), Some((Some(3), 430.0)));
        assert_eq!(stepped(&rows, 0.0, Some(3), Step::Next), None, "the last row");
        assert_eq!(stepped(&rows, 0.0, Some(3), Step::Previous), Some((Some(2), 350.0)));
        assert_eq!(stepped(&rows, 0.0, Some(1), Step::Previous), Some((Some(0), 0.0)));
        assert_eq!(stepped(&rows, 0.0, Some(0), Step::Previous), None, "the first row");
    }

    #[test]
    fn home_goes_to_the_very_top_from_anywhere_but_there_and_marks_nothing() {
        let rows = rows(&[0.0, 100.0, 350.0]);
        assert_eq!(stepped(&rows, 350.0, Some(2), Step::Top), Some((None, 0.0)));
        assert_eq!(stepped(&rows, 12.0, None, Step::Top), Some((None, 0.0)));
        assert_eq!(stepped(&rows, 0.0, None, Step::Top), None);
        assert_eq!(stepped(&rows, 0.4, None, Step::Top), None, "as good as there");
        // At the top with a row marked, as in a list that doesn't scroll:
        // the mark goes.
        assert_eq!(stepped(&rows, 0.0, Some(1), Step::Top), Some((None, 0.0)));
    }

    #[test]
    fn the_keyboard_is_at_the_marked_row_while_the_list_is_about_where_it_is() {
        let rows = rows(&[0.0, 100.0, 350.0, 430.0]);
        let view = 600.0;
        // Put at the top of what is in view, or under it, in view.
        assert_eq!(marked_row(&rows, Some(id(3)), 350.0, view), Some(2));
        assert_eq!(marked_row(&rows, Some(id(3)), 0.0, view), Some(2));
        assert_eq!(marked_row(&rows, Some(id(3)), 349.6, view), Some(2), "not kept to the last fraction");
        // The list scrolled on past it, or back up from it out of view.
        assert_eq!(marked_row(&rows, Some(id(3)), 400.0, view), None);
        assert_eq!(marked_row(&rows, Some(id(4)), 0.0, 300.0), None);
        // Nothing marked, and a mark whose row the list no longer has.
        assert_eq!(marked_row(&rows, None, 0.0, view), None);
        assert_eq!(marked_row(&rows, Some(id(9)), 0.0, view), None);
    }

    #[test]
    fn with_nothing_marked_the_row_opened_is_the_one_at_the_top_of_what_is_in_view() {
        let rows = rows(&[0.0, 100.0, 350.0, 430.0]);
        assert_eq!(row_at_top(&rows, 0.0), Some(0));
        assert_eq!(row_at_top(&rows, 100.0), Some(1), "put there by a step");
        assert_eq!(row_at_top(&rows, 180.0), Some(1), "part of the way down it");
        // With only its last line left, it is the one under it.
        assert_eq!(row_at_top(&rows, 330.0), Some(2));
        assert_eq!(row_at_top(&[], 0.0), None);
    }

    #[test]
    fn on_a_page_whose_list_is_under_something_the_top_is_a_place_and_the_rows_follow_it() {
        // Who someone is takes 300, and their notes are 120, 80 and 200 tall.
        let rows = under(300.0, rows(&[0.0, 120.0, 200.0]));
        let starts: Vec<f32> = rows.iter().map(|row| row.start).collect();
        assert_eq!(starts, [0.0, 300.0, 420.0, 500.0]);
        // The top is no row: nothing to mark, and nothing to open.
        assert_eq!((rows[0].key, rows[0].leads), (None, None));
        assert_eq!((rows[1].key, rows[1].leads), (Some(id(1)), Some(Leads::Thread(id(1)))));

        // From the very top, the first note; from there, back to the top.
        assert_eq!(stepped(&rows, 0.0, None, Step::Next), Some((Some(1), 300.0)));
        assert_eq!(stepped(&rows, 300.0, Some(1), Step::Previous), Some((Some(0), 0.0)));
        assert_eq!(stepped(&rows, 300.0, Some(1), Step::Next), Some((Some(2), 420.0)));
        // Part of the way down who they are, their first note is next.
        assert_eq!(scrolled(&rows, 150.0, Step::Next), Some(300.0));
        assert_eq!(scrolled(&rows, 150.0, Step::Previous), Some(0.0));

        // With no notes yet, there is only the top.
        let alone = under(300.0, Vec::new());
        assert_eq!(alone.len(), 1);
        assert_eq!(stepped(&alone, 0.0, None, Step::Next), None);
    }
}

// SPDX-License-Identifier: MIT

//! A list of notes, newest first: the feed, and the notes on a profile.
//!
//! A list is read in one of two ways, [`FeedMode`]: the notes that start a
//! conversation, or those and the replies among them. Both are kept in
//! order, so that changing the way is showing the other.
//!
//! What a list shows only changes when its reader says so. Notes that turn
//! up after it was first filled wait beside it, as [`Pending`], until they
//! are asked for.
//!
//! ```mermaid
//! flowchart TD
//!     page["A look at the relays brings the newest page"] --> fresh{"Any note the list doesn't have?"}
//!     fresh -->|"no"| nothing["Nothing waits"]
//!     fresh -->|"yes"| reach{"Does the page reach back to the list's newest note?"}
//!     reach -->|"yes, or the page was not full"| joined["They wait, joined to the list: shown, they go in above it"]
//!     reach -->|"no: more than a page turned up"| detached["They wait, detached: shown, the list starts over from them"]
//! ```
//!
//! A list has no holes: from its newest note to its oldest it has every
//! note the relays gave. Notes that wait and don't reach down to the list
//! would leave one, so showing them starts the list over, and what is
//! older comes back as it is scrolled to its end.
//!
//! How this fits with looking for new notes on a timer is in
//! [`crate::guide::flows`].

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use nostr_sdk::prelude::{EventId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::nostr::models::Note;

/// Which of a list's notes are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedMode {
    /// The notes that start a conversation: no replies.
    #[default]
    Notes,
    /// Those, and the replies among them: everything.
    Replies,
}

impl FeedMode {
    /// Both ways, in the order they are offered.
    pub const ALL: [Self; 2] = [Self::Notes, Self::Replies];
}

/// Which page of a feed was asked of the relays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedPage {
    /// The newest notes there are.
    Newest,
    /// The notes before the oldest one in the feed.
    Older,
}

/// The order of a feed: newest first, and by id among notes of the same
/// second, so the order is the same however the notes arrived.
fn feed_order(note: &Note) -> (Reverse<u64>, EventId) {
    (Reverse(note.created_at), note.id)
}

/// The `limit` newest of `notes`, newest first, each note once.
///
/// A page asked of several relays comes back as each relay's newest `limit`,
/// which reach different distances into the past. Keeping only the newest
/// `limit` of them all cuts the page where the busiest relay's answer ends, so
/// the next page, asked from that cut down, leaves no gap in any relay's
/// notes.
pub fn newest(notes: impl IntoIterator<Item = Note>, limit: usize) -> Vec<Note> {
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by_key(feed_order);
    notes.dedup_by_key(|note| note.id);
    notes.truncate(limit);
    notes
}

/// A feed's notes, newest first, each note once.
#[derive(Debug, Default)]
pub struct Feed {
    /// Every note.
    notes: Vec<Note>,
    /// Those of them that answer nothing, over again: a list shows a
    /// run of notes, and these are not one among the others.
    starters: Vec<Note>,
    ids: HashSet<EventId>,
}

impl Feed {
    /// The notes `mode` shows, newest first.
    pub fn notes(&self, mode: FeedMode) -> &[Note] {
        match mode {
            FeedMode::Notes => &self.starters,
            FeedMode::Replies => &self.notes,
        }
    }

    /// Every note, whichever way the feed is read.
    pub fn all(&self) -> &[Note] {
        &self.notes
    }

    /// How many notes there are, of every kind.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether there is no note at all.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// When the oldest note was written; where the next older page starts.
    pub fn oldest(&self) -> Option<u64> {
        self.notes.last().map(|note| note.created_at)
    }

    /// When the newest note was written.
    pub fn newest(&self) -> Option<u64> {
        self.notes.first().map(|note| note.created_at)
    }

    /// Whether the note `id` is among these.
    pub fn contains(&self, id: EventId) -> bool {
        self.ids.contains(&id)
    }

    /// Keep the `count` newest notes, and let go of the rest.
    fn truncate(&mut self, count: usize) {
        for dropped in self.notes.drain(count.min(self.notes.len())..) {
            self.ids.remove(&dropped.id);
        }
        let ids = &self.ids;
        self.starters.retain(|note| ids.contains(&note.id));
    }

    /// Show those of the notes already here that come again in `notes`
    /// with more of the notes they quote as they come now. Only for a list
    /// nobody can have been reading: a note that gains what it quotes gets
    /// taller. Returns the people the notes that changed are shown with.
    pub fn take_quotes_from(&mut self, notes: &[Note]) -> Vec<PublicKey> {
        let better: HashMap<EventId, &Note> = notes
            .iter()
            .filter(|note| self.ids.contains(&note.id))
            .filter(|note| self.notes.iter().any(|known| known.id == note.id && known.quoted_count() < note.quoted_count()))
            .map(|note| (note.id, note))
            .collect();
        for known in self.notes.iter_mut().chain(self.starters.iter_mut()) {
            if let Some(note) = better.get(&known.id) {
                known.quoted.clone_from(&note.quoted);
            }
        }
        let mut seen = HashSet::new();
        better.values().flat_map(|note| note.people()).filter(|person| seen.insert(*person)).collect()
    }

    /// Add `notes`, wherever in the order they belong, skipping those already
    /// here. Returns the people the notes that were new are shown with, their
    /// authors and those they mention, each once.
    pub fn merge(&mut self, notes: impl IntoIterator<Item = Note>) -> Vec<PublicKey> {
        let mut people = Vec::new();
        let mut seen = HashSet::new();
        let before = self.notes.len();
        for note in notes {
            if !self.ids.insert(note.id) {
                continue;
            }
            people.extend(note.people().filter(|person| seen.insert(*person)));
            if !note.is_reply() {
                self.starters.push(note.clone());
            }
            self.notes.push(note);
        }
        if self.notes.len() > before {
            self.notes.sort_by_key(feed_order);
            self.starters.sort_by_key(feed_order);
        }
        people
    }
}

/// How many notes wait to be shown at the most. Past that the oldest are
/// let go of: the applet runs for as long as the session does, and may be
/// left alone for all of it. Showing them asks what was made of each, so
/// it is not many more than are worth asking about at once.
const PENDING_MAX: usize = 200;

/// The notes that turned up for a list after it was first filled: newer
/// than its newest, or missing from among the others. They wait here until
/// the reader asks for them, so that what is being read stays as it is.
///
/// A list has no holes: from its newest note down to its oldest, it has
/// every note the relays gave. Notes that wait may not reach all the way
/// down to the list's newest (a page from the relays is only so long, and
/// there may have been more than a page since). They are then
/// [detached](Self::is_detached), and showing them starts the list over
/// from them rather than leave a hole behind.
#[derive(Debug, Default)]
pub struct Pending {
    /// The notes, newest first, without a hole among them.
    notes: Feed,
    /// There may be notes between these and the list's newest that are not
    /// here.
    detached: bool,
}

impl Pending {
    /// The notes that wait, as `mode` shows them, newest first.
    pub fn notes(&self, mode: FeedMode) -> &[Note] {
        self.notes.notes(mode)
    }

    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Whether there may be more notes between these and the list's newest
    /// than are here: there are then at least as many new notes as wait.
    pub fn is_detached(&self) -> bool {
        self.detached
    }

    /// Take in `page`, the newest notes the relays have for the list whose
    /// notes are `feed`, `full` if the relays may have had more than they
    /// gave. Those `feed` doesn't have wait from now on. Returns the people
    /// the ones that are new here are shown with.
    ///
    /// A page that is not full is everything there is. A full one reaches
    /// as far back as its oldest note, and whatever is known to be newer
    /// than that, it joins up with. If it stops short of the notes that
    /// already wait, those are let go of: the page is newer, and a hole
    /// between the two could not be shown.
    pub fn offer(&mut self, feed: &Feed, page: Vec<Note>, full: bool) -> Vec<PublicKey> {
        let reaches_back_to = page.iter().map(|note| note.created_at).min();
        let fresh: Vec<Note> = page.into_iter().filter(|note| !feed.contains(note.id)).collect();
        if fresh.is_empty() {
            return Vec::new();
        }
        let joins = |known: Option<u64>| !full || known.is_none_or(|known| reaches_back_to.is_some_and(|oldest| oldest <= known));
        if joins(feed.newest()) {
            self.detached = false;
        } else if !joins(self.notes.newest()) {
            self.notes = Feed::default();
            self.detached = true;
        } else if self.notes.is_empty() {
            self.detached = true;
        }
        let people = self.notes.merge(fresh);
        if self.notes.len() > PENDING_MAX {
            self.notes.truncate(PENDING_MAX);
            self.detached = true;
        }
        people
    }

    /// Have `note` wait with the rest: one the reader wrote, on its way to
    /// being shown with them.
    pub fn add(&mut self, note: Note) {
        self.notes.merge([note]);
    }

    /// Stop waiting: the notes, newest first, and whether they were
    /// detached from the list they are for.
    pub fn take(&mut self) -> (Vec<Note>, bool) {
        let taken = std::mem::take(self);
        (taken.notes.notes, taken.detached)
    }
}

#[cfg(test)]
mod tests {
    use super::{Feed, FeedMode, PENDING_MAX, Pending, newest};
    use crate::nostr::content;
    use crate::nostr::models::{Note, ReplyTo};
    use nostr_sdk::prelude::{EventId, Keys, Kind, PublicKey, ToBech32};

    fn author() -> PublicKey {
        Keys::generate().public_key()
    }

    /// A note whose id is `id` repeated, written at `created_at`.
    fn note(id: u8, created_at: u64, author: PublicKey) -> Note {
        Note {
            id: EventId::from_byte_array([id; 32]),
            kind: Kind::TextNote,
            author,
            created_at,
            content: content::parse(&format!("note {id}"), 100),
            reply_to: None,
            quoted: Vec::new(),
        }
    }

    fn ids(notes: &[Note]) -> Vec<u8> {
        notes.iter().map(|note| note.id.as_bytes()[0]).collect()
    }

    #[test]
    fn a_note_that_comes_again_with_what_it_quotes_is_shown_with_it() {
        let (a, b) = (author(), author());
        let mut feed = Feed::default();
        let answer = Note {
            reply_to: Some(ReplyTo { root: EventId::from_byte_array([9; 32]), parent: EventId::from_byte_array([9; 32]) }),
            ..note(2, 20, a)
        };
        feed.merge([note(1, 10, a), answer.clone(), note(3, 30, a)]);

        // The same notes from the relays: the first with the note it
        // quotes, the others as they were; and one the feed doesn't have.
        let quoting = Note { quoted: vec![note(7, 5, b)], ..note(1, 10, a) };
        let people = feed.take_quotes_from(&[
            quoting.clone(),
            answer,
            note(3, 30, a),
            Note { quoted: vec![note(8, 6, b)], ..note(4, 40, a) },
        ]);

        assert_eq!(people, [a, b], "whoever the note that changed is shown with");
        assert_eq!(ids(feed.all()), [3, 2, 1], "nothing is added, and nothing moves");
        for shown in [feed.notes(FeedMode::Replies), feed.notes(FeedMode::Notes)] {
            assert_eq!(shown.iter().find(|note| note.id == quoting.id), Some(&quoting), "in every way the list is read");
        }

        // A copy that has no more to show changes nothing, and nobody.
        assert!(feed.take_quotes_from(&[quoting, note(3, 30, a)]).is_empty());
        assert!(feed.take_quotes_from(&[note(1, 10, a)]).is_empty(), "nor does one with less take anything away");
        assert_eq!(feed.all().iter().map(|note| note.quoted.len()).collect::<Vec<_>>(), [0, 0, 1]);
    }

    #[test]
    fn a_page_is_its_newest_notes_each_once() {
        let a = author();
        // Two relays answer: one busy (recent notes), one quiet (older ones),
        // and both know note 5.
        let busy = [note(5, 500, a), note(4, 400, a), note(3, 300, a)];
        let quiet = [note(5, 500, a), note(2, 200, a), note(1, 100, a)];

        let page = newest(busy.into_iter().chain(quiet), 3);

        // Cut where the busy relay's answer ends: 2 and 1 wait for the next page.
        assert_eq!(ids(&page), [5, 4, 3]);
    }

    #[test]
    fn notes_of_the_same_second_keep_one_order() {
        let a = author();
        let one_way = newest([note(1, 100, a), note(2, 100, a), note(3, 100, a)], 10);
        let another = newest([note(3, 100, a), note(1, 100, a), note(2, 100, a)], 10);
        assert_eq!(ids(&one_way), ids(&another));
    }

    #[test]
    fn merging_keeps_the_feed_newest_first_and_skips_known_notes() {
        let (a, b) = (author(), author());
        let mut feed = Feed::default();

        assert_eq!(feed.merge([note(2, 200, a), note(4, 400, a)]), [a]);
        assert_eq!(ids(feed.notes(FeedMode::Replies)), [4, 2]);

        // An older page and a newer note arrive together; note 2 is known.
        let new_authors = feed.merge([note(1, 100, b), note(2, 200, a), note(5, 500, b)]);
        assert_eq!(ids(feed.notes(FeedMode::Replies)), [5, 4, 2, 1]);
        assert_eq!(new_authors, [b], "only the authors of new notes, each once");
        assert_eq!(feed.oldest(), Some(100));
        assert_eq!(feed.len(), 4);
    }

    #[test]
    fn merging_reports_the_people_new_notes_mention_too() {
        let (a, b, mentioned) = (author(), author(), author());
        let mut feed = Feed::default();
        let mut reply = note(1, 100, a);
        reply.content =
            content::parse(&format!("hey nostr:{} and nostr:{}", mentioned.to_bech32().unwrap(), b.to_bech32().unwrap()), 100);

        assert_eq!(feed.merge([reply, note(2, 200, b)]), [a, mentioned, b], "each person once, in the order met");
    }

    #[test]
    fn replies_show_among_the_notes_or_not_at_all() {
        let a = author();
        let mut feed = Feed::default();
        let answer = |id: u8, created_at: u64, to: u8| {
            let parent = EventId::from_byte_array([to; 32]);
            Note { reply_to: Some(ReplyTo { root: parent, parent }), ..note(id, created_at, a) }
        };

        feed.merge([note(1, 100, a), answer(2, 200, 1), note(3, 300, a)]);
        // An older page, with a reply and a note in it.
        feed.merge([answer(4, 50, 1), note(5, 40, a)]);

        assert_eq!(ids(feed.notes(FeedMode::Replies)), [3, 2, 1, 4, 5], "everything, in order");
        assert_eq!(ids(feed.notes(FeedMode::Notes)), [3, 1, 5], "what starts a conversation, in order");
        assert_eq!(ids(feed.all()), [3, 2, 1, 4, 5]);
        assert_eq!((feed.len(), feed.oldest()), (5, Some(40)));

        // A comment is a reply whatever it is about, even something that
        // can't be named as a note is.
        feed.merge([Note { kind: Kind::Comment, ..note(6, 400, a) }]);
        assert_eq!(ids(feed.notes(FeedMode::Replies)), [6, 3, 2, 1, 4, 5]);
        assert_eq!(ids(feed.notes(FeedMode::Notes)), [3, 1, 5]);
    }

    #[test]
    fn merging_nothing_new_reports_no_authors() {
        let a = author();
        let mut feed = Feed::default();
        feed.merge([note(1, 100, a)]);
        assert!(feed.merge([note(1, 100, a)]).is_empty());
        assert!(Feed::default().oldest().is_none());
    }

    /// A feed of the notes `ids`, each written at ten times its id.
    fn feed_of(ids: impl IntoIterator<Item = u8>, author: PublicKey) -> Feed {
        let mut feed = Feed::default();
        feed.merge(ids.into_iter().map(|id| note(id, u64::from(id) * 10, author)));
        feed
    }

    /// The notes `ids` as a page from the relays, each written at ten times
    /// its id.
    fn page(ids: impl IntoIterator<Item = u8>, author: PublicKey) -> Vec<Note> {
        ids.into_iter().map(|id| note(id, u64::from(id) * 10, author)).collect()
    }

    #[test]
    fn notes_a_list_does_not_have_wait_and_the_list_stays_as_it_is() {
        let a = author();
        let feed = feed_of([1, 2, 3], a);
        let mut pending = Pending::default();

        // The relays' newest page: two notes the feed has, two it hasn't.
        let people = pending.offer(&feed, page([2, 3, 4, 5], a), false);

        assert_eq!(ids(pending.notes(FeedMode::Replies)), [5, 4]);
        assert_eq!(people, [a]);
        assert!(!pending.is_detached());
        assert_eq!(ids(feed.all()), [3, 2, 1], "nothing was added to the feed");

        // The same page again brings nothing new, and one more note joins.
        assert!(pending.offer(&feed, page([2, 3, 4, 5], a), false).is_empty());
        pending.offer(&feed, page([4, 5, 6], a), false);
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [6, 5, 4]);

        let (notes, detached) = pending.take();
        assert_eq!((ids(&notes), detached), (vec![6, 5, 4], false));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_note_that_arrives_late_waits_too() {
        let a = author();
        // The feed has 1, 3 and 5; a relay that was behind now has 2.
        let feed = feed_of([1, 3, 5], a);
        let mut pending = Pending::default();
        pending.offer(&feed, page([1, 2, 3, 5], a), false);
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [2]);
        assert!(!pending.is_detached());
    }

    #[test]
    fn a_full_page_that_stops_short_of_the_list_is_detached_from_it() {
        let a = author();
        let feed = feed_of([1, 2, 3], a);
        let mut pending = Pending::default();

        // A full page of notes all newer than the feed's newest: there may
        // have been more in between.
        pending.offer(&feed, page([7, 8, 9], a), true);
        assert!(pending.is_detached());
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [9, 8, 7]);

        // The next full page reaches back among those that wait: it joins
        // them, and they are still short of the feed.
        pending.offer(&feed, page([9, 10, 11], a), true);
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [11, 10, 9, 8, 7]);
        assert!(pending.is_detached());

        // One that stops short of those that wait takes their place.
        pending.offer(&feed, page([20, 21], a), true);
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [21, 20]);
        assert!(pending.is_detached());
    }

    #[test]
    fn a_page_that_reaches_the_list_joins_what_waits_to_it() {
        let a = author();
        let feed = feed_of([1, 2, 3], a);
        let mut pending = Pending::default();

        // Full, and its oldest note is one the feed has: no hole.
        pending.offer(&feed, page([3, 4, 5], a), true);
        assert!(!pending.is_detached());

        // Detached, and then a page that is all there is: joined up again.
        pending.offer(&feed, page([8, 9], a), true);
        assert!(pending.is_detached());
        pending.offer(&feed, page([1, 2, 3, 4, 5, 6, 7, 8, 9], a), false);
        assert!(!pending.is_detached());
        assert_eq!(ids(pending.notes(FeedMode::Replies)), [9, 8, 7, 6, 5, 4]);
    }

    #[test]
    fn what_waits_is_counted_as_the_list_is_read() {
        let a = author();
        let feed = feed_of([1], a);
        let mut pending = Pending::default();
        let parent = EventId::from_byte_array([1; 32]);
        let answer = Note { reply_to: Some(ReplyTo { root: parent, parent }), ..note(3, 30, a) };

        pending.offer(&feed, vec![note(2, 20, a), answer, note(4, 40, a)], false);

        assert_eq!(ids(pending.notes(FeedMode::Replies)), [4, 3, 2]);
        assert_eq!(ids(pending.notes(FeedMode::Notes)), [4, 2]);
    }

    #[test]
    fn only_so_many_notes_wait() {
        let a = author();
        let feed = Feed::default();
        let mut pending = Pending::default();
        let many = (0..PENDING_MAX as u64 + 20).map(|at| {
            let mut bytes = [0; 32];
            bytes[..8].copy_from_slice(&at.to_be_bytes());
            Note { id: EventId::from_byte_array(bytes), ..note(0, 1_000 + at, a) }
        });

        pending.offer(&feed, many.collect(), false);

        let waiting = pending.notes(FeedMode::Replies);
        assert_eq!(waiting.len(), PENDING_MAX);
        assert_eq!(waiting.first().map(|note| note.created_at), Some(1_000 + PENDING_MAX as u64 + 19), "the newest stay");
        assert!(pending.is_detached(), "the oldest were let go of: a hole");
        assert_eq!(pending.notes(FeedMode::Notes).len(), PENDING_MAX);
    }

    #[test]
    fn a_note_of_ones_own_waits_with_the_rest() {
        let a = author();
        let mut pending = Pending::default();
        pending.offer(&feed_of([1], a), page([2], a), false);
        pending.add(note(3, 30, a));
        assert_eq!(ids(&pending.take().0), [3, 2]);
    }
}

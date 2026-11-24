// SPDX-License-Identifier: MIT

//! A conversation: a note, what it answers, and what answers it.
//!
//! A thread is opened for one note, the *focused* one, and gathers the other
//! notes of its conversation as they are found: in the event store first,
//! on the relays after. However many of them are known, they are shown as
//! one list, in reading order: every note under the one it answers, and the
//! answers to one note oldest first.
//!
//! Notes written in the order of their numbers, each answering the one
//! its arrow comes from:
//!
//! ```mermaid
//! flowchart TD
//!     n1["1: the note that started it"] --> n2["2"]
//!     n1 --> n4["4"]
//!     n2 --> n3["3"]
//!     n2 --> n5["5"]
//!     n4 --> n6["6"]
//! ```
//!
//! are read as this list, each set in by how many answers deep it is:
//!
//! | Row | Note | Depth |
//! |---|---|---|
//! | 1 | 1 | 0 |
//! | 2 | 2 | 1 |
//! | 3 | 3 | 2 |
//! | 4 | 5 | 2 |
//! | 5 | 4 | 1 |
//! | 6 | 6 | 2 |
//!
//! A note whose parent is not among the notes known goes under the root
//! of its conversation, if that is known, and to the top if not. The
//! order is the same however the notes arrived: the list is put in order
//! anew each time notes are added ([`Thread::merge`]).
//!
//! Where a thread's notes come from, and which relays are asked, is in
//! [`crate::guide::flows`].

use std::collections::{HashMap, HashSet};

use nostr_sdk::prelude::{EventId, PublicKey};

use crate::nostr::models::Note;

/// One note of a thread, and how many notes up the conversation starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub note: Note,
    /// How many answers deep the note is: 0 for one that answers nothing
    /// the thread knows.
    pub depth: usize,
}

/// The notes of one conversation, in reading order.
#[derive(Debug)]
pub struct Thread {
    focused: EventId,
    rows: Vec<ThreadRow>,
}

/// The order of the answers to one note: oldest first, and by id among
/// notes of the same second, so the order is the same however they arrived.
fn reading_order(note: &Note) -> (u64, EventId) {
    (note.created_at, note.id)
}

/// `notes` in reading order: each under the note it answers.
///
/// A note whose parent isn't among them goes under the conversation's root
/// if that is, since it is still part of what the root started. Failing
/// both it stands at the top, among the notes that answer nothing.
fn arrange(mut notes: HashMap<EventId, Note>) -> Vec<ThreadRow> {
    let mut tops: Vec<EventId> = Vec::new();
    let mut answers: HashMap<EventId, Vec<EventId>> = HashMap::new();
    for note in notes.values() {
        let under = note
            .reply_to
            .and_then(|reply| [reply.parent, reply.root].into_iter().find(|id| *id != note.id && notes.contains_key(id)));
        match under {
            Some(parent) => answers.entry(parent).or_default().push(note.id),
            None => tops.push(note.id),
        }
    }
    // Newest first, because they are taken from the end of a stack.
    let newest_first = |ids: &mut Vec<EventId>| {
        ids.sort_by_key(|id| std::cmp::Reverse(notes.get(id).map(reading_order)));
    };
    newest_first(&mut tops);
    answers.values_mut().for_each(newest_first);

    let mut rows = Vec::with_capacity(notes.len());
    let mut next: Vec<(EventId, usize)> = tops.into_iter().map(|id| (id, 0)).collect();
    while let Some((id, depth)) = next.pop() {
        if let Some(below) = answers.remove(&id) {
            next.extend(below.into_iter().map(|id| (id, depth + 1)));
        }
        if let Some(note) = notes.remove(&id) {
            rows.push(ThreadRow { note, depth });
        }
    }
    rows
}

impl Thread {
    /// The thread of the note `focused`, which is all it holds if the note
    /// is at hand, and nothing if not.
    pub fn new(focused: EventId, note: Option<Note>) -> Self {
        let rows = note.filter(|note| note.id == focused).map(|note| ThreadRow { note, depth: 0 }).into_iter().collect();
        Self { focused, rows }
    }

    /// The note the thread was opened for.
    pub fn focused(&self) -> EventId {
        self.focused
    }

    /// The notes, in reading order.
    pub fn rows(&self) -> &[ThreadRow] {
        &self.rows
    }

    /// Whether the note `id` is one of the thread's.
    pub fn contains(&self, id: EventId) -> bool {
        self.rows.iter().any(|row| row.note.id == id)
    }

    /// Which row the focused note is, once it is known.
    pub fn focused_row(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.note.id == self.focused)
    }

    /// Whether a note is known that answers the focused one.
    pub fn has_replies(&self) -> bool {
        self.rows.iter().any(|row| row.note.reply_to.is_some_and(|reply| reply.parent == self.focused))
    }

    /// Add `notes`, each where it belongs. One that is already here is
    /// replaced: the same note, perhaps showing more of itself, or more of
    /// the notes it quotes. Returns the people that the notes that were
    /// new, or came with more of what they quote, are shown with: their
    /// authors and those they mention, each once.
    pub fn merge(&mut self, notes: impl IntoIterator<Item = Note>) -> Vec<PublicKey> {
        let mut known: HashMap<EventId, Note> =
            std::mem::take(&mut self.rows).into_iter().map(|row| (row.note.id, row.note)).collect();
        let mut people = Vec::new();
        let mut seen = HashSet::new();
        for note in notes {
            if known.get(&note.id).is_none_or(|known| known.quoted_count() < note.quoted_count()) {
                people.extend(note.people().filter(|person| seen.insert(*person)));
            }
            known.insert(note.id, note);
        }
        self.rows = arrange(known);
        people
    }
}

#[cfg(test)]
mod tests {
    use super::Thread;
    use crate::nostr::content;
    use crate::nostr::models::{Note, ReplyTo};
    use nostr_sdk::prelude::{EventId, Keys, Kind, PublicKey};

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    /// The note `byte`, written at `created_at`, answering `parent` in the
    /// conversation `root` started (0 for neither).
    fn note(byte: u8, created_at: u64, root: u8, parent: u8, author: PublicKey) -> Note {
        Note {
            id: id(byte),
            kind: Kind::TextNote,
            author,
            created_at,
            content: content::parse(&format!("note {byte}"), 100),
            reply_to: (root != 0).then(|| ReplyTo { root: id(root), parent: id(parent) }),
            quoted: Vec::new(),
        }
    }

    /// Each row as (note, depth).
    fn shape(thread: &Thread) -> Vec<(u8, usize)> {
        thread.rows().iter().map(|row| (row.note.id.as_bytes()[0], row.depth)).collect()
    }

    #[test]
    fn every_note_sits_under_the_one_it_answers_oldest_answer_first() {
        let a = Keys::generate().public_key();
        let mut thread = Thread::new(id(3), None);
        // 1 started it. 2 and 4 answer 1; 3 answers 2; 5 answers 3. They
        // arrive in no order.
        thread.merge([
            note(5, 500, 1, 3, a),
            note(4, 400, 1, 1, a),
            note(1, 100, 0, 0, a),
            note(3, 300, 1, 2, a),
            note(2, 200, 1, 1, a),
        ]);

        assert_eq!(shape(&thread), [(1, 0), (2, 1), (3, 2), (5, 3), (4, 1)]);
        assert_eq!(thread.focused_row(), Some(2));
        assert!(thread.contains(id(5)) && !thread.contains(id(9)));
        assert!(thread.has_replies());
    }

    #[test]
    fn a_note_whose_parent_is_unknown_goes_under_the_root_or_else_to_the_top() {
        let a = Keys::generate().public_key();
        // 3 answers 2, which nobody has.
        let mut thread = Thread::new(id(3), None);
        thread.merge([note(3, 300, 1, 2, a), note(4, 400, 1, 3, a)]);
        assert_eq!(shape(&thread), [(3, 0), (4, 1)], "neither parent nor root: at the top");

        thread.merge([note(1, 100, 0, 0, a)]);
        assert_eq!(shape(&thread), [(1, 0), (3, 1), (4, 2)], "the root is known: under it");

        thread.merge([note(2, 200, 1, 1, a)]);
        assert_eq!(shape(&thread), [(1, 0), (2, 1), (3, 2), (4, 3)], "the parent turned up");
    }

    #[test]
    fn a_thread_starts_with_its_note_if_that_is_at_hand() {
        let a = Keys::generate().public_key();
        let thread = Thread::new(id(2), Some(note(2, 200, 1, 1, a)));
        assert_eq!(shape(&thread), [(2, 0)]);
        assert_eq!(thread.focused(), id(2));
        assert!(!thread.has_replies());

        let thread = Thread::new(id(2), None);
        assert!(thread.rows().is_empty());
        assert_eq!(thread.focused_row(), None);

        assert!(Thread::new(id(2), Some(note(9, 900, 0, 0, a))).rows().is_empty(), "another note is not its start");
    }

    #[test]
    fn merging_replaces_a_known_note_and_reports_only_the_people_of_new_ones() {
        let (a, b) = (Keys::generate().public_key(), Keys::generate().public_key());
        let mut thread = Thread::new(id(1), Some(note(1, 100, 0, 0, a)));

        let mut fuller = note(1, 100, 0, 0, a);
        fuller.content = content::parse("all of note 1", 100);
        let people = thread.merge([fuller.clone(), note(2, 200, 1, 1, b), note(3, 300, 1, 1, b)]);

        assert_eq!(people, [b], "the author of the new notes, once");
        assert_eq!(thread.rows()[0].note, fuller);
        assert_eq!(shape(&thread), [(1, 0), (2, 1), (3, 1)]);

        // A known note that comes again with a note it quotes: whoever
        // wrote that one is new to the thread.
        let c = Keys::generate().public_key();
        let mut quoting = fuller.clone();
        quoting.quoted = vec![note(9, 50, 0, 0, c)];
        assert_eq!(thread.merge([quoting.clone(), note(2, 200, 1, 1, b)]), [a, c]);
        assert_eq!(thread.rows()[0].note, quoting);
    }

    #[test]
    fn answers_of_the_same_second_keep_one_order() {
        let a = Keys::generate().public_key();
        let mut one_way = Thread::new(id(1), None);
        one_way.merge([note(1, 100, 0, 0, a), note(2, 200, 1, 1, a), note(3, 200, 1, 1, a)]);
        let mut another = Thread::new(id(1), None);
        another.merge([note(3, 200, 1, 1, a), note(2, 200, 1, 1, a), note(1, 100, 0, 0, a)]);
        assert_eq!(shape(&one_way), shape(&another));
        assert_eq!(shape(&one_way), [(1, 0), (2, 1), (3, 1)]);
    }
}

// SPDX-License-Identifier: MIT

//! The account's bookmarks (NIP-51, kind 10003): the notes it wants to find
//! again.
//!
//! There is one list for an account, and every new one replaces the last
//! everywhere, as with a contact list. It names notes in two places: in its
//! tags, for anyone to read, and in its content, which is a list of tags of
//! the same form, as JSON, that the account encrypted to itself (NIP-44).
//!
//! Avestruz keeps bookmarks in the second and nowhere else, and reads them
//! from the second and nowhere else: what is bookmarked here is nobody
//! else's to know, and what a list says to anyone is not a bookmark here.
//! The list's tags are carried from each list to the next as they are,
//! unread: they are for the clients that keep bookmarks there.
//!
//! This is the list Amethyst shows as "Bookmarks" (its "Old Bookmarks" is
//! kind 30001), and its private half is read and written here as it is
//! there, so that each finds what the other left: the content is the tags
//! as compact JSON, an `e` tag names a note by its id and may say after it
//! where the note is, a new bookmark goes after the others, and a list
//! that keeps nothing to itself has no content at all.
//!
//! A bookmark is one change to the newest list, with everything else in it
//! as it was: the other notes, and the articles, hashtags and links other
//! clients keep there.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph event["A bookmark list, kind 10003"]
//!         direction TB
//!         tags["tags: what is bookmarked for anyone to see"]
//!         content["content: a list of tags of the same form, as JSON, encrypted to the account (NIP-44)"]
//!     end
//!     tags -. "carried from each list to the next, unread" .-> next["The next list"]
//!     content -- "decrypted: takes the key" --> private["e tags: notes. a, t, r tags: what other clients keep"]
//!     private -- "the e tags" --> shown["The bookmarks shown here"]
//!     private -- "one tag more or less, re-encrypted" --> next
//! ```
//!
//! How a bookmark is made without losing what another client wrote
//! meanwhile is in [`crate::guide::flows`].

use std::collections::HashSet;
use std::fmt;

use nostr_sdk::prelude::{Event, EventBuilder, EventId, Kind, PublicKey, RelayUrl, Tag, Timestamp};

use crate::nostr::outbox::Whereabouts;

/// What a list keeps to itself, once it has been decrypted: tags, as the
/// list's own are.
pub type PrivateTags = Vec<Vec<String>>;

/// The tags `json` lists, which is what a bookmark list's content says once
/// it is decrypted.
///
/// The error says nothing of what was read: it is what the account keeps
/// to itself, and errors end up in logs.
pub fn decode(json: &str) -> Result<PrivateTags, String> {
    serde_json::from_str(json).map_err(|_| "what was decrypted is not a list of tags".to_string())
}

/// `tags` as the JSON a bookmark list's content is, before it is encrypted.
pub fn encode(tags: &PrivateTags) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string())
}

/// The note the tag `fields` bookmarks, if it bookmarks one: `["e", <id>, …]`.
fn note_named(fields: &[String]) -> Option<EventId> {
    match fields {
        [name, id, ..] if name == "e" => EventId::from_hex(id).ok(),
        _ => None,
    }
}

/// Where the tag `fields`, which bookmarks a note, says the note is: a
/// relay that has it and who wrote it, each if a field after the id says
/// (`["e", <id>, <relay>, <author>]`, either of the two left out, as
/// Amethyst writes it).
fn whereabouts_named(fields: &[String]) -> Whereabouts {
    let said = || fields.iter().skip(2);
    let relay = said().find_map(|field| RelayUrl::parse(field).ok());
    let author = said().find_map(|field| PublicKey::from_hex(field).ok());
    Whereabouts { author, relay }
}

/// Whether the tag `fields` bookmarks something that is not a note: an
/// article (`a`), a hashtag (`t`) or a link (`r`), which other clients
/// keep in the same list.
fn names_something_else(fields: &[String]) -> bool {
    fields.len() > 1 && fields.first().is_some_and(|name| ["a", "t", "r"].contains(&name.as_str()))
}

/// The tag that bookmarks `note`, which is at `hint` as far as is known.
fn naming(note: EventId, hint: Option<&RelayUrl>) -> Vec<String> {
    let mut fields = vec!["e".to_string(), note.to_hex()];
    fields.extend(hint.map(ToString::to_string));
    fields
}

/// An account's bookmarks, as its newest list has them: what the list
/// keeps to itself.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bookmarks {
    /// The list they were read from. `None` when the account has none.
    list: Option<Event>,
    /// What the list keeps to itself.
    private: PrivateTags,
    /// The notes bookmarked.
    notes: HashSet<EventId>,
}

/// What is kept private is not printed: whatever is `Debug` ends up in a
/// log sooner or later.
impl fmt::Debug for Bookmarks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bookmarks")
            .field("list", &self.list.as_ref().map(|list| list.id))
            .field("notes", &self.notes.len())
            .finish_non_exhaustive()
    }
}

impl Bookmarks {
    /// The bookmarks of `list`, which keeps `private` to itself.
    pub fn new(list: Option<Event>, private: PrivateTags) -> Self {
        let notes = private.iter().filter_map(|fields| note_named(fields)).collect();
        Self { list, private, notes }
    }

    /// The list they were read from, if the account has one.
    pub fn list(&self) -> Option<&Event> {
        self.list.as_ref()
    }

    /// Whether `note` is bookmarked.
    pub fn has(&self, note: &EventId) -> bool {
        self.notes.contains(note)
    }

    /// How many notes are bookmarked.
    pub fn count(&self) -> usize {
        self.notes.len()
    }

    /// The notes bookmarked, the one bookmarked last first, which is the
    /// list's order backwards: a new bookmark goes after the others. Each
    /// note once.
    pub fn notes(&self) -> Vec<EventId> {
        let mut seen = HashSet::new();
        let mut notes: Vec<EventId> =
            self.private.iter().filter_map(|fields| note_named(fields)).filter(|note| seen.insert(*note)).collect();
        notes.reverse();
        notes
    }

    /// Where the list says the bookmarked `note` is, for looking where the
    /// account's own relays don't have it: what the first bookmark of it
    /// that says anything says. The account's own word, as it wrote the
    /// list, or a client of its did.
    pub fn whereabouts(&self, note: &EventId) -> Whereabouts {
        let naming = self.private.iter().filter(|fields| note_named(fields).as_ref() == Some(note));
        naming.map(|fields| whereabouts_named(fields)).find(|said| *said != Whereabouts::default()).unwrap_or_default()
    }

    /// How many things that are not notes are bookmarked.
    pub fn others(&self) -> usize {
        self.private.iter().filter(|fields| names_something_else(fields)).count()
    }

    /// What the list keeps to itself with `note` bookmarked (`bookmark`),
    /// or with no bookmark of it left, and otherwise as it is. `hint` is a
    /// relay that has the note, for whoever reads the list to look on.
    ///
    /// `None` when there is nothing to change: `note` is bookmarked
    /// already, or already isn't.
    pub fn changed(&self, note: EventId, hint: Option<&RelayUrl>, bookmark: bool) -> Option<PrivateTags> {
        if self.has(&note) == bookmark {
            return None;
        }
        let mut private = self.private.clone();
        if bookmark {
            private.push(naming(note, hint));
        } else {
            private.retain(|fields| note_named(fields) != Some(note));
        }
        Some(private)
    }
}

/// The bookmark list that replaces `earlier`, if there is one to replace:
/// with its tags, as they are, and `content` for what it keeps to itself,
/// written at `now` or, if `earlier` claims to be from later, after it: it
/// has to be the newer of the two.
pub fn list(earlier: Option<&Event>, content: String, now: Timestamp) -> EventBuilder {
    let tags: Vec<Tag> = earlier.map(|earlier| earlier.tags.iter().cloned().collect()).unwrap_or_default();
    let after = earlier.map_or(now, |earlier| now.max(Timestamp::from_secs(earlier.created_at.as_secs() + 1)));
    EventBuilder::new(Kind::Bookmarks, content).tags(tags).custom_created_at(after)
}

#[cfg(test)]
mod tests {
    use super::{Bookmarks, PrivateTags, decode, encode, list};
    use crate::nostr::outbox::Whereabouts;
    use nostr_sdk::prelude::{Event, EventBuilder, EventId, FinalizeEvent, Keys, Kind, RelayUrl, Tag, Timestamp};

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    fn e(note: EventId) -> Vec<String> {
        vec!["e".to_string(), note.to_hex()]
    }

    fn tag(fields: &[&str]) -> Tag {
        Tag::parse(fields.iter().copied()).unwrap()
    }

    /// A bookmark list with `tags`, written at `at`. Its content stands for
    /// what is encrypted in a real one.
    fn a_list(tags: Vec<Tag>, at: u64) -> Event {
        EventBuilder::new(Kind::Bookmarks, "encrypted")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn fields(tags: &[Tag]) -> Vec<Vec<String>> {
        tags.iter().map(|tag| tag.as_slice().to_vec()).collect()
    }

    #[test]
    fn what_a_list_keeps_to_itself_is_tags_as_compact_json() {
        // As Amethyst writes it: no spaces, a relay after the id or not.
        let json = format!(r#"[["e","{}"],["e","{}","wss://relay.example.com/"],["a","30023:abc:post"]]"#, id(1), id(2));
        let tags = decode(&json).unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[1], ["e", &id(2).to_hex(), "wss://relay.example.com/"]);
        assert_eq!(encode(&tags), json, "and it is written as it was read");

        assert_eq!(decode("[]"), Ok(PrivateTags::new()));
        // What isn't tags is refused, without a word of it in the reason.
        for not_tags in ["", "{}", "[1]", r#"[["e",1]]"#, r#"["secret words"]"#] {
            let error = decode(not_tags).unwrap_err();
            assert!(!error.contains("secret"), "{error}");
        }
    }

    #[test]
    fn only_a_note_the_list_keeps_to_itself_is_bookmarked() {
        // The first is in the list for anyone to read, where other clients
        // keep bookmarks; the second is kept private.
        let public = a_list(vec![Tag::event(id(1)), tag(&["t", "nostr"])], 100);
        let private =
            vec![e(id(2)), vec!["a".into(), "30023:abc:post".into()], vec!["e".into()], vec!["e".into(), "no id".into()]];
        let bookmarks = Bookmarks::new(Some(public), private);
        assert!(!bookmarks.has(&id(1)));
        assert!(bookmarks.has(&id(2)));
        assert_eq!(bookmarks.count(), 1);
        assert_eq!(bookmarks.notes(), [id(2)]);

        let none = Bookmarks::default();
        assert!(!none.has(&id(1)));
        assert_eq!(none.list(), None);
    }

    #[test]
    fn the_notes_are_listed_from_the_last_bookmarked_each_once() {
        let article = vec!["a".to_string(), "30023:abc:post".to_string()];
        let link = vec!["r".to_string(), "https://example.com".to_string()];
        let public = a_list(vec![Tag::event(id(1)), tag(&["t", "nostr"]), tag(&["a", "30023:abc:other"])], 100);
        let bookmarks = Bookmarks::new(Some(public), vec![e(id(3)), article, e(id(4)), link, e(id(2)), e(id(3))]);

        assert_eq!(bookmarks.notes(), [id(2), id(4), id(3)], "a note named twice is where it was named first");
        // An article and a link: what the list says to anyone is not counted.
        assert_eq!(bookmarks.others(), 2);

        let none = Bookmarks::default();
        assert!(none.notes().is_empty());
        assert_eq!(none.others(), 0);
    }

    #[test]
    fn a_bookmark_is_added_to_what_is_kept_private_after_the_others() {
        let tags = vec![Tag::event(id(1)), tag(&["title", "Mine"])];
        let private = vec![e(id(2)), vec!["t".to_string(), "nostr".to_string()]];
        let bookmarks = Bookmarks::new(Some(a_list(tags, 100)), private.clone());

        let changed = bookmarks.changed(id(3), None, true).unwrap();
        assert_eq!(changed, [private[0].clone(), private[1].clone(), e(id(3))]);

        // With where the note is, for whoever reads the list.
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        let hinted = bookmarks.changed(id(3), Some(&relay), true).unwrap();
        assert_eq!(hinted.last().unwrap(), &["e".to_string(), id(3).to_hex(), relay.to_string()]);

        // Bookmarked already: nothing to change.
        assert!(bookmarks.changed(id(2), None, true).is_none());
        // In the list for anyone to read is not bookmarked here: it is
        // kept private as any other note is.
        assert_eq!(bookmarks.changed(id(1), None, true).unwrap().last(), Some(&e(id(1))));
    }

    #[test]
    fn a_bookmark_says_where_its_note_is_when_it_does() {
        let author = Keys::generate().public_key();
        let relay = "wss://relay.example.com";
        let private = vec![
            e(id(1)),
            vec!["e".into(), id(2).to_hex(), relay.into()],
            // The author alone, with no relay before it.
            vec!["e".into(), id(3).to_hex(), author.to_hex()],
            vec!["e".into(), id(4).to_hex(), "not a relay".into(), "not a key".into()],
            // Named twice: the one that says where is the one that counts.
            e(id(5)),
            vec!["e".into(), id(5).to_hex(), relay.into(), author.to_hex()],
        ];
        let bookmarks = Bookmarks::new(None, private);
        let relay = RelayUrl::parse(relay).unwrap();

        assert_eq!(bookmarks.whereabouts(&id(1)), Whereabouts::default());
        assert_eq!(bookmarks.whereabouts(&id(2)), Whereabouts { author: None, relay: Some(relay.clone()) });
        assert_eq!(bookmarks.whereabouts(&id(3)), Whereabouts { author: Some(author), relay: None });
        assert_eq!(bookmarks.whereabouts(&id(4)), Whereabouts::default());
        assert_eq!(bookmarks.whereabouts(&id(5)), Whereabouts { author: Some(author), relay: Some(relay) });
        assert_eq!(bookmarks.whereabouts(&id(6)), Whereabouts::default(), "not bookmarked");
    }

    #[test]
    fn the_first_bookmark_of_an_account_with_no_list_is_a_list_of_one() {
        assert_eq!(Bookmarks::default().changed(id(1), None, true), Some(vec![e(id(1))]));
        assert!(Bookmarks::default().changed(id(1), None, false).is_none(), "and there is nothing to take off");
    }

    #[test]
    fn taking_a_bookmark_off_takes_the_note_out_of_what_is_kept_private_and_nothing_else() {
        let hinted = vec!["e".to_string(), id(2).to_hex(), "wss://relay.example.com".to_string()];
        let link = vec!["r".to_string(), "https://example.com".to_string()];
        let private = vec![e(id(1)), hinted, e(id(3)), link, e(id(2))];
        let bookmarks = Bookmarks::new(Some(a_list(vec![Tag::event(id(4))], 100)), private.clone());

        // Named twice, once with a relay: both go.
        assert_eq!(bookmarks.changed(id(2), None, false).unwrap(), [private[0].clone(), private[2].clone(), private[3].clone()]);
        assert_eq!(bookmarks.changed(id(3), None, false).unwrap().len(), 4);

        // Not bookmarked, or only in the list for anyone to read: nothing
        // to change.
        assert!(bookmarks.changed(id(5), None, false).is_none());
        assert!(bookmarks.changed(id(4), None, false).is_none());
    }

    #[test]
    fn a_changed_list_is_a_newer_bookmark_list_with_the_tags_of_the_one_it_replaces() {
        let tags = vec![Tag::event(id(1)), tag(&["title", "Mine"]), tag(&["client", "Amethyst"])];
        let earlier = a_list(tags.clone(), 1_000);
        let build = |earlier: Option<&Event>, now: u64| {
            list(earlier, "kept private".to_string(), Timestamp::from_secs(now)).finalize(&Keys::generate()).unwrap()
        };
        let changed = build(Some(&earlier), 2_000);
        assert_eq!(changed.kind, Kind::Bookmarks);
        assert_eq!(changed.kind.as_u16(), 10_003);
        assert_eq!(changed.content, "kept private");
        assert_eq!(fields(&changed.tags.to_vec()), fields(&tags), "what it says to anyone is as it was");
        assert_eq!(changed.created_at.as_secs(), 2_000);

        // A list that claims to be from later is still replaced.
        assert_eq!(build(Some(&earlier), 500).created_at.as_secs(), 1_001);
        // The first list says nothing to anyone.
        let first = build(None, 500);
        assert!(first.tags.is_empty());
        assert_eq!(first.created_at.as_secs(), 500);
    }
}

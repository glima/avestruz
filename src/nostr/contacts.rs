// SPDX-License-Identifier: MIT

//! A contact list (kind 3): who an account follows.
//!
//! There is one list for an account, and every new one replaces the last
//! everywhere. Following or unfollowing someone is therefore the newest
//! list written again with one person more or less, and everything else in
//! it as it was: the others followed, whatever else their tags say, and the
//! content, where older clients keep their relays.

use nostr_sdk::prelude::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp};

/// The people `list` follows, each once, in the list's order.
pub fn followed(list: &Event) -> Vec<PublicKey> {
    let mut seen = std::collections::HashSet::new();
    list.tags.public_keys().filter(|followed| seen.insert(*followed)).collect()
}

/// Whether `tag` says `person` is followed.
fn follows(tag: &Tag, person: &PublicKey) -> bool {
    let fields = tag.as_slice();
    fields.first().map(String::as_str) == Some("p") && fields.get(1).is_some_and(|key| *key == person.to_hex())
}

/// The list that follows `person` (`follow`) or doesn't, and is otherwise
/// `list`, written at `now` or, if `list` claims to be from later, after
/// it: it has to be the newer of the two. With no list to start from, one
/// that follows `person` alone.
///
/// `None` when there is nothing to change: `person` is already followed,
/// or already isn't.
pub fn changed(list: Option<&Event>, person: PublicKey, follow: bool, now: Timestamp) -> Option<EventBuilder> {
    let is_followed = list.is_some_and(|list| list.tags.iter().any(|tag| follows(tag, &person)));
    if is_followed == follow {
        return None;
    }
    let mut tags: Vec<Tag> =
        list.map(|list| list.tags.iter().filter(|tag| !follows(tag, &person)).cloned().collect()).unwrap_or_default();
    if follow {
        tags.push(Tag::public_key(person));
    }
    let content = list.map_or("", |list| list.content.as_str());
    let after = list.map_or(now, |list| now.max(Timestamp::from_secs(list.created_at.as_secs() + 1)));
    Some(EventBuilder::new(Kind::ContactList, content).tags(tags).custom_created_at(after))
}

#[cfg(test)]
mod tests {
    use super::{changed, followed};
    use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, PublicKey, Tag, Timestamp};

    fn person() -> PublicKey {
        Keys::generate().public_key()
    }

    /// A contact list with `tags`, written at `at`.
    fn list(keys: &Keys, tags: Vec<Tag>, content: &str, at: u64) -> Event {
        EventBuilder::new(Kind::ContactList, content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn following_adds_one_person_and_keeps_everything_else() {
        let keys = Keys::generate();
        let (a, b, c) = (person(), person(), person());
        // With a relay and a pet name, as some clients write them, and a
        // tag that is not a person at all.
        let with_hints = Tag::parse(["p", &a.to_hex(), "wss://relay.example.com", "ana"]).unwrap();
        let hashtag = Tag::hashtag("birds");
        let old = list(
            &keys,
            vec![with_hints.clone(), hashtag.clone(), Tag::public_key(b)],
            r#"{"wss://relay.example.com":{}}"#,
            1_000,
        );

        let new = changed(Some(&old), c, true, Timestamp::from_secs(2_000)).unwrap().finalize(&keys).unwrap();

        assert_eq!(new.kind, Kind::ContactList);
        assert_eq!(new.tags.clone().to_vec(), vec![with_hints, hashtag, Tag::public_key(b), Tag::public_key(c)]);
        assert_eq!(new.content, old.content);
        assert_eq!(new.created_at.as_secs(), 2_000);
        assert_eq!(followed(&new), [a, b, c]);
    }

    #[test]
    fn unfollowing_takes_one_person_out_however_often_they_are_in() {
        let keys = Keys::generate();
        let (a, b) = (person(), person());
        let old = list(&keys, vec![Tag::public_key(a), Tag::public_key(b), Tag::public_key(a)], "", 1_000);

        let new = changed(Some(&old), a, false, Timestamp::from_secs(2_000)).unwrap().finalize(&keys).unwrap();

        assert_eq!(followed(&new), [b]);
    }

    #[test]
    fn nothing_is_written_when_nothing_would_change() {
        let keys = Keys::generate();
        let (a, b) = (person(), person());
        let old = list(&keys, vec![Tag::public_key(a)], "", 1_000);
        let now = Timestamp::from_secs(2_000);

        assert!(changed(Some(&old), a, true, now).is_none(), "already followed");
        assert!(changed(Some(&old), b, false, now).is_none(), "not followed anyway");
        assert!(changed(None, a, false, now).is_none(), "no list, nobody followed");
    }

    #[test]
    fn a_first_list_follows_the_one_person() {
        let keys = Keys::generate();
        let a = person();
        let new = changed(None, a, true, Timestamp::from_secs(2_000)).unwrap().finalize(&keys).unwrap();
        assert_eq!((followed(&new), new.content.as_str()), (vec![a], ""));
    }

    #[test]
    fn the_new_list_is_newer_than_one_that_claims_to_be_from_the_future() {
        let keys = Keys::generate();
        let old = list(&keys, Vec::new(), "", 5_000);
        let new = changed(Some(&old), person(), true, Timestamp::from_secs(2_000)).unwrap().finalize(&keys).unwrap();
        assert_eq!(new.created_at.as_secs(), 5_001);
    }

    #[test]
    fn a_list_names_each_person_once_and_may_name_nobody() {
        let keys = Keys::generate();
        let person = person();
        let twice = list(&keys, vec![Tag::public_key(person), Tag::public_key(person)], "", 1_000);
        assert_eq!(followed(&twice), [person]);
        assert!(followed(&list(&keys, Vec::new(), "", 3_000)).is_empty());
    }
}

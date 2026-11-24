// SPDX-License-Identifier: MIT

//! Searching for people and notes (NIP-50): what is asked of the relays
//! that search, and what is made of their answers.
//!
//! A search is a filter with the words in its `search` field, which only
//! some relays answer: the account's search relays (see
//! [`crate::nostr::relay_lists`]). Each sends what it found with the best
//! match first, as NIP-50 has it, and that order is all that says how
//! good a match is: the answers of several relays are taken a rank at a
//! time, the best of each first.
//!
//! What is typed may also name a person or a note outright, by its
//! address (NIP-19). That is no search: it is opened, and no relay is
//! told what was typed. A name at a domain (NIP-05) is asked of its
//! domain, and the address of a relay shows what that relay has. And a
//! secret key is none of these: it is sent nowhere.

use std::collections::HashSet;

use nostr_sdk::prelude::{Event, EventId, Filter, FromBech32, Kind, Nip19, PublicKey, RelayUrl};

use crate::nostr::models::{Note, Profile};
use crate::nostr::nip05;
use crate::nostr::outbox::Whereabouts;
use crate::nostr::relay_lists::typed_relay;

/// How many people, and how many notes, one search asks each relay for.
const PEOPLE_LIMIT: usize = 30;
const NOTES_LIMIT: usize = 50;

/// What is searched for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SearchFor {
    /// People, by what their profile says.
    #[default]
    People,
    /// Notes, by what they say.
    Notes,
}

/// What a search found, the best match first.
#[derive(Debug, Clone)]
pub enum Found {
    /// People, each with their profile.
    People(Vec<(PublicKey, Profile)>),
    /// Notes, each as a row shows it.
    Notes(Vec<Note>),
}

impl Found {
    /// What this is the answer to a search for.
    pub fn what(&self) -> SearchFor {
        match self {
            Self::People(_) => SearchFor::People,
            Self::Notes(_) => SearchFor::Notes,
        }
    }
}

/// What the words typed are: words to search for, or something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Nothing at all.
    Nothing,
    /// Words to search for.
    Words(String),
    /// The address of a person: an `npub` or an `nprofile`.
    Person(PublicKey),
    /// The address of a note, and where it says the note is: a `note` or
    /// an `nevent`.
    Note(EventId, Whereabouts),
    /// A name at a domain (NIP-05), in lower case: whose it is is for
    /// the domain to say.
    Name(String),
    /// The address of a relay, said in full: `wss://…`.
    Relay(RelayUrl),
    /// A secret: a key, or what leads to one. Not something to send to a
    /// relay.
    Secret,
}

/// What `typed` is: an address, with or without `nostr:` before it, a
/// secret (a key, or the address of a remote signer), the address of a
/// relay, a name at a domain, or words.
pub fn typed(typed: &str) -> Typed {
    let words = typed.trim();
    if words.is_empty() {
        return Typed::Nothing;
    }
    let address = words.strip_prefix("nostr:").unwrap_or(words);
    // A key that is mistyped is still a key: nothing that starts as one
    // is searched for.
    // Nor is the address of a remote signer, which has its secret in it.
    if ["nsec1", "ncryptsec1", "bunker://"].iter().any(|secret| address.starts_with(secret)) {
        return Typed::Secret;
    }
    match Nip19::from_bech32(address) {
        Ok(Nip19::Pubkey(person)) => Typed::Person(person),
        Ok(Nip19::Profile(profile)) => Typed::Person(profile.public_key),
        Ok(Nip19::EventId(note)) => Typed::Note(note, Whereabouts::default()),
        Ok(Nip19::Event(note)) => {
            Typed::Note(note.event_id, Whereabouts { author: note.author, relay: note.relays.into_iter().next() })
        }
        Ok(Nip19::Secret(_)) => Typed::Secret,
        Ok(Nip19::Coordinate(_)) | Err(_) => named(words),
    }
}

/// What `words`, which are no address of a person or a note, are: the
/// address of a relay, when they start as one; a name at a domain; or
/// words to search for.
fn named(words: &str) -> Typed {
    let relay = ["wss://", "ws://"].iter().any(|scheme| words.starts_with(scheme)).then(|| typed_relay(words)).flatten();
    match relay {
        Some(relay) => Typed::Relay(relay),
        None if nip05::is_name(words) => Typed::Name(words.to_lowercase()),
        None => Typed::Words(words.to_string()),
    }
}

/// The filter that asks a relay for the notes it got last: what it has,
/// for whoever wants to see what a relay is like.
pub fn latest_notes() -> Filter {
    Filter::new().kinds([Kind::TextNote, Kind::Comment]).limit(NOTES_LIMIT)
}

/// The filter that asks a relay that searches for what `words` find of
/// `what`.
pub fn filter(words: &str, what: SearchFor) -> Filter {
    match what {
        SearchFor::People => Filter::new().kind(Kind::Metadata).search(words).limit(PEOPLE_LIMIT),
        SearchFor::Notes => Filter::new().kinds([Kind::TextNote, Kind::Comment]).search(words).limit(NOTES_LIMIT),
    }
}

/// The answers of several relays as one: `answers` is what each relay
/// sent, the best match first, and the result takes the best of each,
/// then the second of each, and so on, every event once, as it first
/// came.
pub fn ranked(answers: Vec<Vec<Event>>) -> Vec<Event> {
    let longest = answers.iter().map(Vec::len).max().unwrap_or(0);
    let mut answers: Vec<_> = answers.into_iter().map(Vec::into_iter).collect();
    let mut seen = HashSet::new();
    let mut ranked = Vec::new();
    for _ in 0..longest {
        for answer in &mut answers {
            if let Some(event) = answer.next()
                && seen.insert(event.id)
            {
                ranked.push(event);
            }
        }
    }
    ranked
}

/// `people`, with those of them in `followed` first, and otherwise in the
/// order they were in. A relay ranks by what a profile says, which anyone
/// can say: the people an account follows are the ones it meant, when a
/// name is theirs and others' too.
pub fn followed_first(mut people: Vec<PublicKey>, followed: &HashSet<PublicKey>) -> Vec<PublicKey> {
    people.sort_by_key(|person| !followed.contains(person));
    people
}

#[cfg(test)]
mod tests {
    use super::{SearchFor, Typed, filter, followed_first, ranked, typed};
    use crate::nostr::outbox::Whereabouts;
    use nostr_sdk::prelude::{
        Event, EventBuilder, EventId, FinalizeEvent, Keys, Kind, Nip19Event, Nip19Profile, RelayUrl, ToBech32,
    };
    use std::collections::HashSet;

    #[test]
    fn an_address_is_opened_and_a_secret_key_goes_nowhere() {
        let keys = Keys::generate();
        let person = keys.public_key();
        let note = EventId::from_byte_array([7; 32]);
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();

        assert_eq!(typed("  "), Typed::Nothing);
        assert_eq!(typed(" tokens for friends "), Typed::Words("tokens for friends".to_string()));
        assert_eq!(typed(&person.to_bech32().unwrap()), Typed::Person(person));
        let nprofile = Nip19Profile::new(person, [relay.clone()]).to_bech32().unwrap();
        assert_eq!(typed(&format!("nostr:{nprofile}")), Typed::Person(person));
        assert_eq!(typed(&note.to_bech32().unwrap()), Typed::Note(note, Whereabouts::default()));
        let nevent = Nip19Event::new(note).author(person).relays([relay.clone()]).to_bech32().unwrap();
        assert_eq!(typed(&nevent), Typed::Note(note, Whereabouts { author: Some(person), relay: Some(relay) }));

        // A secret key, whole or mistyped, is not words to search for.
        let nsec = keys.secret_key().to_bech32().unwrap();
        assert_eq!(typed(&nsec), Typed::Secret);
        assert_eq!(typed(&nsec[..20]), Typed::Secret);
        assert_eq!(
            typed(
                "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p"
            ),
            Typed::Secret
        );
        assert_eq!(typed("bunker://abcdef?relay=wss://relay.example.com&secret=hush"), Typed::Secret);
        // A relay said in full is a relay, and a name at a domain a name.
        assert_eq!(typed("wss://relay.example.com"), Typed::Relay(RelayUrl::parse("wss://relay.example.com").unwrap()));
        assert_eq!(typed("Ana@Example.com"), Typed::Name("ana@example.com".to_string()));
        for words in ["relay.example.com", "wss://", "wss://two words", "ana@ example.com", "mail me @ana"] {
            assert_eq!(typed(words), Typed::Words(words.to_string()), "{words}");
        }
        // What only looks like an address is words.
        assert_eq!(typed("npub1notakey"), Typed::Words("npub1notakey".to_string()));
    }

    #[test]
    fn people_are_searched_by_their_profiles_and_notes_by_what_they_say() {
        let people = filter("karnage", SearchFor::People);
        assert_eq!(people.search.as_deref(), Some("karnage"));
        assert_eq!(people.kinds.unwrap().into_iter().collect::<Vec<_>>(), [Kind::Metadata]);
        let notes = filter("tokens for friends", SearchFor::Notes);
        assert_eq!(notes.search.as_deref(), Some("tokens for friends"));
        assert!(notes.kinds.unwrap().contains(&Kind::TextNote));
        assert_eq!(notes.limit, Some(50));
    }

    #[test]
    fn the_people_followed_come_before_the_others_who_go_by_the_same_name() {
        let people: Vec<_> = (0..5).map(|_| Keys::generate().public_key()).collect();
        let followed = HashSet::from([people[3], people[1]]);
        assert_eq!(followed_first(people.clone(), &followed), [people[1], people[3], people[0], people[2], people[4]]);
        assert_eq!(followed_first(people.clone(), &HashSet::new()), people);
    }

    #[test]
    fn the_answers_of_several_relays_are_taken_a_rank_at_a_time() {
        let keys = Keys::generate();
        let event = |text: &str| -> Event { EventBuilder::new(Kind::TextNote, text).finalize(&keys).unwrap() };
        let (a, b, c, d) = (event("a"), event("b"), event("c"), event("d"));
        // The second relay's best is the first's second, and it has more.
        let answers = vec![vec![a.clone(), b.clone()], vec![b.clone(), c.clone(), d.clone()], vec![]];
        let ids = |events: &[Event]| events.iter().map(|event| event.id).collect::<Vec<_>>();
        assert_eq!(ids(&ranked(answers)), [a.id, b.id, c.id, d.id]);
        assert!(ranked(Vec::new()).is_empty());
    }
}

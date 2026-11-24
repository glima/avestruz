// SPDX-License-Identifier: MIT

//! Where people's events are: their relay lists (NIP-65), and which of the
//! relays those name are worth asking.
//!
//! Someone's relay list (kind 10002) says where they write, which is where
//! their notes are to be read, and where they read, which is where what is
//! meant for them is to be sent. Reading the people an account follows from
//! the relays they write to, rather than from the account's own alone, is
//! what finds the notes of those who write elsewhere (the outbox model).
//!
//! Everyone names several relays and everyone's differ, so asking all of
//! them would be a connection to half the network. [`plan`] picks few: the
//! relays that between them have the most people, so many for each person
//! and so many in all, counting the account's own relays, which are asked
//! anyway, as the ones they are.
//!
//! What a relay list names is somebody else's word, as is the relay the
//! address of a note names, and is not connected to without a look: see
//! [`public`].
//!
//! ```mermaid
//! flowchart TD
//!     start["Everyone is short of so many relays, less those of the account's own they list"] --> best["Of the relays not yet taken: which helps the most people who are still short?"]
//!     best --> any{"Does any help anyone?"}
//!     any -->|"no"| done["Done"]
//!     any -->|"yes"| take["Take it. Everyone who lists it is short of one fewer"]
//!     take --> cap{"As many taken as may be?"}
//!     cap -->|"yes"| done
//!     cap -->|"no"| best
//! ```
//!
//! A worked example of a plan, and the rest of what is decided about
//! relays, is in [`crate::guide::relays`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use nostr_sdk::prelude::{Event, PublicKey, RelayMetadata, RelayUrl, RelayUrlScheme, nip65};

/// Which of the relays someone lists are meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listed {
    /// Those they write to: where their own events are.
    Write,
    /// Those they read: where events meant for them are sent, and so are.
    Read,
    /// Both: where a conversation of theirs is, their side of it and the
    /// others'.
    Either,
}

/// What is known of where a note is, beside that it may be on the
/// account's own relays: what whoever named the note said of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Whereabouts {
    /// Who wrote it: it is on the relays they write to.
    pub author: Option<PublicKey>,
    /// A relay said to have it.
    pub relay: Option<RelayUrl>,
}

/// The relays the relay list `list` names for `listed`, each once, in the
/// list's order. A relay named without saying which it is for is for both.
pub fn relays(list: &Event, listed: Listed) -> Vec<RelayUrl> {
    let mut seen = HashSet::new();
    nip65::extract_relay_list(list)
        .filter(|(_, marked)| match (listed, marked) {
            (Listed::Either, _) | (_, None) => true,
            (Listed::Write, Some(marked)) => *marked == RelayMetadata::Write,
            (Listed::Read, Some(marked)) => *marked == RelayMetadata::Read,
        })
        .map(|(relay, _)| relay)
        .filter(|relay| seen.insert(relay.clone()))
        .collect()
}

/// Whether `relay`, which somebody else names (in their relay list, or in
/// the address of a note), is one to connect to: on the open internet, and
/// encrypted.
///
/// Not a relay on this machine or this network, which a list could name to
/// have its readers knock on doors at home; not one without encryption,
/// which shows whoever is on the way what is asked; and not an onion
/// service, which is not reached from here.
pub fn public(relay: &RelayUrl) -> bool {
    relay.scheme() == RelayUrlScheme::Wss && !relay.is_localhost() && !relay.is_local_addr() && !relay.is_onion()
}

/// The relays to ask about the people in `listed`, beside the account's
/// own, and whom to ask each about: a relay is asked about everyone who
/// lists it.
///
/// `listed` is, for each person, the relays their events are on. Each
/// person is to be had from `per_person` relays, of which those in `home`,
/// the account's own, count: they are asked about everyone as it is. The
/// relay that helps the most people who are still short is taken first
/// (of several that help as many, the one more people list), then the
/// next, until nobody is short or `cap` relays are taken. A relay in
/// `avoid` is never taken.
///
/// The same people and lists always give the same relays, in the same
/// order.
pub fn plan(
    listed: &HashMap<PublicKey, Vec<RelayUrl>>,
    home: &HashSet<RelayUrl>,
    avoid: &HashSet<RelayUrl>,
    per_person: usize,
    cap: usize,
) -> Vec<(RelayUrl, Vec<PublicKey>)> {
    // Who lists each relay that could be taken, and how many relays each
    // person is still short of. Ordered, so that ties fall the same way
    // every time.
    let mut listing: BTreeMap<&RelayUrl, BTreeSet<PublicKey>> = BTreeMap::new();
    let mut short: HashMap<PublicKey, usize> = HashMap::new();
    for (person, relays) in listed {
        let at_home = relays.iter().filter(|relay| home.contains(relay)).count();
        short.insert(*person, per_person.saturating_sub(at_home));
        for relay in relays.iter().filter(|relay| !home.contains(relay) && !avoid.contains(relay)) {
            listing.entry(relay).or_default().insert(*person);
        }
    }

    let mut taken = Vec::new();
    while taken.len() < cap {
        let helps = |people: &BTreeSet<PublicKey>| {
            people.iter().filter(|person| short.get(person).is_some_and(|short| *short > 0)).count()
        };
        // The relay that helps the most people; of several that do, the
        // one the most people list, and of those the first.
        let worth = listing.iter().map(|(relay, people)| ((helps(people), people.len()), *relay));
        let best =
            worth.filter(|((helped, _), _)| *helped > 0).fold(
                None,
                |best: Option<((usize, usize), &RelayUrl)>, found| match best {
                    Some(best) if best.0 >= found.0 => Some(best),
                    _ => Some(found),
                },
            );
        let Some((_, relay)) = best else {
            break;
        };
        let people = listing.remove(relay).unwrap_or_default();
        for person in &people {
            if let Some(short) = short.get_mut(person) {
                *short = short.saturating_sub(1);
            }
        }
        taken.push((relay.clone(), people.into_iter().collect()));
    }
    taken
}

/// Which of the relays in the pool of other people's to take out of it.
/// `pooled` is each with how long it has gone unused. Those unused for
/// `keep` or longer go; and if more than `cap` are left, the ones used
/// longest ago go too, but none used within `busy`, which may be in the
/// middle of answering.
pub fn to_let_go(mut pooled: Vec<(Duration, RelayUrl)>, cap: usize, keep: Duration, busy: Duration) -> Vec<RelayUrl> {
    // The one used last first: those are the ones kept.
    pooled.sort();
    pooled
        .into_iter()
        .enumerate()
        .filter(|(place, (unused, _))| *unused >= keep || (*place >= cap && *unused >= busy))
        .map(|(_, (_, relay))| relay)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Listed, plan, public, relays, to_let_go};
    use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, PublicKey, RelayUrl, Tag};
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    fn relay(name: &str) -> RelayUrl {
        RelayUrl::parse(&format!("wss://{name}.example.com")).unwrap()
    }

    fn people(count: usize) -> Vec<PublicKey> {
        let mut people: Vec<PublicKey> = (0..count).map(|_| Keys::generate().public_key()).collect();
        people.sort();
        people
    }

    fn list(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::RelayList, "")
            .tags(tags.iter().map(|tag| Tag::parse(tag.iter().copied()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn names(relays: &[RelayUrl]) -> Vec<&str> {
        relays.iter().map(RelayUrl::as_str_without_trailing_slash).collect()
    }

    #[test]
    fn a_relay_list_says_where_someone_writes_and_where_they_read() {
        let named = list(&[
            &["r", "wss://both.example.com"],
            &["r", "wss://out.example.com/", "write"],
            &["r", "wss://in.example.com", "read"],
            // Named twice, not a relay, and not a relay tag at all.
            &["r", "wss://both.example.com/"],
            &["r", "https://example.com"],
            &["p", "wss://other.example.com"],
        ]);
        assert_eq!(names(&relays(&named, Listed::Write)), ["wss://both.example.com", "wss://out.example.com"]);
        assert_eq!(names(&relays(&named, Listed::Read)), ["wss://both.example.com", "wss://in.example.com"]);
        assert_eq!(
            names(&relays(&named, Listed::Either)),
            ["wss://both.example.com", "wss://out.example.com", "wss://in.example.com"]
        );
        assert!(relays(&list(&[]), Listed::Either).is_empty());
    }

    #[test]
    fn only_an_encrypted_relay_on_the_open_internet_is_connected_to_on_somebody_elses_word() {
        for open in ["wss://relay.example.com", "wss://relay.example.com:4443/path", "wss://8.8.8.8"] {
            assert!(public(&RelayUrl::parse(open).unwrap()), "{open}");
        }
        for closed in [
            "ws://relay.example.com",
            "wss://localhost",
            "wss://127.0.0.1:7777",
            "ws://127.0.0.1:7777",
            "wss://192.168.1.1",
            "wss://10.0.0.7:8080",
            "wss://[::1]",
            "wss://abcdefghijklmnop.onion",
        ] {
            assert!(!public(&RelayUrl::parse(closed).unwrap()), "{closed}");
        }
    }

    #[test]
    fn the_relays_that_have_the_most_people_are_taken_first() {
        let people = people(6);
        let (big, small, lone) = (relay("big"), relay("small"), relay("lone"));
        let mut listed: HashMap<PublicKey, Vec<RelayUrl>> = HashMap::new();
        for person in &people[..4] {
            listed.insert(*person, vec![big.clone()]);
        }
        listed.insert(people[4], vec![small.clone(), big.clone()]);
        listed.insert(people[5], vec![small.clone(), lone.clone()]);
        let (home, avoid) = (HashSet::new(), HashSet::new());

        // One relay each: the big one has five, and the small one the last.
        let taken = plan(&listed, &home, &avoid, 1, 10);
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0], (big.clone(), people[..5].to_vec()), "asked about everyone who lists it");
        assert_eq!(taken[1], (small.clone(), people[4..].to_vec()));

        // Two each, for those who list two.
        let taken = plan(&listed, &home, &avoid, 2, 10);
        assert_eq!(taken.iter().map(|(relay, _)| relay).collect::<Vec<_>>(), [&big, &small, &lone]);

        // No more relays than may be had, and the ones that help most.
        assert_eq!(plan(&listed, &home, &avoid, 2, 1), [(big.clone(), people[..5].to_vec())]);
        assert!(plan(&listed, &home, &avoid, 2, 0).is_empty());
        assert_eq!(plan(&listed, &home, &avoid, 2, 10), plan(&listed, &home, &avoid, 2, 10), "the same every time");
    }

    /// The example [`crate::guide::relays`] works through, so that the
    /// guide can't say one thing and the plan do another.
    #[test]
    fn the_plan_the_guide_works_through_is_the_plan_that_is_made() {
        let [ana, bia, caio, davi, eva] = people(5)[..] else {
            unreachable!("five people were asked for");
        };
        let [h, a, b, c, d, e, f] = ["h", "a", "b", "c", "d", "e", "f"].map(relay);
        let listed = HashMap::from([
            (ana, vec![h.clone(), a.clone()]),
            (bia, vec![a.clone(), b.clone()]),
            (caio, vec![a.clone(), c.clone()]),
            (davi, vec![b.clone(), d.clone()]),
            (eva, vec![h.clone(), e.clone(), f.clone()]),
        ]);
        let home = HashSet::from([h]);

        let taken = plan(&listed, &home, &HashSet::new(), 2, 20);
        let expected = [(a, vec![ana, bia, caio]), (b, vec![bia, davi]), (c, vec![caio]), (d, vec![davi]), (e, vec![eva])];
        assert_eq!(taken, expected, "f is not taken: Eva has her two");
    }

    #[test]
    fn relays_long_unused_are_let_go_of_and_the_least_used_when_there_are_too_many() {
        let unused = |minutes: u64, name: &str| (Duration::from_secs(60 * minutes), relay(name));
        let (keep, busy) = (Duration::from_secs(15 * 60), Duration::from_secs(60));
        let pooled = vec![unused(20, "stale"), unused(0, "now"), unused(5, "lately"), unused(9, "a-while")];

        // Room for all: only the one unused for too long goes.
        assert_eq!(to_let_go(pooled.clone(), 20, keep, busy), [relay("stale")]);
        // Room for two: the two used last stay.
        assert_eq!(to_let_go(pooled.clone(), 2, keep, busy), [relay("a-while"), relay("stale")]);
        // Room for none, and still the one in use stays.
        assert_eq!(to_let_go(pooled, 0, keep, busy), [relay("lately"), relay("a-while"), relay("stale")]);
        assert!(to_let_go(Vec::new(), 0, keep, busy).is_empty());
    }

    #[test]
    fn the_accounts_own_relays_count_and_are_not_taken_again() {
        let people = people(3);
        let (mine, theirs, other) = (relay("mine"), relay("theirs"), relay("other"));
        let listed = HashMap::from([
            // On one of mine and one of theirs: short of one, of two.
            (people[0], vec![mine.clone(), theirs.clone()]),
            // On mine alone: nothing to add.
            (people[1], vec![mine.clone()]),
            // On none of mine.
            (people[2], vec![other.clone(), theirs.clone()]),
        ]);
        let home = HashSet::from([mine.clone()]);

        let taken = plan(&listed, &home, &HashSet::new(), 2, 10);
        assert_eq!(taken, [(theirs.clone(), vec![people[0], people[2]]), (other.clone(), vec![people[2]])]);
        // With one relay each, whoever is on mine is had already.
        assert_eq!(plan(&listed, &home, &HashSet::new(), 1, 10), [(theirs.clone(), vec![people[0], people[2]])]);

        // A relay to avoid is passed over, and another takes its people
        // if another has them.
        let avoid = HashSet::from([theirs]);
        assert_eq!(plan(&listed, &home, &avoid, 2, 10), [(other, vec![people[2]])]);
        // Nobody listed, nothing to ask.
        assert!(plan(&HashMap::new(), &home, &avoid, 2, 10).is_empty());
    }
}

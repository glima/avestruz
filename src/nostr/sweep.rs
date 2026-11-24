// SPDX-License-Identifier: MIT

//! What the event store lets go of: the notes it has had for longer than
//! they are likely to be looked at again.
//!
//! Everything the relays answer with is kept in the store, which is why a
//! feed, a thread or a profile shows at once the second time. Nothing in
//! it was ever let go of, and it grew for as long as it was used. A sweep
//! takes out what is older than a given moment, by the date each event
//! says it was written: not by when it was last looked at, which the
//! store doesn't know.
//!
//! What is swept is what can be asked of the relays again, and is asked
//! again when it is looked at: other people's notes, comments, reposts,
//! reactions and zap receipts. What is kept, however old:
//!
//! - what the account wrote itself, of any kind;
//! - whatever there is one of for each person, since its date says
//!   nothing of whether it still holds: profiles, contact lists, relay
//!   lists and every other list (the replaceable and addressable kinds),
//!   whoever wrote them;
//! - the notes the account bookmarked.
//!
//! It goes through the store a page at a time, newest first from the
//! moment given, and takes each page's old events out in one go, so that
//! what the relays send meanwhile is written between two pages rather than
//! after the whole sweep.
//!
//! ```mermaid
//! flowchart TD
//!     event["An event written before the limit"] --> mine{"Did the account write it?"}
//!     mine -->|"yes"| stays["Stays"]
//!     mine -->|"no"| kind{"Is there one of it for each person: replaceable, or addressable?"}
//!     kind -->|"yes: a profile, a contact list, a relay list, an article"| stays
//!     kind -->|"no: a note, a comment, a repost, a reaction, a zap receipt"| bookmarked{"Did the account bookmark it?"}
//!     bookmarked -->|"yes"| stays
//!     bookmarked -->|"no"| goes["Goes"]
//! ```
//!
//! When a sweep runs, and what it waits for, is in
//! [`crate::handlers::store`] and [`crate::guide::flows`].

use std::collections::HashSet;

use nostr_sdk::prelude::{Event, EventId, Filter, NostrDatabase, PublicKey, Timestamp};

/// How many events are looked at, and at most taken out, at a time.
const PAGE: usize = 500;

/// Whether `event` stays in the store however old it is: `account` wrote
/// it, or bookmarked it, or it is of a kind there is one of for each
/// person.
fn stays(event: &Event, account: PublicKey, bookmarked: &HashSet<EventId>) -> bool {
    event.pubkey == account || event.kind.is_replaceable() || event.kind.is_addressable() || bookmarked.contains(&event.id)
}

/// Take out of `database` what was written at `before` or earlier and
/// doesn't stay (see [`stays`]). Returns how many events that was.
pub async fn sweep(
    database: &dyn NostrDatabase,
    account: PublicKey,
    bookmarked: &HashSet<EventId>,
    before: Timestamp,
) -> Result<usize, String> {
    let mut swept = 0;
    let mut until = before;
    loop {
        let page = database.query(Filter::new().until(until).limit(PAGE)).await.map_err(|e| e.to_string())?;
        let Some(oldest) = page.iter().map(|event| event.created_at).min() else {
            break;
        };
        let old: Vec<EventId> = page.iter().filter(|event| !stays(event, account, bookmarked)).map(|event| event.id).collect();
        let nothing_old = old.is_empty();
        if !nothing_old {
            swept += old.len();
            database.delete(Filter::new().ids(old)).await.map_err(|e| e.to_string())?;
        }
        if page.len() < PAGE {
            break;
        }
        // The next page starts at the second this one ended in: what was
        // cut off there is on it, after what stayed. A page that is all
        // of that one second and all staying says nothing new the second
        // time, and the second before it is gone on to.
        until = if nothing_old && oldest == until {
            match oldest.as_secs().checked_sub(1) {
                Some(earlier) => Timestamp::from_secs(earlier),
                None => break,
            }
        } else {
            oldest
        };
    }
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::{PAGE, sweep};
    use nostr_lmdb::NostrLmdb;
    use nostr_sdk::prelude::{
        Event, EventBuilder, EventId, Filter, FinalizeEvent, IntoNostrDatabase, Keys, Kind, NostrDatabase, Tag, Timestamp,
    };
    use std::collections::HashSet;
    use std::sync::Arc;

    /// An event of `kind` that `keys` wrote at `at`.
    fn event(keys: &Keys, kind: Kind, content: &str, at: u64) -> Event {
        EventBuilder::new(kind, content).custom_created_at(Timestamp::from_secs(at)).finalize(keys).unwrap()
    }

    /// Run `test` over an empty store, which is taken away after.
    fn with_store(name: &str, test: impl AsyncFnOnce(Arc<dyn NostrDatabase>)) {
        let store = std::env::temp_dir().join(format!("avestruz-test-{}-sweep-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&store);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            tokio::fs::create_dir_all(&store).await.unwrap();
            test(NostrLmdb::open(&store).await.unwrap().into_nostr_database()).await;
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&store);
    }

    async fn ids_left(database: &dyn NostrDatabase) -> HashSet<EventId> {
        database.query(Filter::new()).await.unwrap().into_iter().map(|event| event.id).collect()
    }

    #[test]
    fn what_is_old_and_can_be_asked_again_is_swept() {
        with_store("kinds", async |database| {
            let (me, other) = (Keys::generate(), Keys::generate());
            let (old, new) = (1_000, 2_000);
            let article = EventBuilder::new(Kind::LongFormTextNote, "an article")
                .tag(Tag::identifier("post"))
                .custom_created_at(Timestamp::from_secs(old))
                .finalize(&other)
                .unwrap();
            let stay = [
                event(&me, Kind::TextNote, "mine, old", old),
                event(&me, Kind::Reaction, "+", old),
                event(&other, Kind::Metadata, r#"{"name":"other"}"#, old),
                event(&other, Kind::ContactList, "", old),
                event(&other, Kind::RelayList, "", old),
                article,
                event(&other, Kind::TextNote, "bookmarked", old),
                event(&other, Kind::TextNote, "new", new),
                event(&other, Kind::Reaction, "+", new),
                event(&other, Kind::TextNote, "written the second after", 1_501),
            ];
            let go = [
                event(&other, Kind::TextNote, "old", old),
                event(&other, Kind::Comment, "an old comment", old),
                event(&other, Kind::Repost, "", old),
                event(&other, Kind::Reaction, "+", old),
                event(&other, Kind::ZapReceipt, "", old),
                event(&other, Kind::TextNote, "written that second", 1_500),
            ];
            for event in stay.iter().chain(&go) {
                database.save_event(event).await.unwrap();
            }
            let bookmarked = HashSet::from([stay[6].id]);

            let swept = sweep(database.as_ref(), me.public_key(), &bookmarked, Timestamp::from_secs(1_500)).await;
            assert_eq!(swept, Ok(go.len()));
            assert_eq!(ids_left(database.as_ref()).await, stay.iter().map(|event| event.id).collect());

            // A second sweep finds nothing, and what was swept can be
            // kept again when a relay sends it.
            assert_eq!(sweep(database.as_ref(), me.public_key(), &bookmarked, Timestamp::from_secs(1_500)).await, Ok(0));
            assert!(database.save_event(&go[0]).await.unwrap().is_success());
            assert_eq!(database.query(Filter::new().id(go[0].id)).await.unwrap().len(), 1);
        });
    }

    #[test]
    fn a_sweep_goes_through_more_than_a_page() {
        with_store("pages", async |database| {
            let (me, other) = (Keys::generate(), Keys::generate());
            // More of what stays in one second than a page holds, with
            // what goes in that second, after it and before it.
            let mut stay = Vec::new();
            let mut go = Vec::new();
            for nth in 0..PAGE + 20 {
                stay.push(event(&me, Kind::TextNote, &format!("mine {nth}"), 5_000));
            }
            for nth in 0..PAGE + 20 {
                let at = 4_000 + (nth as u64 % 7);
                go.push(event(&other, Kind::TextNote, &format!("theirs {nth}"), at));
                stay.push(event(&me, Kind::TextNote, &format!("mine too {nth}"), at));
            }
            for nth in 0..30 {
                go.push(event(&other, Kind::Reaction, &format!("{nth}"), 6_000));
                go.push(event(&other, Kind::Reaction, &format!("{nth}"), 10));
            }
            for event in stay.iter().chain(&go) {
                database.save_event(event).await.unwrap();
            }

            let swept = sweep(database.as_ref(), me.public_key(), &HashSet::new(), Timestamp::from_secs(7_000)).await;
            assert_eq!(swept, Ok(go.len()));
            assert_eq!(ids_left(database.as_ref()).await, stay.iter().map(|event| event.id).collect());
        });
    }
}

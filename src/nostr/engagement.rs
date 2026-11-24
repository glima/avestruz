// SPDX-License-Identifier: MIT

//! What others made of a note: how many answered it, how many passed it on,
//! how many reacted to it, and how much it was zapped; and what the
//! account itself left on it: a repost, a reaction.
//!
//! It is counted from the events that name the note: replies (kind 1),
//! reposts (kinds 6 and 16, NIP-18), reactions (kind 7, NIP-25) and zap
//! receipts (kind 9735, NIP-57). The count is of what the event store
//! holds, which is what the relays were willing to send: for a note that
//! went everywhere, less than everything.
//!
//! Who they were is read from the same events ([`who_did`]): who
//! reposted, who reacted and what with, who zapped and how much. So what
//! a count names and who is listed under it agree.

use std::collections::{HashMap, HashSet};

use nostr_sdk::prelude::{Event, EventBuilder, EventId, IntoEventBuilder, Kind, PublicKey, RelayUrl, RepostBuilder, Tag};

use crate::emoji;
use crate::nostr::models::reply_to;
use crate::nostr::zap::invoice_millisats;

/// What a reaction says (NIP-25): that the note is liked, or an emoji.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reaction {
    /// A like: `+`, or nothing at all.
    Like,
    /// An emoji, as it is written.
    Emoji(String),
}

impl Reaction {
    /// What a reaction that says `content` is, as it is shown. Anything
    /// that is not an emoji is a like: a `+`, nothing, and what this
    /// client has no picture for, as a custom emoji's short code.
    pub fn said(content: &str) -> Self {
        emoji::known(content.trim()).map_or(Self::Like, |emoji| Self::Emoji(emoji.to_string()))
    }

    /// What a reaction says to be this one.
    pub fn content(&self) -> &str {
        match self {
            Self::Like => "+",
            Self::Emoji(emoji) => emoji,
        }
    }
}

/// What others made of one note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Engagement {
    /// How many notes answer it directly.
    pub replies: usize,
    /// How many people reposted it.
    pub reposts: usize,
    /// Whether the account is one of them.
    pub reposted: bool,
    /// How many people reacted to it, with a like or with an emoji.
    pub likes: usize,
    /// What the account reacted to it with, if it did.
    pub reaction: Option<Reaction>,
    /// How much it was zapped, in sats.
    pub zap_sats: u64,
}

/// How some notes stand, as far as one source knows: what was made of each,
/// and which relays each was seen on, for those seen on any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Standing {
    pub engagement: HashMap<EventId, Engagement>,
    pub seen_on: HashMap<EventId, Vec<RelayUrl>>,
}

/// What the account leaves on a note, and once only: it shows at once,
/// and is published behind what shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Mark {
    /// A reaction (NIP-25): a like, or an emoji.
    React(Reaction),
    /// A repost (NIP-18).
    Repost,
}

impl Engagement {
    /// Whether the account left a mark of the kind of `mark` on the note:
    /// a repost, or a reaction, whatever it says.
    pub fn has(&self, mark: &Mark) -> bool {
        match mark {
            Mark::React(_) => self.reaction.is_some(),
            Mark::Repost => self.reposted,
        }
    }

    /// This, with the account's `mark` on the note: counted, unless one
    /// of its kind was there already, which then stays as it is.
    #[must_use]
    pub fn with(mut self, mark: &Mark) -> Self {
        if !self.has(mark) {
            match mark {
                Mark::React(reaction) => {
                    self.reaction = Some(reaction.clone());
                    self.likes += 1;
                }
                Mark::Repost => {
                    self.reposted = true;
                    self.reposts += 1;
                }
            }
        }
        self
    }

    /// This, without the account's mark of the kind of `mark` on the
    /// note: no longer counted, if it was.
    #[must_use]
    pub fn without(mut self, mark: &Mark) -> Self {
        if self.has(mark) {
            match mark {
                Mark::React(_) => {
                    self.reaction = None;
                    self.likes = self.likes.saturating_sub(1);
                }
                Mark::Repost => {
                    self.reposted = false;
                    self.reposts = self.reposts.saturating_sub(1);
                }
            }
        }
        self
    }
}

/// The note a reaction is to: the last one it names (NIP-25). `None` for a
/// downvote, which is not counted for or against.
fn reaction_target(event: &Event) -> Option<EventId> {
    if event.content == "-" {
        return None;
    }
    event.tags.event_ids().last()
}

/// The note a zap receipt is for, and what was paid, in millisats: the
/// amount of the invoice the receipt carries.
///
/// A receipt is taken at its word. Whether it comes from the wallet
/// service of the person zapped is not checked: that takes asking their
/// Lightning address, once for every author.
fn zap(event: &Event) -> Option<(EventId, u64)> {
    let invoice = event.tags.iter().find(|tag| tag.kind() == "bolt11")?.content()?;
    Some((event.tags.event_ids().next()?, invoice_millisats(invoice)?))
}

/// The zap request a zap receipt carries, if it carries one that is what
/// it says: signed by whoever it says paid.
pub(crate) fn zap_request(receipt: &Event) -> Option<Event> {
    let description = receipt.tags.iter().find(|tag| tag.kind() == "description")?.content()?;
    let request = Event::from_json(description).ok()?;
    (request.kind == Kind::ZapRequest && request.verify().is_ok()).then_some(request)
}

/// Which of what others left on a note: what one of its counts is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Left {
    /// The reposts.
    Reposts,
    /// The reactions: likes, and emoji.
    Reactions,
    /// The zaps.
    Zaps,
}

impl Left {
    /// The kinds of the events that are it.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Self::Reposts => &[Kind::Repost, Kind::GenericRepost],
            Self::Reactions => &[Kind::Reaction],
            Self::Zaps => &[Kind::ZapReceipt],
        }
    }
}

/// What one of them left on a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Reposted it.
    Reposted,
    /// Reacted to it, with this.
    Reacted(Reaction),
    /// Zapped it, so many sats in all.
    Zapped(u64),
}

/// Someone who left something on a note, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doer {
    /// Who. `None` for the zaps whose receipts don't say who paid, or say
    /// it in a way that does not hold: all of those are one.
    pub who: Option<PublicKey>,
    pub did: Did,
}

/// Who left `what` on `note`, going by `events`, each of them once: the
/// last first, for reposts and reactions, and whoever zapped most first.
/// Someone who reposted twice reposted once; of someone who reacted more
/// than once, it is the reaction made last; and what someone zapped in
/// several goes is added up. A downvote is no reaction, here as in the
/// count, and a zap of less than a sat is none.
///
/// A zap is whose its receipt says it is, by the request the receipt
/// carries, if that request is signed by them (NIP-57). Whether the
/// receipt itself comes from the wallet of the one zapped is not looked
/// into, as it is not for the count (see [`tally`]).
pub fn who_did(events: impl IntoIterator<Item = Event>, note: EventId, what: Left) -> Vec<Doer> {
    // Each of them, with when they last did it, or how much in all.
    let mut by: HashMap<Option<PublicKey>, (u64, Did)> = HashMap::new();
    for event in events.into_iter().filter(|event| what.kinds().contains(&event.kind)) {
        let at = event.created_at.as_secs();
        match what {
            Left::Reposts if event.tags.event_ids().next() == Some(note) => {
                let last = by.entry(Some(event.pubkey)).or_insert((at, Did::Reposted));
                last.0 = last.0.max(at);
            }
            Left::Reactions if reaction_target(&event) == Some(note) => {
                let last = by.entry(Some(event.pubkey)).or_insert((at, Did::Reacted(Reaction::said(&event.content))));
                if at >= last.0 {
                    *last = (at, Did::Reacted(Reaction::said(&event.content)));
                }
            }
            Left::Zaps => {
                let Some((_, millisats)) = zap(&event).filter(|(zapped, _)| *zapped == note) else {
                    continue;
                };
                let who = zap_request(&event).map(|request| request.pubkey);
                let paid = by.entry(who).or_insert((0, Did::Zapped(0)));
                paid.0 = paid.0.saturating_add(millisats);
            }
            _ => {}
        }
    }
    let mut doers: Vec<(Option<PublicKey>, u64, Did)> = by
        .into_iter()
        .map(|(who, (weight, did))| match did {
            Did::Zapped(_) => (who, weight, Did::Zapped(weight / 1000)),
            did => (who, weight, did),
        })
        .filter(|(_, _, did)| *did != Did::Zapped(0))
        .collect();
    // The most first; among equals, whoever is known before whoever is
    // not, and always in the same order.
    doers.sort_by(|(one, weight, _), (other, heavier, _)| {
        heavier.cmp(weight).then_with(|| one.is_none().cmp(&other.is_none())).then_with(|| one.cmp(other))
    });
    doers.into_iter().map(|(who, _, did)| Doer { who, did }).collect()
}

/// What `events` make of each of `notes`, with `account` the one whose own
/// reactions and reposts are told apart. Events about other notes, or of
/// other kinds, count for nothing; someone who reacted twice reacted once,
/// and someone who reposted twice reposted once. Of an account that
/// reacted more than once, elsewhere, the reaction made last is its own.
pub fn tally(
    events: impl IntoIterator<Item = Event>,
    notes: &HashSet<EventId>,
    account: Option<PublicKey>,
) -> HashMap<EventId, Engagement> {
    let mut tally: HashMap<EventId, Engagement> = notes.iter().map(|id| (*id, Engagement::default())).collect();
    let mut reacted: HashSet<(EventId, PublicKey)> = HashSet::new();
    let mut own_reaction_at: HashMap<EventId, u64> = HashMap::new();
    let mut reposted: HashSet<(EventId, PublicKey)> = HashSet::new();
    let mut millisats: HashMap<EventId, u64> = HashMap::new();
    for event in events {
        match event.kind {
            Kind::TextNote | Kind::Comment => {
                if let Some(engagement) = reply_to(&event).and_then(|reply| tally.get_mut(&reply.parent)) {
                    engagement.replies += 1;
                }
            }
            Kind::Reaction => {
                let Some(note) = reaction_target(&event) else {
                    continue;
                };
                let Some(engagement) = tally.get_mut(&note) else {
                    continue;
                };
                if reacted.insert((note, event.pubkey)) {
                    engagement.likes += 1;
                }
                let at = event.created_at.as_secs();
                if Some(event.pubkey) == account && own_reaction_at.get(&note).is_none_or(|known| at >= *known) {
                    own_reaction_at.insert(note, at);
                    engagement.reaction = Some(Reaction::said(&event.content));
                }
            }
            // A repost names the one note it passes on (NIP-18).
            Kind::Repost | Kind::GenericRepost => {
                let Some(note) = event.tags.event_ids().next() else {
                    continue;
                };
                if let Some(engagement) = tally.get_mut(&note)
                    && reposted.insert((note, event.pubkey))
                {
                    engagement.reposts += 1;
                    engagement.reposted |= Some(event.pubkey) == account;
                }
            }
            Kind::ZapReceipt => {
                if let Some((note, paid)) = zap(&event).filter(|(note, _)| notes.contains(note)) {
                    let total = millisats.entry(note).or_default();
                    *total = total.saturating_add(paid);
                }
            }
            _ => {}
        }
    }
    for (note, paid) in millisats {
        if let Some(engagement) = tally.get_mut(&note) {
            engagement.zap_sats = paid / 1000;
        }
    }
    tally
}

/// The reaction `reaction` to the event `id`, of kind `kind`, written by
/// `author`, yet to be signed (NIP-25).
pub fn react(id: EventId, kind: Kind, author: PublicKey, reaction: &Reaction) -> EventBuilder {
    EventBuilder::new(Kind::Reaction, reaction.content()).tags([
        Tag::event(id),
        Tag::public_key(author),
        Tag::custom("k", [kind.as_u16().to_string()]),
    ])
}

/// A repost of `event`, yet to be signed (NIP-18): of kind 6 for a text
/// note and of kind 16, naming the kind, for anything else. It carries the
/// event whole, so that it can be shown without being looked for, unless
/// its author asked that it go nowhere they didn't send it (NIP-70).
/// `relay` is one that has the event, for those who look anyway.
pub fn repost(event: &Event, relay: Option<RelayUrl>) -> EventBuilder {
    let repost = RepostBuilder::new(event);
    match relay {
        Some(relay) => repost.relay_url(relay),
        None => repost,
    }
    .into_event_builder()
}

#[cfg(test)]
mod tests {
    use super::{Did, Doer, Engagement, Left, Mark, Reaction, react, repost, tally, who_did};
    use nostr_sdk::prelude::{Event, EventBuilder, EventId, FinalizeEvent, Keys, Kind, RelayUrl, Tag, Timestamp};
    use std::collections::HashSet;

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    fn event(kind: Kind, content: &str, tags: Vec<Tag>, keys: &Keys) -> Event {
        EventBuilder::new(kind, content).tags(tags).finalize(keys).unwrap()
    }

    fn reaction(content: &str, note: EventId, keys: &Keys) -> Event {
        event(Kind::Reaction, content, vec![Tag::event(note)], keys)
    }

    fn reposting(note: EventId, kind: Kind, keys: &Keys) -> Event {
        event(kind, "", vec![Tag::event(note)], keys)
    }

    fn receipt(note: EventId, invoice: &str) -> Event {
        event(Kind::ZapReceipt, "", vec![Tag::event(note), Tag::custom("bolt11", [invoice])], &Keys::generate())
    }

    /// As [`event`], made at `at`.
    fn event_at(at: u64, kind: Kind, content: &str, tags: Vec<Tag>, keys: &Keys) -> Event {
        EventBuilder::new(kind, content).tags(tags).custom_created_at(Timestamp::from_secs(at)).finalize(keys).unwrap()
    }

    /// A receipt for `note` of what `invoice` asks for, which says that
    /// `payer` paid it, in a request signed by them.
    fn receipt_from(payer: &Keys, note: EventId, invoice: &str) -> Event {
        let request = event(Kind::ZapRequest, "", vec![Tag::event(note)], payer);
        let tags = vec![Tag::event(note), Tag::custom("bolt11", [invoice]), Tag::custom("description", [request.as_json()])];
        event(Kind::ZapReceipt, "", tags, &Keys::generate())
    }

    #[test]
    fn who_reposted_a_note_is_each_of_them_once_the_last_first() {
        let (first, second) = (Keys::generate(), Keys::generate());
        let (note, another) = (id(1), id(2));
        let events = vec![
            event_at(10, Kind::Repost, "", vec![Tag::event(note)], &first),
            event_at(20, Kind::GenericRepost, "", vec![Tag::event(note)], &second),
            // Again, later: once is what counts, and where it puts them.
            event_at(30, Kind::Repost, "", vec![Tag::event(note)], &first),
            // Of another note, and of another kind: not this.
            event_at(40, Kind::Repost, "", vec![Tag::event(another)], &second),
            event_at(50, Kind::Reaction, "+", vec![Tag::event(note)], &second),
        ];
        let reposted = |who: &Keys| Doer { who: Some(who.public_key()), did: Did::Reposted };
        assert_eq!(who_did(events.clone(), note, Left::Reposts), [reposted(&first), reposted(&second)]);
        assert_eq!(who_did(events, id(9), Left::Reposts), []);
    }

    #[test]
    fn who_reacted_to_a_note_is_each_of_them_with_what_they_reacted_with_last() {
        let (first, second, third) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (note, another) = (id(1), id(2));
        let events = vec![
            event_at(10, Kind::Reaction, "+", vec![Tag::event(note)], &first),
            event_at(20, Kind::Reaction, "\u{1f525}", vec![Tag::event(note)], &first),
            event_at(30, Kind::Reaction, "+", vec![Tag::event(note)], &second),
            // A downvote is no reaction, and one to a note it names last is
            // to that note, not to this one.
            event_at(40, Kind::Reaction, "-", vec![Tag::event(note)], &third),
            event_at(50, Kind::Reaction, "+", vec![Tag::event(note), Tag::event(another)], &third),
        ];
        let reacted = |who: &Keys, with: Reaction| Doer { who: Some(who.public_key()), did: Did::Reacted(with) };
        assert_eq!(
            who_did(events, note, Left::Reactions),
            [reacted(&second, Reaction::Like), reacted(&first, Reaction::Emoji("\u{1f525}".into()))]
        );
    }

    #[test]
    fn who_zapped_a_note_is_each_of_them_with_all_they_paid_the_most_first() {
        let (generous, modest) = (Keys::generate(), Keys::generate());
        let (note, another) = (id(1), id(2));
        // A request that says who paid, altered after it was signed: it
        // does not hold, and the zap is nobody's in particular.
        let forged = {
            let request = event(Kind::ZapRequest, "", vec![Tag::event(note)], &generous);
            let mut altered: serde_json::Value = serde_json::from_str(&request.as_json()).unwrap();
            altered["content"] = "said after signing".into();
            let tags = vec![
                Tag::event(note),
                Tag::custom("bolt11", ["lnbc10u1pjq9xyz"]),
                Tag::custom("description", [altered.to_string()]),
            ];
            event(Kind::ZapReceipt, "", tags, &Keys::generate())
        };
        let events = vec![
            receipt_from(&modest, note, "lnbc210n1pjq9xyz"),
            receipt_from(&generous, note, "lnbc10u1pjq9xyz"),
            receipt_from(&generous, note, "lnbc210n1pjq9xyz"),
            // Receipts that say nobody paid, or say it falsely: one line.
            receipt(note, "lnbc210n1pjq9xyz"),
            forged,
            // No amount, and another note's: not counted here.
            receipt_from(&modest, note, "lnbc1pjq9xyz"),
            receipt_from(&modest, another, "lnbc10u1pjq9xyz"),
        ];
        let zapped = |who: Option<&Keys>, sats: u64| Doer { who: who.map(Keys::public_key), did: Did::Zapped(sats) };
        assert_eq!(
            who_did(events, note, Left::Zaps),
            [zapped(Some(&generous), 1_021), zapped(None, 1_021), zapped(Some(&modest), 21)]
        );
    }

    #[test]
    fn each_note_is_counted_by_the_events_about_it() {
        let (me, other, third) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (note, another, unasked) = (id(1), id(2), id(3));
        let reply = |to: EventId| {
            event(Kind::TextNote, "an answer", vec![Tag::custom("e", [to.to_hex(), String::new(), "root".into()])], &other)
        };
        let deeper = event(
            Kind::TextNote,
            "an answer to an answer",
            vec![
                Tag::custom("e", [note.to_hex(), String::new(), "root".into()]),
                Tag::custom("e", [id(9).to_hex(), String::new(), "reply".into()]),
            ],
            &other,
        );
        let events = vec![
            reply(note),
            reply(note),
            deeper,
            reaction("+", note, &me),
            reaction("🔥", note, &other),
            reaction("+", note, &other),
            reaction("-", note, &third),
            reaction("+", another, &other),
            reaction("+", unasked, &me),
            reposting(note, Kind::Repost, &other),
            reposting(note, Kind::Repost, &other),
            reposting(note, Kind::GenericRepost, &third),
            reposting(another, Kind::Repost, &me),
            reposting(unasked, Kind::Repost, &me),
            receipt(note, "lnbc210n1pjq9xyz"),
            receipt(note, "lnbc10u1pjq9xyz"),
            receipt(note, "lnbc1pjq9xyz"),
            receipt(unasked, "lnbc10u1pjq9xyz"),
        ];

        let counted = tally(events, &HashSet::from([note, another, id(4)]), Some(me.public_key()));

        assert_eq!(
            counted.get(&note),
            Some(&Engagement {
                replies: 2,
                reposts: 2,
                reposted: false,
                likes: 2,
                reaction: Some(Reaction::Like),
                zap_sats: 1_021
            }),
            "direct replies; each person once; a downvote not at all; invoices with an amount"
        );
        assert_eq!(counted.get(&another), Some(&Engagement { reposts: 1, reposted: true, likes: 1, ..Engagement::default() }));
        assert_eq!(counted.get(&id(4)), Some(&Engagement::default()), "asked about, and nothing known");
        assert_eq!(counted.get(&unasked), None, "not asked about");
    }

    #[test]
    fn a_reaction_is_to_the_last_note_it_names() {
        let keys = Keys::generate();
        let reaction = event(Kind::Reaction, "+", vec![Tag::event(id(1)), Tag::event(id(2))], &keys);
        let counted = tally([reaction], &HashSet::from([id(1), id(2)]), None);
        assert_eq!(counted.get(&id(1)).map(|engagement| engagement.likes), Some(0));
        assert_eq!(counted.get(&id(2)).map(|engagement| engagement.likes), Some(1));
    }

    #[test]
    fn a_reaction_counts_as_one_for_the_note_and_its_author_hears_of_it() {
        let (me, them) = (Keys::generate(), Keys::generate());
        for (reaction, content) in [(Reaction::Like, "+"), (Reaction::Emoji("🤌".into()), "🤌")] {
            let reacted = react(id(1), Kind::Comment, them.public_key(), &reaction).finalize(&me).unwrap();

            assert_eq!(reacted.kind, Kind::Reaction);
            assert_eq!(reacted.content, content);
            assert_eq!(reacted.tags.public_keys().collect::<Vec<_>>(), [them.public_key()]);
            assert_eq!(reacted.tags.event_ids().collect::<Vec<_>>(), [id(1)]);
            let counted = tally([reacted], &HashSet::from([id(1)]), Some(me.public_key()));
            assert_eq!(counted.get(&id(1)), Some(&Engagement { likes: 1, reaction: Some(reaction), ..Engagement::default() }));
        }
    }

    #[test]
    fn a_reaction_is_an_emoji_if_it_says_one_and_a_like_if_it_says_anything_else() {
        assert_eq!(Reaction::said("🤌"), Reaction::Emoji("🤌".into()));
        assert_eq!(Reaction::said(" 😂\n"), Reaction::Emoji("😂".into()));
        // However the emoji was written, it is the same reaction.
        assert_eq!(Reaction::said("❤"), Reaction::said("❤️"));
        for like in ["+", "", ":soapbox:", "nice one", "😂😂😂"] {
            assert_eq!(Reaction::said(like), Reaction::Like, "{like:?}");
        }
        assert_eq!((Reaction::Like.content(), Reaction::Emoji("🤌".into()).content()), ("+", "🤌"));
    }

    #[test]
    fn the_accounts_own_reaction_is_the_one_it_made_last() {
        let (me, other) = (Keys::generate(), Keys::generate());
        let at = |content: &str, seconds: u64, keys: &Keys| {
            EventBuilder::new(Kind::Reaction, content)
                .tags([Tag::event(id(1))])
                .custom_created_at(nostr_sdk::prelude::Timestamp::from_secs(seconds))
                .finalize(keys)
                .unwrap()
        };
        let (first, last, theirs) = (at("+", 100, &me), at("🧨", 200, &me), at("🤡", 300, &other));
        // In whatever order they come, and counted once.
        for events in [[first.clone(), last.clone(), theirs.clone()], [theirs, last, first]] {
            let counted = tally(events, &HashSet::from([id(1)]), Some(me.public_key()));
            let expected = Engagement { likes: 2, reaction: Some(Reaction::Emoji("🧨".into())), ..Engagement::default() };
            assert_eq!(counted.get(&id(1)), Some(&expected));
        }
        // With nobody signed in, nobody's reaction is one's own.
        let counted = tally([at("🧨", 200, &me)], &HashSet::from([id(1)]), None);
        assert_eq!(counted.get(&id(1)).and_then(|engagement| engagement.reaction.clone()), None);
    }

    /// The tags of `event`, each as its fields.
    fn tags(event: &Event) -> Vec<Vec<&str>> {
        event.tags.iter().map(|tag| tag.as_slice().iter().map(String::as_str).collect()).collect()
    }

    #[test]
    fn a_repost_of_a_note_carries_it_whole_and_says_where_it_is() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let note = event(Kind::TextNote, "worth passing on", Vec::new(), &them);
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        let reposted = repost(&note, Some(relay)).finalize(&me).unwrap();

        assert_eq!(reposted.kind, Kind::Repost);
        let (note_id, their_key) = (note.id.to_hex(), them.public_key().to_hex());
        assert_eq!(tags(&reposted), [vec!["e", note_id.as_str(), "wss://relay.example.com"], vec!["p", their_key.as_str()]]);
        assert_eq!(Event::from_json(&reposted.content).unwrap(), note, "the note, as it was signed");

        let counted = tally([reposted], &HashSet::from([note.id]), Some(me.public_key()));
        assert_eq!(counted.get(&note.id), Some(&Engagement { reposts: 1, reposted: true, ..Engagement::default() }));
    }

    #[test]
    fn a_repost_of_anything_else_says_what_kind_it_is() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let comment = event(Kind::Comment, "what a bird", Vec::new(), &them);
        let reposted = repost(&comment, None).finalize(&me).unwrap();

        assert_eq!(reposted.kind, Kind::GenericRepost);
        let (comment_id, their_key) = (comment.id.to_hex(), them.public_key().to_hex());
        assert_eq!(tags(&reposted), [vec!["e", comment_id.as_str()], vec!["p", their_key.as_str()], vec!["k", "1111"]]);
        assert_eq!(Event::from_json(&reposted.content).unwrap(), comment);
    }

    #[test]
    fn a_note_that_was_to_stay_where_it_was_sent_is_reposted_by_name_only() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let note = event(Kind::TextNote, "for this relay only", vec![Tag::protected()], &them);
        assert_eq!(repost(&note, None).finalize(&me).unwrap().content, "");
    }

    #[test]
    fn a_mark_is_counted_once_and_taken_back_once() {
        let others = Engagement { likes: 3, reposts: 1, ..Engagement::default() };

        let (like, clown) = (Mark::React(Reaction::Like), Mark::React(Reaction::Emoji("🤡".into())));
        let liked = others.clone().with(&like);
        assert_eq!(liked, Engagement { likes: 4, reaction: Some(Reaction::Like), ..others.clone() });
        assert_eq!(liked.clone().with(&like), liked, "once is enough");
        assert_eq!(liked.clone().with(&clown), liked, "whatever the second would say");
        assert!(liked.has(&like) && liked.has(&clown) && !liked.has(&Mark::Repost));

        let both = liked.clone().with(&Mark::Repost);
        assert_eq!(both, Engagement { reposts: 2, reposted: true, ..liked });
        assert_eq!(both.without(&clown), Engagement { reposts: 2, reposted: true, ..others.clone() });
        assert_eq!(others.clone().without(&Mark::Repost), others, "what was not there is not taken away");

        // An emoji is counted as a like is, and shows as itself.
        let clowned = others.clone().with(&clown);
        assert_eq!(clowned, Engagement { likes: 4, reaction: Some(Reaction::Emoji("🤡".into())), ..others });
    }
}

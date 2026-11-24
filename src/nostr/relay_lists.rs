// SPDX-License-Identifier: MIT

//! The relays an account says are its own, by what they are for, as the
//! relays page shows and changes them.
//!
//! Seven sets, in six lists:
//!
//! - **Outbox** and **inbox**: the relays the account writes to, where its
//!   notes are to be found, and the relays it reads, where what is meant
//!   for it is to be sent. Both are in its relay list (NIP-65, kind 10002):
//!   one `r` tag for each relay, marked `write` or `read`, or unmarked
//!   when it is both.
//! - The others are each a list of `relay` tags (NIP-51): in the list's
//!   tags, for anyone to read, or among the tags it keeps to itself,
//!   encrypted in its content as a bookmark list's are. Where direct
//!   messages go (NIP-17, kind 10050), which is for anyone to read; the
//!   relays it searches (kind 10007); the relays profiles and relay lists
//!   are looked up on (kind 10086); the relays it trusts (kind 10089);
//!   and the relays it never connects to (kind 10006).
//!
//! The list of relays an account keeps what is its alone on (kind 10013)
//! is not among them: what is kept here and not published, a draft, is
//! kept on this machine, and that list is neither read nor written.
//!
//! A list is changed as Amethyst changes it, so that each client finds
//! what the other left: everything in it that is not a relay stays as it
//! was, and a relay stays where it was, said to anyone or kept private.
//! One that is new goes where the list keeps its relays: among the public
//! ones if those are all it has, and among the private ones otherwise;
//! and in a first list, where lists of its kind keep them.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph page["The relays page"]
//!         outbox["Outbox"]
//!         inbox["Inbox"]
//!         dm["Direct messages"]
//!         search["Search"]
//!         indexer["Indexer"]
//!         trusted["Trusted"]
//!         blocked["Blocked"]
//!     end
//!     outbox --> k10002["Kind 10002: r tags, marked write or read"]
//!     inbox --> k10002
//!     dm --> k10050["Kind 10050: relay tags, public"]
//!     search --> k10007["Kind 10007"]
//!     indexer --> k10086["Kind 10086"]
//!     trusted --> k10089["Kind 10089"]
//!     blocked --> k10006["Kind 10006"]
//!     k10007 --> halves["relay tags, in the list's tags or encrypted in its content"]
//!     k10086 --> halves
//!     k10089 --> halves
//!     k10006 --> halves
//! ```
//!
//! What each set is used for, and what a relay has to do to be of use in
//! one ([`Section::hindrances`]), is in [`crate::guide::relays`].

use std::collections::{HashMap, HashSet};
use std::fmt;

use nostr_sdk::prelude::{Event, EventBuilder, Kind, RelayUrl, Tag, Timestamp};

use crate::nostr::account::SignError;
use crate::nostr::bookmarks::PrivateTags;
use crate::nostr::outbox::{self, Listed};
use crate::nostr::relay_info::RelayInfo;

/// The name of the tag that names a relay in a relay list (NIP-65), and
/// in the other lists of relays (NIP-51, NIP-17).
const ADVERTISED_TAG: &str = "r";
const RELAY_TAG: &str = "relay";

/// The NIPs a relay has to support to be of use in one of the sets:
/// search, and saying who is asking.
pub const NIP_SEARCH: u16 = 50;
pub const NIP_AUTH: u16 = 42;

/// What a relay says of itself that gets in the way of what the set it is
/// in is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hindrance {
    /// It names the NIPs it supports, and not this one, which the set
    /// needs.
    Lacks(u16),
    /// It takes events only of some, where anyone is to write.
    RestrictedWrites,
    /// It has to be paid, where anyone is to write.
    PaymentRequired,
    /// Whoever connects has to say who they are first, where anyone is to
    /// write: a client that doesn't is turned away.
    AuthToWrite,
    /// Whoever connects has to say who they are first, where anyone is to
    /// read: a client that doesn't sees nothing.
    AuthToRead,
}

/// One of an account's sets of relays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    /// The relays it writes to (NIP-65).
    Outbox,
    /// The relays it reads (NIP-65).
    Inbox,
    /// The relays its direct messages are sent to (NIP-17, kind 10050).
    Messages,
    /// The relays it searches (NIP-51, kind 10007).
    Search,
    /// The relays that keep everyone's profile and relay list, where
    /// those are looked up (kind 10086).
    Indexer,
    /// The relays it trusts with who it is (kind 10089).
    Trusted,
    /// The relays it never connects to (NIP-51, kind 10006).
    Blocked,
}

impl Section {
    /// Every section, in the order the page shows them.
    pub const ALL: [Self; 7] =
        [Self::Outbox, Self::Inbox, Self::Messages, Self::Search, Self::Indexer, Self::Trusted, Self::Blocked];

    /// The sections that are a list of their own, of `relay` tags: all
    /// but the two of the relay list.
    pub const LISTS: [Self; 5] = [Self::Messages, Self::Search, Self::Indexer, Self::Trusted, Self::Blocked];

    /// The kind of the list this set of relays is in.
    pub fn kind(self) -> Kind {
        match self {
            Self::Outbox | Self::Inbox => Kind::RelayList,
            Self::Messages => Kind::InboxRelays,
            Self::Search => Kind::SearchRelays,
            Self::Indexer => Kind::Custom(10086),
            Self::Trusted => Kind::Custom(10089),
            Self::Blocked => Kind::BlockedRelays,
        }
    }

    /// The NIP a relay has to support to do what the relays of this set
    /// are for, where there is one beyond the protocol itself. A search
    /// relay is asked to search (NIP-50). A relay direct messages are
    /// sent to is to give them to nobody but whom they are for, which
    /// takes knowing who is asking (NIP-17, by NIP-42). The other sets
    /// ask nothing of a relay that every relay doesn't do.
    pub fn needs(self) -> Option<u16> {
        match self {
            Self::Search => Some(NIP_SEARCH),
            Self::Messages => Some(NIP_AUTH),
            Self::Outbox | Self::Inbox | Self::Indexer | Self::Trusted | Self::Blocked => None,
        }
    }

    /// Whether the relays of this set are where other people write to
    /// reach the account: what is meant for it, and its direct messages.
    fn written_to_by_others(self) -> bool {
        matches!(self, Self::Inbox | Self::Messages)
    }

    /// Whether the relays of this set are where other people read what
    /// the account writes.
    fn read_by_others(self) -> bool {
        self == Self::Outbox
    }

    /// Whether what a relay says of itself can get in the way of what
    /// this set is for: whether its relays are worth asking.
    pub fn asks_of_relays(self) -> bool {
        self.needs().is_some() || self.written_to_by_others() || self.read_by_others()
    }

    /// What `info`, which a relay of this set says of itself, has that
    /// gets in the way of what the set is for: a NIP it needs and the
    /// relay doesn't name; where other people are to write, what the
    /// relay asks before it takes anything; and where they are to read,
    /// that it asks who they are. Not that it asks who is connecting
    /// where that is what the set needs of it. Whether a relay the
    /// account writes to takes what it writes, or is paid for, is the
    /// account's own business.
    pub fn hindrances(self, info: &RelayInfo) -> Vec<Hindrance> {
        let mut found = Vec::new();
        if let Some(nip) = self.needs()
            && info.supports(nip) == Some(false)
        {
            found.push(Hindrance::Lacks(nip));
        }
        if self.written_to_by_others() {
            let limits = info.limits;
            found.extend(limits.restricted_writes.then_some(Hindrance::RestrictedWrites));
            found.extend(limits.payment_required.then_some(Hindrance::PaymentRequired));
            found.extend((limits.auth_required && self.needs() != Some(NIP_AUTH)).then_some(Hindrance::AuthToWrite));
        }
        if self.read_by_others() {
            found.extend(info.limits.auth_required.then_some(Hindrance::AuthToRead));
        }
        found
    }

    /// Whether the list says its relays to anyone, always: where direct
    /// messages go is for whoever writes one to read.
    fn always_public(self) -> bool {
        self == Self::Messages
    }

    /// Whether a first list of this kind says its relays to anyone, as
    /// Amethyst writes one: the others keep them to themselves.
    fn first_is_public(self) -> bool {
        matches!(self, Self::Messages | Self::Search)
    }
}

/// An account's relays, by what they are for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaySets(HashMap<Section, Vec<RelayUrl>>);

impl RelaySets {
    /// The relays of `section`, in their order.
    pub fn of(&self, section: Section) -> &[RelayUrl] {
        self.0.get(&section).map_or(&[], Vec::as_slice)
    }

    /// Have `relays` for the relays of `section`.
    pub fn set(&mut self, section: Section, relays: Vec<RelayUrl>) {
        self.0.insert(section, relays);
    }

    /// Have `relay` in `section`, after the others. Returns whether it
    /// wasn't there already.
    pub fn add(&mut self, section: Section, relay: RelayUrl) -> bool {
        let relays = self.0.entry(section).or_default();
        let new = !relays.contains(&relay);
        if new {
            relays.push(relay);
        }
        new
    }

    /// Take `relay` out of `section`.
    pub fn remove(&mut self, section: Section, relay: &RelayUrl) {
        self.0.entry(section).or_default().retain(|listed| listed != relay);
    }

    /// Move `relay` one place towards the start of `section` (`up`), or
    /// towards its end. Nothing moves past either.
    pub fn shift(&mut self, section: Section, relay: &RelayUrl, up: bool) {
        let relays = self.0.entry(section).or_default();
        let Some(place) = relays.iter().position(|listed| listed == relay) else {
            return;
        };
        let other = if up { place.checked_sub(1) } else { Some(place + 1).filter(|next| *next < relays.len()) };
        if let Some(other) = other {
            relays.swap(place, other);
        }
    }

    /// Whether these are the relays `other` has, set for set and in the
    /// same order.
    pub fn same_as(&self, other: &Self) -> bool {
        Section::ALL.iter().all(|section| self.of(*section) == other.of(*section))
    }
}

/// The relay `typed` names, as someone types one: its address, with or
/// without the `wss://` nearly all of them start with.
pub fn typed_relay(typed: &str) -> Option<RelayUrl> {
    let typed = typed.trim();
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    let address = if typed.contains("://") { typed.to_string() } else { format!("wss://{typed}") };
    RelayUrl::parse(&address).ok()
}

/// When the list that replaces `earlier` is written: at `now` or, if
/// `earlier` claims to be from later, after it. It has to be the newer of
/// the two.
fn replacing(earlier: Option<&Event>, now: Timestamp) -> Timestamp {
    earlier.map_or(now, |earlier| now.max(Timestamp::from_secs(earlier.created_at.as_secs() + 1)))
}

/// Whether `tag` names a relay, as tags called `name` do in a list.
fn names_relay(fields: &[String], name: &str) -> bool {
    matches!(fields, [called, relay, ..] if called == name && RelayUrl::parse(relay).is_ok())
}

/// The relays that the tags `fields` call `name`, each once, in their order.
fn relays_named<'a>(tags: impl IntoIterator<Item = &'a [String]>, name: &str) -> Vec<RelayUrl> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .filter(|fields| fields.first().is_some_and(|called| called == name))
        .filter_map(|fields| fields.get(1).and_then(|relay| RelayUrl::parse(relay).ok()))
        .filter(|relay| seen.insert(relay.clone()))
        .collect()
}

/// The relays the relay list `list` says its account writes to, and those
/// it says it reads. None of either without a list.
pub fn advertised(list: Option<&Event>) -> (Vec<RelayUrl>, Vec<RelayUrl>) {
    list.map(|list| (outbox::relays(list, Listed::Write), outbox::relays(list, Listed::Read))).unwrap_or_default()
}

/// The relay list (NIP-65) that replaces `earlier`, if there is one to
/// replace: with `outbox` for the relays written to and `inbox` for those
/// read, a relay in both named once, and whatever else `earlier` had in
/// it as it was. The relays written to come in their order, and after
/// them those that are only read, in theirs: a list has one order for
/// both.
///
/// `None` when `earlier` names the same relays for the same things in
/// the same order: there is nothing to write.
pub fn advertised_list(earlier: Option<&Event>, outbox: &[RelayUrl], inbox: &[RelayUrl], now: Timestamp) -> Option<EventBuilder> {
    let names = |tag: &&Tag| names_relay(tag.as_slice(), ADVERTISED_TAG);
    let naming = |relay: &RelayUrl, marker: Option<&str>| {
        let fields = [ADVERTISED_TAG, relay.as_str()].into_iter().chain(marker);
        Tag::parse(fields).ok()
    };
    let written = outbox.iter().filter_map(|relay| naming(relay, (!inbox.contains(relay)).then_some("write")));
    let read = inbox.iter().filter(|relay| !outbox.contains(relay)).filter_map(|relay| naming(relay, Some("read")));
    let relays: Vec<Tag> = written.chain(read).collect();

    // Said already, whatever form the addresses were written in.
    let said = |tags: &[Tag]| -> Vec<(Option<RelayUrl>, Option<String>)> {
        tags.iter()
            .map(|tag| (tag.as_slice().get(1).and_then(|relay| RelayUrl::parse(relay).ok()), tag.as_slice().get(2).cloned()))
            .collect()
    };
    let before: Vec<Tag> = earlier.into_iter().flat_map(|earlier| earlier.tags.iter()).filter(names).cloned().collect();
    if said(&before) == said(&relays) {
        return None;
    }
    let kept = earlier.into_iter().flat_map(|earlier| earlier.tags.iter()).filter(|tag| !names(tag));
    let tags: Vec<Tag> = kept.cloned().chain(relays).collect();
    let content = earlier.map(|earlier| earlier.content.clone()).unwrap_or_default();
    Some(EventBuilder::new(Kind::RelayList, content).tags(tags).custom_created_at(replacing(earlier, now)))
}

/// What a list keeps to itself, as far as it was read.
#[derive(Clone, PartialEq, Eq)]
pub enum Kept {
    /// These tags: none, for a list that keeps nothing to itself.
    Read(PrivateTags),
    /// Something, which was not asked to be decrypted: a signer may ask a
    /// person about each request, and not every list is needed.
    Unread,
    /// Something that could not be read, and why.
    Unreadable(SignError),
}

/// What is kept private is not printed: whatever is `Debug` ends up in a
/// log sooner or later.
impl fmt::Debug for Kept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(private) => write!(f, "Read({} tag(s))", private.len()),
            Self::Unread => f.write_str("Unread"),
            Self::Unreadable(why) => write!(f, "Unreadable({why:?})"),
        }
    }
}

/// An account's relays of one kind, as its newest list of that kind has
/// them: a list of `relay` tags, some for anyone to read and some kept to
/// itself, encrypted in its content as a bookmark list's are.
#[derive(Clone, PartialEq, Eq)]
pub struct ListedRelays {
    /// The list they were read from. `None` when the account has none.
    list: Option<Event>,
    /// What the list keeps to itself.
    kept: Kept,
}

impl Default for ListedRelays {
    fn default() -> Self {
        Self { list: None, kept: Kept::Read(Vec::new()) }
    }
}

/// What is kept private is not printed: whatever is `Debug` ends up in a
/// log sooner or later.
impl fmt::Debug for ListedRelays {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListedRelays")
            .field("list", &self.list.as_ref().map(|list| list.id))
            .field("relays", &self.relays().len())
            .finish_non_exhaustive()
    }
}

impl ListedRelays {
    /// The relays of `list`, which keeps `kept` to itself.
    pub fn new(list: Option<Event>, kept: Kept) -> Self {
        Self { list, kept }
    }

    /// The list they were read from, if the account has one.
    pub fn list(&self) -> Option<&Event> {
        self.list.as_ref()
    }

    /// What the list keeps to itself, as far as it was read.
    pub fn kept(&self) -> &Kept {
        &self.kept
    }

    /// The relays the list names for anyone to read.
    fn public(&self) -> Vec<RelayUrl> {
        relays_named(self.list.iter().flat_map(|list| list.tags.iter()).map(Tag::as_slice), RELAY_TAG)
    }

    /// The relays the list keeps to itself, as far as they were read.
    fn private(&self) -> Vec<RelayUrl> {
        match &self.kept {
            Kept::Read(private) => relays_named(private.iter().map(Vec::as_slice), RELAY_TAG),
            Kept::Unread | Kept::Unreadable(_) => Vec::new(),
        }
    }

    /// The relays, as far as they were read: those the list names for
    /// anyone, then those it keeps to itself, each once.
    pub fn relays(&self) -> Vec<RelayUrl> {
        let mut relays = self.public();
        for relay in self.private() {
            if !relays.contains(&relay) {
                relays.push(relay);
            }
        }
        relays
    }

    /// The list's tags and what it keeps to itself, with `relays` for the
    /// relays of `section` and everything else as it is: each relay where
    /// it was, and a new one where the list keeps its relays (see the
    /// module documentation), in the order of `relays` there.
    ///
    /// `None` when there is nothing to write: the list says the same
    /// already. And when what the list keeps to itself was not read: a
    /// list written without it would lose it.
    pub fn changed(&self, section: Section, relays: &[RelayUrl]) -> Option<(Vec<Tag>, PrivateTags)> {
        let Kept::Read(kept) = &self.kept else {
            return None;
        };
        let (was_public, was_private) = (self.public(), self.private());
        let new_are_public = section.always_public()
            || if self.list.is_none() { section.first_is_public() } else { !was_public.is_empty() && was_private.is_empty() };
        let is_public = |relay: &&RelayUrl| {
            section.always_public() || was_public.contains(relay) || (!was_private.contains(relay) && new_are_public)
        };
        let (public, private): (Vec<&RelayUrl>, Vec<&RelayUrl>) = relays.iter().partition(is_public);
        let same =
            |one: &[&RelayUrl], other: &[RelayUrl]| one.len() == other.len() && one.iter().zip(other).all(|(a, b)| *a == b);
        // In the order each half has them, which is all a list can say.
        let was_private_alone: Vec<RelayUrl> = was_private.iter().filter(|relay| !was_public.contains(relay)).cloned().collect();
        if same(&public, &was_public) && same(&private, &was_private_alone) {
            return None;
        }
        let naming = |relay: &RelayUrl| vec![RELAY_TAG.to_string(), relay.to_string()];
        let others = self.list.iter().flat_map(|list| list.tags.iter()).filter(|tag| !names_relay(tag.as_slice(), RELAY_TAG));
        let tags: Vec<Tag> =
            others.cloned().chain(public.into_iter().filter_map(|relay| Tag::parse(naming(relay)).ok())).collect();
        let mut private_tags: PrivateTags = kept.iter().filter(|fields| !names_relay(fields, RELAY_TAG)).cloned().collect();
        private_tags.extend(private.into_iter().map(naming));
        Some((tags, private_tags))
    }
}

/// The list of `section` that replaces `earlier`, if there is one to
/// replace: with `tags`, and `content` for what it keeps to itself.
pub fn listed(section: Section, earlier: Option<&Event>, tags: Vec<Tag>, content: String, now: Timestamp) -> EventBuilder {
    EventBuilder::new(section.kind(), content).tags(tags).custom_created_at(replacing(earlier, now))
}

/// An account's relay lists, as they were last read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayLists {
    /// Its relay list (NIP-65), if it has one.
    pub advertised: Option<Event>,
    /// Its other lists of relays, each as far as it was read.
    pub listed: HashMap<Section, ListedRelays>,
}

impl RelayLists {
    /// The list of `section`, for the sections that are a list of their
    /// own: an account with none has no relays there.
    pub fn of(&self, section: Section) -> ListedRelays {
        self.listed.get(&section).cloned().unwrap_or_default()
    }

    /// The relays the lists name, by what they are for, as far as the
    /// lists were read.
    pub fn sets(&self) -> RelaySets {
        let (outbox, inbox) = advertised(self.advertised.as_ref());
        let mut sets = RelaySets::default();
        sets.set(Section::Outbox, outbox);
        sets.set(Section::Inbox, inbox);
        for section in Section::LISTS {
            sets.set(section, self.listed.get(&section).map(ListedRelays::relays).unwrap_or_default());
        }
        sets
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Hindrance, Kept, ListedRelays, NIP_AUTH, NIP_SEARCH, RelayLists, RelaySets, Section, advertised, advertised_list, listed,
        typed_relay,
    };
    use crate::nostr::account::SignError;
    use crate::nostr::relay_info::{Limits, RelayInfo};
    use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, RelayUrl, Tag, Timestamp};

    fn relay(name: &str) -> RelayUrl {
        RelayUrl::parse(&format!("wss://{name}.example.com")).unwrap()
    }

    fn tag(fields: &[&str]) -> Tag {
        Tag::parse(fields.iter().copied()).unwrap()
    }

    fn a_list(kind: Kind, tags: Vec<Tag>, content: &str, at: u64) -> Event {
        EventBuilder::new(kind, content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn fields(event: &Event) -> Vec<Vec<String>> {
        event.tags.iter().map(|tag| tag.as_slice().to_vec()).collect()
    }

    fn private(relays: &[&RelayUrl]) -> Vec<Vec<String>> {
        relays.iter().map(|relay| vec!["relay".to_string(), relay.to_string()]).collect()
    }

    #[test]
    fn what_a_relay_says_of_itself_gets_in_the_way_of_some_sets_and_not_of_others() {
        let says = |nips: &[u16], limits: Limits| RelayInfo { nips: Some(nips.to_vec()), limits, ..RelayInfo::default() };
        let open = Limits::default();
        let closed = Limits { auth_required: true, payment_required: true, restricted_writes: true };

        // A set that needs a NIP is hindered by a relay that names
        // others, and not by one that names none.
        assert_eq!(Section::Search.hindrances(&says(&[1, 11], open)), [Hindrance::Lacks(NIP_SEARCH)]);
        assert_eq!(Section::Search.hindrances(&says(&[1, 50], open)), []);
        assert_eq!(Section::Search.hindrances(&RelayInfo::default()), []);
        assert_eq!(Section::Messages.hindrances(&says(&[1], open)), [Hindrance::Lacks(NIP_AUTH)]);

        // Where others write, by what a relay asks before it takes
        // anything, whatever NIPs it names.
        let all = [Hindrance::RestrictedWrites, Hindrance::PaymentRequired, Hindrance::AuthToWrite];
        assert_eq!(Section::Inbox.hindrances(&says(&[1], closed)), all);
        assert_eq!(Section::Inbox.hindrances(&RelayInfo { limits: closed, ..RelayInfo::default() }), all);
        assert_eq!(Section::Inbox.hindrances(&says(&[1], open)), []);
        let asking = Limits { auth_required: true, ..open };
        assert_eq!(Section::Inbox.hindrances(&says(&[1, 42], asking)), [Hindrance::AuthToWrite]);
        // A relay direct messages go to is to ask who connects.
        assert_eq!(Section::Messages.hindrances(&says(&[1, 42], asking)), []);
        assert_eq!(Section::Messages.hindrances(&says(&[1, 42], closed)), all[..2]);

        // Where others read, by a relay asking who they are; whom it
        // takes events of, and whether it is paid, is the account's own
        // business there.
        assert_eq!(Section::Outbox.hindrances(&says(&[1, 42], closed)), [Hindrance::AuthToRead]);
        let members = Limits { payment_required: true, restricted_writes: true, ..open };
        assert_eq!(Section::Outbox.hindrances(&says(&[1], members)), []);
        assert!(Section::Outbox.asks_of_relays());

        // Nothing gets in the way of the other sets.
        for section in [Section::Indexer, Section::Trusted, Section::Blocked] {
            assert_eq!(section.hindrances(&says(&[1], closed)), [], "{section:?}");
            assert!(!section.asks_of_relays());
        }
        assert!(Section::Inbox.asks_of_relays() && Section::Messages.asks_of_relays() && Section::Search.asks_of_relays());
        assert_eq!(Section::Search.hindrances(&says(&[1, 50], closed)), []);
    }

    #[test]
    fn a_relay_is_typed_with_or_without_its_scheme() {
        let ditto = RelayUrl::parse("wss://relay.ditto.pub").unwrap();
        for typed in ["relay.ditto.pub", " wss://relay.ditto.pub/ ", "wss://relay.ditto.pub"] {
            assert_eq!(typed_relay(typed), Some(ditto.clone()), "{typed}");
        }
        assert_eq!(typed_relay("ws://127.0.0.1:7777"), RelayUrl::parse("ws://127.0.0.1:7777").ok());
        for not_a_relay in ["", "   ", "https://example.com", "two words", "wss://"] {
            assert_eq!(typed_relay(not_a_relay), None, "{not_a_relay:?}");
        }
    }

    #[test]
    fn relays_are_added_once_taken_out_and_moved() {
        let mut sets = RelaySets::default();
        assert!(sets.add(Section::Search, relay("one")));
        assert!(sets.add(Section::Search, relay("two")));
        assert!(!sets.add(Section::Search, relay("one")), "there already");
        assert!(sets.add(Section::Outbox, relay("one")), "each set for itself");
        assert_eq!(sets.of(Section::Search), [relay("one"), relay("two")]);
        sets.remove(Section::Search, &relay("one"));
        assert_eq!(sets.of(Section::Search), [relay("two")]);
        assert_eq!(sets.of(Section::Outbox), [relay("one")]);
        assert!(sets.of(Section::Inbox).is_empty());

        // Moved a place at a time, and not past either end.
        let mut sets = RelaySets::default();
        for name in ["a", "b", "c"] {
            sets.add(Section::Outbox, relay(name));
        }
        sets.shift(Section::Outbox, &relay("c"), true);
        assert_eq!(sets.of(Section::Outbox), [relay("a"), relay("c"), relay("b")]);
        sets.shift(Section::Outbox, &relay("a"), true);
        sets.shift(Section::Outbox, &relay("b"), false);
        sets.shift(Section::Outbox, &relay("nowhere"), false);
        assert_eq!(sets.of(Section::Outbox), [relay("a"), relay("c"), relay("b")]);
        sets.shift(Section::Outbox, &relay("a"), false);
        assert_eq!(sets.of(Section::Outbox), [relay("c"), relay("a"), relay("b")]);

        // The same relays in another order are not the same sets.
        let mut other = sets.clone();
        assert!(sets.same_as(&other));
        other.shift(Section::Outbox, &relay("b"), true);
        assert!(!sets.same_as(&other));
    }

    #[test]
    fn a_relay_list_is_written_with_each_relay_marked_for_what_it_is_for() {
        // As Amethyst leaves it: unmarked relays, and a tag of its own.
        let (both, out, into, new) = (relay("both"), relay("out"), relay("in"), relay("new"));
        let tags = vec![
            tag(&["r", both.as_str()]),
            tag(&["r", out.as_str(), "write"]),
            tag(&["r", into.as_str(), "read"]),
            tag(&["client", "Amethyst"]),
        ];
        let earlier = a_list(Kind::RelayList, tags, "", 100);
        assert_eq!(advertised(Some(&earlier)), (vec![both.clone(), out.clone()], vec![both.clone(), into.clone()]));
        assert_eq!(advertised(None), (vec![], vec![]));

        // One taken out of the outbox, one added to it, one moved to both.
        let keys = Keys::generate();
        let now = Timestamp::from_secs(200);
        let (outbox, inbox) = (vec![both.clone(), new.clone(), into.clone()], vec![both.clone(), into.clone()]);
        let changed = advertised_list(Some(&earlier), &outbox, &inbox, now).unwrap().finalize(&keys).unwrap();
        assert_eq!(
            fields(&changed),
            [vec!["client", "Amethyst"], vec!["r", both.as_str()], vec!["r", new.as_str(), "write"], vec!["r", into.as_str()],]
        );
        assert_eq!(changed.created_at, now);
        assert_eq!(advertised(Some(&changed)), (outbox.clone(), inbox.clone()));

        // The same relays for the same things: nothing to write. In
        // another order, there is.
        assert!(advertised_list(Some(&changed), &outbox, &inbox, now).is_none());
        let reordered = vec![new.clone(), both.clone(), into.clone()];
        let moved = advertised_list(Some(&changed), &reordered, &inbox, now).unwrap().finalize(&keys).unwrap();
        assert_eq!(advertised(Some(&moved)).0, reordered);

        // A first list, and one that replaces a list from the future.
        let first = advertised_list(None, &[out.clone()], &[into.clone()], now).unwrap().finalize(&keys).unwrap();
        assert_eq!(fields(&first), [vec!["r", out.as_str(), "write"], vec!["r", into.as_str(), "read"]]);
        assert!(advertised_list(None, &[], &[], now).is_none(), "no relays and no list");
        let ahead = a_list(Kind::RelayList, vec![tag(&["r", out.as_str()])], "", 500);
        assert_eq!(advertised_list(Some(&ahead), &[], &[], now).unwrap().finalize(&keys).unwrap().created_at.as_secs(), 501);
    }

    #[test]
    fn listed_relays_are_those_said_to_anyone_and_those_kept_private() {
        let (open, kept, twice) = (relay("open"), relay("kept"), relay("twice"));
        let list = a_list(Kind::SearchRelays, vec![tag(&["relay", open.as_str()]), tag(&["relay", twice.as_str()])], "x", 100);
        let search = ListedRelays::new(Some(list.clone()), Kept::Read(private(&[&kept, &twice])));
        assert_eq!(search.relays(), [open.clone(), twice.clone(), kept.clone()]);
        assert_eq!(search.list(), Some(&list));
        assert!(ListedRelays::default().relays().is_empty());
        // What is kept private is not in what is printed.
        assert!(!format!("{search:?}").contains("kept"));

        // What was not read, or could not be, shows what anyone can read,
        // and is not written from.
        for unread in [Kept::Unread, Kept::Unreadable(SignError::Cipher("no".to_string()))] {
            let closed = ListedRelays::new(Some(list.clone()), unread);
            assert_eq!(closed.relays(), [open.clone(), twice.clone()]);
            assert!(closed.changed(Section::Search, &[open.clone()]).is_none());
        }

        let lists = RelayLists { advertised: None, listed: [(Section::Search, search.clone())].into() };
        assert_eq!(lists.sets().of(Section::Search), search.relays());
        assert_eq!(lists.of(Section::Blocked), ListedRelays::default());
    }

    #[test]
    fn a_listed_relay_stays_where_it_was_and_a_new_one_goes_where_the_list_keeps_them() {
        let (open, kept, new) = (relay("open"), relay("kept"), relay("new"));
        let other = vec!["t".to_string(), "nostr".to_string()];
        let said = |tags: &[Tag]| -> Vec<Vec<String>> { tags.iter().map(|tag| tag.as_slice().to_vec()).collect() };

        // A list that keeps relays to itself, as Amethyst writes one.
        let list = a_list(Kind::SearchRelays, vec![tag(&["client", "Amethyst"]), tag(&["relay", open.as_str()])], "x", 100);
        let mut kept_private = private(&[&kept]);
        kept_private.push(other.clone());
        let search = ListedRelays::new(Some(list), Kept::Read(kept_private));
        assert!(search.changed(Section::Search, &[open.clone(), kept.clone()]).is_none(), "as it is");

        let (tags, private_tags) = search.changed(Section::Search, &[open.clone(), kept.clone(), new.clone()]).unwrap();
        assert_eq!(said(&tags), [vec!["client", "Amethyst"], vec!["relay", open.as_str()]]);
        assert_eq!(private_tags, [other.clone(), private(&[&kept])[0].clone(), private(&[&new])[0].clone()]);
        // The new one before the old: that is the order it is kept in.
        let (_, private_tags) = search.changed(Section::Search, &[new.clone(), open.clone(), kept.clone()]).unwrap();
        assert_eq!(private_tags, [other.clone(), private(&[&new])[0].clone(), private(&[&kept])[0].clone()]);

        // Taking the private one out leaves what else is kept private.
        let (tags, private_tags) = search.changed(Section::Search, &[open.clone()]).unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(private_tags, [other]);

        // A list with public relays alone gets the new one in public.
        let public = a_list(Kind::SearchRelays, vec![tag(&["relay", open.as_str()])], "", 100);
        let public = ListedRelays::new(Some(public), Kept::Read(vec![]));
        let (tags, private_tags) = public.changed(Section::Search, &[open.clone(), new.clone()]).unwrap();
        assert_eq!(tags.len(), 2);
        assert!(private_tags.is_empty());

        // A list with no relays at all keeps a new one to itself.
        let empty = ListedRelays::new(Some(a_list(Kind::SearchRelays, vec![], "", 100)), Kept::Read(vec![]));
        let (tags, private_tags) = empty.changed(Section::Search, &[new.clone()]).unwrap();
        assert!(tags.is_empty());
        assert_eq!(private_tags, private(&[&new]));
        assert!(ListedRelays::default().changed(Section::Search, &[]).is_none());
    }

    #[test]
    fn a_first_list_says_its_relays_or_keeps_them_as_its_kind_does() {
        let new = relay("new");
        let none = ListedRelays::default();
        // Search relays and where messages go are for anyone to read.
        for section in [Section::Search, Section::Messages] {
            let (tags, private_tags) = none.changed(section, &[new.clone()]).unwrap();
            assert_eq!(tags, [tag(&["relay", new.as_str()])], "{section:?}");
            assert!(private_tags.is_empty());
        }
        // The others are kept private.
        for section in [Section::Indexer, Section::Trusted, Section::Blocked] {
            let (tags, private_tags) = none.changed(section, &[new.clone()]).unwrap();
            assert!(tags.is_empty(), "{section:?}");
            assert_eq!(private_tags, private(&[&new]));
        }
        // Where messages go is said to anyone even by a list that says nothing yet.
        let empty = ListedRelays::new(Some(a_list(Kind::InboxRelays, vec![], "", 100)), Kept::Read(vec![]));
        let (tags, private_tags) = empty.changed(Section::Messages, &[new.clone()]).unwrap();
        assert_eq!(tags.len(), 1);
        assert!(private_tags.is_empty());

        assert_eq!(Section::Blocked.kind().as_u16(), 10006);
        assert_eq!(Section::Search.kind().as_u16(), 10007);
        // The list of private relays is nobody's to read or write here.
        assert!(Section::ALL.iter().all(|section| section.kind().as_u16() != 10013));
        assert_eq!(Section::Messages.kind().as_u16(), 10050);
        assert_eq!(Section::Indexer.kind().as_u16(), 10086);
        assert_eq!(Section::Trusted.kind().as_u16(), 10089);
        let ahead = a_list(Kind::SearchRelays, vec![], "", 500);
        let written = listed(Section::Blocked, Some(&ahead), vec![], String::new(), Timestamp::from_secs(200));
        let written = written.finalize(&Keys::generate()).unwrap();
        assert_eq!((written.kind.as_u16(), written.created_at.as_secs()), (10006, 501));
    }
}

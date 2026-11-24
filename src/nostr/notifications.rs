// SPDX-License-Identifier: MIT

//! What others did that concerns the account: answered it, named it,
//! quoted it, reacted to a note of its own, passed one on, zapped it.
//!
//! All of these are events that name the account in a `p` tag, which is
//! what the relays are asked for: notes and comments (kinds 1 and 1111),
//! reactions (kind 7, NIP-25), reposts (kinds 6 and 16, NIP-18) and zap
//! receipts (kind 9735, NIP-57). What each one is, is read off the event
//! and off the note it is about, where that is at hand (see [`read`]):
//!
//! ```mermaid
//! flowchart TD
//!     event["An event that names the account"] --> own{"Did the account do it itself?"}
//!     own -->|"yes"| nothing["Nothing to tell"]
//!     own -->|"no"| kind{"Its kind"}
//!     kind -->|"a note, a comment"| said{"What it says"}
//!     said -->|"names the account in its text"| mention["Mentioned you"]
//!     said -->|"answers a note of the account's"| reply["Replied to you"]
//!     said -->|"answers someone else"| thread["Replied in a thread you are in"]
//!     said -->|"quotes a note of the account's"| quote["Quoted you"]
//!     said -->|"none of these"| mention
//!     kind -->|"a reaction, to the account last of all"| reaction["Reacted to your note"]
//!     kind -->|"a repost"| repost["Reposted your note"]
//!     kind -->|"a zap receipt"| zap["Zapped you: shown once the account's wallet service is known to have signed it"]
//! ```
//!
//! A zap receipt says who paid and how much, and anyone can write one. It
//! counts when it is signed by the key the account's own Lightning
//! address answers with (NIP-57): until that key is known, and for an
//! account with no such address, no zap is told of. See
//! [`Notifications::set_zapper`].
//!
//! Which of them are new is one moment, kept for the account on this
//! machine: what was written after the notifications were last looked at
//! (see [`Notifications::unread`]).

use std::collections::{HashMap, HashSet};

use nostr_sdk::prelude::{Event, EventId, Filter, Kind, PublicKey, Timestamp};

use crate::helpers::truncate_chars;
use crate::nostr::content::Segment;
use crate::nostr::engagement::Reaction;
use crate::nostr::models::Note;
use crate::nostr::zap::invoice_millisats;

/// The kinds of event that tell the account of something.
const NOTIFICATION_KINDS: [Kind; 6] =
    [Kind::TextNote, Kind::Comment, Kind::Reaction, Kind::Repost, Kind::GenericRepost, Kind::ZapReceipt];

/// How many events are asked for at a time.
pub const NOTIFICATIONS_PAGE: usize = 100;

/// How much of the note something is about is shown under it, and how
/// much of what was said with a zap, in characters.
const PREVIEW_MAX_CHARS: usize = 140;
const COMMENT_MAX_CHARS: usize = 140;

/// What someone did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    /// Answered a note of the account's.
    Reply,
    /// Answered someone else, in a conversation the account is named in.
    ThreadReply,
    /// Named the account in what they wrote.
    Mention,
    /// Quoted a note of the account's.
    Quote,
    /// Reacted to a note of the account's: with a like, or an emoji.
    Reaction(Reaction),
    /// Passed a note of the account's on.
    Repost,
    /// Zapped the account, or a note of its own: so many sats, with what
    /// was said along with them, as the receipt signed by `issuer` has it.
    Zap { sats: u64, comment: String, issuer: PublicKey },
}

/// Which of the notifications a list shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Every one.
    #[default]
    All,
    /// What was written to the account, or about it: replies, mentions
    /// and quotes.
    Mentions,
    /// What was made of its notes: reactions and reposts.
    Reactions,
    /// What it was paid.
    Zaps,
}

impl Tab {
    /// Every tab, in the order they are offered.
    pub const ALL: [Self; 4] = [Self::All, Self::Mentions, Self::Reactions, Self::Zaps];
}

/// Where in the list of all notifications the ones each tab shows are.
#[derive(Debug, Default)]
struct Listed {
    all: Vec<usize>,
    mentions: Vec<usize>,
    reactions: Vec<usize>,
    zaps: Vec<usize>,
}

impl Listed {
    /// The ones `tab` shows.
    fn of(&self, tab: Tab) -> &[usize] {
        match tab {
            Tab::All => &self.all,
            Tab::Mentions => &self.mentions,
            Tab::Reactions => &self.reactions,
            Tab::Zaps => &self.zaps,
        }
    }

    /// The ones `tab` shows, to add to.
    fn of_mut(&mut self, tab: Tab) -> &mut Vec<usize> {
        match tab {
            Tab::All => &mut self.all,
            Tab::Mentions => &mut self.mentions,
            Tab::Reactions => &mut self.reactions,
            Tab::Zaps => &mut self.zaps,
        }
    }
}

impl What {
    /// The tab this is under, beside the one everything is under.
    pub fn tab(&self) -> Tab {
        match self {
            Self::Reply | Self::ThreadReply | Self::Mention | Self::Quote => Tab::Mentions,
            Self::Reaction(_) | Self::Repost => Tab::Reactions,
            Self::Zap { .. } => Tab::Zaps,
        }
    }
}

/// One thing the account is told of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The event that says so.
    pub id: EventId,
    /// When it says it was written, in seconds since the Unix epoch.
    pub at: u64,
    /// Who did it.
    pub who: PublicKey,
    pub what: What,
    /// The note it is, when it is one: a reply, a mention, a quote.
    pub note: Option<Note>,
    /// The note it is about: the one answered, quoted, reacted to, passed
    /// on or zapped.
    pub about: Option<EventId>,
    /// The start of what that note says, if it was at hand.
    pub preview: Option<String>,
}

/// The question that finds what names `account`: so many events at most,
/// written after `since` and at or before `until`, where those are given.
pub fn filter(account: PublicKey, since: Option<u64>, until: Option<u64>, limit: usize) -> Filter {
    let mut filter = Filter::new().kinds(NOTIFICATION_KINDS).pubkey(account).limit(limit);
    if let Some(since) = since {
        filter = filter.since(Timestamp::from_secs(since.saturating_add(1)));
    }
    if let Some(until) = until {
        filter = filter.until(Timestamp::from_secs(until));
    }
    filter
}

/// The zap request a zap receipt carries, if it carries one that is what
/// it says: signed by whoever it says paid.
fn zap_request(receipt: &Event) -> Option<Event> {
    let description = receipt.tags.iter().find(|tag| tag.kind() == "description")?.content()?;
    let request = Event::from_json(description).ok()?;
    (request.kind == Kind::ZapRequest && request.verify().is_ok()).then_some(request)
}

/// The note `event` is about, which says what it tells of: the note a
/// note answers, or else the first it quotes; the note a reaction is to,
/// which is the last it names (NIP-25); the one a repost passes on or a
/// zap receipt is for, which is the first.
pub fn about(event: &Event) -> Option<EventId> {
    match event.kind {
        Kind::TextNote | Kind::Comment => {
            let note = Note::from(event.clone());
            note.reply_to.map(|reply| reply.parent).or_else(|| note.content.quotes.first().map(|quote| quote.id))
        }
        Kind::Reaction => event.tags.event_ids().last(),
        Kind::Repost | Kind::GenericRepost | Kind::ZapReceipt => event.tags.event_ids().next(),
        _ => None,
    }
}

/// The start of what `note` says, on one line: what stands for it where
/// there is no room for it.
fn preview(note: &Note) -> String {
    let said: String = note
        .content
        .segments
        .iter()
        .map(|segment| match segment {
            Segment::Text(text) => text.clone(),
            Segment::Link { label, .. } => label.clone(),
            Segment::Hashtag(tag) => format!("#{tag}"),
            Segment::Reference(label) => label.clone(),
            Segment::Mention(_) => "@\u{2026}".to_string(),
        })
        .collect();
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&said, PREVIEW_MAX_CHARS).into_owned()
}

/// What `event` tells `account` of, if anything, with `at_hand` the notes
/// such events are [`about`], as far as they were found.
///
/// Nothing, of what the account did itself, of an event that doesn't name
/// it, and of what is about a note that is at hand and is somebody
/// else's: a reaction to another's note in a conversation the account is
/// named in is no news for it.
pub fn read(event: &Event, account: PublicKey, at_hand: &HashMap<EventId, Note>) -> Option<Notification> {
    if !event.tags.public_keys().any(|named| named == account) {
        return None;
    }
    let about = about(event);
    let target = about.and_then(|id| at_hand.get(&id));
    let of_account = |note: &Note| note.author == account;
    let (who, what, note) = match event.kind {
        Kind::TextNote | Kind::Comment => {
            let note = Note::from(event.clone());
            let what = if note.content.mentions().any(|named| named == account) {
                What::Mention
            } else if note.reply_to.is_some() {
                if target.is_some_and(of_account) { What::Reply } else { What::ThreadReply }
            } else if !note.content.quotes.is_empty() && target.is_some_and(of_account) {
                What::Quote
            } else {
                What::Mention
            };
            (event.pubkey, what, Some(note))
        }
        Kind::Reaction => {
            // To the account, and not only in a conversation it is in:
            // the one reacted to is named last. A downvote is not told.
            let to_account = event.tags.public_keys().last() == Some(account);
            if !to_account || event.content == "-" || target.is_some_and(|note| !of_account(note)) {
                return None;
            }
            (event.pubkey, What::Reaction(Reaction::said(&event.content)), None)
        }
        Kind::Repost | Kind::GenericRepost => {
            if target.is_some_and(|note| !of_account(note)) {
                return None;
            }
            (event.pubkey, What::Repost, None)
        }
        Kind::ZapReceipt => {
            let request = zap_request(event)?;
            let invoice = event.tags.iter().find(|tag| tag.kind() == "bolt11")?.content()?;
            let sats = invoice_millisats(invoice)? / 1000;
            if sats == 0 {
                return None;
            }
            let comment = truncate_chars(request.content.trim(), COMMENT_MAX_CHARS).into_owned();
            (request.pubkey, What::Zap { sats, comment, issuer: event.pubkey }, None)
        }
        _ => return None,
    };
    if who == account {
        return None;
    }
    Some(Notification { id: event.id, at: event.created_at.as_secs(), who, what, note, about, preview: target.map(preview) })
}

/// What the account is told of, newest first, and which of it each tab
/// shows.
#[derive(Debug, Default)]
pub struct Notifications {
    all: Vec<Notification>,
    ids: HashSet<EventId>,
    /// Where in `all` the ones each tab shows are.
    listed: Listed,
    /// The key the account's wallet service signs zap receipts with, once
    /// it is known.
    zapper: Option<PublicKey>,
}

impl Notifications {
    /// Whether `notification` is shown: all are, but a zap whose receipt
    /// was not signed by the account's wallet service, as far as that is
    /// known.
    fn shown(&self, notification: &Notification) -> bool {
        match &notification.what {
            What::Zap { issuer, .. } => self.zapper == Some(*issuer),
            _ => true,
        }
    }

    /// Work out again which notifications each tab shows.
    fn arrange(&mut self) {
        let mut listed = Listed::default();
        for (at, notification) in self.all.iter().enumerate() {
            if self.shown(notification) {
                listed.of_mut(Tab::All).push(at);
                listed.of_mut(notification.what.tab()).push(at);
            }
        }
        self.listed = listed;
    }

    /// Add `found` to what is known, each once. Returns those that are
    /// news: not known before.
    pub fn merge(&mut self, found: Vec<Notification>) -> Vec<EventId> {
        let new: Vec<Notification> = found.into_iter().filter(|notification| self.ids.insert(notification.id)).collect();
        let ids = new.iter().map(|notification| notification.id).collect();
        self.all.extend(new);
        self.all.sort_by(|a, b| b.at.cmp(&a.at).then(a.id.cmp(&b.id)));
        self.arrange();
        ids
    }

    /// Let go of what was written before `oldest`.
    pub fn forget_before(&mut self, oldest: u64) {
        self.all.retain(|notification| notification.at >= oldest);
        self.ids = self.all.iter().map(|notification| notification.id).collect();
        self.arrange();
    }

    /// The key the account's wallet service signs zap receipts with is
    /// `zapper`: the zaps it signed show from now on, and no others.
    pub fn set_zapper(&mut self, zapper: Option<PublicKey>) {
        self.zapper = zapper;
        self.arrange();
    }

    /// Where the notifications `tab` shows are, newest first: what
    /// [`Self::get`] is asked with.
    pub fn listed(&self, tab: Tab) -> &[usize] {
        self.listed.of(tab)
    }

    /// The notification at `place`.
    pub fn get(&self, place: usize) -> Option<&Notification> {
        self.all.get(place)
    }

    /// The notes among them: the replies, mentions and quotes.
    pub fn notes(&self) -> impl Iterator<Item = &Note> + '_ {
        self.all.iter().filter_map(|notification| notification.note.as_ref())
    }

    /// When the newest of them was written, shown or not.
    pub fn newest(&self) -> Option<u64> {
        self.all.first().map(|notification| notification.at)
    }

    /// Whether `notification`, one of those shown, is new to someone who
    /// last looked at `seen_at`.
    pub fn is_new(notification: &Notification, seen_at: u64) -> bool {
        notification.at > seen_at
    }

    /// How many of those shown were written after `seen_at`.
    pub fn unread(&self, seen_at: u64) -> usize {
        // Newest first: the new ones are the first so many.
        self.listed(Tab::All).iter().take_while(|place| self.get(**place).is_some_and(|n| Self::is_new(n, seen_at))).count()
    }
}

#[cfg(test)]
mod tests {
    use super::{Notification, Notifications, Tab, What, about, filter, read};
    use crate::nostr::engagement::Reaction;
    use crate::nostr::models::Note;
    use nostr_sdk::prelude::{Event, EventBuilder, EventId, FinalizeEvent, Keys, Kind, PublicKey, Tag, Timestamp, ToBech32};
    use std::collections::HashMap;

    /// An event of `kind` saying `content`, with `tags`, signed by `keys`
    /// at the moment `at`.
    fn event(kind: Kind, content: &str, tags: Vec<Tag>, keys: &Keys, at: u64) -> Event {
        EventBuilder::new(kind, content).tags(tags).custom_created_at(Timestamp::from_secs(at)).finalize(keys).unwrap()
    }

    /// A tag `name` with `values`.
    fn tag(name: &str, values: &[&str]) -> Tag {
        Tag::parse(std::iter::once(name).chain(values.iter().copied())).unwrap()
    }

    /// The notes of `events`, by id: what is at hand.
    fn at_hand(events: &[&Event]) -> HashMap<EventId, Note> {
        events.iter().map(|event| (event.id, Note::from((*event).clone()))).collect()
    }

    /// A receipt, signed by `wallet`, of `payer` zapping `recipient` what
    /// `invoice` asks for, for the note `note`, saying `comment`.
    fn receipt(wallet: &Keys, payer: &Keys, recipient: PublicKey, note: Option<EventId>, invoice: &str, comment: &str) -> Event {
        let request = event(Kind::ZapRequest, comment, vec![Tag::public_key(recipient)], payer, 90);
        let mut tags = vec![Tag::public_key(recipient), tag("bolt11", &[invoice]), tag("description", &[&request.as_json()])];
        tags.extend(note.map(Tag::event));
        event(Kind::ZapReceipt, "", tags, wallet, 100)
    }

    #[test]
    fn what_a_note_tells_is_read_off_what_it_says_and_what_it_answers() {
        let (me, them, other) = (Keys::generate(), Keys::generate(), Keys::generate());
        let mine = event(Kind::TextNote, "what I said", vec![], &me, 10);
        let theirs = event(Kind::TextNote, "what someone else said", vec![Tag::public_key(me.public_key())], &other, 20);
        let known = at_hand(&[&mine, &theirs]);
        let naming = |tags: Vec<Tag>| [vec![Tag::public_key(me.public_key())], tags].concat();
        let told = |event: &Event| read(event, me.public_key(), &known).map(|told| told.what);

        // An answer to the account's note; to someone else's; to a note
        // that is not at hand, which is not known to be the account's.
        let reply = event(Kind::TextNote, "I agree", naming(vec![Tag::event(mine.id)]), &them, 30);
        assert_eq!(told(&reply), Some(What::Reply));
        let in_thread = event(Kind::TextNote, "so do I", naming(vec![Tag::event(theirs.id)]), &them, 31);
        assert_eq!(told(&in_thread), Some(What::ThreadReply));
        let unknown = event(Kind::TextNote, "and I", naming(vec![Tag::event(EventId::from_byte_array([9; 32]))]), &them, 32);
        assert_eq!(told(&unknown), Some(What::ThreadReply));

        // Named in the text, answering or not.
        let npub = me.public_key().to_bech32().unwrap();
        let mention = event(Kind::TextNote, &format!("ask nostr:{npub}"), naming(vec![]), &them, 33);
        assert_eq!(told(&mention), Some(What::Mention));
        let both = event(Kind::TextNote, &format!("nostr:{npub} look"), naming(vec![Tag::event(theirs.id)]), &them, 34);
        assert_eq!(told(&both), Some(What::Mention));

        // A quote of the account's note; of another's, which only names it.
        let quoting = |id: EventId| format!("this: nostr:{}", id.to_bech32().unwrap());
        let quote = event(Kind::TextNote, &quoting(mine.id), naming(vec![]), &them, 35);
        assert_eq!(told(&quote), Some(What::Quote));
        assert_eq!(about(&quote), Some(mine.id));
        let cited = event(Kind::TextNote, &quoting(theirs.id), naming(vec![]), &them, 36);
        assert_eq!(told(&cited), Some(What::Mention));

        // The note is there to show, with the start of what it answers.
        let read = read(&reply, me.public_key(), &known).unwrap();
        assert_eq!((read.id, read.at, read.who), (reply.id, 30, them.public_key()));
        assert_eq!(read.note.map(|note| note.id), Some(reply.id));
        assert_eq!((read.about, read.preview.as_deref()), (Some(mine.id), Some("what I said")));

        // Nothing of what the account wrote itself, or of what doesn't
        // name it.
        let own = event(Kind::TextNote, "to myself", naming(vec![Tag::event(mine.id)]), &me, 37);
        assert_eq!(told(&own), None);
        let elsewhere = event(Kind::TextNote, "I agree", vec![Tag::event(mine.id)], &them, 38);
        assert_eq!(told(&elsewhere), None);
    }

    #[test]
    fn a_reaction_or_a_repost_is_told_when_it_is_to_a_note_of_the_accounts() {
        let (me, them, other) = (Keys::generate(), Keys::generate(), Keys::generate());
        let mine = event(Kind::TextNote, "what I said\n\nat some length", vec![], &me, 10);
        let theirs = event(Kind::TextNote, "what someone else said", vec![], &other, 20);
        let known = at_hand(&[&mine, &theirs]);
        let told = |event: &Event| read(event, me.public_key(), &known);
        let (to_me, to_other) = (Tag::public_key(me.public_key()), Tag::public_key(other.public_key()));

        let liked = event(Kind::Reaction, "+", vec![Tag::event(mine.id), to_me.clone()], &them, 30);
        let read = told(&liked).unwrap();
        assert_eq!(read.what, What::Reaction(Reaction::Like));
        assert_eq!((read.about, read.preview.as_deref()), (Some(mine.id), Some("what I said at some length")));
        assert!(read.note.is_none());
        let clown = event(Kind::Reaction, "🤡", vec![Tag::event(mine.id), to_me.clone()], &them, 31);
        assert_eq!(told(&clown).map(|told| told.what), Some(What::Reaction(Reaction::Emoji("🤡".into()))));

        // To a note that is not at hand: told, with nothing to show of it.
        let unknown = event(Kind::Reaction, "+", vec![Tag::event(EventId::from_byte_array([9; 32])), to_me.clone()], &them, 32);
        assert_eq!(told(&unknown).map(|told| told.preview), Some(None));

        // Not a downvote; not a reaction to another in a conversation
        // the account is named in; not one to another's note.
        let down = event(Kind::Reaction, "-", vec![Tag::event(mine.id), to_me.clone()], &them, 33);
        assert!(told(&down).is_none());
        let beside = event(Kind::Reaction, "+", vec![Tag::event(mine.id), to_me.clone(), to_other], &them, 34);
        assert!(told(&beside).is_none());
        let to_theirs = event(Kind::Reaction, "+", vec![Tag::event(theirs.id), to_me.clone()], &them, 35);
        assert!(told(&to_theirs).is_none());

        let repost = event(Kind::Repost, "", vec![Tag::event(mine.id), to_me.clone()], &them, 36);
        assert_eq!(told(&repost).map(|told| (told.what, told.about)), Some((What::Repost, Some(mine.id))));
        let of_theirs = event(Kind::Repost, "", vec![Tag::event(theirs.id), to_me], &them, 37);
        assert!(told(&of_theirs).is_none());
    }

    #[test]
    fn a_zap_is_told_as_its_receipt_has_it_and_shown_when_the_accounts_wallet_signed_it() {
        let (me, payer, wallet, forger) = (Keys::generate(), Keys::generate(), Keys::generate(), Keys::generate());
        let note = EventId::from_byte_array([4; 32]);
        let told = |event: &Event| read(event, me.public_key(), &HashMap::new());

        let zapped = receipt(&wallet, &payer, me.public_key(), Some(note), "lnbc210n1pjq9xyz", " thanks ");
        let read = told(&zapped).unwrap();
        assert_eq!((read.who, read.about), (payer.public_key(), Some(note)));
        assert_eq!(read.what, What::Zap { sats: 21, comment: "thanks".into(), issuer: wallet.public_key() });

        // Not one with no amount, with no request, or with a request
        // somebody else signed; not what the account paid itself.
        assert!(told(&receipt(&wallet, &payer, me.public_key(), None, "lnbc1pjq9xyz", "")).is_none());
        let bare =
            event(Kind::ZapReceipt, "", vec![Tag::public_key(me.public_key()), tag("bolt11", &["lnbc210n1pjq9xyz"])], &wallet, 1);
        assert!(told(&bare).is_none());
        let request = event(Kind::ZapRequest, "", vec![Tag::public_key(me.public_key())], &payer, 90);
        let mut altered = serde_json::from_str::<serde_json::Value>(&request.as_json()).unwrap();
        altered["pubkey"] = forger.public_key().to_hex().into();
        let tags = vec![
            Tag::public_key(me.public_key()),
            tag("bolt11", &["lnbc210n1pjq9xyz"]),
            tag("description", &[&altered.to_string()]),
        ];
        assert!(told(&event(Kind::ZapReceipt, "", tags, &wallet, 1)).is_none());
        assert!(told(&receipt(&wallet, &me, me.public_key(), None, "lnbc210n1pjq9xyz", "")).is_none());

        // Shown once the wallet's key is known, and only what it signed.
        let forged = receipt(&forger, &payer, me.public_key(), None, "lnbc2500u1pvjluez", "a fortune");
        let mut notifications = Notifications::default();
        notifications.merge([&zapped, &forged].into_iter().filter_map(told).collect());
        assert!(notifications.listed(Tab::All).is_empty() && notifications.listed(Tab::Zaps).is_empty());
        assert_eq!(notifications.unread(0), 0);
        notifications.set_zapper(Some(wallet.public_key()));
        let shown: Vec<EventId> =
            notifications.listed(Tab::Zaps).iter().filter_map(|at| notifications.get(*at)).map(|n| n.id).collect();
        assert_eq!(shown, [zapped.id]);
        assert_eq!(notifications.unread(0), 1);
    }

    /// A notification of `what` by nobody in particular, written at `at`.
    fn told(byte: u8, at: u64, what: What) -> Notification {
        Notification {
            id: EventId::from_byte_array([byte; 32]),
            at,
            who: Keys::generate().public_key(),
            what,
            note: None,
            about: None,
            preview: None,
        }
    }

    #[test]
    fn they_are_kept_newest_first_each_once_under_its_tab_and_the_new_ones_are_counted() {
        let mut notifications = Notifications::default();
        let like = What::Reaction(Reaction::Like);
        let new = notifications.merge(vec![told(1, 10, What::Reply), told(2, 30, like.clone()), told(3, 20, What::Repost)]);
        assert_eq!(new.len(), 3);
        // Again, with one more: only that one is news.
        let new = notifications.merge(vec![told(2, 30, like), told(4, 40, What::Mention)]);
        assert_eq!(new, [EventId::from_byte_array([4; 32])]);

        let ids = |notifications: &Notifications, tab: Tab| -> Vec<u8> {
            notifications.listed(tab).iter().filter_map(|at| notifications.get(*at)).map(|n| n.id.as_bytes()[0]).collect()
        };
        assert_eq!(ids(&notifications, Tab::All), [4, 2, 3, 1]);
        assert_eq!(ids(&notifications, Tab::Mentions), [4, 1]);
        assert_eq!(ids(&notifications, Tab::Reactions), [2, 3]);
        assert!(ids(&notifications, Tab::Zaps).is_empty());
        assert_eq!(notifications.newest(), Some(40));

        // New is what was written after the last look.
        assert_eq!(notifications.unread(0), 4);
        assert_eq!(notifications.unread(20), 2);
        assert_eq!(notifications.unread(40), 0);

        // What is older than a moment can be let go of, and is then news
        // again when it comes back.
        notifications.forget_before(20);
        assert_eq!(ids(&notifications, Tab::All), [4, 2, 3]);
        assert_eq!(ids(&notifications, Tab::Mentions), [4]);
        assert_eq!(notifications.merge(vec![told(1, 10, What::Reply)]).len(), 1);
    }

    #[test]
    fn the_question_names_the_account_and_the_time_it_is_about() {
        let account = Keys::generate().public_key();
        let asked = filter(account, Some(100), Some(200), 50);
        assert_eq!(asked.limit, Some(50));
        // After `since`, not at it: what was written then is known.
        assert_eq!((asked.since, asked.until), (Some(Timestamp::from_secs(101)), Some(Timestamp::from_secs(200))));
        assert_eq!(asked.kinds.map(|kinds| kinds.len()), Some(6));
        let open = filter(account, None, None, 50);
        assert_eq!((open.since, open.until), (None, None));
    }
}

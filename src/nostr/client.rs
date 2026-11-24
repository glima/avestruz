// SPDX-License-Identifier: MIT

//! The relay client and the local event store behind it.
//!
//! [`NostrService`] wraps the `nostr-sdk` relay pool. Every event a relay sends
//! is checked (signature and id) by the SDK and kept in an LMDB store, so what
//! was seen once is there at once on the next start, before any relay answers.
//!
//! Each relay is asked for itself, and which of them sent a note is kept
//! (see [`Sightings`]): the pool, asked as one, gives each event once and
//! says no more than who was first.
//!
//! There are two pools. One holds the account's own relays: those the
//! config names, and those the account's relay list (NIP-65) says it writes
//! to. They are asked about everything, and are where what the account
//! writes goes. The other holds other people's relays, the ones their
//! relay lists name: asked about those people alone, for what the
//! account's own relays may not have, and sent what is meant for them (see
//! [`crate::nostr::outbox`]). They come and go: so many at a time, each
//! let go of when it has not been used for a while.
//!
//! ```mermaid
//! flowchart TB
//!     subgraph service["NostrService"]
//!         home["The home pool: the account's own relays"]
//!         others["The pool of other people's relays"]
//!         private["The relay it runs itself"]
//!         store[("One event store")]
//!     end
//!     asked["A question"] -->|"about anyone"| home
//!     asked -->|"about the people who list a relay"| others
//!     written["What the account writes"] --> home
//!     written -->|"if it names someone"| others
//!     draft["A draft"] --> private
//!     home --> store
//!     others --> store
//!     private --> store
//!     store -->|"what is shown is read from here"| shown["The model"]
//! ```
//!
//! # Finding one's way in this file
//!
//! It is long, and in this order:
//!
//! | Part | What is there |
//! |---|---|
//! | The constants | Every timeout and limit, each with its reason |
//! | The types | [`Reach`], [`Source`], [`RelayUse`], and the errors: [`FetchError`], [`PublishError`], [`FollowError`], [`ListError`] |
//! | Opening, and asking a relay | [`NostrService::open`], and the private `ask`, `fetch`, `gather` |
//! | Other people's relays | The private `relays_of`, `reach_for`, `connect_to`, `ask_their_relays` |
//! | The account's own relays | [`NostrService::use_relays_of`], and the config's standing down |
//! | Events | [`NostrService::event`], [`NostrService::publish`], [`NostrService::standing`] |
//! | Notes | [`NostrService::thread`], [`NostrService::cached_feed`], [`NostrService::fetch_feed`] |
//! | Lists | [`NostrService::set_following`], [`NostrService::set_bookmarked`], and the private `list_to_change` they both stand on |
//! | Drafts | [`NostrService::keep_draft`], [`NostrService::kept_drafts`] |
//! | Searching | [`NostrService::search`] |
//! | The account's relay lists | [`NostrService::relay_lists`], [`NostrService::set_advertised_relays`], [`NostrService::set_listed_relays`] |
//! | Profiles | [`NostrService::cached_profiles`], [`NostrService::fetch_profiles`] |
//! | The tests | Against relays started in the test: see [`crate::guide::testing`] |
//!
//! The decisions all of this carries out (which relay is asked what,
//! whose word is taken for a relay, where what is written goes) are told
//! in one place, with their diagrams: [`crate::guide::relays`].

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::future::{join, join_all};
use nostr_lmdb::NostrLmdb;
use nostr_sdk::prelude::{
    Client, Event, EventBuilder, EventId, Filter, Kind, PublicKey, RelayStatus, RelayUrl, ReqExitPolicy, SleepWhenIdle, Tag,
    Timestamp, Url,
};

use crate::nostr::account::{SignError, Signer};
use crate::nostr::blossom;
use crate::nostr::bookmarks::{self, Bookmarks, PrivateTags};
use crate::nostr::compose::Target;
use crate::nostr::contacts;
use crate::nostr::content::QUOTE_DEPTH;
use crate::nostr::engagement::{self, Doer, Left, Standing};
use crate::nostr::feed;
use crate::nostr::kept_drafts::{self, DRAFT_KIND};
use crate::nostr::models::{Note, Profile, reply_to};
use crate::nostr::notifications::{self, NOTIFICATIONS_PAGE, Notification};
use crate::nostr::outbox::{self, Listed, Whereabouts};
use crate::nostr::private_relay::PrivateRelay;
use crate::nostr::relay_lists::{self, Kept, ListedRelays, RelayLists, Section};
use crate::nostr::search::{self, Found, SearchFor};
use crate::nostr::sightings::Sightings;
use crate::nostr::sweep;

/// How many notes one page of a feed asks for.
pub const FEED_PAGE: usize = 50;

/// How long a request waits for a first relay to connect before giving up.
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// How long after startup a request still waits for relays that are in the
/// middle of connecting, so the first page comes from all of them rather
/// than from whichever answered first. After this, a relay that is still
/// trying is left out of a request instead of holding it up.
const CONNECT_GRACE: Duration = Duration::from_secs(2);

/// How often the relays' connection states are looked at while waiting.
const CONNECT_POLL: Duration = Duration::from_millis(100);

/// How long relays get to answer a request. Every query has one, so nothing
/// loads forever on a relay that stopped answering.
pub(crate) const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How many of other people's relays one question reaches for, and how
/// many are kept in their pool once those in use are done with, unless
/// said otherwise.
pub(crate) const REACH_CAP: usize = 20;

/// How many relays each person is read from, the account's own among them,
/// when their relay list names as many; and how many of the relays each
/// person reads are sent what is meant for them.
pub(crate) const RELAYS_PER_PERSON: usize = 2;

/// How long one of other people's relays gets to connect, and to answer.
/// Shorter than the account's own get: they are asked for what may well be
/// had already, and a list of notes waits for the slowest of them.
pub(crate) const REACH_CONNECT_WAIT: Duration = Duration::from_secs(2);
pub(crate) const REACH_TIMEOUT: Duration = Duration::from_secs(6);

/// How long a relay that did not connect, said no to what it was asked,
/// or did not answer in time is left alone, and the others asked without
/// it; and how near the time it had counts as running out of it.
pub(crate) const REACH_AVOID: Duration = Duration::from_secs(10 * 60);
const TIMEOUT_SLACK: Duration = Duration::from_millis(250);

/// How long one of other people's relays stays connected with nothing
/// asked of it, how long it stays in the pool unused, and how lately it
/// must have been used not to be let go of to make room for another.
pub(crate) const REACH_SLEEP: Duration = Duration::from_secs(2 * 60);
pub(crate) const REACH_KEEP: Duration = Duration::from_secs(15 * 60);
const REACH_BUSY: Duration = Duration::from_secs(60);

/// How long someone's relay list is taken as known before the relays are
/// asked for it again.
pub(crate) const RELAY_LISTS_FRESH: Duration = Duration::from_secs(60 * 60);

/// How long a relay that searches gets to answer.
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(8);

/// How many of the relays an account's own relay list says it writes to
/// are joined to the ones the config names.
pub(crate) const ACCOUNT_RELAYS_MAX: usize = 8;

/// How many of the people a note names are taken for part of its
/// conversation, when their relays are asked about it.
pub(crate) const THREAD_PEOPLE: usize = 5;

/// The kinds of event that are sent to the people they name, as well as
/// to the account's own relays: what is said to someone, or done to what
/// they wrote. Not a contact list, say, which names everyone followed.
const DELIVERED_KINDS: [Kind; 5] = [Kind::TextNote, Kind::Comment, Kind::Repost, Kind::GenericRepost, Kind::Reaction];

/// How far other people's relays are reached for.
#[derive(Debug, Clone, Copy)]
pub struct Reach {
    /// How many of them one question reaches for, and how many are kept
    /// once those in use are done with.
    pub cap: usize,
    /// Whether a relay that somebody's relay list names is one to connect
    /// to.
    pub accepts: fn(&RelayUrl) -> bool,
    /// Whether a relay the config names is on this machine or this
    /// network: one that stays in use whatever the account's relay list
    /// says, since no list would name it.
    pub local: fn(&RelayUrl) -> bool,
}

impl Default for Reach {
    fn default() -> Self {
        Self { cap: REACH_CAP, accepts: outbox::public, local: |relay| relay.is_localhost() || relay.is_local_addr() }
    }
}

/// The account's own relays: those the config names, those its relay
/// list names, and which of them are in use.
///
/// The config's are where an account is until it says where it is: where
/// its lists are looked for, and where it reads and writes while it has
/// no relay list. Once one of the relays its list says it writes to is
/// connected, those are its own, and the config's stand down: they are
/// not asked and not written to. Except those on this machine or this
/// network, which no list would name and which are there to be used.
#[derive(Default)]
struct Own {
    /// The relays the config names.
    config: Vec<RelayUrl>,
    /// The relays the account's relay list says it writes to, so many of
    /// them.
    listed: Vec<RelayUrl>,
    /// Those of `listed` that were put in the pool for it, the config not
    /// naming them, and are taken out when the list no longer does.
    added: Vec<RelayUrl>,
    /// Whether the config's relays stand down.
    stood_down: bool,
}

/// Which pool a relay is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pool {
    /// The account's own relays.
    Home,
    /// Other people's.
    Others,
}

/// What is remembered of reaching for other people's relays.
#[derive(Default)]
struct Reached {
    /// When each person's relay list was last asked of the relays.
    lists_asked: HashMap<PublicKey, Instant>,
    /// When each of other people's relays was last used.
    used: HashMap<RelayUrl, Instant>,
    /// When each relay, of either pool, last failed: did not connect,
    /// said no, or did not answer in time.
    failed: HashMap<RelayUrl, Instant>,
    /// The relays profiles and relay lists are looked up on, beside the
    /// account's own: those its list of them names.
    lookup: Vec<RelayUrl>,
    /// The relays the account never connects to on anyone's word.
    blocked: HashSet<RelayUrl>,
}

/// How a relay this is connected to has been doing, since the applet
/// started: what the relays page says under its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayUse {
    /// Whether it is connected now.
    pub connected: bool,
    /// How many bytes it sent, and how many were sent to it.
    pub received: usize,
    pub sent: usize,
    /// How many times it was connected to, and how many of those worked.
    pub attempts: usize,
    pub succeeded: usize,
    /// How long it takes to answer, on average, if that was measured.
    pub latency: Option<Duration>,
}

/// What a relay sent when it was asked.
struct Asked {
    events: Vec<Event>,
    /// It said that was all before its time ran out.
    in_time: bool,
}

/// How many notes of a conversation one request asks for. A thread that
/// went everywhere has thousands; this many are read, and held, at most.
const THREAD_LIMIT: usize = 300;

/// A conversation with fewer notes than this naming its root is asked about
/// again, for the notes that name only the note they answer. A larger one
/// has enough to show without the second request.
const THREAD_ASK_AGAIN_UNDER: usize = 50;

/// How many notes up from the one a thread was opened for are looked for one
/// by one, when nothing else brought them.
const THREAD_ANCESTORS: usize = 5;

/// Where events are looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The local event store: what is known already, at once.
    Store,
    /// The relays: what there is now, after a wait.
    Relays,
}

/// How long the relays get to send the notes that the notes of a page
/// quote, and again for those that these quote in turn. The page waits
/// for them, so that a note is shown with what it quotes or without, and
/// not first one way and then the other.
const QUOTES_WAIT: Duration = Duration::from_secs(4);

/// How many notes one filter asks for by id.
const IDS_PER_FILTER: usize = 100;

/// How long each relay gets to say whether it took an event.
pub(crate) const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the relays get to say which of them has an event that is about
/// to be pointed at. Short: what points at it goes out without, rather
/// than late.
const HOLDER_WAIT: Duration = Duration::from_secs(3);

/// How many notes one request asks about at a time, when asking what was
/// made of them, and how many events of a kind it asks for: a note that
/// went everywhere has more reactions than are worth carrying.
const ENGAGEMENT_NOTES_PER_REQUEST: usize = 25;
const ENGAGEMENT_LIMIT: usize = 500;

/// How many events of a kind the store is read for, for as many notes. More
/// than a request brings, since the store keeps what earlier requests
/// brought.
const ENGAGEMENT_STORE_LIMIT: usize = 5_000;

/// How long the receipt of a zap is waited for after its invoice is shown:
/// about as long as such an invoice can be paid.
const RECEIPT_WAIT: Duration = Duration::from_secs(10 * 60);

/// How long to wait before listening again, when the relays stopped
/// answering in the middle of waiting for a receipt.
const RECEIPT_RETRY: Duration = Duration::from_secs(5);

/// How many of a note's newest receipts are looked at when the listening
/// starts: enough to find one that arrived just before.
const RECEIPT_LIMIT: usize = 20;

/// Why an event did not go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// It could not be signed.
    Sign(SignError),
    /// Not one relay is connected, nor became so while waiting.
    NoRelayReachable,
    /// Every relay asked turned it down, or didn't answer: what one of them
    /// said.
    Refused(String),
    /// The relay pool refused the request.
    Failed(String),
}

/// Why a request to the relays brought nothing back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Not one relay is connected, nor became so while waiting.
    NoRelayReachable,
    /// The relay pool refused the request.
    Failed(String),
    /// Some relays answered and some didn't, and those that did have no
    /// list of the account's. Only said where that is not the same as
    /// there being none: before a list is written, which would replace
    /// the one a relay that didn't answer may have.
    Incomplete,
}

/// One page of what an account is told of.
#[derive(Debug, Clone, Default)]
pub struct Notified {
    /// What the events of the page tell of: fewer than there were events,
    /// when some had nothing to tell.
    pub notifications: Vec<Notification>,
    /// When the oldest event of the page was written: where the page
    /// before it ends.
    pub oldest: Option<u64>,
    /// Whether the page was as many events as were asked for: there may
    /// be older ones.
    pub full: bool,
}

/// How long the relays are given to send the notes notifications are
/// about, which only say what was answered or reacted to.
const ABOUT_WAIT: Duration = Duration::from_secs(4);

/// Where the event store lives: the user's data directory, not the cache.
/// It is what the embedded relay will serve, so it is not disposable.
pub fn default_store_path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("cosmic-applet-avestruz").join("events")
}

/// How many authors one filter names. Relays refuse filters past some size,
/// each its own; a long follow list is asked about in filters of this many.
const AUTHORS_PER_FILTER: usize = 250;

/// The kinds of event a list of notes is made of: text notes, and comments
/// (NIP-22), which are replies to what is not a text note.
const FEED_KINDS: [Kind; 2] = [Kind::TextNote, Kind::Comment];

/// The filters that ask for one page of the notes of `authors`: text notes
/// and comments, the `limit` newest written at or before `until`. One filter for every
/// [`AUTHORS_PER_FILTER`] authors, and none for nobody.
///
/// Each filter of a long list comes back with its own newest `limit`, which
/// [`feed::newest`] cuts as it cuts the answers of several relays, and for
/// the same reason.
/// The filters that ask for the events `ids` name: one for every
/// [`IDS_PER_FILTER`] of them.
fn filters_by_id(ids: &[EventId]) -> Vec<Filter> {
    ids.chunks(IDS_PER_FILTER).map(|chunk| Filter::new().ids(chunk.iter().copied())).collect()
}

fn feed_filters(authors: &[PublicKey], until: Option<u64>, limit: usize) -> Vec<Filter> {
    let page = Filter::new().kinds(FEED_KINDS).limit(limit);
    let page = match until {
        Some(until) => page.until(Timestamp::from_secs(until)),
        None => page,
    };
    authors.chunks(AUTHORS_PER_FILTER).map(|chunk| page.clone().authors(chunk.iter().copied())).collect()
}

/// The filters that ask what was made of `notes`: the replies (notes and
/// comments), reposts, reactions and zap receipts that name them, `limit` of
/// each at most, and the reactions and reposts of `account`, which must not
/// be among those left out.
fn engagement_filters(notes: &[EventId], account: Option<PublicKey>, limit: usize) -> Vec<Filter> {
    let about = |kinds: &[Kind]| Filter::new().kinds(kinds.iter().copied()).events(notes.iter().copied());
    let mut filters: Vec<Filter> = [
        &[Kind::TextNote][..],
        &[Kind::Comment],
        // Of either kind, a repost is one repost.
        &[Kind::Repost, Kind::GenericRepost],
        &[Kind::Reaction],
        &[Kind::ZapReceipt],
    ]
    .into_iter()
    .map(|kinds| about(kinds).limit(limit))
    .collect();
    if let Some(account) = account {
        filters.push(about(&[Kind::Reaction, Kind::Repost, Kind::GenericRepost]).author(account));
    }
    filters
}

/// Why an account's contact list was not changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowError {
    /// The newest list could not be asked for, and changing an older one
    /// would undo whatever came after it.
    Fetch(FetchError),
    /// The relays have no list of the account's. Writing a first one is
    /// for the account to decide: it replaces any list kept elsewhere.
    NoList,
    /// The changed list did not go out.
    Publish(PublishError),
}

/// Why a list an account keeps part of to itself (its bookmarks, its
/// search relays) or its relay list was not read, or not changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    /// The newest list could not be asked for, and changing an older one
    /// would undo whatever came after it.
    Fetch(FetchError),
    /// The relays have no list of the account's. Writing a first one is
    /// for the account to decide: it replaces any list kept elsewhere.
    NoList,
    /// What the list keeps to itself could not be read: it was not
    /// decrypted, or is not what such a list holds. A list written
    /// without it would lose it.
    Unreadable(SignError),
    /// The changed list did not go out.
    Publish(PublishError),
}

/// The newest of `events`, which are the versions of something only the
/// last of which counts: a contact list, a bookmark list. `None` if there
/// is none.
fn newest(events: impl IntoIterator<Item = Event>) -> Option<Event> {
    events.into_iter().max_by_key(|event| event.created_at)
}

/// Each author's newest profile among `events`.
fn newest_profiles(events: impl IntoIterator<Item = Event>) -> Vec<(PublicKey, Profile)> {
    let mut newest: HashMap<PublicKey, Event> = HashMap::new();
    for event in events {
        match newest.get(&event.pubkey) {
            Some(known) if known.created_at >= event.created_at => {}
            _ => {
                newest.insert(event.pubkey, event);
            }
        }
    }
    newest.into_iter().map(|(author, event)| (author, Profile::from_content(&event.content))).collect()
}

/// The Nostr client: a relay pool over a local event store. Cheap to clone;
/// clones share the pool and the store.
#[derive(Clone)]
pub struct NostrService {
    /// The pool of the account's own relays, over the event store.
    client: Client,
    /// The pool of other people's relays, over the same store.
    others: Client,
    /// The relay this runs itself, over the same store, for what is the
    /// account's alone.
    private: PrivateRelay,
    /// How far other people's relays are reached for.
    reach: Reach,
    /// What is remembered of reaching for them.
    reached: Arc<Mutex<Reached>>,
    /// Held while relays are being connected to in the pool of other
    /// people's: two questions that reach for the same relay at once
    /// would each take the other's connecting for a relay that didn't.
    reaching: Arc<tokio::sync::Mutex<()>>,
    /// The account's own relays, and which are in use.
    own: Arc<Mutex<Own>>,
    /// Held while the account's relays are changed, so that one
    /// account's are out before another's are in.
    joining: Arc<tokio::sync::Mutex<()>>,
    /// When the relays were first told to connect.
    opened_at: Instant,
    /// Which relays each note was seen on, since then.
    sightings: Arc<Mutex<Sightings>>,
}

impl fmt::Debug for NostrService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NostrService").finish_non_exhaustive()
    }
}

impl NostrService {
    /// Open the event store at `store_path` and start connecting to `relays`.
    /// Returns once the store is open; relays connect in the background.
    pub async fn open(store_path: PathBuf, relays: Vec<String>) -> Result<Self, String> {
        Self::open_reaching(store_path, relays, Reach::default()).await
    }

    /// As [`Self::open`], reaching for other people's relays as far as
    /// `reach` says.
    pub async fn open_reaching(store_path: PathBuf, relays: Vec<String>, reach: Reach) -> Result<Self, String> {
        tokio::fs::create_dir_all(&store_path).await.map_err(|e| format!("{}: {e}", store_path.display()))?;
        let store = NostrLmdb::open(&store_path).await.map_err(|e| e.to_string())?;
        let client = Client::builder().database(store).build();
        // Other people's relays are let go of when nothing is asked of
        // them, and woken when something is.
        let others = Client::builder()
            .database(client.database().clone())
            .sleep_when_idle(SleepWhenIdle::Enabled { timeout: REACH_SLEEP })
            .build();

        let mut config = Vec::new();
        for url in &relays {
            match RelayUrl::parse(url) {
                Ok(relay) => match client.add_relay(&relay).await {
                    Ok(_) => config.push(relay),
                    Err(e) => tracing::warn!("relay {url} was not added: {e}"),
                },
                Err(e) => tracing::warn!("relay {url} was not added: {e}"),
            }
        }
        client.connect().await;
        tracing::info!("event store open at {}; connecting to {} relay(s)", store_path.display(), relays.len());

        // The relay of this machine, for what goes to no other.
        let private = PrivateRelay::over(client.database().clone());

        Ok(Self {
            client,
            others,
            private,
            reach,
            reached: Arc::default(),
            reaching: Arc::default(),
            own: Arc::new(Mutex::new(Own { config, ..Own::default() })),
            joining: Arc::default(),
            opened_at: Instant::now(),
            sightings: Arc::default(),
        })
    }

    /// The relays a request can be sent to now: those that are connected.
    ///
    /// A request is only sent to these. Sent to the whole pool, it would
    /// wait its full timeout on every relay that is down, and one dead relay
    /// would make every page slow.
    ///
    /// With none connected, waits up to [`CONNECT_WAIT`] for the first. Just
    /// after startup it also gives relays that are still connecting until
    /// [`CONNECT_GRACE`] to join.
    async fn reachable_relays(&self) -> Vec<RelayUrl> {
        let deadline = Instant::now() + CONNECT_WAIT;
        loop {
            let mut reachable = Vec::new();
            let mut connecting = false;
            let in_use = self.in_use();
            for (url, relay) in self.client.relays().await.into_iter().filter(|(url, _)| in_use.contains(url)) {
                match relay.status() {
                    // A sleeping relay was connected, idled, and wakes when asked.
                    RelayStatus::Connected | RelayStatus::Sleeping => reachable.push(url),
                    RelayStatus::Initialized | RelayStatus::Pending | RelayStatus::Connecting => connecting = true,
                    RelayStatus::Disconnected | RelayStatus::Terminated | RelayStatus::Banned | RelayStatus::Shutdown => {}
                }
            }
            let settled = !connecting || self.opened_at.elapsed() >= CONNECT_GRACE;
            if (!reachable.is_empty() && settled) || Instant::now() >= deadline {
                return reachable;
            }
            tokio::time::sleep(CONNECT_POLL).await;
        }
    }

    /// Which relays each note was seen on. What is kept there is whole
    /// after every change to it, so it is as good after a panic as before.
    fn sightings(&self) -> MutexGuard<'_, Sightings> {
        self.sightings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The relays each of `notes` was seen on, for those that were seen on
    /// any.
    pub fn seen_on(&self, notes: &[EventId]) -> HashMap<EventId, Vec<RelayUrl>> {
        let sightings = self.sightings();
        notes.iter().map(|note| (*note, sightings.of(note))).filter(|(_, relays)| !relays.is_empty()).collect()
    }

    /// The pool `pool` names.
    fn pool(&self, pool: Pool) -> &Client {
        match pool {
            Pool::Home => &self.client,
            Pool::Others => &self.others,
        }
    }

    /// What `relay`, of the pool `pool`, sends for `filters`, until it says
    /// that is all or `timeout` has passed. An event it sends that can't be
    /// taken (a bad signature, say) is left out, and the rest still counts.
    ///
    /// A relay that says no to the question (it wants to know who asks, it
    /// keeps no such events) and sends nothing is an error. One that ran
    /// out of time is not, and what it sent counts; that it did is said
    /// with what it sent.
    async fn ask(&self, pool: Pool, relay: &RelayUrl, filters: Vec<Filter>, timeout: Duration) -> Result<Asked, String> {
        let started = Instant::now();
        let connection = self.pool(pool).relay(relay).await.map_err(|e| e.to_string())?.ok_or("not in the pool")?;
        let mut sent = connection.stream_events(filters).timeout(timeout).await.map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        let mut refusal = None;
        while let Some(event) = sent.next().await {
            match event {
                Ok(event) => events.push(event),
                Err(e) => refusal = Some(e.to_string()),
            }
        }
        match refusal {
            Some(refusal) if events.is_empty() => Err(refusal),
            refusal => {
                if let Some(refusal) = refusal {
                    tracing::warn!("{relay} sent an event that was not taken: {refusal}");
                }
                Ok(Asked { events, in_time: started.elapsed() + TIMEOUT_SLACK < timeout })
            }
        }
    }

    /// Leave `relay` alone for a while: it said no, or did not answer in
    /// time, and everything asked of it meanwhile would wait as long.
    fn leave_alone(&self, relay: &RelayUrl, why: &str) {
        tracing::debug!("{relay} is left alone for a while: {why}");
        self.reached().failed.insert(relay.clone(), Instant::now());
    }

    /// The relays that are being left alone, and those the account has
    /// blocked, which always are.
    fn left_alone(&self) -> HashSet<RelayUrl> {
        let now = Instant::now();
        let mut reached = self.reached();
        reached.failed.retain(|_, at| now.duration_since(*at) < REACH_AVOID);
        reached.failed.keys().chain(&reached.blocked).cloned().collect()
    }

    /// The account's own relays to ask: those that are connected, less
    /// the ones being left alone, unless that leaves none.
    async fn answering_relays(&self) -> Vec<RelayUrl> {
        let connected = self.reachable_relays().await;
        let left_alone = self.left_alone();
        let answering: Vec<RelayUrl> = connected.iter().filter(|relay| !left_alone.contains(relay)).cloned().collect();
        if answering.is_empty() { connected } else { answering }
    }

    /// Ask the connected relays for the events any of `filters` matches,
    /// newest first, and note which relay sent which note.
    async fn fetch(&self, filters: Vec<Filter>) -> Result<impl Iterator<Item = Event>, FetchError> {
        self.fetch_within(filters, FETCH_TIMEOUT).await
    }

    /// As [`Self::fetch`], giving the relays `timeout` to answer. A relay
    /// that fails is left out; if every one does, that is the error.
    async fn fetch_within(&self, filters: Vec<Filter>, timeout: Duration) -> Result<impl Iterator<Item = Event>, FetchError> {
        Ok(self.fetch_from_each(filters, timeout).await?.0.into_iter())
    }

    /// As [`Self::fetch_within`], and whether every relay of the pool
    /// answered: whether what didn't come is nowhere, as far as this
    /// client can know.
    async fn fetch_from_each(&self, filters: Vec<Filter>, timeout: Duration) -> Result<(Vec<Event>, bool), FetchError> {
        let relays = self.answering_relays().await;
        if relays.is_empty() {
            return Err(FetchError::NoRelayReachable);
        }
        let asks = relays.into_iter().map(|relay| (relay, filters.clone())).collect();
        let (events, answered, failure) = self.gather(Pool::Home, asks, timeout).await;
        if answered == 0 {
            return Err(FetchError::Failed(failure.unwrap_or_default()));
        }
        let everywhere = answered == self.in_use().len();
        Ok((events, everywhere))
    }

    /// What the relays of `asks`, which are of the pool `pool`, send for
    /// the filters each is asked with, each event once and the newest
    /// first; how many of them answered; and what one that didn't said.
    /// Which relay sent which note is noted, and a relay that said no, or
    /// ran out of time, is left alone for a while.
    async fn gather(
        &self,
        pool: Pool,
        asks: Vec<(RelayUrl, Vec<Filter>)>,
        timeout: Duration,
    ) -> (Vec<Event>, usize, Option<String>) {
        let answers = join_all(asks.iter().map(|(relay, filters)| self.ask(pool, relay, filters.clone(), timeout))).await;

        let mut events: HashMap<EventId, Event> = HashMap::new();
        let mut failure = None;
        let mut answered = 0;
        let (mut slow, mut refused) = (Vec::new(), Vec::new());
        {
            let mut sightings = self.sightings();
            for ((relay, _), answer) in asks.iter().zip(answers) {
                match answer {
                    Ok(asked) => {
                        answered += 1;
                        if !asked.in_time {
                            slow.push(relay);
                        }
                        for event in asked.events {
                            if FEED_KINDS.contains(&event.kind) {
                                sightings.record(event.id, relay);
                            }
                            events.entry(event.id).or_insert(event);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("{relay} could not be asked: {e}");
                        refused.push((relay, e.clone()));
                        failure = Some(e);
                    }
                }
            }
        }
        for relay in slow {
            self.leave_alone(relay, "it did not answer in time");
        }
        // One of the account's own that says no to one question may have
        // an answer to the next, and said no at once: it is asked again.
        if pool == Pool::Others {
            for (relay, why) in refused {
                self.leave_alone(relay, &why);
            }
        }
        let mut events: Vec<Event> = events.into_values().collect();
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        (events, answered, failure)
    }

    // =========================================================================
    // Other people's relays
    // =========================================================================

    /// What is remembered of reaching for other people's relays. Whole
    /// after every change to it, as [`Self::sightings`] is.
    fn reached(&self) -> MutexGuard<'_, Reached> {
        self.reached.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The relays `people` list for `listed`, for those of them who have a
    /// relay list with any that may be connected to.
    ///
    /// The account's own relays are asked for the lists of those not asked
    /// about lately, and what they send is the store's, which is what is
    /// read: a list seen once is known without a relay, and the newest
    /// seen is the one that counts. With `patient`, their answer is
    /// waited for; without, only when the store knows nobody's list, and
    /// otherwise it serves the next time: a list of notes is not kept
    /// waiting for lists that seldom change.
    async fn relays_of(&self, people: &[PublicKey], listed: Listed, patient: bool) -> HashMap<PublicKey, Vec<RelayUrl>> {
        let now = Instant::now();
        let stale: Vec<PublicKey> = {
            let mut reached = self.reached();
            let stale: Vec<PublicKey> = people
                .iter()
                .filter(|person| reached.lists_asked.get(person).is_none_or(|at| now.duration_since(*at) >= RELAY_LISTS_FRESH))
                .copied()
                .collect();
            // Marked before they are asked for, so that they are asked for once.
            reached.lists_asked.extend(stale.iter().map(|person| (*person, now)));
            stale
        };
        let mut lists = self.stored_relay_lists(people).await;
        if !stale.is_empty() {
            if patient || lists.is_empty() {
                self.ask_for_relay_lists(stale).await;
                lists = self.stored_relay_lists(people).await;
            } else {
                // What the store has serves now, and what the relays have
                // since serves the next time.
                let service = self.clone();
                tokio::spawn(async move { service.ask_for_relay_lists(stale).await });
            }
        }
        lists
            .into_iter()
            .map(|list| {
                let mut relays = outbox::relays(&list, listed);
                relays.retain(|relay| (self.reach.accepts)(relay));
                (list.pubkey, relays)
            })
            .filter(|(_, relays)| !relays.is_empty())
            .collect()
    }

    /// The filters that ask for the relay lists of `people`.
    fn relay_lists_of(people: &[PublicKey]) -> Vec<Filter> {
        people
            .chunks(AUTHORS_PER_FILTER)
            .map(|chunk| Filter::new().kind(Kind::RelayList).authors(chunk.iter().copied()))
            .collect()
    }

    /// Ask the account's own relays for the relay lists of `people`, for
    /// the store's sake: it is what is read. If they can't be asked,
    /// `people` count as not asked about.
    async fn ask_for_relay_lists(&self, people: Vec<PublicKey>) {
        let filters = Self::relay_lists_of(&people);
        let (own, _looked_up) = join(self.fetch_within(filters.clone(), REACH_TIMEOUT), self.ask_lookup(filters)).await;
        if let Err(e) = own {
            tracing::warn!("{} relay list(s) could not be asked for: {e:?}", people.len());
            let mut reached = self.reached();
            for person in &people {
                reached.lists_asked.remove(person);
            }
        }
    }

    /// The newest relay list of each of `people` that the store has one of.
    async fn stored_relay_lists(&self, people: &[PublicKey]) -> Vec<Event> {
        let mut newest: HashMap<PublicKey, Event> = HashMap::new();
        for list in self.events(Source::Store, Self::relay_lists_of(people)).await.unwrap_or_default() {
            match newest.get(&list.pubkey) {
                Some(known) if known.created_at >= list.created_at => {}
                _ => {
                    newest.insert(list.pubkey, list);
                }
            }
        }
        newest.into_values().collect()
    }

    /// The relays to ask about, or send to, the people of `listed`, beside
    /// the account's own, and who is on each: so many for each person, as
    /// many in all as may be reached for, none that failed lately, and
    /// only those that are connected now. See [`outbox::plan`].
    async fn reach_for(&self, listed: &HashMap<PublicKey, Vec<RelayUrl>>) -> Vec<(RelayUrl, Vec<PublicKey>)> {
        if listed.is_empty() || self.reach.cap == 0 {
            return Vec::new();
        }
        let home: HashSet<RelayUrl> = self.in_use().into_iter().collect();
        let planned = outbox::plan(listed, &home, &self.left_alone(), RELAYS_PER_PERSON, self.reach.cap);
        if planned.is_empty() {
            return Vec::new();
        }
        let wanted = planned.len();
        let reached = self.connected(planned).await;
        let said = |(relay, people): &(RelayUrl, Vec<PublicKey>)| {
            format!("{} for {}", relay.as_str_without_trailing_slash(), people.len())
        };
        tracing::debug!(
            "{} people with a relay list: {} of {wanted} of their relays reached ({})",
            listed.len(),
            reached.len(),
            reached.iter().map(said).collect::<Vec<_>>().join(", ")
        );
        reached
    }

    /// Those of `planned`, each a relay and what it is wanted for, whose
    /// relay is in the pool of other people's relays and connected, once
    /// all have been given the time to connect. The pool is then no bigger
    /// than it may be, but for the relays in use.
    ///
    /// The only way into [`Self::connect_to`], which is not to be in
    /// twice at once for the same relay.
    async fn connected<T>(&self, planned: Vec<(RelayUrl, T)>) -> Vec<(RelayUrl, T)> {
        // One question's relays at a time: the next finds those it shares
        // with this one connected already.
        let _reaching = self.reaching.lock().await;
        let connected = join_all(planned.iter().map(|(relay, _)| self.connect_to(relay))).await;
        self.let_go_of_unused().await;
        planned.into_iter().zip(connected).filter(|(_, connected)| *connected).map(|(planned, _)| planned).collect()
    }

    /// What the relays `said` to have notes send of them: each relay is
    /// asked for the notes it is said to have. A relay that may not be
    /// connected to, or is being left alone, is not asked, and neither is
    /// one of the account's own, which were asked; if there are more than
    /// may be reached for, the ones said to have the most are. Nothing,
    /// and never an error, when none is reached: this is on top of what
    /// was had.
    async fn ask_where_said(&self, said: &[(EventId, RelayUrl)]) -> Vec<Event> {
        if said.is_empty() || self.reach.cap == 0 {
            return Vec::new();
        }
        let home: HashSet<RelayUrl> = self.in_use().into_iter().collect();
        let left_alone = self.left_alone();
        // Ordered, so that the same relays are asked every time.
        let mut having: BTreeMap<&RelayUrl, Vec<EventId>> = BTreeMap::new();
        for (note, relay) in said {
            if (self.reach.accepts)(relay) && !home.contains(relay) && !left_alone.contains(relay) {
                having.entry(relay).or_default().push(*note);
            }
        }
        let mut having: Vec<(RelayUrl, Vec<EventId>)> = having.into_iter().map(|(relay, notes)| (relay.clone(), notes)).collect();
        having.sort_by_key(|(_, notes)| std::cmp::Reverse(notes.len()));
        having.truncate(self.reach.cap);
        let asks: Vec<(RelayUrl, Vec<Filter>)> =
            self.connected(having).await.into_iter().map(|(relay, notes)| (relay, filters_by_id(&notes))).collect();
        if asks.is_empty() {
            return Vec::new();
        }
        let asked = asks.len();
        let (events, answered, _) = self.gather(Pool::Others, asks, REACH_TIMEOUT).await;
        tracing::debug!("{} note(s) from {answered} of {asked} relay(s) said to have them", events.len());
        events
    }

    /// Have `relay` in the pool of other people's relays, connected.
    /// Returns whether it is. One that doesn't connect is taken out again,
    /// and left alone for a while.
    async fn connect_to(&self, relay: &RelayUrl) -> bool {
        if self.reached().blocked.contains(relay) {
            return false;
        }
        let connected = |status: RelayStatus| matches!(status, RelayStatus::Connected | RelayStatus::Sleeping);
        let status = match self.others.relay(relay).await {
            Ok(Some(known)) => known.status(),
            _ => {
                if let Err(e) = self.others.add_relay(relay).await {
                    tracing::warn!("{relay} was not added to the pool: {e}");
                }
                RelayStatus::Initialized
            }
        };
        if !connected(status) {
            let _connecting = self.others.try_connect_relay(relay, REACH_CONNECT_WAIT).await;
        }
        let now_connected = matches!(self.others.relay(relay).await, Ok(Some(known)) if connected(known.status()));
        if now_connected {
            self.reached().used.insert(relay.clone(), Instant::now());
        } else {
            self.leave_alone(relay, "it did not connect");
            self.reached().used.remove(relay);
            if let Err(e) = self.others.remove_relay(relay).force().await {
                tracing::debug!("{relay} could not be taken out of the pool: {e}");
            }
        }
        now_connected
    }

    /// Take out of the pool of other people's relays those that have not
    /// been used for a while, and, if there are more than may be reached
    /// for, the ones used longest ago; but none used just now, which may
    /// be in the middle of answering.
    async fn let_go_of_unused(&self) {
        let pooled: Vec<RelayUrl> = self.others.relays().await.into_keys().collect();
        let now = Instant::now();
        let let_go: Vec<RelayUrl> = {
            let mut reached = self.reached();
            let unused = |relay: &RelayUrl| reached.used.get(relay).map_or(Duration::MAX, |at| now.duration_since(*at));
            let pooled = pooled.into_iter().map(|relay| (unused(&relay), relay)).collect();
            let let_go = outbox::to_let_go(pooled, self.reach.cap, REACH_KEEP, REACH_BUSY);
            for relay in &let_go {
                reached.used.remove(relay);
            }
            let_go
        };
        for relay in let_go {
            tracing::debug!("{relay} is let go of");
            if let Err(e) = self.others.remove_relay(&relay).force().await {
                tracing::debug!("{relay} could not be taken out of the pool: {e}");
            }
        }
    }

    /// What the relays that `people` list for `listed` send about them,
    /// beside what the account's own relays have: each relay reached for
    /// is asked with the filters `asking` makes for the people who list it.
    /// Nothing, when nobody lists a relay that is reached; never an error,
    /// since what is asked for here is on top of what was had. See
    /// [`Self::relays_of`] for `patient`.
    async fn ask_their_relays(
        &self,
        people: &[PublicKey],
        listed: Listed,
        patient: bool,
        asking: impl Fn(&[PublicKey]) -> Vec<Filter>,
    ) -> Vec<Event> {
        if people.is_empty() {
            return Vec::new();
        }
        let listed = self.relays_of(people, listed, patient).await;
        let asks: Vec<(RelayUrl, Vec<Filter>)> = self
            .reach_for(&listed)
            .await
            .into_iter()
            .map(|(relay, people)| (relay, asking(&people)))
            .filter(|(_, filters)| !filters.is_empty())
            .collect();
        if asks.is_empty() {
            return Vec::new();
        }
        let asked = asks.len();
        let (events, answered, _) = self.gather(Pool::Others, asks, REACH_TIMEOUT).await;
        tracing::debug!("{} event(s) from {answered} of {asked} of other people's relays", events.len());
        events
    }

    /// The account's own relays and which are in use, whole after every
    /// change, as [`Self::sightings`] is.
    fn own(&self) -> MutexGuard<'_, Own> {
        self.own.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The account's own relays that are in use, connected or not: those
    /// its relay list says it writes to, and those the config names
    /// unless they stand down (see [`Own`]).
    fn in_use(&self) -> Vec<RelayUrl> {
        let own = self.own();
        let stays = |relay: &&RelayUrl| !own.stood_down || (self.reach.local)(relay) || own.listed.contains(relay);
        let mut in_use: Vec<RelayUrl> = own.config.iter().filter(stays).cloned().collect();
        for relay in &own.listed {
            if !in_use.contains(relay) {
                in_use.push(relay.clone());
            }
        }
        in_use
    }

    /// Have the relays that the relay list of `account` says it writes
    /// to, so many of them, be its own: where what it wrote is and where
    /// what it writes goes, in place of those the config names once one
    /// of them is connected (see [`Own`]). Those of the account before,
    /// if any, are taken out first; with no account, that is all, and the
    /// config's relays are the ones in use. Returns how many joined the
    /// pool.
    ///
    /// The list the store has is gone by at once, and the newest the
    /// relays have when they have answered.
    pub async fn use_relays_of(&self, account: Option<PublicKey>) -> usize {
        // The relay of this machine takes its account's events, and
        // nobody's when nobody is signed in.
        self.private.set_owner(account);
        let Some(account) = account else {
            return self.join_account_relays(Vec::new()).await;
        };
        // The account's own word: any relay it names, on this machine too.
        let writes = |list: &Event| outbox::relays(list, Listed::Write);
        let stored = self.stored_list(Kind::RelayList, account).await;
        if let Some(stored) = &stored {
            self.join_account_relays(writes(stored)).await;
        }
        let list = match self.newest_list(Kind::RelayList, account).await {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!("the account's relay list could not be asked for: {e:?}");
                stored
            }
        };
        self.join_account_relays(list.as_ref().map(writes).unwrap_or_default()).await
    }

    /// Have `writes`, so many of them, be the relays the account's relay
    /// list names as its own: those that were put in the pool for it and
    /// are not among them are taken out, those that are stay as they are,
    /// connected, and the new ones get a moment to connect. Whether the
    /// config's relays stand down is settled by whether any of `writes`
    /// is connected then. Returns how many are in the pool for the
    /// account's list.
    async fn join_account_relays(&self, writes: Vec<RelayUrl>) -> usize {
        let wanted: Vec<RelayUrl> = writes.into_iter().filter(|relay| !relay.is_onion()).take(ACCOUNT_RELAYS_MAX).collect();
        let _joining = self.joining.lock().await;
        let before = self.own().added.clone();
        for relay in before.iter().filter(|relay| !wanted.contains(relay)) {
            if let Err(e) = self.client.remove_relay(relay).force().await {
                tracing::warn!("{relay} could not be taken out of the pool: {e}");
            }
        }
        let (mut added, mut new) = (Vec::new(), Vec::new());
        for relay in &wanted {
            if before.contains(relay) {
                added.push(relay.clone());
                continue;
            }
            match self.client.add_relay(relay).await {
                // Not one the config names: it is here for the account.
                Ok(true) => new.push(relay.clone()),
                // One the config names, which may have been hung up on.
                Ok(false) => {
                    if let Err(e) = self.client.connect_relay(relay).await {
                        tracing::warn!("{relay} could not be told to connect: {e}");
                    }
                }
                Err(e) => tracing::warn!("{relay} was not added to the pool: {e}"),
            }
        }
        let connecting = new.iter().map(|relay| self.client.try_connect_relay(relay, REACH_CONNECT_WAIT));
        join_all(connecting).await;
        added.extend(new);

        let pool = self.client.relays().await;
        let connected = |relay: &RelayUrl| {
            pool.get(relay).is_some_and(|known| matches!(known.status(), RelayStatus::Connected | RelayStatus::Sleeping))
        };
        let stood_down = wanted.iter().any(connected);
        let joined = added.len();
        {
            let mut own = self.own();
            (own.listed, own.added, own.stood_down) = (wanted, added, stood_down);
        }
        tracing::info!("{joined} relay(s) of the account's relay list joined its own; the config's stand down: {stood_down}");
        self.settle_config_relays().await;
        joined
    }

    /// Have the config's relays that are in use connected, and hang up on
    /// those that stand down: after a while, so that what was asked of
    /// them before they did is answered.
    async fn settle_config_relays(&self) {
        let in_use = self.in_use();
        let config = self.own().config.clone();
        for relay in config.iter().filter(|relay| in_use.contains(relay)) {
            if let Err(e) = self.client.connect_relay(relay).await {
                tracing::warn!("{relay} could not be told to connect: {e}");
            }
        }
        if config.iter().all(|relay| in_use.contains(relay)) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FETCH_TIMEOUT).await;
            // As things stand by now: an account may have left meanwhile.
            let in_use = service.in_use();
            for relay in config.iter().filter(|relay| !in_use.contains(relay)) {
                tracing::debug!("{relay}, which the config names, stands down");
                if let Err(e) = service.client.disconnect_relay(relay).await {
                    tracing::debug!("{relay} could not be hung up on: {e}");
                }
            }
        });
    }

    // =========================================================================
    // Events
    // =========================================================================

    /// The event `id`, if the store has it.
    pub async fn stored_event(&self, id: EventId) -> Option<Event> {
        self.events(Source::Store, vec![Filter::new().id(id)]).await.ok()?.into_iter().next()
    }

    /// The event `id`, which is a note on show: the store's, or, since
    /// the store lets go of what is old while it may still be showing (see
    /// [`sweep`]), the one the relays send again: the account's own, and
    /// then the ones `author` writes to. `None` if none of them has it.
    pub async fn event(&self, id: EventId, author: PublicKey) -> Option<Event> {
        if let Some(stored) = self.stored_event(id).await {
            return Some(stored);
        }
        let is_it = |event: &Event| event.id == id;
        if let Ok(mut fetched) = self.fetch_within(filters_by_id(&[id]), QUOTES_WAIT).await
            && let Some(event) = fetched.find(is_it)
        {
            return Some(event);
        }
        let whereabouts = Whereabouts { author: Some(author), relay: None };
        self.events_elsewhere(&[(id, &whereabouts)], QUOTES_WAIT).await.into_iter().find(is_it)
    }

    /// Have the store let go of what was written at `before` or earlier
    /// and can be asked of the relays again (see [`sweep`]): not what
    /// `account` wrote or `bookmarked`. Returns how many events went.
    pub async fn sweep(&self, account: PublicKey, bookmarked: &HashSet<EventId>, before: Timestamp) -> Result<usize, String> {
        sweep::sweep(self.client.database().as_ref(), account, bookmarked, before).await
    }

    /// A connected relay that has the event `id`: where someone else can be
    /// told to look for it. The first to send it; `None` if none does
    /// before long.
    pub async fn relay_holding(&self, id: EventId) -> Option<RelayUrl> {
        let filter = Filter::new().id(id).limit(1);
        let targets: HashMap<RelayUrl, Vec<Filter>> =
            self.answering_relays().await.into_iter().map(|url| (url, vec![filter.clone()])).collect();
        if targets.is_empty() {
            return None;
        }
        let mut answers = self.client.stream_events(targets).timeout(HOLDER_WAIT).await.ok()?;
        while let Some((relay, event)) = answers.next().await {
            if event.is_ok_and(|event| event.id == id) {
                return Some(relay);
            }
        }
        None
    }

    /// Have `signer` sign the event `unsigned` describes as `author`, and
    /// send it to the connected relays.
    pub async fn sign_and_publish(
        &self,
        signer: &Signer,
        author: PublicKey,
        unsigned: EventBuilder,
    ) -> Result<Event, PublishError> {
        let event = signer.sign(author, unsigned).await.map_err(PublishError::Sign)?;
        let accepted = self.publish(&event).await?;
        tracing::info!("published {} (kind {}) to {accepted} relay(s)", event.id, event.kind.as_u16());
        Ok(event)
    }

    /// How `notes` stand, as far as `source` knows: what was made of them,
    /// with the likes and reposts of `account` told apart, and which relays
    /// they were seen on.
    ///
    /// The count is always of what the store holds. Asking the relays
    /// brings what they have into the store first, so an answer from them
    /// is never less than the store's own.
    ///
    /// The relays are asked for the notes themselves as well: each that
    /// sends one has it, and is one of those it was [seen on](Standing).
    pub async fn standing(
        &self,
        source: Source,
        notes: Vec<EventId>,
        account: Option<PublicKey>,
    ) -> Result<Standing, FetchError> {
        let mut events = Vec::new();
        for chunk in notes.chunks(ENGAGEMENT_NOTES_PER_REQUEST) {
            if source == Source::Relays {
                let mut filters = engagement_filters(chunk, account, ENGAGEMENT_LIMIT);
                filters.push(Filter::new().ids(chunk.iter().copied()));
                // Fetched for the store's sake: it is what is counted.
                let _into_the_store = self.fetch(filters).await?;
            }
            events.extend(self.events(Source::Store, engagement_filters(chunk, account, ENGAGEMENT_STORE_LIMIT)).await?);
        }
        let seen_on = self.seen_on(&notes);
        Ok(Standing { engagement: engagement::tally(events, &notes.into_iter().collect(), account), seen_on })
    }

    /// Who left `what` on `note`, as far as the store holds: the same
    /// events its count is of (see [`Self::standing`]), so that who is
    /// listed is who was counted.
    pub async fn who_did(&self, note: EventId, what: Left) -> Result<Vec<Doer>, FetchError> {
        let filter = Filter::new().kinds(what.kinds().iter().copied()).event(note).limit(ENGAGEMENT_STORE_LIMIT);
        let events = self.events(Source::Store, vec![filter]).await?;
        Ok(engagement::who_did(events, note, what))
    }

    /// Listen on the connected relays for the receipts of `note` until one
    /// of them `is_it`. Returns whether one was; `false` when the relays
    /// stopped sending first.
    async fn receipts_until(&self, note: EventId, is_it: &impl Fn(&Event) -> bool) -> bool {
        let relays = self.reachable_relays().await;
        let filter = Filter::new().kind(Kind::ZapReceipt).event(note).limit(RECEIPT_LIMIT);
        let targets: HashMap<RelayUrl, Vec<Filter>> = relays.into_iter().map(|url| (url, vec![filter.clone()])).collect();
        if targets.is_empty() {
            return false;
        }
        // One request, kept open: what the relays have now, and whatever
        // arrives after.
        let listening = self
            .client
            .stream_events(targets)
            .timeout(RECEIPT_WAIT)
            .policy(ReqExitPolicy::WaitDurationAfterEOSE(RECEIPT_WAIT))
            .await;
        let Ok(mut receipts) = listening else {
            return false;
        };
        while let Some((_, receipt)) = receipts.next().await {
            if receipt.is_ok_and(|receipt| is_it(&receipt)) {
                return true;
            }
        }
        false
    }

    /// Wait for the receipt that says the invoice `bolt11`, which zaps
    /// `note`, was paid. Returns whether it came within [`RECEIPT_WAIT`].
    ///
    /// The receipt is kept in the store as everything a relay sends is, so
    /// the note's count has it afterwards.
    pub async fn await_zap_receipt(&self, note: EventId, bolt11: String) -> bool {
        let is_it = |receipt: &Event| {
            receipt
                .tags
                .iter()
                .any(|tag| tag.kind() == "bolt11" && tag.content().is_some_and(|invoice| invoice.eq_ignore_ascii_case(&bolt11)))
        };
        let waiting = async {
            // Relays drop connections; a wait this long outlives some.
            while !self.receipts_until(note, &is_it).await {
                tokio::time::sleep(RECEIPT_RETRY).await;
            }
        };
        tokio::time::timeout(RECEIPT_WAIT, waiting).await.is_ok()
    }

    /// Send `event` to the connected relays. It has gone out when at least
    /// one took it, and is then kept in the store as well, as anything a
    /// relay sends is. Returns how many took it.
    ///
    /// An event no relay took is not kept: the store would show as
    /// published what nobody else can see.
    pub async fn publish(&self, event: &Event) -> Result<usize, PublishError> {
        let (own, theirs) = join(self.send_home(event), self.deliver(event)).await;
        let took: Vec<RelayUrl> = match own {
            Ok(own) => own.into_iter().chain(theirs).collect(),
            Err(e) if theirs.is_empty() => return Err(e),
            Err(e) => {
                tracing::warn!("none of the account's own relays took the event: {e:?}");
                theirs
            }
        };
        if let Err(e) = self.client.database().save_event(event).await {
            tracing::warn!("the published event could not be kept in the store: {e}");
        }
        if FEED_KINDS.contains(&event.kind) {
            // The relays that took it have it.
            let mut sightings = self.sightings();
            for relay in &took {
                sightings.record(event.id, relay);
            }
        }
        Ok(took.len())
    }

    /// Send `event` to the account's own relays that are connected: those
    /// that took it, or why none did.
    async fn send_home(&self, event: &Event) -> Result<Vec<RelayUrl>, PublishError> {
        let relays = self.reachable_relays().await;
        if relays.is_empty() {
            return Err(PublishError::NoRelayReachable);
        }
        let output = self
            .client
            .send_event(event)
            .to(relays)
            .save_into_database(false)
            .ok_timeout(PUBLISH_TIMEOUT)
            .await
            .map_err(|e| PublishError::Failed(e.to_string()))?;
        if output.success.is_empty() {
            let reason = output.failed.into_values().next().unwrap_or_default();
            return Err(PublishError::Refused(reason));
        }
        for (relay, reason) in &output.failed {
            tracing::warn!("{relay} did not take the event: {reason}");
        }
        Ok(output.success.into_keys().collect())
    }

    /// Send `event` to the people it names, if it is of a kind that is
    /// meant for them: to the relays they read, as their relay lists have
    /// them, so many for each and no more in all than may be reached for.
    /// Returns those that took it. That none did is no error: it is on the
    /// account's own relays, where anyone can find it.
    async fn deliver(&self, event: &Event) -> Vec<RelayUrl> {
        if !DELIVERED_KINDS.contains(&event.kind) {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        let people: Vec<PublicKey> =
            event.tags.public_keys().filter(|person| *person != event.pubkey && seen.insert(*person)).collect();
        if people.is_empty() {
            return Vec::new();
        }
        let listed = self.relays_of(&people, Listed::Read, true).await;
        let relays: Vec<RelayUrl> = self.reach_for(&listed).await.into_iter().map(|(relay, _)| relay).collect();
        if relays.is_empty() {
            return Vec::new();
        }
        match self.others.send_event(event).to(relays).save_into_database(false).ok_timeout(PUBLISH_TIMEOUT).await {
            Ok(output) => {
                for (relay, reason) in &output.failed {
                    tracing::debug!("{relay}, which someone named reads, did not take the event: {reason}");
                }
                output.success.into_keys().collect()
            }
            Err(e) => {
                tracing::warn!("the event could not be sent to the people it names: {e}");
                Vec::new()
            }
        }
    }

    /// The events any of `filters` matches, according to `source`. A store
    /// that can't be read knows of none.
    async fn events(&self, source: Source, filters: Vec<Filter>) -> Result<Vec<Event>, FetchError> {
        match source {
            Source::Relays => Ok(self.fetch(filters).await?.collect()),
            Source::Store => {
                let mut events = Vec::new();
                for filter in filters {
                    match self.client.database().query(filter).await {
                        Ok(found) => events.extend(found),
                        Err(e) => tracing::warn!("the event store could not be read: {e}"),
                    }
                }
                Ok(events)
            }
        }
    }

    /// The notes `wanted`, each as a row shows it, as far as `source` has
    /// them: the store, and for the relays' turn the relays as well, for
    /// the ones the store doesn't have, which get `wait` to send them: the
    /// account's own, and then, for a note `wanted` knows more of, the
    /// relays its author writes to and the relay it is said to be on. One
    /// that isn't found is left out, and so is one that is no note a row
    /// shows.
    async fn notes_by_id(
        &self,
        wanted: &HashMap<EventId, Whereabouts>,
        source: Source,
        wait: Duration,
    ) -> HashMap<EventId, Note> {
        let known = wanted;
        let wanted: Vec<EventId> = wanted.keys().copied().collect();
        let mut found: HashMap<EventId, Event> = HashMap::new();
        if let Ok(stored) = self.events(Source::Store, filters_by_id(&wanted)).await {
            found.extend(stored.into_iter().map(|event| (event.id, event)));
        }
        let missing: Vec<EventId> = wanted.iter().filter(|id| !found.contains_key(id)).copied().collect();
        if source == Source::Relays && !missing.is_empty() {
            match self.fetch_within(filters_by_id(&missing), wait).await {
                Ok(fetched) => found.extend(fetched.map(|event| (event.id, event))),
                Err(e) => tracing::warn!("{} note(s) could not be asked for by id: {e:?}", missing.len()),
            }
            // Those the account's own relays don't have may be elsewhere.
            // Given as long as the others were, and no longer: a list of
            // notes waits here.
            let elsewhere: Vec<(EventId, &Whereabouts)> = missing
                .iter()
                .filter(|id| !found.contains_key(id))
                .filter_map(|id| known.get(id).map(|whereabouts| (*id, whereabouts)))
                .collect();
            found.extend(self.events_elsewhere(&elsewhere, wait).await.into_iter().map(|event| (event.id, event)));
        }

        found
            .into_values()
            .filter(|event| wanted.contains(&event.id) && FEED_KINDS.contains(&event.kind))
            .map(|event| (event.id, Note::from(event)))
            .collect()
    }

    /// Those of the events `sought` that the relays they may be on have,
    /// beside the account's own: the relays an event's author writes to,
    /// if who that is is known, and the relay it is said to be on. They
    /// get `wait` between them; nothing, if that runs out, or if nothing
    /// is known of where any is.
    async fn events_elsewhere(&self, sought: &[(EventId, &Whereabouts)], wait: Duration) -> Vec<Event> {
        let written_by = |people: &[PublicKey]| -> Vec<EventId> {
            let written = |known: &Whereabouts| known.author.is_some_and(|author| people.contains(&author));
            sought.iter().filter(|(_, known)| written(known)).map(|(id, _)| *id).collect()
        };
        let people: Vec<PublicKey> = {
            let mut seen = HashSet::new();
            sought.iter().filter_map(|(_, known)| known.author).filter(|author| seen.insert(*author)).collect()
        };
        let said: Vec<(EventId, RelayUrl)> =
            sought.iter().filter_map(|(id, known)| known.relay.clone().map(|relay| (*id, relay))).collect();
        if people.is_empty() && said.is_empty() {
            return Vec::new();
        }
        let theirs = self.ask_their_relays(&people, Listed::Write, true, |people| filters_by_id(&written_by(people)));
        match tokio::time::timeout(wait, join(theirs, self.ask_where_said(&said))).await {
            Ok((theirs, said)) => theirs.into_iter().chain(said).collect(),
            Err(_) => {
                tracing::debug!("the relays {} event(s) may be on took too long to ask", sought.len());
                Vec::new()
            }
        }
    }

    /// `notes`, each with the notes its row shows it quoting, and those
    /// with the notes they quote in turn, [`QUOTE_DEPTH`] notes deep and no
    /// deeper, as far as `source` has them (see [`Self::notes_by_id`]).
    async fn with_quotes(&self, notes: Vec<Note>, source: Source) -> Vec<Note> {
        // With where each is, as far as the address that names it says.
        let quoted_by = |notes: &mut dyn Iterator<Item = &Note>| -> HashMap<EventId, Whereabouts> {
            notes.flat_map(|note| note.quotes_shown().map(|quote| (quote.id, quote.whereabouts()))).collect()
        };
        // What is quoted is found a level at a time, from the notes down.
        let mut levels: Vec<HashMap<EventId, Note>> = Vec::new();
        let mut wanted = quoted_by(&mut notes.iter());
        while !wanted.is_empty() && levels.len() < QUOTE_DEPTH {
            let found = self.notes_by_id(&wanted, source, QUOTES_WAIT).await;
            wanted = quoted_by(&mut found.values());
            levels.push(found);
        }
        // And put together from the deepest up: each note with the notes
        // it quotes, which have theirs already.
        let mut quoted: HashMap<EventId, Note> = HashMap::new();
        for level in levels.into_iter().rev() {
            quoted = level.into_iter().map(|(id, note)| (id, note.with_quoted(&quoted))).collect();
        }
        notes.into_iter().map(|note| note.with_quoted(&quoted)).collect()
    }

    /// The notes `listed` names, in its order, each as a row shows it,
    /// with the notes it quotes: from the store, and from the relays for
    /// those the store doesn't have, the account's own and then wherever
    /// `listed` says each may be. One that nobody has is left out, and so
    /// is one that is no note a row shows.
    pub async fn notes_listed(&self, listed: Vec<(EventId, Whereabouts)>) -> Vec<Note> {
        let ids: Vec<EventId> = listed.iter().map(|(id, _)| *id).collect();
        let wanted: HashMap<EventId, Whereabouts> = listed.into_iter().collect();
        let mut found = self.notes_by_id(&wanted, Source::Relays, FETCH_TIMEOUT).await;
        let notes = ids.iter().filter_map(|id| found.remove(id)).collect();
        self.with_quotes(notes, Source::Relays).await
    }

    /// `event`, which the store just got, as a row shows it: with the
    /// notes it quotes, if the store has them.
    pub async fn note(&self, event: Event) -> Note {
        let mut notes = self.with_quotes(vec![Note::from(event.clone())], Source::Store).await;
        notes.pop().unwrap_or_else(|| Note::from(event))
    }

    /// The conversation the note `focused` is part of, as far as `source`
    /// knows it: the note in full, and the others as rows show them. Empty
    /// if the note itself is not to be found.
    ///
    /// Three questions are asked, as Vega asks them. Which notes name the
    /// conversation's root (and the root itself), which is asked of the
    /// relays of those whose conversation it is as well; then, if those are few,
    /// which notes name any of them, since some clients tag only the note
    /// they answer; and last, one at a time, the notes between the focused
    /// one and the root that neither brought.
    ///
    /// Naming a note is not answering it: a note that only cites one of
    /// these is no part of the conversation, and is left out.
    pub async fn thread(&self, source: Source, focused: EventId, whereabouts: &Whereabouts) -> Result<Vec<Note>, FetchError> {
        // The store may have the note when it is the relays' turn: they are
        // then not asked for it again.
        let mut note = self.events(Source::Store, vec![Filter::new().id(focused)]).await?.into_iter().next();
        if note.is_none() && source == Source::Relays {
            note = self.events(source, vec![Filter::new().id(focused)]).await?.into_iter().next();
        }
        // Not on the account's own relays: where whatever named it said.
        if note.is_none() && source == Source::Relays {
            let elsewhere = self.events_elsewhere(&[(focused, whereabouts)], FETCH_TIMEOUT).await;
            note = elsewhere.into_iter().find(|event| event.id == focused);
        }
        let Some(note) = note else {
            return Ok(Vec::new());
        };
        let root = reply_to(&note).map_or(focused, |reply| reply.root);
        let mut parent = reply_to(&note).map(|reply| reply.parent);
        // Whose conversation it is, as far as the note says: who wrote
        // it, and the first few of those it names.
        let people: Vec<PublicKey> = {
            let mut seen = HashSet::new();
            let named = std::iter::once(note.pubkey).chain(note.tags.public_keys());
            named.filter(|person| seen.insert(*person)).take(THREAD_PEOPLE).collect()
        };
        let mut found: HashMap<EventId, Event> = HashMap::from([(focused, note)]);

        let naming = |ids: Vec<EventId>| Filter::new().kinds(FEED_KINDS).events(ids).limit(THREAD_LIMIT);
        // Relays are free to send more than the limit asks for, so it is
        // kept to here: the root, and so many of the rest.
        let keep = |found: &mut HashMap<EventId, Event>, events: Vec<Event>| {
            let (roots, rest): (Vec<Event>, Vec<Event>) = events.into_iter().partition(|event| event.id == root);
            found.extend(roots.into_iter().chain(rest.into_iter().take(THREAD_LIMIT)).map(|event| (event.id, event)));
        };

        let mut filters = vec![naming(vec![root])];
        if root != focused {
            filters.push(Filter::new().id(root));
        }
        // The account's own relays, and the relays of those whose
        // conversation it is: what they wrote is where they write, and
        // what was written to them is where they read.
        let theirs = async {
            match source {
                Source::Relays => self.ask_their_relays(&people, Listed::Either, true, |_| filters.clone()).await,
                Source::Store => Vec::new(),
            }
        };
        let (own, theirs) = join(self.events(source, filters.clone()), theirs).await;
        keep(&mut found, own?);
        keep(&mut found, theirs);

        if found.len() < THREAD_ASK_AGAIN_UNDER {
            let known = found.keys().copied().collect();
            keep(&mut found, self.events(source, vec![naming(known)]).await?);
        }

        let mut ancestors = HashSet::new();
        for _ in 0..THREAD_ANCESTORS {
            let Some(id) = parent else {
                break;
            };
            if let Entry::Vacant(missing) = found.entry(id) {
                let Some(event) = self.events(source, vec![Filter::new().id(id)]).await?.into_iter().next() else {
                    break;
                };
                missing.insert(event);
            }
            ancestors.insert(id);
            parent = found.get(&id).and_then(reply_to).map(|reply| reply.parent);
        }

        let belongs = |event: &Event| {
            event.id == focused
                || event.id == root
                || ancestors.contains(&event.id)
                || reply_to(event).is_some_and(|reply| reply.root == root || found.contains_key(&reply.parent))
        };
        let strangers: Vec<EventId> = found.values().filter(|event| !belongs(event)).map(|event| event.id).collect();
        for id in strangers {
            found.remove(&id);
        }

        let notes = found.into_values().map(|event| if event.id == focused { Note::in_full(&event) } else { Note::from(event) });
        Ok(self.with_quotes(notes.collect(), source).await)
    }

    /// The newest notes of `authors` that are already in the store.
    pub async fn cached_feed(&self, authors: &[PublicKey], limit: usize) -> Result<Vec<Note>, String> {
        let mut notes = Vec::new();
        for filter in feed_filters(authors, None, limit) {
            let events = self.client.database().query(filter).await.map_err(|e| e.to_string())?;
            notes.extend(events.into_iter().map(Note::from));
        }
        Ok(self.with_quotes(feed::newest(notes, limit), Source::Store).await)
    }

    /// One page of the notes of `authors` from the relays: the `limit`
    /// newest written at or before `until`, or the newest of all, of what
    /// the account's own relays have and what the relays the authors
    /// write to have. Nobody's notes are none, without asking.
    pub async fn fetch_feed(&self, authors: &[PublicKey], until: Option<u64>, limit: usize) -> Result<Vec<Note>, FetchError> {
        let filters = feed_filters(authors, until, limit);
        if filters.is_empty() {
            return Ok(Vec::new());
        }
        // The account's own relays about everyone, and at the same time
        // the relays the authors write to, each about those who do.
        let theirs = self.ask_their_relays(authors, Listed::Write, false, |people| feed_filters(people, until, limit));
        let (own, theirs) = join(self.fetch(filters), theirs).await;
        let events: Vec<Event> = match own {
            Ok(own) => own.chain(theirs).collect(),
            Err(e) if theirs.is_empty() => return Err(e),
            Err(e) => {
                tracing::warn!("the account's own relays could not be asked for notes: {e:?}");
                theirs
            }
        };
        Ok(self.with_quotes(feed::newest(events.into_iter().map(Note::from), limit), Source::Relays).await)
    }

    // =========================================================================
    // Notifications
    // =========================================================================

    /// What `source` has to tell `account` of: one page of the events
    /// that name it, the newest written after `since` and at or before
    /// `until` where those are given, each read with the note it is about
    /// (see [`notifications`]).
    ///
    /// The relays asked are the account's own and the ones its relay list
    /// says it reads: where others send what they write to it (NIP-65).
    pub async fn notifications(
        &self,
        source: Source,
        account: PublicKey,
        since: Option<u64>,
        until: Option<u64>,
    ) -> Result<Notified, FetchError> {
        let filter = notifications::filter(account, since, until, NOTIFICATIONS_PAGE);
        let mut events: Vec<Event> = match source {
            Source::Store => self.events(Source::Store, vec![filter]).await?,
            Source::Relays => {
                let whose = [account];
                let inbox = self.ask_their_relays(&whose, Listed::Read, true, |_| vec![filter.clone()]);
                let (own, inbox) = join(self.fetch(vec![filter.clone()]), inbox).await;
                match own {
                    Ok(own) => own.chain(inbox).collect(),
                    Err(e) if inbox.is_empty() => return Err(e),
                    Err(e) => {
                        tracing::warn!("the account's own relays could not be asked what names it: {e:?}");
                        inbox
                    }
                }
            }
        };
        let mut seen = HashSet::new();
        events.retain(|event| seen.insert(event.id));
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        // A page is the newest so many, whatever the relays sent: what is
        // older is asked for from where this one ends.
        let full = events.len() >= NOTIFICATIONS_PAGE;
        events.truncate(NOTIFICATIONS_PAGE);
        let oldest = events.last().map(|event| event.created_at.as_secs());

        // The notes they are about say what they are: an answer to the
        // account, or to someone else; a reaction to a note of its own.
        let wanted: HashMap<EventId, Whereabouts> =
            events.iter().filter_map(notifications::about).map(|id| (id, Whereabouts::default())).collect();
        let at_hand = self.notes_by_id(&wanted, source, ABOUT_WAIT).await;
        let mut read: Vec<Notification> =
            events.iter().filter_map(|event| notifications::read(event, account, &at_hand)).collect();

        // Those that are notes show as notes do: with what they quote.
        let notes: Vec<Note> = read.iter().filter_map(|notification| notification.note.clone()).collect();
        let mut with_quotes: HashMap<EventId, Note> =
            self.with_quotes(notes, source).await.into_iter().map(|note| (note.id, note)).collect();
        for notification in &mut read {
            if let Some(note) = with_quotes.remove(&notification.id) {
                notification.note = Some(note);
            }
        }
        Ok(Notified { notifications: read, oldest, full })
    }

    /// The newest list of `kind` that `account` wrote, of those in the
    /// store, if it has one: a contact list, a bookmark list.
    async fn stored_list(&self, kind: Kind, account: PublicKey) -> Option<Event> {
        let filter = Filter::new().kind(kind).author(account);
        match self.client.database().query(filter).await {
            Ok(events) => newest(events),
            Err(e) => {
                tracing::warn!("a list (kind {}) could not be read from the store: {e}", kind.as_u16());
                None
            }
        }
    }

    /// The newest list of `kind` that `account` wrote, after asking the
    /// relays for it, and whether every relay of the pool answered. What
    /// the relays send is kept in the store, which only ever keeps the
    /// newest list, so the store is read as well: a relay that is behind
    /// doesn't undo a newer list already seen.
    async fn asked_list(&self, kind: Kind, account: PublicKey) -> Result<(Option<Event>, bool), FetchError> {
        let filter = Filter::new().kind(kind).author(account).limit(1);
        let (fetched, everywhere) = self.fetch_from_each(vec![filter], FETCH_TIMEOUT).await?;
        Ok((newest(self.stored_list(kind, account).await.into_iter().chain(fetched)), everywhere))
    }

    /// The newest list of `kind` that `account` wrote, after asking the
    /// relays for it. `None` if none of those that answered has one.
    async fn newest_list(&self, kind: Kind, account: PublicKey) -> Result<Option<Event>, FetchError> {
        Ok(self.asked_list(kind, account).await?.0)
    }

    /// The newest list of `kind` that `account` wrote, to be written again
    /// with a change. `None`, which is leave to write a first one, only if
    /// every relay of the pool answered and none has one: a relay that
    /// didn't answer may be the one that does, and a first list would
    /// replace it there as everywhere.
    async fn list_to_change(&self, kind: Kind, account: PublicKey) -> Result<Option<Event>, FetchError> {
        match self.asked_list(kind, account).await? {
            (None, false) => Err(FetchError::Incomplete),
            (list, _) => Ok(list),
        }
    }

    /// Who `account` follows, by the contact list already in the store.
    /// `None` if the store has no list of theirs.
    pub async fn cached_contacts(&self, account: PublicKey) -> Option<Vec<PublicKey>> {
        self.stored_list(Kind::ContactList, account).await.as_ref().map(contacts::followed)
    }

    /// Who `account` follows, after asking the relays for their contact
    /// list. `None` if nobody has a list of theirs.
    pub async fn fetch_contacts(&self, account: PublicKey) -> Result<Option<Vec<PublicKey>>, FetchError> {
        Ok(self.newest_list(Kind::ContactList, account).await?.as_ref().map(contacts::followed))
    }

    /// Have `account` follow `person` (`follow`) or stop following them:
    /// its newest contact list, asked of the relays just now, signed again
    /// with that one change and sent out. Returns who `account` follows
    /// after it.
    ///
    /// With no list anywhere, a first one is only written if `first_list`
    /// says so; otherwise that is the error, and nothing is sent.
    pub async fn set_following(
        &self,
        signer: &Signer,
        account: PublicKey,
        person: PublicKey,
        follow: bool,
        first_list: bool,
    ) -> Result<Vec<PublicKey>, FollowError> {
        let list = self.list_to_change(Kind::ContactList, account).await.map_err(FollowError::Fetch)?;
        if list.is_none() && follow && !first_list {
            return Err(FollowError::NoList);
        }
        let unchanged = || list.as_ref().map(contacts::followed).unwrap_or_default();
        let Some(changed) = contacts::changed(list.as_ref(), person, follow, Timestamp::now()) else {
            return Ok(unchanged());
        };
        let sent = self.sign_and_publish(signer, account, changed).await.map_err(FollowError::Publish)?;
        Ok(contacts::followed(&sent))
    }

    /// The bookmarks of `account` that `list`, its newest bookmark list,
    /// keeps to itself, which `signer` decrypts.
    ///
    /// `known` is what was read before. If it was read from this very
    /// list, it is the answer, and nothing is decrypted again: a remote
    /// signer may ask a person about each request.
    async fn read_bookmarks(
        &self,
        signer: &Signer,
        account: PublicKey,
        list: Option<Event>,
        known: Option<Bookmarks>,
    ) -> Result<Bookmarks, ListError> {
        let Some(list) = list else {
            return Ok(Bookmarks::default());
        };
        if let Some(known) = known.filter(|known| known.list().is_some_and(|read| read.id == list.id)) {
            return Ok(known);
        }
        let private = Self::read_private(signer, account, &list).await?;
        Ok(Bookmarks::new(Some(list), private))
    }

    /// The tags `list` keeps to itself (NIP-51), which `signer` decrypts.
    /// A list that keeps nothing to itself has no content, and is not
    /// taken to the signer.
    async fn read_private(signer: &Signer, account: PublicKey, list: &Event) -> Result<PrivateTags, ListError> {
        if list.content.trim().is_empty() {
            return Ok(Vec::new());
        }
        let json = signer.decrypt_from_self(account, &list.content).await.map_err(ListError::Unreadable)?;
        bookmarks::decode(&json).map_err(|e| ListError::Unreadable(SignError::Cipher(e)))
    }

    /// `private`, the tags a list keeps to itself, as its content:
    /// encrypted by `signer` to `account` itself, or nothing at all when
    /// there are none.
    async fn written_private(signer: &Signer, account: PublicKey, private: &PrivateTags) -> Result<String, ListError> {
        if private.is_empty() {
            return Ok(String::new());
        }
        let sign_error = |e| ListError::Publish(PublishError::Sign(e));
        signer.encrypt_to_self(account, &bookmarks::encode(private)).await.map_err(sign_error)
    }

    /// The bookmarks of `account`, as far as `source` knows: by the list
    /// in the store, or by the newest the relays have. An account with no
    /// list has none. See [`Self::read_bookmarks`] for `known`.
    pub async fn bookmarks(
        &self,
        source: Source,
        signer: &Signer,
        account: PublicKey,
        known: Option<Bookmarks>,
    ) -> Result<Bookmarks, ListError> {
        let list = match source {
            Source::Store => self.stored_list(Kind::Bookmarks, account).await,
            Source::Relays => self.newest_list(Kind::Bookmarks, account).await.map_err(ListError::Fetch)?,
        };
        self.read_bookmarks(signer, account, list, known).await
    }

    /// Have `account` bookmark `note` (`bookmark`), privately, or take its
    /// bookmark off: its newest bookmark list, asked of the relays just
    /// now, with that one change to what it keeps to itself, encrypted
    /// anew, signed and sent out. What the list says to anyone goes out
    /// as it came. Returns the account's bookmarks after it. See
    /// [`Self::read_bookmarks`] for `known`.
    ///
    /// With no list anywhere, a first one is only written if `first_list`
    /// says so; otherwise that is the error, and nothing is sent. Nor is
    /// anything sent when what the list keeps to itself can't be read.
    pub async fn set_bookmarked(
        &self,
        signer: &Signer,
        account: PublicKey,
        note: EventId,
        bookmark: bool,
        first_list: bool,
        known: Option<Bookmarks>,
    ) -> Result<Bookmarks, ListError> {
        let list = self.list_to_change(Kind::Bookmarks, account).await.map_err(ListError::Fetch)?;
        if list.is_none() && bookmark && !first_list {
            return Err(ListError::NoList);
        }
        let current = self.read_bookmarks(signer, account, list, known).await?;
        // Where the note is, for whoever reads the list elsewhere.
        let hint = self.seen_on(&[note]).remove(&note).and_then(|relays| relays.into_iter().next());
        let Some(private) = current.changed(note, hint.as_ref(), bookmark) else {
            return Ok(current);
        };
        let content = Self::written_private(signer, account, &private).await?;
        let unsigned = bookmarks::list(current.list(), content, Timestamp::now());
        let sent = self.sign_and_publish(signer, account, unsigned).await.map_err(ListError::Publish)?;
        Ok(Bookmarks::new(Some(sent), private))
    }

    // =========================================================================
    // Drafts
    // =========================================================================

    /// Have the relay of this machine keep `text` as the draft of
    /// `account` for `target`: a draft wrap (see [`kept_drafts`]), which
    /// `signer` encrypts to the account itself and signs, in place of the
    /// one it had, with `said` for what it says of the files it links to.
    /// A blank `text` is a draft done with, and is kept as that. It goes
    /// to that relay and to no other.
    pub async fn keep_draft(
        &self,
        signer: &Signer,
        account: PublicKey,
        target: Target,
        text: &str,
        said: &[Tag],
    ) -> Result<(), String> {
        let which = Filter::new().kind(DRAFT_KIND).author(account).identifier(kept_drafts::identifier(target));
        let earlier = newest(self.private.ask(which).await?);
        let now = Timestamp::now();
        let content = if text.trim().is_empty() {
            String::new()
        } else {
            let written = kept_drafts::written(account, text, said, now);
            signer.encrypt_to_self(account, &written).await.map_err(|e| format!("{e:?}"))?
        };
        let unsigned = kept_drafts::wrap(target, content, earlier.as_ref(), now);
        let event = signer.sign(account, unsigned).await.map_err(|e| format!("{e:?}"))?;
        self.private.send(&event).await
    }

    /// The drafts of `account` that the relay of this machine keeps, each
    /// with what it is a draft of and what it says of the files it links
    /// to: those of this client's that are not done with, which `signer`
    /// decrypts. One that can't be read is left out.
    pub async fn kept_drafts(&self, signer: &Signer, account: PublicKey) -> Result<Vec<(Target, String, Vec<Tag>)>, String> {
        let wraps = self.private.ask(Filter::new().kind(DRAFT_KIND).author(account)).await?;
        let mut drafts = Vec::new();
        for wrap in &wraps {
            let Some((target, content)) = kept_drafts::kept(wrap).filter(|(_, content)| !content.trim().is_empty()) else {
                continue;
            };
            match signer.decrypt_from_self(account, content).await {
                Ok(json) => drafts.extend(kept_drafts::text_of(&json).map(|text| (target, text, kept_drafts::said_of(&json)))),
                Err(e) => tracing::warn!("a draft could not be read: {e:?}"),
            }
        }
        Ok(drafts)
    }

    // =========================================================================
    // Searching
    // =========================================================================

    /// What `relays`, the account's search relays, find of `what` for
    /// `words` (NIP-50), the best matches first, as each relay had them
    /// (see [`search::ranked`]).
    ///
    /// The relays are the account's own word, and any it names is asked:
    /// where it is, if it is one of the account's own, and otherwise among
    /// other people's relays, where it is let go of when it is not used.
    /// What was searched for goes to those relays and nowhere else: it is
    /// not logged, and not kept.
    async fn search_events(&self, relays: &[RelayUrl], words: &str, what: SearchFor) -> Result<Vec<Event>, FetchError> {
        Ok(search::ranked(self.ask_named(relays, search::filter(words, what)).await?))
    }

    /// What each of `relays`, which the account itself named, sends for
    /// `filter`: one answer for each that gave one, as it gave it. That
    /// none of them is reached, or that none answers, is the error.
    async fn ask_named(&self, relays: &[RelayUrl], filter: Filter) -> Result<Vec<Vec<Event>>, FetchError> {
        let home: HashSet<RelayUrl> = self.reachable_relays().await.into_iter().collect();
        let (own, others): (Vec<RelayUrl>, Vec<RelayUrl>) = relays.iter().cloned().partition(|relay| home.contains(relay));
        let others = self.connected(others.into_iter().map(|relay| (relay, ())).collect()).await;
        let asks: Vec<(Pool, RelayUrl)> = own
            .into_iter()
            .map(|relay| (Pool::Home, relay))
            .chain(others.into_iter().map(|(relay, ())| (Pool::Others, relay)))
            .collect();
        if asks.is_empty() {
            return Err(FetchError::NoRelayReachable);
        }
        let asking = asks.iter().map(|(pool, relay)| self.ask(*pool, relay, vec![filter.clone()], SEARCH_TIMEOUT));
        let answers = join_all(asking).await;

        let mut found = Vec::new();
        let mut failure = None;
        {
            let mut sightings = self.sightings();
            for ((_, relay), answer) in asks.iter().zip(answers) {
                match answer {
                    Ok(asked) => {
                        for event in asked.events.iter().filter(|event| FEED_KINDS.contains(&event.kind)) {
                            sightings.record(event.id, relay);
                        }
                        found.push(asked.events);
                    }
                    Err(e) => {
                        tracing::warn!("{relay} could not be searched: {e}");
                        failure = Some(e);
                    }
                }
            }
        }
        if found.is_empty() {
            return Err(FetchError::Failed(failure.unwrap_or_default()));
        }
        tracing::debug!("{} relay(s) the account named were asked: {} answered", asks.len(), found.len());
        Ok(found)
    }

    /// The notes `relay` got last, as rows show them, the newest first:
    /// what a relay is like, for whoever typed its address. The account's
    /// own word, as its search relays are; and as with those, a relay it
    /// has blocked is not connected to.
    pub async fn relay_notes(&self, relay: &RelayUrl) -> Result<Vec<Note>, FetchError> {
        let answers = self.ask_named(std::slice::from_ref(relay), search::latest_notes()).await?;
        let mut events: Vec<Event> = answers.into_iter().flatten().filter(|event| FEED_KINDS.contains(&event.kind)).collect();
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        let notes = events.into_iter().map(Note::from).collect();
        Ok(self.with_quotes(notes, Source::Relays).await)
    }

    /// What the account's search relays `relays` find of `what` for
    /// `words`: people with their profiles, or notes as rows show them,
    /// with the notes they quote. See [`Self::search_events`].
    pub async fn search(&self, relays: &[RelayUrl], words: &str, what: SearchFor) -> Result<Found, FetchError> {
        let events = self.search_events(relays, words, what).await?;
        match what {
            SearchFor::People => {
                // Each person once, where they first came; with their
                // profile as it stands now, which the store knows.
                let mut seen = HashSet::new();
                let ranked: Vec<PublicKey> = events
                    .iter()
                    .filter(|event| event.kind == Kind::Metadata)
                    .map(|event| event.pubkey)
                    .filter(|person| seen.insert(*person))
                    .collect();
                let mut profiles: HashMap<PublicKey, Profile> = newest_profiles(events).into_iter().collect();
                profiles.extend(self.cached_profiles(ranked.clone()).await);
                let people = ranked.into_iter().filter_map(|person| profiles.remove(&person).map(|profile| (person, profile)));
                Ok(Found::People(people.collect()))
            }
            SearchFor::Notes => {
                let notes = events.into_iter().filter(|event| FEED_KINDS.contains(&event.kind)).map(Note::from).collect();
                Ok(Found::Notes(self.with_quotes(notes, Source::Relays).await))
            }
        }
    }

    // =========================================================================
    // The account's relays
    // =========================================================================

    /// The relays of `account` that `list`, its newest list of one kind,
    /// names, with those it keeps to itself if `open` says to read them:
    /// that takes the signer, which may ask a person about each request.
    /// A list with no content keeps nothing, and is not taken to the
    /// signer. If `known` was read from this very list, it is the answer,
    /// and nothing is decrypted again.
    async fn read_listed(
        signer: &Signer,
        account: PublicKey,
        list: Option<Event>,
        known: Option<&ListedRelays>,
        open: bool,
    ) -> ListedRelays {
        let Some(list) = list else {
            return ListedRelays::default();
        };
        let same = |known: &&ListedRelays| known.list().is_some_and(|read| read.id == list.id);
        // Unless it left unread what is now to be read.
        if let Some(known) = known.filter(same).filter(|known| !open || *known.kept() != Kept::Unread) {
            return known.clone();
        }
        if !open && !list.content.trim().is_empty() {
            return ListedRelays::new(Some(list), Kept::Unread);
        }
        let kept = match Self::read_private(signer, account, &list).await {
            Ok(private) => Kept::Read(private),
            Err(ListError::Unreadable(why)) => {
                tracing::warn!("what a list (kind {}) keeps to itself could not be read: {why:?}", list.kind.as_u16());
                Kept::Unreadable(why)
            }
            // Reading asks no relay and sends nothing.
            Err(_) => Kept::Unread,
        };
        ListedRelays::new(Some(list), kept)
    }

    /// The newest list of each of `kinds` that `account` wrote, after
    /// asking the relays for them in one question. As
    /// [`Self::newest_list`], for several.
    async fn newest_lists(&self, kinds: &[Kind], account: PublicKey) -> Result<HashMap<Kind, Event>, FetchError> {
        let filter = Filter::new().kinds(kinds.iter().copied()).author(account);
        let (fetched, _) = self.fetch_from_each(vec![filter], FETCH_TIMEOUT).await?;
        let mut lists: HashMap<Kind, Event> = HashMap::new();
        let mut take = |list: Event| match lists.get(&list.kind) {
            Some(known) if known.created_at >= list.created_at => {}
            _ => {
                lists.insert(list.kind, list);
            }
        };
        fetched.into_iter().for_each(&mut take);
        for kind in kinds {
            self.stored_list(*kind, account).await.into_iter().for_each(&mut take);
        }
        Ok(lists)
    }

    /// The relay lists of `account`, as far as `source` knows: its relay
    /// list (NIP-65) and its other lists of relays (see
    /// [`relay_lists`]), by the lists in the store or by the newest the
    /// relays have. What the lists of `open` keep to themselves is read,
    /// and the others' is left unread. `known` is what was read before:
    /// nothing is decrypted twice.
    ///
    /// A list whose private part can't be read is no error: the relays it
    /// names for anyone are there, with why the others aren't.
    pub async fn relay_lists(
        &self,
        source: Source,
        signer: &Signer,
        account: PublicKey,
        known: Option<RelayLists>,
        open: &[Section],
    ) -> Result<RelayLists, FetchError> {
        let kinds: Vec<Kind> =
            std::iter::once(Kind::RelayList).chain(Section::LISTS.iter().map(|section| section.kind())).collect();
        let mut lists = match source {
            Source::Store => {
                let mut lists = HashMap::new();
                for kind in &kinds {
                    lists.extend(self.stored_list(*kind, account).await.map(|list| (*kind, list)));
                }
                lists
            }
            Source::Relays => self.newest_lists(&kinds, account).await?,
        };
        let advertised = lists.remove(&Kind::RelayList);
        let mut listed = HashMap::new();
        for section in Section::LISTS {
            let known = known.as_ref().and_then(|known| known.listed.get(&section));
            let read = Self::read_listed(signer, account, lists.remove(&section.kind()), known, open.contains(&section)).await;
            listed.insert(section, read);
        }
        Ok(RelayLists { advertised, listed })
    }

    /// Have the relay list (NIP-65) of `account` name `outbox` for the
    /// relays it writes to and `inbox` for those it reads: its newest
    /// list, asked of the relays just now, with its relays replaced and
    /// the rest as it was, signed and sent out, to the relays it wrote to
    /// before and to those it writes to now, which are its own from then
    /// on. Returns the list after it; nothing is sent when it says so
    /// already.
    ///
    /// With no list anywhere, a first one is only written if `first_list`
    /// says so; otherwise that is the error, and nothing is sent.
    pub async fn set_advertised_relays(
        &self,
        signer: &Signer,
        account: PublicKey,
        outbox: &[RelayUrl],
        inbox: &[RelayUrl],
        first_list: bool,
    ) -> Result<Option<Event>, ListError> {
        let list = self.list_to_change(Kind::RelayList, account).await.map_err(ListError::Fetch)?;
        let Some(unsigned) = relay_lists::advertised_list(list.as_ref(), outbox, inbox, Timestamp::now()) else {
            return Ok(list);
        };
        if list.is_none() && !first_list {
            return Err(ListError::NoList);
        }
        let event = signer.sign(account, unsigned).await.map_err(|e| ListError::Publish(PublishError::Sign(e)))?;
        // Told to the relays it leaves as well as to those it joins:
        // whoever asks either finds where the account is now.
        let before = self.own().listed.clone();
        let both = outbox.iter().chain(&before).cloned().collect();
        self.join_account_relays(both).await;
        let published = self.publish(&event).await;
        self.join_account_relays(outbox.to_vec()).await;
        let accepted = published.map_err(ListError::Publish)?;
        tracing::info!("published the relay list {} to {accepted} relay(s)", event.id);
        Ok(Some(event))
    }

    /// Have the relays of `section`, one of the lists of `relay` tags of
    /// `account`, be `relays`: its newest list of that kind, asked of the
    /// relays just now, with that change (see [`ListedRelays::changed`]),
    /// what it keeps to itself encrypted anew, signed and sent out.
    /// Returns the list's relays after it. See [`Self::read_listed`] for
    /// `known`.
    ///
    /// With no list anywhere, a first one is only written if `first_list`
    /// says so; otherwise that is the error, and nothing is sent. Nor is
    /// anything sent when what the list keeps to itself can't be read.
    pub async fn set_listed_relays(
        &self,
        signer: &Signer,
        account: PublicKey,
        section: Section,
        relays: &[RelayUrl],
        first_list: bool,
        known: Option<ListedRelays>,
    ) -> Result<ListedRelays, ListError> {
        let list = self.list_to_change(section.kind(), account).await.map_err(ListError::Fetch)?;
        let current = Self::read_listed(signer, account, list, known.as_ref(), true).await;
        if let Kept::Unreadable(why) = current.kept() {
            return Err(ListError::Unreadable(why.clone()));
        }
        let Some((tags, private)) = current.changed(section, relays) else {
            return Ok(current);
        };
        if current.list().is_none() && !first_list {
            return Err(ListError::NoList);
        }
        let content = Self::written_private(signer, account, &private).await?;
        let unsigned = relay_lists::listed(section, current.list(), tags, content, Timestamp::now());
        let sent = self.sign_and_publish(signer, account, unsigned).await.map_err(ListError::Publish)?;
        Ok(ListedRelays::new(Some(sent), Kept::Read(private)))
    }

    /// The media server list of `account` (BUD-03, kind 10063), as far as
    /// `source` knows: its newest. `None` if it has none there.
    pub async fn media_server_list(&self, source: Source, account: PublicKey) -> Result<Option<Event>, FetchError> {
        match source {
            Source::Store => Ok(self.stored_list(Kind::BlossomServerList, account).await),
            Source::Relays => self.newest_list(Kind::BlossomServerList, account).await,
        }
    }

    /// Have the media servers of `account` be `servers`, the one files
    /// are sent to first at the head: its newest server list, asked of
    /// the relays just now, with its servers replaced and the rest as it
    /// was, signed and sent out. Returns the list after it; nothing is
    /// sent when it says so already.
    ///
    /// With no list anywhere, a first one is only written if `first_list`
    /// says so; otherwise that is the error, and nothing is sent.
    pub async fn set_media_servers(
        &self,
        signer: &Signer,
        account: PublicKey,
        servers: &[Url],
        first_list: bool,
    ) -> Result<Option<Event>, ListError> {
        let list = self.list_to_change(Kind::BlossomServerList, account).await.map_err(ListError::Fetch)?;
        let Some(unsigned) = blossom::server_list(list.as_ref(), servers, Timestamp::now()) else {
            return Ok(list);
        };
        if list.is_none() && !first_list {
            return Err(ListError::NoList);
        }
        let sent = self.sign_and_publish(signer, account, unsigned).await.map_err(ListError::Publish)?;
        Ok(Some(sent))
    }

    /// How every relay this is connected to has been doing: the account's
    /// own, and other people's that are in their pool now. A relay that
    /// is in both counts as the account's own.
    pub async fn relay_use(&self) -> HashMap<RelayUrl, RelayUse> {
        let mut in_use = HashMap::new();
        for pool in [&self.others, &self.client] {
            for (url, relay) in pool.relays().await {
                let stats = relay.stats();
                let doing = RelayUse {
                    connected: matches!(relay.status(), RelayStatus::Connected | RelayStatus::Sleeping),
                    received: stats.bytes_received(),
                    sent: stats.bytes_sent(),
                    attempts: stats.attempts(),
                    succeeded: stats.success(),
                    latency: stats.latency(),
                };
                in_use.insert(url, doing);
            }
        }
        in_use
    }

    /// Go by these of the account's relays from now on: `lookup`, where
    /// profiles and relay lists that its own relays don't have are looked
    /// up, and `blocked`, which are never connected to on anyone's word.
    /// Relay lists are looked for again when there are other relays to
    /// look on.
    pub fn use_listed(&self, lookup: Vec<RelayUrl>, blocked: Vec<RelayUrl>) {
        let mut reached = self.reached();
        if reached.lookup != lookup {
            reached.lists_asked.clear();
            reached.lookup = lookup;
        }
        reached.blocked = blocked.into_iter().collect();
    }

    /// What the relays that look things up send for `filters`, beside
    /// what the account's own have: those of them that are not its own,
    /// which were asked. Nothing, and never an error, when there are
    /// none, or none answers.
    async fn ask_lookup(&self, filters: Vec<Filter>) -> Vec<Event> {
        let in_use = self.in_use();
        let relays: Vec<RelayUrl> = self.reached().lookup.iter().filter(|relay| !in_use.contains(relay)).cloned().collect();
        if relays.is_empty() || filters.is_empty() {
            return Vec::new();
        }
        let asks = self.connected(relays.into_iter().map(|relay| (relay, filters.clone())).collect()).await;
        if asks.is_empty() {
            return Vec::new();
        }
        let asked = asks.len();
        let (events, answered, _) = self.gather(Pool::Others, asks, REACH_TIMEOUT).await;
        tracing::debug!("{} event(s) looked up on {answered} of {asked} relay(s)", events.len());
        events
    }

    /// The profiles of `authors` that are already in the store.
    pub async fn cached_profiles(&self, authors: Vec<PublicKey>) -> Vec<(PublicKey, Profile)> {
        let filter = Filter::new().kind(Kind::Metadata).authors(authors);
        match self.client.database().query(filter).await {
            Ok(events) => newest_profiles(events),
            Err(e) => {
                tracing::warn!("profiles could not be read from the store: {e}");
                Vec::new()
            }
        }
    }

    /// The profiles of `authors`, after asking the relays for them. As with
    /// a contact list, what the relays send is kept in the store, which only
    /// ever keeps the newest, and the answer is read back from there: a
    /// relay that is behind doesn't undo a newer profile already seen.
    pub async fn fetch_profiles(&self, authors: Vec<PublicKey>) -> Vec<(PublicKey, Profile)> {
        let fetched = match self.fetch(vec![Filter::new().kind(Kind::Metadata).authors(authors.clone())]).await {
            Ok(events) => newest_profiles(events),
            Err(e) => {
                tracing::warn!("profiles could not be fetched: {e:?}");
                return Vec::new();
            }
        };
        let mut profiles: HashMap<PublicKey, Profile> = fetched.into_iter().collect();
        profiles.extend(self.cached_profiles(authors).await);
        profiles.into_iter().collect()
    }

    /// The profiles of those of `people` that the account's own relays
    /// don't have, after asking the relays those people write to. Asked
    /// for beside [`Self::fetch_profiles`], which answers sooner for most:
    /// who is still missing is seen once the relay lists are known, by
    /// when the account's own relays have had their say.
    pub async fn fetch_profiles_where_written(&self, people: Vec<PublicKey>) -> Vec<(PublicKey, Profile)> {
        self.relays_of(&people, Listed::Write, true).await;
        let known: HashSet<PublicKey> =
            self.cached_profiles(people.clone()).await.into_iter().map(|(person, _)| person).collect();
        let missing: Vec<PublicKey> = people.into_iter().filter(|person| !known.contains(person)).collect();
        let profiles_of = |people: &[PublicKey]| -> Vec<Filter> {
            let asking = |chunk: &[PublicKey]| Filter::new().kind(Kind::Metadata).authors(chunk.iter().copied());
            people.chunks(AUTHORS_PER_FILTER).map(asking).collect()
        };
        // Where they write, and where everyone's profile is looked up.
        let theirs = self.ask_their_relays(&missing, Listed::Write, true, profiles_of);
        let (theirs, looked_up) = join(theirs, self.ask_lookup(profiles_of(&missing))).await;
        if theirs.is_empty() && looked_up.is_empty() {
            return Vec::new();
        }
        self.cached_profiles(missing).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUTHORS_PER_FILTER, FetchError, FollowError, ListError, NostrService, PublishError, Reach, Source, feed_filters, newest,
        newest_profiles,
    };
    use crate::nostr::account::testing::{Approves, with_remote_signer_for};
    use crate::nostr::account::{SignError, Signer};
    use crate::nostr::blossom;
    use crate::nostr::bookmarks::{self, Bookmarks, PrivateTags};
    use crate::nostr::compose;
    use crate::nostr::compose::Target;
    use crate::nostr::content::FEED_NOTE_MAX_CHARS;
    use crate::nostr::engagement::{self, Engagement, Reaction};
    use crate::nostr::kept_drafts::DRAFT_KIND;
    use crate::nostr::models::Note;
    use crate::nostr::outbox::Whereabouts;
    use crate::nostr::relay_lists::{Kept, RelayLists, Section};
    use crate::nostr::search::{Found, SearchFor};
    use nostr_connect::prelude::{Nip04, Nip44};
    use nostr_sdk::prelude::{
        Event, EventBuilder, EventId, Filter, FinalizeEvent, Keys, Kind, LocalRelay, MachineReadablePrefix, Nip19Event,
        PublicKey, RelayUrl, Tag, Timestamp, ToBech32, Url, WritePolicy, WritePolicyResult,
    };
    use std::collections::HashMap;
    use std::future::Future;
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::time::{Duration, Instant};

    fn people(count: usize) -> Vec<PublicKey> {
        (0..count).map(|_| Keys::generate().public_key()).collect()
    }

    /// A note saying `text`, written at `at`, with the `e` tags `(id,
    /// marker)`, an empty marker being none.
    fn note(text: &str, at: u64, tags: &[(EventId, &str)]) -> Event {
        let tags = tags.iter().map(|(id, marker)| match *marker {
            "" => Tag::parse(["e", &id.to_hex()]).unwrap(),
            marker => Tag::parse(["e", &id.to_hex(), "", marker]).unwrap(),
        });
        EventBuilder::new(Kind::TextNote, text)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(&Keys::generate())
            .unwrap()
    }

    /// What the store knows of the thread of `focused` when it holds
    /// `events`: the ids, sorted, and how many characters the focused note
    /// shows.
    fn thread_in_store(name: &str, events: &[Event], focused: EventId) -> (Vec<EventId>, usize) {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let notes = runtime.block_on(async {
            let service = NostrService::open(path.clone(), Vec::new()).await.unwrap();
            for event in events {
                service.client.database().save_event(event).await.unwrap();
            }
            service.thread(Source::Store, focused, &Whereabouts::default()).await.unwrap()
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        let shown = notes.iter().find(|note| note.id == focused).map_or(0, |note| note.content.line_chars.iter().sum::<usize>());
        let mut ids: Vec<EventId> = notes.iter().map(|note| note.id).collect();
        ids.sort();
        (ids, shown)
    }

    fn sorted(mut ids: Vec<EventId>) -> Vec<EventId> {
        ids.sort();
        ids
    }

    #[test]
    fn a_thread_is_the_root_what_names_it_and_what_names_those() {
        let long = "word ".repeat(FEED_NOTE_MAX_CHARS);
        let root = note("it starts", 100, &[]);
        let first = note("an answer", 200, &[(root.id, "root")]);
        let focused = note(&long, 300, &[(root.id, "root"), (first.id, "reply")]);
        // Tags only the note it answers, as some clients do.
        let deeper = note("an answer to the answer", 400, &[(focused.id, "")]);
        // Cites a note of the thread without answering anything.
        let citing = note("look at this", 450, &[(first.id, "mention")]);
        let elsewhere = note("another conversation", 500, &[]);
        let events = [root.clone(), first.clone(), focused.clone(), deeper.clone(), citing, elsewhere];

        let (ids, shown) = thread_in_store("thread", &events, focused.id);

        assert_eq!(ids, sorted(vec![root.id, first.id, focused.id, deeper.id]));
        assert_eq!(shown, long.trim().chars().count(), "the note the thread is for shows in full");
    }

    #[test]
    fn a_thread_reaches_for_the_parents_that_nothing_else_brought() {
        // The parent names a root of its own, which nobody has; the focused
        // note names another. Only the walk up from the note finds the parent.
        let (lost_root, other_root) = (EventId::from_byte_array([1; 32]), EventId::from_byte_array([2; 32]));
        let parent = note("the parent", 100, &[(lost_root, "root")]);
        let focused = note("the note", 200, &[(other_root, "root"), (parent.id, "reply")]);

        let (ids, _) = thread_in_store("parents", &[parent.clone(), focused.clone()], focused.id);

        assert_eq!(ids, sorted(vec![parent.id, focused.id]));
    }

    #[test]
    fn a_thread_of_comments_is_what_they_are_about_and_the_comments_under_it() {
        let keys = Keys::generate();
        let at = |builder: EventBuilder, at: u64| builder.custom_created_at(Timestamp::from_secs(at)).finalize(&keys).unwrap();
        let comment = |text: &str, on: &Event, when: u64| at(compose::build(text, Some(on), keys.public_key()), when);

        let picture = at(EventBuilder::new(Kind::Custom(20), "a heron"), 100);
        let first = comment("what a bird", &picture, 200);
        let answer = comment("isn't it", &first, 300);
        let other = comment("lovely", &picture, 400);
        let elsewhere = note("another conversation", 500, &[]);
        let events = [picture.clone(), first.clone(), answer.clone(), other.clone(), elsewhere];

        // Opened for the answer to a comment: the picture, both comments on
        // it, and the answer.
        let (ids, _) = thread_in_store("comments", &events, answer.id);
        assert_eq!(ids, sorted(vec![picture.id, first.id, answer.id, other.id]));
    }

    /// A relay that takes nothing.
    #[derive(Debug)]
    struct TakesNothing;

    impl WritePolicy for TakesNothing {
        fn admit_event<'a>(
            &'a self,
            _event: &'a Event,
            _addr: &'a SocketAddr,
        ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
            Box::pin(async { WritePolicyResult::reject(MachineReadablePrefix::Blocked, "not here") })
        }
    }

    /// Publish a note to a relay in this process, which takes it or (with
    /// `refusing`) doesn't. Returns what publishing said, and whether the
    /// store has the note afterwards.
    fn publish_to_a_relay(name: &str, refusing: bool) -> (Result<usize, PublishError>, bool) {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let outcome = runtime.block_on(async {
            let builder = LocalRelay::builder();
            let relay = if refusing { builder.write_policy(TakesNothing).build() } else { builder.build() };
            relay.run().await.unwrap();
            let service = NostrService::open(path.clone(), vec![relay.url().await.to_string()]).await.unwrap();

            let event = note("hello", 100, &[]);
            let result = service.publish(&event).await;
            let stored = service.stored_event(event.id).await.is_some();
            relay.shutdown();
            (result, stored)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);
        outcome
    }

    #[test]
    fn what_was_made_of_a_note_is_counted_from_the_store_which_the_relays_fill() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-engagement", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (me, about) = (Keys::generate(), note("it starts", 100, &[]));
        let (before, after) = runtime.block_on(async {
            let relay = LocalRelay::builder().build();
            relay.run().await.unwrap();
            let like = |keys: &Keys| EventBuilder::new(Kind::Reaction, "+").tag(Tag::event(about.id)).finalize(keys).unwrap();
            let repost = engagement::repost(&about, None).finalize(&me).unwrap();
            for event in
                [about.clone(), note("an answer", 200, &[(about.id, "root")]), like(&me), like(&Keys::generate()), repost]
            {
                relay.add_event(event).await.unwrap();
            }
            let service = NostrService::open(path.clone(), vec![relay.url().await.to_string()]).await.unwrap();

            let url = relay.url().await;
            let ask = |source| service.standing(source, vec![about.id], Some(me.public_key()));
            let before = ask(Source::Store).await.unwrap();
            let after = ask(Source::Relays).await.unwrap();
            assert_eq!(ask(Source::Store).await.unwrap(), after, "what the relays sent is the store's now");
            relay.shutdown();
            assert_eq!(after.seen_on, HashMap::from([(about.id, vec![url])]), "the relay that has the note said so");
            (before, after)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        assert_eq!(before.engagement.get(&about.id), Some(&Engagement::default()), "nothing known yet");
        assert!(before.seen_on.is_empty(), "and seen nowhere: no relay was asked");
        assert_eq!(
            after.engagement.get(&about.id),
            Some(&Engagement { replies: 1, reposts: 1, reposted: true, likes: 2, reaction: Some(Reaction::Like), zap_sats: 0 })
        );
    }

    #[test]
    fn a_page_comes_with_the_notes_its_notes_quote_and_the_store_keeps_them() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-quotes", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (me, them) = (Keys::generate(), Keys::generate());
        let at = |builder: EventBuilder, keys: &Keys, at: u64| {
            builder.custom_created_at(Timestamp::from_secs(at)).finalize(keys).unwrap()
        };
        let address = |event: &Event| format!("nostr:{}", event.id.to_bech32().unwrap());
        // A chain of quotes, three notes long under the note of mine: the
        // custom, the note that passed it on, and the one that did so again.
        let innermost = at(EventBuilder::new(Kind::TextNote, "where it started"), &them, 70);
        let inner = at(EventBuilder::new(Kind::TextNote, format!("the custom {}", address(&innermost))), &them, 80);
        // A note, a picture (which is no note a row shows), and a note
        // nobody has; and a note of mine that quotes all three.
        let quoted = at(EventBuilder::new(Kind::TextNote, format!("worth a second look {}", address(&inner))), &them, 100);
        let picture = at(EventBuilder::new(Kind::Custom(20), "a heron"), &them, 110);
        let lost = at(EventBuilder::new(Kind::TextNote, "nobody kept this"), &them, 120);
        let text = format!("look {} {} {}", address(&quoted), address(&picture), address(&lost));
        let quoting = at(EventBuilder::new(Kind::TextNote, text), &me, 200);

        let (before, fetched, cached, thread) = runtime.block_on(async {
            let relay = LocalRelay::builder().build();
            relay.run().await.unwrap();
            for event in [innermost.clone(), inner.clone(), quoted.clone(), picture.clone(), quoting.clone()] {
                relay.add_event(event).await.unwrap();
            }
            let service = NostrService::open(path.clone(), vec![relay.url().await.to_string()]).await.unwrap();
            let authors = [me.public_key()];

            // The store has the note already, and not what it quotes.
            service.client.database().save_event(&quoting).await.unwrap();
            let before = service.cached_feed(&authors, 50).await.unwrap();
            let fetched = service.fetch_feed(&authors, None, 50).await.unwrap();
            // The relays are gone: what they sent is the store's now.
            relay.shutdown();
            let cached = service.cached_feed(&authors, 50).await.unwrap();
            let thread = service.thread(Source::Store, quoting.id, &Whereabouts::default()).await.unwrap();
            (before, fetched, cached, thread)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        let quoted_by = |notes: &[Note]| -> Vec<EventId> {
            assert_eq!(notes.iter().map(|note| note.id).collect::<Vec<_>>(), [quoting.id]);
            notes[0].quoted.iter().map(|note| note.id).collect()
        };
        assert!(quoted_by(&before).is_empty(), "the store alone has nothing to show for them");
        assert_eq!(quoted_by(&fetched), [quoted.id], "the note; not the picture, and not what nobody has");
        assert_eq!(quoted_by(&cached), [quoted.id]);
        assert_eq!(quoted_by(&thread), [quoted.id]);
        assert_eq!(fetched[0].content.quotes.len(), 3, "all three are named, and two show as links");

        // The note quoted comes with the note it quotes, and that one
        // without: two notes deep, and the third is a link.
        for notes in [&fetched, &cached, &thread] {
            let first = &notes[0].quoted[0];
            assert_eq!(first.quoted.iter().map(|note| note.id).collect::<Vec<_>>(), [inner.id]);
            assert!(first.quoted[0].quoted.is_empty(), "no deeper");
            assert_eq!(first.quoted[0].content.quotes.iter().map(|quote| quote.id).collect::<Vec<_>>(), [innermost.id]);
        }
    }

    #[test]
    fn a_note_is_seen_on_the_relays_that_have_it_and_on_those_that_took_it() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-seen", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let keys = Keys::generate();
        let at = |text: &str, at: u64| {
            EventBuilder::new(Kind::TextNote, text).custom_created_at(Timestamp::from_secs(at)).finalize(&keys).unwrap()
        };
        let (everywhere, on_one, mine) = (at("on both", 300), at("on the first", 200), at("just written", 400));
        let (first, second, feed, seen) = runtime.block_on(async {
            let (first, second) = (LocalRelay::builder().build(), LocalRelay::builder().build());
            first.run().await.unwrap();
            second.run().await.unwrap();
            first.add_event(everywhere.clone()).await.unwrap();
            first.add_event(on_one.clone()).await.unwrap();
            second.add_event(everywhere.clone()).await.unwrap();
            let urls = (first.url().await, second.url().await);
            let service = NostrService::open(path.clone(), vec![urls.0.to_string(), urls.1.to_string()]).await.unwrap();

            // A page of the feed says where its notes are; and publishing
            // a note, which relays took it.
            let feed = service.fetch_feed(&[keys.public_key()], None, 50).await.unwrap();
            service.publish(&mine).await.unwrap();
            let seen = service.seen_on(&[everywhere.id, on_one.id, mine.id, EventId::from_byte_array([9; 32])]);
            first.shutdown();
            second.shutdown();
            (urls.0, urls.1, feed, seen)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        let ids: Vec<EventId> = feed.iter().map(|note| note.id).collect();
        assert_eq!(ids, [everywhere.id, on_one.id], "each note once, newest first, whoever sent it");
        let on = |id: EventId| {
            let mut relays = seen.get(&id).cloned().unwrap_or_default();
            relays.sort();
            relays
        };
        let mut both = vec![first.clone(), second];
        both.sort();
        assert_eq!(on(everywhere.id), both);
        assert_eq!(on(on_one.id), [first]);
        assert_eq!(on(mine.id), both);
        assert_eq!(seen.len(), 3, "a note nobody sent is seen nowhere");
    }

    #[test]
    fn the_relay_that_has_an_event_is_the_one_pointed_at() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-holder", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (held, missing) = (note("here", 100, &[]), note("nowhere", 200, &[]));
        let (with, without, holder, nobody) = runtime.block_on(async {
            let (with, without) = (LocalRelay::builder().build(), LocalRelay::builder().build());
            with.run().await.unwrap();
            without.run().await.unwrap();
            with.add_event(held.clone()).await.unwrap();
            let urls = (with.url().await, without.url().await);
            let service = NostrService::open(path.clone(), vec![urls.1.to_string(), urls.0.to_string()]).await.unwrap();

            let found = (service.relay_holding(held.id).await, service.relay_holding(missing.id).await);
            with.shutdown();
            without.shutdown();
            (urls.0, urls.1, found.0, found.1)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        assert_ne!(with, without);
        assert_eq!(holder, Some(with));
        assert_eq!(nobody, None);
    }

    #[test]
    fn the_receipt_of_a_zap_is_waited_for_and_then_counted() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-receipt", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let about = note("it starts", 100, &[]);
        let receipt = |invoice: &str| {
            EventBuilder::new(Kind::ZapReceipt, "")
                .tags([Tag::event(about.id), Tag::custom("bolt11", [invoice])])
                .finalize(&Keys::generate())
                .unwrap()
        };
        let (paid, counted) = runtime.block_on(async {
            let relay = LocalRelay::builder().build();
            relay.run().await.unwrap();
            // Someone else's zap of the same note is there already.
            relay.add_event(receipt("lnbc10u1theirs")).await.unwrap();
            let service = NostrService::open(path.clone(), vec![relay.url().await.to_string()]).await.unwrap();

            let waiting =
                tokio::time::timeout(Duration::from_secs(10), service.await_zap_receipt(about.id, "lnbc210n1mine".into()));
            let paying = async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                // As a wallet service may write it: in capitals.
                assert!(relay.notify_event(receipt("LNBC210N1MINE")));
            };
            let (paid, ()) = tokio::join!(waiting, paying);
            let counted = service.standing(Source::Store, vec![about.id], None).await.unwrap().engagement;
            relay.shutdown();
            (paid, counted)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        assert_eq!(paid, Ok(true), "the receipt for this invoice, not the one that was there");
        assert_eq!(counted.get(&about.id).map(|engagement| engagement.zap_sats), Some(1_021), "both are in the store");
    }

    #[test]
    fn a_published_event_is_kept_once_a_relay_took_it() {
        assert_eq!(publish_to_a_relay("published", false), (Ok(1), true));
    }

    #[test]
    fn an_event_no_relay_took_is_not_kept_and_says_why() {
        let (result, stored) = publish_to_a_relay("refused", true);
        match result {
            Err(PublishError::Refused(reason)) => assert!(reason.contains("not here"), "{reason}"),
            other => panic!("refused, not {other:?}"),
        }
        assert!(!stored, "nobody else can see it: the store doesn't show it either");
    }

    #[test]
    fn notes_named_by_id_come_in_the_order_they_were_named_from_the_store_or_the_relays() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-listed", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let address = |event: &Event| format!("nostr:{}", event.id.to_bech32().unwrap());
        let (kept, far) = (note("in the store", 100, &[]), note("on the relay", 200, &[]));
        let quoting = EventBuilder::new(Kind::TextNote, format!("on the relay too, quoting {}", address(&far)))
            .custom_created_at(Timestamp::from_secs(300))
            .finalize(&Keys::generate())
            .unwrap();
        let picture = EventBuilder::new(Kind::Custom(20), "a heron").finalize(&Keys::generate()).unwrap();
        let lost = note("nobody kept this", 400, &[]);

        let (listed, again) = runtime.block_on(async {
            let relay = LocalRelay::builder().build();
            relay.run().await.unwrap();
            for event in [far.clone(), quoting.clone(), picture.clone()] {
                relay.add_event(event).await.unwrap();
            }
            let service = NostrService::open(path.clone(), vec![relay.url().await.to_string()]).await.unwrap();
            service.client.database().save_event(&kept).await.unwrap();

            let ids: Vec<(EventId, Whereabouts)> =
                [quoting.id, lost.id, kept.id, picture.id, far.id].map(|id| (id, Whereabouts::default())).to_vec();
            let listed = service.notes_listed(ids.clone()).await;
            // The relay is gone: what it sent is the store's now.
            relay.shutdown();
            (listed, service.notes_listed(ids).await)
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);

        // The one nobody has and the one that is no note are left out.
        let ids = |notes: &[Note]| notes.iter().map(|note| note.id).collect::<Vec<_>>();
        assert_eq!(ids(&listed), [quoting.id, kept.id, far.id]);
        assert_eq!(ids(&listed[0].quoted), [far.id], "with what it quotes");
        assert_eq!(ids(&again), ids(&listed));
    }

    /// Run `test` with `count` relays in this process and a client whose
    /// own relay is the first of them. It takes anyone's word for a relay,
    /// since none of these is on the open internet, and reaches for `cap`
    /// of other people's at a time.
    fn with_relays<F: Future<Output = ()>>(
        name: &str,
        count: usize,
        cap: usize,
        test: impl FnOnce(NostrService, Vec<LocalRelay>, Vec<RelayUrl>) -> F,
    ) {
        with_relays_reaching(name, count, Reach { cap, accepts: |_| true, ..Reach::default() }, test);
    }

    /// As [`with_relays`], reaching as far as `reach` says.
    fn with_relays_reaching<F: Future<Output = ()>>(
        name: &str,
        count: usize,
        reach: Reach,
        test: impl FnOnce(NostrService, Vec<LocalRelay>, Vec<RelayUrl>) -> F,
    ) {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let mut relays = Vec::new();
            let mut urls = Vec::new();
            for _ in 0..count {
                let relay = LocalRelay::builder().build();
                relay.run().await.unwrap();
                urls.push(relay.url().await);
                relays.push(relay);
            }
            let service = NostrService::open_reaching(path.clone(), vec![urls[0].to_string()], reach).await.unwrap();
            tokio::time::timeout(Duration::from_secs(60), test(service, relays.clone(), urls)).await.unwrap();
            for relay in relays {
                relay.shutdown();
            }
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// The relay list of `keys`: the relays it writes to, and those it reads.
    fn relay_list(keys: &Keys, write: &[&RelayUrl], read: &[&RelayUrl]) -> Event {
        let marked = |relays: &[&RelayUrl], marker: &str| -> Vec<Tag> {
            relays.iter().map(|relay| Tag::parse(["r", relay.as_str(), marker]).unwrap()).collect()
        };
        EventBuilder::new(Kind::RelayList, "")
            .tags(marked(write, "write").into_iter().chain(marked(read, "read")))
            .finalize(keys)
            .unwrap()
    }

    /// A note by `keys`, written at `at`, with `tags`.
    fn note_by(keys: &Keys, text: &str, at: u64, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::TextNote, text).tags(tags).custom_created_at(Timestamp::from_secs(at)).finalize(keys).unwrap()
    }

    /// Whether the relay at `relay` has the event `id`, asked by a client
    /// of its own.
    async fn holds(relay: &RelayUrl, id: EventId) -> bool {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-holds-{}", std::process::id(), id.to_hex()));
        let _ = std::fs::remove_dir_all(&path);
        let asking = NostrService::open(path.clone(), vec![relay.to_string()]).await.unwrap();
        let held = asking.events(Source::Relays, vec![Filter::new().id(id)]).await.is_ok_and(|events| !events.is_empty());
        let _ = std::fs::remove_dir_all(&path);
        held
    }

    fn ids(notes: &[Note]) -> Vec<EventId> {
        notes.iter().map(|note| note.id).collect()
    }

    #[test]
    fn a_note_the_store_let_go_of_is_asked_of_the_relays_again() {
        with_relays("swept-note", 2, 20, async |service, relays, urls| {
            let (me, near, far) = (Keys::generate(), Keys::generate(), Keys::generate());
            let here = note_by(&near, "on my relay", 300, vec![]);
            let there = note_by(&far, "on theirs", 200, vec![]);
            relays[0].add_event(relay_list(&far, &[&urls[1]], &[])).await.unwrap();
            relays[0].add_event(here.clone()).await.unwrap();
            relays[1].add_event(there.clone()).await.unwrap();
            let authors = [near.public_key(), far.public_key()];
            assert_eq!(service.fetch_feed(&authors, None, 50).await.unwrap().len(), 2);

            // Swept, neither is in the store, and each is found again
            // where it is: on the account's relay, and where its author
            // writes.
            let swept = service.sweep(me.public_key(), &std::collections::HashSet::new(), Timestamp::from_secs(1_000)).await;
            assert_eq!(swept, Ok(2));
            assert!(service.stored_event(here.id).await.is_none() && service.stored_event(there.id).await.is_none());
            assert_eq!(service.event(here.id, near.public_key()).await, Some(here));
            assert_eq!(service.event(there.id, far.public_key()).await, Some(there.clone()));
            // What the relays sent is the store's again.
            assert_eq!(service.stored_event(there.id).await, Some(there));
            assert_eq!(service.event(id(9), far.public_key()).await, None);
        });
    }

    #[test]
    fn notes_are_read_from_the_relays_their_authors_write_to() {
        with_relays("outbox-feed", 3, 20, async |service, relays, urls| {
            let (near, far, nowhere) = (Keys::generate(), Keys::generate(), Keys::generate());
            // One writes to the account's own relay, one elsewhere, and
            // one has no relay list: all three lists are on the own relay.
            let here = note_by(&near, "on my relay", 300, vec![]);
            let there = note_by(&far, "on theirs", 200, vec![]);
            let older = note_by(&far, "on theirs, long ago", 100, vec![]);
            let lost = note_by(&nowhere, "on a relay nobody names", 250, vec![]);
            relays[0].add_event(relay_list(&near, &[&urls[0]], &[])).await.unwrap();
            relays[0].add_event(relay_list(&far, &[&urls[1]], &[])).await.unwrap();
            relays[0].add_event(here.clone()).await.unwrap();
            relays[1].add_event(there.clone()).await.unwrap();
            relays[1].add_event(older.clone()).await.unwrap();
            relays[2].add_event(lost.clone()).await.unwrap();

            let authors = [near.public_key(), far.public_key(), nowhere.public_key()];
            let page = service.fetch_feed(&authors, None, 50).await.unwrap();
            assert_eq!(ids(&page), [here.id, there.id, older.id], "newest first, from wherever they are");
            // Each was seen on the relay that sent it.
            let seen = service.seen_on(&[here.id, there.id]);
            assert_eq!(seen.get(&here.id), Some(&vec![urls[0].clone()]));
            assert_eq!(seen.get(&there.id), Some(&vec![urls[1].clone()]));
            // Only the relay someone writes to is in the pool of other
            // people's, and the account's own pool is as it was.
            assert_eq!(service.others.relays().await.into_keys().collect::<Vec<_>>(), [urls[1].clone()]);
            assert_eq!(service.client.relays().await.len(), 1);

            // A page further back comes from there as well, and what came
            // is in the store.
            let before = service.fetch_feed(&authors, Some(150), 50).await.unwrap();
            assert_eq!(ids(&before), [older.id]);
            assert!(service.stored_event(there.id).await.is_some());

            // With the own relay gone, their relays still answer: the
            // lists are in the store.
            relays[0].shutdown();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let without = service.fetch_feed(&[far.public_key()], None, 50).await.unwrap();
            assert_eq!(ids(&without), [there.id, older.id]);
        });
    }

    #[test]
    fn no_more_of_other_peoples_relays_are_reached_for_than_may_be() {
        // Two authors on two relays, and room for one.
        with_relays("outbox-cap", 3, 1, async |service, relays, urls| {
            let (one, other) = (Keys::generate(), Keys::generate());
            relays[0].add_event(relay_list(&one, &[&urls[1]], &[])).await.unwrap();
            relays[0].add_event(relay_list(&other, &[&urls[2]], &[])).await.unwrap();
            relays[1].add_event(note_by(&one, "on one", 100, vec![])).await.unwrap();
            relays[2].add_event(note_by(&other, "on the other", 200, vec![])).await.unwrap();

            let page = service.fetch_feed(&[one.public_key(), other.public_key()], None, 50).await.unwrap();
            assert_eq!(page.len(), 1, "one relay was asked, and not the other");
            assert_eq!(service.others.relays().await.len(), 1);
        });
        // And none at all when none may be.
        with_relays("outbox-none", 2, 0, async |service, relays, urls| {
            let far = Keys::generate();
            relays[0].add_event(relay_list(&far, &[&urls[1]], &[])).await.unwrap();
            relays[1].add_event(note_by(&far, "on theirs", 100, vec![])).await.unwrap();
            assert!(service.fetch_feed(&[far.public_key()], None, 50).await.unwrap().is_empty());
            assert!(service.others.relays().await.is_empty());
        });
    }

    #[test]
    fn a_relay_on_this_machine_is_not_connected_to_on_somebody_elses_word() {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-outbox-word", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let (mine, theirs) = (LocalRelay::builder().build(), LocalRelay::builder().build());
            mine.run().await.unwrap();
            theirs.run().await.unwrap();
            let far = Keys::generate();
            mine.add_event(relay_list(&far, &[&theirs.url().await], &[])).await.unwrap();
            theirs.add_event(note_by(&far, "on theirs", 100, vec![])).await.unwrap();

            // As the applet opens it: what a list names has to be public.
            let service = NostrService::open(path.clone(), vec![mine.url().await.to_string()]).await.unwrap();
            assert!(service.fetch_feed(&[far.public_key()], None, 50).await.unwrap().is_empty());
            assert!(service.others.relays().await.is_empty());
            mine.shutdown();
            theirs.shutdown();
        });
        drop(runtime);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_relay_that_does_not_connect_is_left_alone_and_the_others_still_asked() {
        with_relays("outbox-dead", 2, 20, async |service, relays, urls| {
            let far = Keys::generate();
            // Nothing listens at the first relay they name.
            let dead = RelayUrl::parse("ws://127.0.0.1:9").unwrap();
            let there = note_by(&far, "on theirs", 100, vec![]);
            relays[0].add_event(relay_list(&far, &[&dead, &urls[1]], &[])).await.unwrap();
            relays[1].add_event(there.clone()).await.unwrap();

            let page = service.fetch_feed(&[far.public_key()], None, 50).await.unwrap();
            assert_eq!(ids(&page), [there.id]);
            assert_eq!(service.others.relays().await.into_keys().collect::<Vec<_>>(), [urls[1].clone()]);
            assert!(service.reached().failed.contains_key(&dead));

            // Asked again, it is not tried again: nothing waits for it.
            let asked = Instant::now();
            assert_eq!(ids(&service.fetch_feed(&[far.public_key()], None, 50).await.unwrap()), [there.id]);
            assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
        });
    }

    #[test]
    fn a_profile_the_accounts_relays_do_not_have_is_asked_of_where_its_person_writes() {
        with_relays("outbox-profiles", 2, 20, async |service, relays, urls| {
            let (near, far, unlisted) = (Keys::generate(), Keys::generate(), Keys::generate());
            let profile = |keys: &Keys, name: &str| {
                EventBuilder::new(Kind::Metadata, format!(r#"{{"name":"{name}"}}"#)).finalize(keys).unwrap()
            };
            relays[0].add_event(profile(&near, "Near")).await.unwrap();
            relays[0].add_event(relay_list(&far, &[&urls[1]], &[])).await.unwrap();
            relays[1].add_event(profile(&far, "Far")).await.unwrap();
            // On their relay too, which nothing says.
            relays[1].add_event(profile(&unlisted, "Unlisted")).await.unwrap();

            let people = vec![near.public_key(), far.public_key(), unlisted.public_key()];
            let own = service.fetch_profiles(people.clone()).await;
            assert_eq!(own.iter().map(|(person, _)| *person).collect::<Vec<_>>(), [near.public_key()]);

            let theirs = service.fetch_profiles_where_written(people.clone()).await;
            assert_eq!(theirs.len(), 1);
            assert_eq!(theirs[0].0, far.public_key());
            assert_eq!(theirs[0].1.name.as_deref(), Some("Far"));
            // Known by now: nobody is asked about again.
            assert!(service.fetch_profiles_where_written(vec![near.public_key(), far.public_key()]).await.is_empty());
        });
    }

    #[test]
    fn a_quoted_note_is_asked_of_where_its_address_says_it_is() {
        with_relays("outbox-quotes", 3, 20, async |service, relays, urls| {
            let (me, far, quiet) = (Keys::generate(), Keys::generate(), Keys::generate());
            let quoted = note_by(&far, "said elsewhere", 100, vec![]);
            let unsigned = note_by(&far, "said elsewhere too", 110, vec![]);
            let hinted = note_by(&quiet, "on a relay the address names", 120, vec![]);
            relays[0].add_event(relay_list(&far, &[&urls[1]], &[])).await.unwrap();
            relays[1].add_event(quoted.clone()).await.unwrap();
            relays[1].add_event(unsigned.clone()).await.unwrap();
            // Nobody's relay list names this one.
            relays[2].add_event(hinted.clone()).await.unwrap();

            // One address says who wrote the note, one says nothing, and
            // one says which relay has it.
            let with_author = Nip19Event::new(quoted.id).author(far.public_key()).to_bech32().unwrap();
            let bare = unsigned.id.to_bech32().unwrap();
            let with_relay = Nip19Event::new(hinted.id).relays([urls[2].clone()]).to_bech32().unwrap();
            let text = format!("look nostr:{with_author} and nostr:{bare} and nostr:{with_relay}");
            let quoting = note_by(&me, &text, 200, vec![]);
            relays[0].add_event(quoting.clone()).await.unwrap();

            let page = service.fetch_feed(&[me.public_key()], None, 50).await.unwrap();
            assert_eq!(ids(&page), [quoting.id]);
            assert_eq!(
                ids(&page[0].quoted),
                [quoted.id, hinted.id],
                "found where its author writes, and on the relay its address names"
            );
            assert_eq!(service.seen_on(&[hinted.id]).get(&hinted.id), Some(&vec![urls[2].clone()]));
        });
    }

    #[test]
    fn a_search_asks_the_accounts_search_relays_and_no_others() {
        with_relays("search", 3, 20, async |service, relays, urls| {
            let (author, other) = (Keys::generate(), Keys::generate());
            let profile = |keys: &Keys, name: &str| {
                EventBuilder::new(Kind::Metadata, format!(r#"{{"name":"{name}"}}"#)).finalize(keys).unwrap()
            };
            let wanted = note_by(&author, "tokens for friends", 100, vec![]);
            relays[1].add_event(wanted.clone()).await.unwrap();
            relays[1].add_event(note_by(&author, "something else", 200, vec![])).await.unwrap();
            relays[1].add_event(profile(&author, "Karnage")).await.unwrap();
            relays[1].add_event(profile(&other, "Somebody")).await.unwrap();
            // A relay that is not searched has a match nobody finds.
            relays[2].add_event(note_by(&other, "tokens elsewhere", 300, vec![])).await.unwrap();

            let searched = [urls[1].clone()];
            let Found::Notes(notes) = service.search(&searched, "tokens", SearchFor::Notes).await.unwrap() else {
                panic!("notes were searched for");
            };
            assert_eq!(ids(&notes), [wanted.id]);
            assert_eq!(service.seen_on(&[wanted.id]).get(&wanted.id), Some(&vec![urls[1].clone()]));
            let Found::People(people) = service.search(&searched, "karnage", SearchFor::People).await.unwrap() else {
                panic!("people were searched for");
            };
            assert_eq!(people.len(), 1);
            assert_eq!((people[0].0, people[0].1.name.as_deref()), (author.public_key(), Some("Karnage")));
            assert_eq!(service.others.relays().await.into_keys().collect::<Vec<_>>(), searched);

            // One of the account's own relays that searches is asked where it is.
            let at_home = note_by(&other, "tokens at home", 400, vec![]);
            relays[0].add_event(at_home.clone()).await.unwrap();
            let both = [urls[0].clone(), urls[1].clone()];
            let Found::Notes(notes) = service.search(&both, "tokens", SearchFor::Notes).await.unwrap() else {
                panic!("notes were searched for");
            };
            assert_eq!(sorted(ids(&notes)), sorted(vec![wanted.id, at_home.id]));
            assert_eq!(service.others.relays().await.len(), 1);

            // Two searches at once, as a search page makes them, reach
            // for the same relay and both get its answer.
            let newly = [urls[2].clone()];
            let (one, other) = futures_util::future::join(
                service.search(&newly, "tokens", SearchFor::Notes),
                service.search(&newly, "tokens", SearchFor::People),
            )
            .await;
            assert!(matches!(one, Ok(Found::Notes(notes)) if notes.len() == 1));
            assert!(matches!(other, Ok(Found::People(people)) if people.is_empty()));

            // A relay's own notes, for whoever names it: the newest first.
            let later = note_by(&author, "later, on the same relay", 500, vec![]);
            relays[2].add_event(later.clone()).await.unwrap();
            let on_relay = service.relay_notes(&urls[2]).await.unwrap();
            assert_eq!(ids(&on_relay).first(), Some(&later.id));
            assert_eq!(on_relay.len(), 2);

            // Nothing found is no error; nobody to ask is.
            let Found::Notes(none) = service.search(&searched, "nothing says this", SearchFor::Notes).await.unwrap() else {
                panic!("notes were searched for");
            };
            assert!(none.is_empty());
            let dead = [RelayUrl::parse("ws://127.0.0.1:9").unwrap()];
            assert!(matches!(service.search(&dead, "tokens", SearchFor::Notes).await, Err(FetchError::NoRelayReachable)));
            assert!(matches!(service.search(&[], "tokens", SearchFor::Notes).await, Err(FetchError::NoRelayReachable)));
        });
    }

    #[test]
    fn a_thread_opens_on_a_note_that_is_only_where_its_address_says() {
        with_relays("outbox-thread-hint", 3, 20, async |service, relays, urls| {
            let (far, quiet) = (Keys::generate(), Keys::generate());
            let hinted = note_by(&quiet, "on a relay the address names", 100, vec![]);
            let written = note_by(&far, "where its author writes", 200, vec![]);
            relays[0].add_event(relay_list(&far, &[&urls[2]], &[])).await.unwrap();
            relays[1].add_event(hinted.clone()).await.unwrap();
            relays[2].add_event(written.clone()).await.unwrap();

            // With nothing said of where it is, it is not found.
            let nowhere = Whereabouts::default();
            assert!(service.thread(Source::Relays, hinted.id, &nowhere).await.unwrap().is_empty());
            assert!(service.others.relays().await.is_empty());

            let on = Whereabouts { author: None, relay: Some(urls[1].clone()) };
            assert_eq!(ids(&service.thread(Source::Relays, hinted.id, &on).await.unwrap()), [hinted.id]);
            let by = Whereabouts { author: Some(far.public_key()), relay: None };
            assert_eq!(ids(&service.thread(Source::Relays, written.id, &by).await.unwrap()), [written.id]);
            // The store's turn asks nobody, wherever the note is said to be.
            let unseen = note_by(&quiet, "another, there too", 300, vec![]);
            relays[1].add_event(unseen.clone()).await.unwrap();
            assert!(service.thread(Source::Store, unseen.id, &on).await.unwrap().is_empty());
        });
    }

    #[test]
    fn a_listed_note_is_asked_of_the_relay_it_is_said_to_be_on_and_of_where_its_author_writes() {
        with_relays("outbox-listed", 4, 20, async |service, relays, urls| {
            let (far, quiet) = (Keys::generate(), Keys::generate());
            let said = note_by(&quiet, "on the relay the list names", 100, vec![]);
            let written = note_by(&far, "where its author writes", 200, vec![]);
            let unsaid = note_by(&quiet, "on a relay nothing names", 300, vec![]);
            relays[0].add_event(relay_list(&far, &[&urls[2]], &[])).await.unwrap();
            relays[1].add_event(said.clone()).await.unwrap();
            relays[2].add_event(written.clone()).await.unwrap();
            relays[3].add_event(unsaid.clone()).await.unwrap();

            let on = |relay: &RelayUrl| Whereabouts { author: None, relay: Some(relay.clone()) };
            let by = |author: &Keys| Whereabouts { author: Some(author.public_key()), relay: None };
            let listed = vec![(said.id, on(&urls[1])), (written.id, by(&far)), (unsaid.id, Whereabouts::default())];
            let found = service.notes_listed(listed.clone()).await;
            assert_eq!(ids(&found), [said.id, written.id]);
            assert_eq!(service.seen_on(&[said.id]).get(&said.id), Some(&vec![urls[1].clone()]));

            // Said to be where it isn't: not found, and nothing fails.
            assert!(service.notes_listed(vec![(unsaid.id, on(&urls[1]))]).await.is_empty());
        });
        // Nothing is reached for when nothing may be.
        with_relays("outbox-listed-none", 2, 0, async |service, relays, urls| {
            let said = note_by(&Keys::generate(), "on the relay the list names", 100, vec![]);
            relays[1].add_event(said.clone()).await.unwrap();
            let listed = vec![(said.id, Whereabouts { author: None, relay: Some(urls[1].clone()) })];
            assert!(service.notes_listed(listed).await.is_empty());
            assert!(service.others.relays().await.is_empty());
        });
    }

    #[test]
    fn a_thread_is_asked_of_the_relays_of_those_whose_conversation_it_is() {
        with_relays("outbox-thread", 4, 20, async |service, relays, urls| {
            let (asker, answerer, third) = (Keys::generate(), Keys::generate(), Keys::generate());
            // The root is where its author writes; the answer, which is
            // the note opened, is on the account's own relay; and an
            // answer to that is where the answerer reads.
            let root = note_by(&asker, "a question", 100, vec![]);
            let naming = |parent: &Event, marker: &str| Tag::parse(["e", &parent.id.to_hex(), "", marker]).unwrap();
            let answer = note_by(&answerer, "an answer", 200, vec![naming(&root, "root"), Tag::public_key(asker.public_key())]);
            let reply = note_by(
                &third,
                "a reply to the answer",
                300,
                vec![naming(&root, "root"), naming(&answer, "reply"), Tag::public_key(answerer.public_key())],
            );
            relays[0].add_event(relay_list(&asker, &[&urls[1]], &[])).await.unwrap();
            relays[0].add_event(relay_list(&answerer, &[&urls[0]], &[&urls[2]])).await.unwrap();
            relays[0].add_event(answer.clone()).await.unwrap();
            relays[1].add_event(root.clone()).await.unwrap();
            relays[2].add_event(reply.clone()).await.unwrap();
            // A relay nobody names has an answer nobody will find.
            relays[3].add_event(note_by(&third, "unheard", 400, vec![naming(&root, "root")])).await.unwrap();

            let thread = service.thread(Source::Relays, answer.id, &Whereabouts::default()).await.unwrap();
            assert_eq!(sorted(ids(&thread)), sorted(vec![root.id, answer.id, reply.id]));
        });
    }

    #[test]
    fn the_configs_relays_stand_down_once_the_account_says_where_it_writes() {
        // The config's relay counts as one on the open internet here: one
        // on this machine stays in use, as in every other test.
        let reach = Reach { cap: 20, accepts: |_| true, local: |_| false };
        with_relays_reaching("own-stand-down", 2, reach, async |service, relays, urls| {
            let (me, other) = (Keys::generate(), Keys::generate());
            relays[0].add_event(relay_list(&me, &[&urls[1]], &[])).await.unwrap();
            let elsewhere = note_by(&other, "on the config's relay alone", 50, vec![]);
            relays[0].add_event(elsewhere.clone()).await.unwrap();

            // With no account, the config's relay is where things are.
            let before = note_by(&me, "before", 100, vec![]);
            assert_eq!(service.publish(&before).await, Ok(1));
            assert!(holds(&urls[0], before.id).await);
            assert_eq!(ids(&service.fetch_feed(&[other.public_key()], None, 50).await.unwrap()), [elsewhere.id]);

            // The account writes to another relay: that one is its own,
            // and the config's is neither written to nor asked.
            assert_eq!(service.use_relays_of(Some(me.public_key())).await, 1);
            assert_eq!(service.reachable_relays().await, [urls[1].clone()]);
            let after = note_by(&me, "after", 200, vec![]);
            assert_eq!(service.publish(&after).await, Ok(1));
            assert!(holds(&urls[1], after.id).await && !holds(&urls[0], after.id).await);
            let unseen = note_by(&other, "on the config's relay, later", 300, vec![]);
            relays[0].add_event(unseen.clone()).await.unwrap();
            assert!(
                service.fetch_feed(&[other.public_key()], Some(350), 50).await.unwrap().iter().all(|note| note.id != unseen.id)
            );

            // Signed out, the config's relay is back in use.
            assert_eq!(service.use_relays_of(None).await, 0);
            assert_eq!(service.reachable_relays().await, [urls[0].clone()]);
            assert!(ids(&service.fetch_feed(&[other.public_key()], None, 50).await.unwrap()).contains(&unseen.id));
        });
        // A relay list that names nowhere that answers changes nothing.
        with_relays_reaching("own-no-stand-down", 1, reach, async |service, relays, urls| {
            let me = Keys::generate();
            let dead = RelayUrl::parse("ws://127.0.0.1:9").unwrap();
            relays[0].add_event(relay_list(&me, &[&dead], &[])).await.unwrap();
            service.use_relays_of(Some(me.public_key())).await;
            assert_eq!(service.reachable_relays().await, [urls[0].clone()]);
            assert_eq!(service.publish(&note_by(&me, "still here", 100, vec![])).await, Ok(1));
        });
    }

    #[test]
    fn what_the_account_writes_goes_to_the_relays_its_list_names_and_to_those_it_is_meant_for() {
        with_relays("outbox-publish", 4, 20, async |service, relays, urls| {
            let (me, friend) = (Keys::generate(), Keys::generate());
            // The account writes to a relay the config doesn't name, and
            // to one it does; a friend reads a third.
            relays[0].add_event(relay_list(&me, &[&urls[0], &urls[1]], &[&urls[3]])).await.unwrap();
            relays[0].add_event(relay_list(&friend, &[&urls[3]], &[&urls[2]])).await.unwrap();
            assert_eq!(service.use_relays_of(Some(me.public_key())).await, 1);
            assert_eq!(service.client.relays().await.len(), 2);
            tokio::time::sleep(Duration::from_millis(300)).await;

            // A note that names nobody: the account's own relays, both.
            let alone = note_by(&me, "to nobody", 100, vec![]);
            assert_eq!(service.publish(&alone).await, Ok(2));
            assert!(holds(&urls[1], alone.id).await);
            assert!(!holds(&urls[2], alone.id).await);

            // A note that names the friend: their relay too, the one they
            // read and not the one they write to.
            let said = note_by(&me, "to a friend", 200, vec![Tag::public_key(friend.public_key())]);
            assert_eq!(service.publish(&said).await, Ok(3));
            assert!(holds(&urls[0], said.id).await && holds(&urls[1], said.id).await && holds(&urls[2], said.id).await);
            assert!(!holds(&urls[3], said.id).await);
            assert_eq!(service.seen_on(&[said.id]).get(&said.id).map(Vec::len), Some(3));

            // A contact list names the friend, and is not meant for them.
            let follows =
                EventBuilder::new(Kind::ContactList, "").tag(Tag::public_key(friend.public_key())).finalize(&me).unwrap();
            assert_eq!(service.publish(&follows).await, Ok(2));
            assert!(holds(&urls[1], follows.id).await);
            assert!(!holds(&urls[2], follows.id).await);

            // Signed out, the relays the list named are nobody's.
            assert_eq!(service.use_relays_of(None).await, 0);
            assert_eq!(service.client.relays().await.into_keys().collect::<Vec<_>>(), [urls[0].clone()]);
            let after = note_by(&me, "after", 300, vec![]);
            assert_eq!(service.publish(&after).await, Ok(1));
            assert!(!holds(&urls[1], after.id).await);
        });
    }

    #[test]
    fn the_newest_list_is_the_one_that_counts() {
        let keys = Keys::generate();
        let list =
            |at: u64| EventBuilder::new(Kind::Bookmarks, "").custom_created_at(Timestamp::from_secs(at)).finalize(&keys).unwrap();
        let (older, newer) = (list(1_000), list(2_000));
        assert_eq!(newest([older.clone(), newer.clone()]).map(|list| list.id), Some(newer.id));
        assert_eq!(newest([newer.clone(), older]).map(|list| list.id), Some(newer.id), "whichever order they arrive in");
        assert!(newest([]).is_none(), "no list at all");
    }

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    fn e(note: EventId) -> Vec<String> {
        vec!["e".to_string(), note.to_hex()]
    }

    fn fields(tags: &[&[&str]]) -> PrivateTags {
        tags.iter().map(|tag| tag.iter().map(ToString::to_string).collect()).collect()
    }

    /// The bookmark list another client would have written for the account
    /// `keys`: `tags` for anyone to read, and `content`, as it stands.
    fn list_of(keys: &Keys, tags: &PrivateTags, content: &str, at: u64) -> Event {
        EventBuilder::new(Kind::Bookmarks, content)
            .tags(tags.iter().map(|tag| Tag::parse(tag).unwrap()))
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(keys)
            .unwrap()
    }

    /// What `keys` encrypts to itself the new way: `private`, as a
    /// bookmark list's content.
    fn kept(keys: &Keys, private: &PrivateTags) -> String {
        keys.nip44_encrypt(&keys.public_key(), &bookmarks::encode(private)).unwrap()
    }

    /// What the bookmark list `list` of `keys` keeps to itself, read with
    /// the key itself, as another client would.
    fn kept_in(keys: &Keys, list: &Event) -> PrivateTags {
        bookmarks::decode(&keys.nip44_decrypt(&keys.public_key(), &list.content).unwrap()).unwrap()
    }

    fn tags_of(list: &Event) -> PrivateTags {
        list.tags.iter().map(|tag| tag.as_slice().to_vec()).collect()
    }

    /// Run `test` for an account whose key a remote signer keeps, which
    /// decrypts for it (`decrypting`) or won't, against a relay in this
    /// process, and `other_relays` beside it: the client, what signs for
    /// the account, and the account's key, with which another client would
    /// have written its lists.
    fn with_bookmarking_account<F: Future<Output = ()>>(
        name: &str,
        decrypting: bool,
        other_relays: &[&str],
        test: impl FnOnce(NostrService, Signer, Keys) -> F,
    ) {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let keys = Keys::generate();
        let approves = Approves { signing: true, decrypting };
        with_remote_signer_for(keys.clone(), None, approves, async |bunker| {
            let relay = bunker.relays().first().unwrap().to_string();
            let relays = std::iter::once(relay).chain(other_relays.iter().map(ToString::to_string)).collect();
            let service = NostrService::open(path.clone(), relays).await.unwrap();
            let signer = Signer::connected_to(bunker).await;
            test(service, signer, keys).await;
        });
        let _ = std::fs::remove_dir_all(&path);
    }

    /// A list of `kind` by `keys` with `tags` and `content`, written at `at`.
    fn a_list(keys: &Keys, kind: Kind, tags: &PrivateTags, content: &str, at: u64) -> Event {
        EventBuilder::new(kind, content)
            .tags(tags.iter().map(|tag| Tag::parse(tag).unwrap()))
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(keys)
            .unwrap()
    }

    fn url(relay: &str) -> RelayUrl {
        RelayUrl::parse(relay).unwrap()
    }

    #[test]
    fn the_accounts_relays_are_read_from_its_lists_and_changed_as_another_client_left_them() {
        with_bookmarking_account("relays-change", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let (home, open, kept_relay) = ("wss://home.example.com/", "wss://open.example.com/", "wss://kept.example.com/");
            // As Amethyst leaves them: unmarked relays, a tag of its own,
            // and search relays said to anyone and kept private.
            let advertised = a_list(&keys, Kind::RelayList, &fields(&[&["r", home], &["client", "Amethyst"]]), "", 1_000);
            let private = fields(&[&["relay", kept_relay]]);
            let public = fields(&[&["client", "Amethyst"], &["relay", open]]);
            let search = a_list(&keys, Kind::SearchRelays, &public, &kept(&keys, &private), 1_000);
            service.publish(&advertised).await.unwrap();
            service.publish(&search).await.unwrap();

            // Not asked to be read, what a list keeps to itself is left
            // alone: only what anyone can read is there.
            let closed = service.relay_lists(Source::Relays, &signer, account, None, &[]).await.unwrap();
            assert_eq!(closed.sets().of(Section::Search), [url(open)]);
            assert_eq!(*closed.of(Section::Search).kept(), Kept::Unread);

            let read = service.relay_lists(Source::Relays, &signer, account, Some(closed), &Section::LISTS).await.unwrap();
            let sets = read.sets();
            assert_eq!((sets.of(Section::Outbox), sets.of(Section::Inbox)), (&[url(home)][..], &[url(home)][..]));
            assert_eq!(sets.of(Section::Search), [url(open), url(kept_relay)]);
            assert!(matches!(read.of(Section::Search).kept(), Kept::Read(_)));
            assert!(sets.of(Section::Blocked).is_empty());
            let again = service.relay_lists(Source::Store, &signer, account, Some(read.clone()), &Section::LISTS).await;
            assert_eq!(again, Ok(read.clone()));

            // A relay the account now writes to is one of its own, and is
            // told so; the list is otherwise as it was.
            let joining = LocalRelay::builder().build();
            joining.run().await.unwrap();
            let joined = joining.url().await;
            let (outbox, inbox) = ([url(home), joined.clone()], [url(home)]);
            let changed = service.set_advertised_relays(&signer, account, &outbox, &inbox, false).await.unwrap().unwrap();
            assert!(changed.created_at > advertised.created_at && changed.verify().is_ok());
            assert_eq!(tags_of(&changed), fields(&[&["client", "Amethyst"], &["r", home], &["r", joined.as_str(), "write"]]));
            // The config's relay, and the two the list says it writes to.
            assert_eq!(service.client.relays().await.len(), 3);
            assert!(holds(&joined, changed.id).await);
            assert_eq!(service.stored_list(Kind::RelayList, account).await.map(|list| list.id), Some(changed.id));
            // The same relays again: nothing goes out.
            let same = service.set_advertised_relays(&signer, account, &outbox, &inbox, false).await.unwrap();
            assert_eq!(same.map(|list| list.id), Some(changed.id));
            // Left again: the relay is told that too, and is no longer one
            // of the account's.
            let left = service.set_advertised_relays(&signer, account, &[url(home)], &[], false).await.unwrap().unwrap();
            assert_eq!(tags_of(&left), fields(&[&["client", "Amethyst"], &["r", home, "write"]]));
            assert!(holds(&joined, left.id).await);
            assert_eq!(service.client.relays().await.len(), 2);
            joining.shutdown();

            // A new search relay goes where the list keeps them, to itself;
            // the others stay where they were.
            let new = "wss://new.example.com/";
            let wanted = [url(open), url(kept_relay), url(new)];
            let known = Some(read.of(Section::Search));
            let added = service.set_listed_relays(&signer, account, Section::Search, &wanted, false, known).await.unwrap();
            assert_eq!(added.relays(), wanted);
            let ours = added.list().unwrap().clone();
            assert!(ours.created_at > search.created_at && ours.verify().is_ok());
            assert_eq!(tags_of(&ours), public);
            assert_eq!(kept_in(&keys, &ours), fields(&[&["relay", kept_relay], &["relay", new]]));
            // The same again: nothing goes out. In another order, it does.
            let same = service.set_listed_relays(&signer, account, Section::Search, &wanted, false, Some(added.clone())).await;
            assert_eq!(same.unwrap().list().map(|list| list.id), Some(ours.id));
            let moved = [url(open), url(new), url(kept_relay)];
            let moved_list =
                service.set_listed_relays(&signer, account, Section::Search, &moved, false, Some(added)).await.unwrap();
            assert_eq!(moved_list.relays(), moved);
            // One said to anyone is taken out of what is said to anyone.
            let fewer = [url(kept_relay), url(new)];
            let fewer_list =
                service.set_listed_relays(&signer, account, Section::Search, &fewer, false, Some(moved_list)).await.unwrap();
            assert_eq!(tags_of(fewer_list.list().unwrap()), fields(&[&["client", "Amethyst"]]));
            let again = service.relay_lists(Source::Relays, &signer, account, None, &[Section::Search]).await.unwrap();
            assert_eq!(again.sets().of(Section::Search), fewer);
        });
    }

    #[test]
    fn a_first_relay_list_is_only_written_after_saying_so() {
        with_bookmarking_account("relays-first", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let none = service.relay_lists(Source::Relays, &signer, account, None, &Section::LISTS).await.unwrap();
            assert_eq!(none.sets(), RelayLists::default().sets());
            assert_eq!(none.advertised, None);

            // The relay it will write to is there to answer: a first list
            // is not written while one of the account's relays doesn't.
            let writing = LocalRelay::builder().build();
            writing.run().await.unwrap();
            let (outbox, search) = ([writing.url().await], [url("wss://search.example.com")]);
            // Nothing to write is nothing to refuse.
            assert_eq!(service.set_advertised_relays(&signer, account, &[], &[], false).await, Ok(None));
            assert_eq!(service.set_advertised_relays(&signer, account, &outbox, &[], false).await, Err(ListError::NoList));
            let refused = service.set_listed_relays(&signer, account, Section::Search, &search, false, None).await;
            assert_eq!(refused, Err(ListError::NoList));
            assert_eq!(service.stored_list(Kind::RelayList, account).await, None);
            assert_eq!(service.stored_list(Kind::SearchRelays, account).await, None);

            let first = service.set_advertised_relays(&signer, account, &outbox, &[], true).await.unwrap().unwrap();
            assert_eq!(tags_of(&first), fields(&[&["r", outbox[0].as_str(), "write"]]));
            // A first search relay list says its relays to anyone, and a
            // first list of blocked relays keeps them to itself.
            let first = service.set_listed_relays(&signer, account, Section::Search, &search, true, None).await.unwrap();
            let list = first.list().unwrap();
            assert_eq!(tags_of(list), fields(&[&["relay", search[0].as_str()]]));
            assert_eq!(list.content, "");
            let blocked = [url("wss://blocked.example.com")];
            let first = service.set_listed_relays(&signer, account, Section::Blocked, &blocked, true, None).await.unwrap();
            let list = first.list().unwrap();
            assert_eq!((list.kind.as_u16(), list.tags.len()), (10006, 0));
            assert_eq!(kept_in(&keys, list), fields(&[&["relay", blocked[0].as_str()]]));
            let read = service.relay_lists(Source::Relays, &signer, account, None, &Section::LISTS).await.unwrap();
            assert_eq!(read.sets().of(Section::Blocked), blocked);
            writing.shutdown();
        });
    }

    #[test]
    fn the_accounts_media_servers_are_its_list_and_a_first_one_is_only_written_after_saying_so() {
        with_bookmarking_account("media-servers", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let server = |name: &str| Url::parse(name).unwrap();
            let (first, second) = (server("https://one.example.com/"), server("https://two.example.com/"));
            assert_eq!(service.media_server_list(Source::Relays, account).await, Ok(None));

            let wanted = [first.clone()];
            assert_eq!(service.set_media_servers(&signer, account, &wanted, false).await, Err(ListError::NoList));
            assert_eq!(service.stored_list(Kind::BlossomServerList, account).await, None);
            let written = service.set_media_servers(&signer, account, &wanted, true).await.unwrap().unwrap();
            assert_eq!(tags_of(&written), fields(&[&["server", "https://one.example.com"]]));
            assert_eq!(written.pubkey, account);
            // The store has it at once, and the relays when asked.
            assert_eq!(service.media_server_list(Source::Store, account).await, Ok(Some(written.clone())));
            assert_eq!(service.media_server_list(Source::Relays, account).await, Ok(Some(written.clone())));
            // What it says already is not written again.
            assert_eq!(service.set_media_servers(&signer, account, &wanted, false).await, Ok(Some(written.clone())));

            // Another put first: the list that replaces it is the newer.
            let changed =
                service.set_media_servers(&signer, account, &[second.clone(), first.clone()], false).await.unwrap().unwrap();
            assert_eq!(blossom::servers(Some(&changed)), [second, first]);
            assert!(changed.created_at > written.created_at);
            assert_eq!(service.media_server_list(Source::Relays, account).await, Ok(Some(changed)));
        });
    }

    #[test]
    fn listed_relays_that_cannot_be_read_are_not_written_over() {
        with_bookmarking_account("relays-closed", false, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let (open, kept_relay) = ("wss://open.example.com/", "wss://kept.example.com/");
            let private = fields(&[&["relay", kept_relay]]);
            let search = a_list(&keys, Kind::SearchRelays, &fields(&[&["relay", open]]), &kept(&keys, &private), 1_000);
            service.publish(&search).await.unwrap();

            // What anyone can read is read, and that the rest wasn't is said.
            let read = service.relay_lists(Source::Relays, &signer, account, None, &Section::LISTS).await.unwrap();
            assert_eq!(read.sets().of(Section::Search), [url(open)]);
            assert!(matches!(read.of(Section::Search).kept(), Kept::Unreadable(_)));

            let wanted = [url(open), url("wss://new.example.com")];
            let refused = service.set_listed_relays(&signer, account, Section::Search, &wanted, true, None).await;
            assert!(matches!(refused, Err(ListError::Unreadable(_))), "{refused:?}");
            assert_eq!(service.stored_list(Kind::SearchRelays, account).await.map(|list| list.id), Some(search.id));
            // The relay list keeps nothing to itself, and is changed all the same.
            let outbox = [url("wss://out.example.com")];
            assert!(service.set_advertised_relays(&signer, account, &outbox, &[], true).await.unwrap().is_some());
        });
    }

    #[test]
    fn what_the_accounts_relays_do_not_have_is_looked_up_where_its_list_says() {
        with_relays("lookup", 3, 20, async |service, relays, urls| {
            let far = Keys::generate();
            let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"Far"}"#).finalize(&far).unwrap();
            let there = note_by(&far, "on theirs", 100, vec![]);
            // Their relay list and their profile are on the indexer alone,
            // and their notes where the list says.
            relays[1].add_event(relay_list(&far, &[&urls[2]], &[])).await.unwrap();
            relays[1].add_event(profile).await.unwrap();
            relays[2].add_event(there.clone()).await.unwrap();

            // With nowhere to look things up, they are not found.
            assert!(service.fetch_feed(&[far.public_key()], None, 50).await.unwrap().is_empty());
            assert!(service.fetch_profiles_where_written(vec![far.public_key()]).await.is_empty());

            // With the indexer, they are, and asked for again at once.
            service.use_listed(vec![urls[1].clone()], Vec::new());
            assert_eq!(ids(&service.fetch_feed(&[far.public_key()], None, 50).await.unwrap()), [there.id]);
            let found = service.fetch_profiles_where_written(vec![far.public_key()]).await;
            assert_eq!(found.iter().map(|(_, profile)| profile.name.as_deref()).collect::<Vec<_>>(), [Some("Far")]);
        });
    }

    #[test]
    fn a_blocked_relay_is_never_connected_to_on_anyones_word() {
        with_relays("blocked", 3, 20, async |service, relays, urls| {
            let far = Keys::generate();
            let (blocked_note, other_note) =
                (note_by(&far, "on the blocked one", 100, vec![]), note_by(&far, "elsewhere", 200, vec![]));
            relays[0].add_event(relay_list(&far, &[&urls[1], &urls[2]], &[])).await.unwrap();
            relays[1].add_event(blocked_note.clone()).await.unwrap();
            relays[2].add_event(other_note.clone()).await.unwrap();
            service.use_listed(Vec::new(), vec![urls[1].clone()]);

            // Not for someone's notes, not for a note said to be there,
            // and not to search.
            assert_eq!(ids(&service.fetch_feed(&[far.public_key()], None, 50).await.unwrap()), [other_note.id]);
            let said = vec![(blocked_note.id, Whereabouts { author: None, relay: Some(urls[1].clone()) })];
            assert!(service.notes_listed(said).await.is_empty());
            let searched = service.search(&[urls[1].clone()], "blocked", SearchFor::Notes).await;
            assert!(matches!(searched, Err(FetchError::NoRelayReachable)));
            assert_eq!(service.others.relays().await.into_keys().collect::<Vec<_>>(), [urls[2].clone()]);
        });
    }

    #[test]
    fn a_draft_is_kept_on_the_relay_of_this_machine_and_on_no_other() {
        with_bookmarking_account("drafts", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let reply = Target::Reply(id(1));
            // With nobody signed in, the relay of this machine takes nothing.
            assert!(service.keep_draft(&signer, account, Target::Note, "too soon", &[]).await.is_err());
            service.use_relays_of(Some(account)).await;

            service.keep_draft(&signer, account, Target::Note, "half a thought", &[]).await.unwrap();
            let said = Tag::parse(["imeta", "url https://cdn.example.com/abc.png", "m image/png"]).unwrap();
            service.keep_draft(&signer, account, reply, "an answer", std::slice::from_ref(&said)).await.unwrap();
            // Written again within the second, it replaces what was there.
            service.keep_draft(&signer, account, Target::Note, "a whole thought", &[]).await.unwrap();
            let kept = service.kept_drafts(&signer, account).await.unwrap();
            assert_eq!(kept.len(), 2, "{kept:?}");
            assert!(kept.contains(&(Target::Note, "a whole thought".to_string(), Vec::new())));
            // With what it says of the file it links to.
            assert!(kept.contains(&(reply, "an answer".to_string(), vec![said.clone()])));

            // They are in the store, where nobody else can read them, and
            // on none of the account's other relays.
            let wraps = service.events(Source::Store, vec![Filter::new().kind(DRAFT_KIND).author(account)]).await.unwrap();
            assert_eq!(wraps.len(), 2);
            let elsewhere = service.client.relays().await.into_keys().next().unwrap();
            for wrap in &wraps {
                assert!(wrap.verify().is_ok() && !wrap.content.contains("thought") && !wrap.content.contains("answer"));
                assert!(!holds(&elsewhere, wrap.id).await);
            }

            // Done with, it is kept as that, and is no draft any more.
            service.keep_draft(&signer, account, Target::Note, "  ", &[]).await.unwrap();
            assert_eq!(service.kept_drafts(&signer, account).await.unwrap(), [(reply, "an answer".to_string(), vec![said])]);
            // Signed out, the relay takes nothing again.
            service.use_relays_of(None).await;
            assert!(service.keep_draft(&signer, account, reply, "later", &[]).await.is_err());
        });
    }

    #[test]
    fn a_bookmark_is_one_change_to_the_list_another_client_wrote() {
        with_bookmarking_account("bookmark-change", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let public = fields(&[&["e", &id(1).to_hex()], &["t", "nostr"], &["title", "Mine"]]);
            let private = fields(&[&["e", &id(2).to_hex(), "wss://relay.example.com/"], &["a", "30023:abc:post"]]);
            let theirs = list_of(&keys, &public, &kept(&keys, &private), 1_000);
            service.publish(&theirs).await.unwrap();

            // Read: what only the account can see. What anyone can is no
            // bookmark here.
            let read = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert_eq!(read.list().map(|list| list.id), Some(theirs.id));
            assert!(read.has(&id(2)) && !read.has(&id(1)) && !read.has(&id(3)));
            assert_eq!(service.bookmarks(Source::Store, &signer, account, None).await.as_ref(), Ok(&read));

            // A note that is on the relay is bookmarked with where it is.
            let found = note("worth finding again", 500, &[]);
            service.publish(&found).await.unwrap();
            let added = service.set_bookmarked(&signer, account, found.id, true, false, Some(read)).await.unwrap();
            assert!(added.has(&found.id) && added.has(&id(2)));
            let ours = added.list().unwrap().clone();
            assert_eq!(service.stored_list(Kind::Bookmarks, account).await.map(|list| list.id), Some(ours.id));
            assert_eq!((ours.kind, ours.pubkey), (Kind::Bookmarks, account));
            assert!(ours.created_at > theirs.created_at && ours.verify().is_ok());
            assert_eq!(tags_of(&ours), public, "what anyone can read is as it was");
            let hinted = kept_in(&keys, &ours);
            assert_eq!(hinted[..2], private[..], "and so is what was kept private");
            assert_eq!(hinted[2][..2], e(found.id)[..]);
            assert!(hinted[2][2].starts_with("ws://"), "{:?}", hinted[2]);
            assert_eq!(hinted.len(), 3);

            // Bookmarked already: nothing goes out.
            let again = service.set_bookmarked(&signer, account, found.id, true, false, Some(added.clone())).await.unwrap();
            assert_eq!(again.list().map(|list| list.id), Some(ours.id));

            // A note that anyone can see in the list has no bookmark here
            // to take off: nothing goes out, and it stays where it is.
            let same = service.set_bookmarked(&signer, account, id(1), false, false, Some(again)).await.unwrap();
            assert_eq!(same.list().map(|list| list.id), Some(ours.id));
            // Bookmarked here, it is kept private as any other note is.
            let both = service.set_bookmarked(&signer, account, id(1), true, false, Some(same)).await.unwrap();
            assert!(both.has(&id(1)));
            assert_eq!(tags_of(both.list().unwrap()), public);
            assert_eq!(kept_in(&keys, both.list().unwrap()).last(), Some(&e(id(1))));

            // Taken off a note kept private, read anew from the relays.
            let least = service.set_bookmarked(&signer, account, id(2), false, false, None).await.unwrap();
            assert!(!least.has(&id(2)) && least.has(&found.id) && least.has(&id(1)));
            let list = least.list().unwrap();
            assert_eq!(tags_of(list), public, "what anyone can read is still as it was");
            assert_eq!(kept_in(&keys, list), [private[1].clone(), hinted[2].clone(), e(id(1))]);
            // The relay has the last one, and no other.
            let on_relay = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert_eq!(on_relay.list().map(|list| list.id), Some(list.id));
        });
    }

    #[test]
    fn the_first_bookmark_list_is_only_written_when_the_account_says_so() {
        with_bookmarking_account("bookmark-first", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let none = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert_eq!(none, Bookmarks::default());

            let refused = service.set_bookmarked(&signer, account, id(1), true, false, Some(none.clone())).await;
            assert_eq!(refused, Err(ListError::NoList));
            assert!(service.stored_list(Kind::Bookmarks, account).await.is_none(), "nothing was written");
            // Taking off what isn't there needs no list either.
            assert_eq!(service.set_bookmarked(&signer, account, id(1), false, false, None).await, Ok(none));

            let first = service.set_bookmarked(&signer, account, id(1), true, true, None).await.unwrap();
            let list = first.list().unwrap();
            assert!(first.has(&id(1)) && list.tags.is_empty());
            assert_eq!(kept_in(&keys, list), [e(id(1))], "as another client reads it");

            // With nothing left to keep, the list has no content.
            let emptied = service.set_bookmarked(&signer, account, id(1), false, false, Some(first)).await.unwrap();
            let list = emptied.list().unwrap();
            assert!(!emptied.has(&id(1)) && list.tags.is_empty() && list.content.is_empty());
            // Which is read without decrypting anything, and added to.
            let read = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert_eq!(read, emptied);
            let refilled = service.set_bookmarked(&signer, account, id(2), true, false, None).await.unwrap();
            assert_eq!(kept_in(&keys, refilled.list().unwrap()), [e(id(2))]);
        });
    }

    #[test]
    fn no_first_list_is_written_while_a_relay_that_may_have_one_does_not_answer() {
        // Nothing listens at the second relay.
        with_bookmarking_account("bookmark-incomplete", true, &["ws://127.0.0.1:9"], async |service, signer, keys| {
            let account = keys.public_key();
            // To read, the relay that answers is enough.
            let none = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert_eq!(none, Bookmarks::default());

            // To write a first list, it isn't, whatever the account says.
            for first_list in [false, true] {
                let unsure = service.set_bookmarked(&signer, account, id(1), true, first_list, None).await;
                assert_eq!(unsure, Err(ListError::Fetch(FetchError::Incomplete)));
            }
            assert!(service.stored_list(Kind::Bookmarks, account).await.is_none(), "nothing was written");
            let person = Keys::generate().public_key();
            let unsure = service.set_following(&signer, account, person, true, true).await;
            assert_eq!(unsure, Err(FollowError::Fetch(FetchError::Incomplete)), "nor a first contact list");

            // A list that is found is changed, wherever it was found.
            service.publish(&list_of(&keys, &PrivateTags::new(), "", 1_000)).await.unwrap();
            let added = service.set_bookmarked(&signer, account, id(1), true, false, None).await.unwrap();
            assert_eq!(kept_in(&keys, added.list().unwrap()), [e(id(1))]);
        });
    }

    #[test]
    fn a_list_from_an_older_client_is_read_and_written_back_the_new_way() {
        with_bookmarking_account("bookmark-old", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let private = fields(&[&["e", &id(2).to_hex()]]);
            let old_way = keys.nip04_encrypt(&account, &bookmarks::encode(&private)).unwrap();
            service.publish(&list_of(&keys, &PrivateTags::new(), &old_way, 1_000)).await.unwrap();

            let read = service.bookmarks(Source::Relays, &signer, account, None).await.unwrap();
            assert!(read.has(&id(2)));
            let added = service.set_bookmarked(&signer, account, id(3), true, false, Some(read)).await.unwrap();
            let list = added.list().unwrap();
            assert!(!list.content.contains("?iv="), "{}", list.content);
            assert_eq!(kept_in(&keys, list), [e(id(2)), e(id(3))]);
        });
    }

    #[test]
    fn a_list_that_cannot_be_read_is_not_written_over() {
        // The remote signer signs, and decrypts nothing.
        with_bookmarking_account("bookmark-unreadable", false, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let private = fields(&[&["e", &id(2).to_hex()]]);
            let public = fields(&[&["e", &id(1).to_hex()]]);
            let theirs = list_of(&keys, &public, &kept(&keys, &private), 1_000);
            service.publish(&theirs).await.unwrap();

            let unread = service.bookmarks(Source::Relays, &signer, account, None).await;
            assert!(matches!(unread, Err(ListError::Unreadable(SignError::Remote(_)))), "{unread:?}");
            for bookmark in [true, false] {
                let unchanged = service.set_bookmarked(&signer, account, id(1), bookmark, true, None).await;
                assert!(matches!(unchanged, Err(ListError::Unreadable(_))), "{unchanged:?}");
            }
            assert_eq!(service.stored_list(Kind::Bookmarks, account).await.map(|list| list.id), Some(theirs.id));

            // A list read before is not decrypted again: nothing is asked
            // of the signer but to encrypt and to sign.
            let known = Bookmarks::new(Some(theirs.clone()), private.clone());
            assert_eq!(service.bookmarks(Source::Relays, &signer, account, Some(known.clone())).await.as_ref(), Ok(&known));
            let added = service.set_bookmarked(&signer, account, id(3), true, false, Some(known)).await.unwrap();
            assert_eq!(kept_in(&keys, added.list().unwrap()), [e(id(2)), e(id(3))]);
        });
    }

    #[test]
    fn what_decrypts_to_something_else_than_tags_is_not_a_list_to_change() {
        with_bookmarking_account("bookmark-not-tags", true, &[], async |service, signer, keys| {
            let account = keys.public_key();
            let words = keys.nip44_encrypt(&account, "secret words").unwrap();
            let theirs = list_of(&keys, &PrivateTags::new(), &words, 1_000);
            service.publish(&theirs).await.unwrap();

            let unread = service.bookmarks(Source::Relays, &signer, account, None).await;
            match unread {
                Err(ListError::Unreadable(SignError::Cipher(why))) => assert!(!why.contains("secret"), "{why}"),
                other => panic!("unreadable, not {other:?}"),
            }
            let unchanged = service.set_bookmarked(&signer, account, id(1), true, false, None).await;
            assert!(matches!(unchanged, Err(ListError::Unreadable(_))), "{unchanged:?}");
            assert_eq!(service.stored_list(Kind::Bookmarks, account).await.map(|list| list.id), Some(theirs.id));
        });
    }

    #[test]
    fn a_note_nobody_has_has_no_thread() {
        let (ids, _) = thread_in_store("nothing", &[note("something else", 100, &[])], EventId::from_byte_array([9; 32]));
        assert!(ids.is_empty());
    }

    #[test]
    fn a_list_of_authors_is_asked_about_in_one_filter_for_every_so_many_of_them() {
        assert_eq!(feed_filters(&people(3), None, 50).len(), 1);
        assert_eq!(feed_filters(&people(AUTHORS_PER_FILTER + 1), None, 50).len(), 2);
        assert!(feed_filters(&[], None, 50).is_empty(), "nobody followed, nothing to ask");
    }

    #[test]
    fn a_feed_filter_names_its_authors_its_page_and_only_notes_and_comments() {
        let authors = people(2);
        let filters = feed_filters(&authors, Some(1_000), 50);
        let filter = &filters[0];
        assert_eq!(filter.limit, Some(50));
        assert_eq!(filter.until, Some(Timestamp::from_secs(1_000)));
        assert_eq!(
            filter.kinds.as_ref().map(|kinds| kinds.iter().copied().collect::<Vec<_>>()),
            Some(vec![Kind::TextNote, Kind::Comment])
        );
        let named = filter.authors.as_ref().unwrap();
        assert!(authors.iter().all(|author| named.contains(author)) && named.len() == 2);
    }

    #[test]
    fn an_author_who_published_twice_is_known_by_the_later_profile() {
        let keys = Keys::generate();
        let profile = |name: &str, at: u64| {
            EventBuilder::new(Kind::Metadata, format!(r#"{{"name":"{name}"}}"#))
                .custom_created_at(Timestamp::from_secs(at))
                .finalize(&keys)
                .unwrap()
        };

        for events in [[profile("old", 100), profile("new", 200)], [profile("new", 200), profile("old", 100)]] {
            let profiles = newest_profiles(events);
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].0, keys.public_key());
            assert_eq!(profiles[0].1.name.as_deref(), Some("new"));
        }
    }
}

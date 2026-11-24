// SPDX-License-Identifier: MIT

//! Application messages for Avestruz.
//!
//! This module defines all the messages that can be sent to update the application state.
//!
//! A message is the only thing that changes the model (see
//! [`crate::guide::architecture`]). There are three kinds of them, told
//! apart by where they come from:
//!
//! ```mermaid
//! flowchart LR
//!     reader(["The reader"]) -->|"pressed, typed, scrolled"| did["What was done: OpenThread, Like, Publish"]
//!     timer(["A timer, the config watch, a view that needs a picture"]) --> time["What is due: CheckForNewNotes, SweepStore, LoadImage"]
//!     task(["A task that finished"]) --> answer["What came back: FeedFetched, ThreadLoaded, Published"]
//!     did --> update["app: update"]
//!     time --> update
//!     answer --> update
//! ```
//!
//! A message of the third kind says what it is an answer to (a list's
//! id, a search's serial number, an account's public key), so that its
//! handler can tell an answer that came too late from one that is still
//! wanted.
//!
//! Messages are `Debug`, and may end up in a log: one that would carry
//! what is typed in the sign-in field carries a [`SecretInput`] instead,
//! which prints nothing.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use cosmic::iced::window::Id;
use cosmic::widget::{segmented_button, text_editor};
use nostr_sdk::prelude::{Event, EventId, PublicKey, RelayUrl, Tag, Url};

use crate::config::{Config, LogLevel};
use crate::emoji::Group;
use crate::handlers::media::Pasted;
use crate::images::{ImageRequest, LoadedImage};
use crate::media::MediaError;
use crate::nostr::account::{Account, SignError, SignInError};
use crate::nostr::blossom::{Attachment, UploadError};
use crate::nostr::bookmarks::Bookmarks;
use crate::nostr::client::{FetchError, FollowError, ListError, NostrService, Notified, PublishError, RelayUse, Source};
use crate::nostr::compose::Target;
use crate::nostr::engagement::{Doer, Left, Mark, Reaction, Standing};
use crate::nostr::feed::FeedPage;
use crate::nostr::models::{Note, Profile};
use crate::nostr::outbox::Whereabouts;
use crate::nostr::relay_info::RelayInfo;
use crate::nostr::relay_lists::{ListedRelays, RelayLists, Section};
use crate::nostr::search::{Found, SearchFor};
use crate::nostr::zap::{Invoice, ZapError};
use crate::previews::Linked;
use crate::screen::Screen;
use crate::state::Step;
use crate::video::Speed;
use crate::youtube::FindError;

/// Text from the sign-in field, which may be a secret key.
///
/// Messages are `Debug`, and whatever is `Debug` ends up in a log sooner or
/// later. This never prints what it holds.
#[derive(Clone)]
pub struct SecretInput(pub String);

impl fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretInput(…)")
    }
}

/// Why what was written, or a repost, did not go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishFailure {
    /// The note it is about — replied to, reposted — is not in the event
    /// store any more, and the relays did not send it again.
    NoteGone,
    /// It could not be signed, or the relays did not take it.
    Failed(PublishError),
}

/// Why a file was not attached to what is being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachFailure {
    /// It could not be read from where it is.
    Read(String),
    /// It is not something to send (see [`crate::media`]).
    Media(MediaError),
    /// The authorization to send it could not be signed.
    Sign(SignError),
    /// No media server took it: each that was tried, and why not.
    Refused(Vec<(Url, UploadError)>),
}

/// Application messages for state updates
#[derive(Debug, Clone)]
pub enum Message {
    // Popup management
    /// Toggle the popup window visibility
    TogglePopup,
    /// `j`, `k` or Home was pressed where no text is being typed: on to the next note of the feed, back to the one before, or to its very top
    StepNote(Step),
    /// `l` or Enter was pressed where no widget had a use for it: open the row the keyboard is at
    OpenMarked,
    /// `y` was pressed there: copy a share link to the note the keyboard is at
    ShareMarked,
    /// Space was pressed there: play the video the note the keyboard is at links to first, or hold it; or follow its link, if that is first
    PlayMarked,
    /// `r` was pressed there: reply to the note the keyboard is at
    ReplyMarked,
    /// `R` was pressed there: offer the ways to pass on the note the keyboard is at
    RepostMarked,
    /// An arrow up or down was pressed there: to the way before, or the next, of the ways a menu offers
    StepChoice(Step),
    /// `z` was pressed there: zap the note the keyboard is at
    ZapMarked,
    /// `b` was pressed there: bookmark the note the keyboard is at, or take its bookmark off
    BookmarkMarked,
    /// `B` was pressed there: show the bookmarks
    GoToBookmarks,
    /// `?` was pressed there: show what the keys do
    ShowKeys,
    /// `.` was pressed there: show the notes that wait for the list on show
    ShowWaiting,
    /// `h` was pressed there: go back, as the arrow at the top of the view does
    GoBack,
    /// Backspace was pressed there, or is `held` down: go back; out of the composer too, which `h` is not
    Backspace { held: bool },
    /// Popup window was closed
    PopupClosed(Id),
    /// The popup lost the keyboard: something outside it was clicked
    PopupUnfocused(Id),
    /// The button in the panel is gone: there is nothing left to open the popup with
    ButtonGone,
    /// The screen the popup opens on: how tall it is, and how many of its pixels a point is, if that could be found out
    ScreenMeasured(Option<Screen>),

    // Subscription/background events
    /// Configuration was updated
    UpdateConfig(Config),
    /// The event store is open and relays are connecting (or the store failed to open)
    NostrReady(Result<NostrService, String>),

    // Signing in
    /// The user edited the sign-in field
    LoginInputChanged(SecretInput),
    /// Show or hide what is typed in the sign-in field
    ToggleLoginInputVisibility,
    /// Sign in with the key in the sign-in field
    SubmitLogin,
    /// Call off a sign-in that is waiting on a remote signer
    CancelLogin,
    /// Copy the key a remote signer knows this client by, to authorize it there
    CopyClientKey,
    /// Signing in finished: the account, or why nobody was signed in
    LoginComplete(Result<Account, SignInError>),
    /// The account named in the config was looked up in the keyring at startup
    AccountRestored(Result<Account, String>),
    /// Sign the account out
    SignOut,
    /// The account's secret key left the keyring (or the keyring refused)
    SignedOut(Result<(), String>),

    // The feed, and the lists of notes on profiles
    /// The notes of a list already in the event store were read. With the
    /// list's id.
    FeedCached(u64, Result<Vec<Note>, String>),
    /// Ask the relays for the newest notes of the feed
    RefreshFeed,
    /// A page of the list of notes `list` arrived from the relays: the newest, or the
    /// one before the note written at `until`
    FeedFetched { list: u64, page: FeedPage, until: Option<u64>, result: Result<Vec<Note>, FetchError> },
    /// It is time to look for new notes for the feed
    CheckForNewNotes,
    /// Show the notes that wait for the list with this id, and go to its top
    ShowNewNotes(u64),
    /// The list with this id was scrolled near its end, or asked to look further back:
    /// fetch the page before its oldest note
    LoadOlderNotes(u64),
    /// Which notes the lists show was chosen: with replies, or without
    SelectFeedMode(segmented_button::Entity),
    /// Who the account follows, as far as one source knows: `None` if it has
    /// no contact list of theirs. With the id of the feed it was asked for.
    ContactsLoaded { feed: u64, source: Source, result: Result<Option<Vec<PublicKey>>, FetchError> },
    /// Profiles of note authors were read from the event store; the rest are asked of the relays
    ProfilesCached { requested: Vec<PublicKey>, found: Vec<(PublicKey, Profile)> },
    /// Profiles arrived, as they stand now: from the relays, or read from the event store
    ProfilesFetched(Vec<(PublicKey, Profile)>),

    // Threads
    /// Show the thread of this note, which may be where the second says
    /// if it is not at hand
    OpenThread(EventId, Whereabouts),
    /// The conversation around a thread's note, as far as one source knows it
    ThreadLoaded { focused: EventId, source: Source, result: Result<Vec<Note>, FetchError> },

    // Profiles
    /// Show this person's profile
    OpenProfile(PublicKey),
    /// Copy this person's `npub` to the clipboard
    CopyNpub(PublicKey),
    /// The domain in the name this person gives as theirs was asked whether it is
    Nip05Checked { person: PublicKey, name: String, verified: bool },
    /// Follow this person, or stop following them. `first_list` when the account
    /// agreed to a first contact list being written, having none
    Follow { person: PublicKey, follow: bool, first_list: bool },
    /// The contact list of `account` changed for this person, or didn't: who it follows now
    FollowChanged { account: PublicKey, person: PublicKey, result: Result<Vec<PublicKey>, FollowError> },

    // Navigation
    /// Close the page on show, a thread or a profile: back to what it was opened from
    ClosePage,
    /// Show what the cog above the feed offers, or put it away
    ToggleMenu,
    /// Put away what the cog above the feed offers
    CloseMenu,
    /// Show the settings
    OpenSettings,
    /// Leave the settings
    CloseSettings,
    /// Show what the keys do, in place of the list on show
    OpenKeys,
    /// Back from what the keys do to the list that was on show
    CloseKeys,

    // Search
    /// Show a page to search for people and notes on
    OpenSearch,
    /// What is typed on this search page changed
    SearchTyped(u64, String),
    /// Search for what is typed on this search page, or open what it names
    Search(u64),
    /// Show the people this search page found, or the notes
    SearchShow(u64, SearchFor),
    /// What this search page's search, the one so numbered, found of people or of notes; or that it didn't
    SearchFound { page: u64, serial: u64, what: SearchFor, result: Result<Found, FetchError> },

    /// The domain of the name typed on this search page said whose it is, or didn't
    SearchNamed { page: u64, name: String, person: Option<PublicKey> },
    /// What the relay this search page shows says of itself, for the search so numbered
    RelayInfoLoaded { page: u64, serial: u64, info: Option<RelayInfo> },

    // Relays
    /// Show the account's relays
    OpenRelays,
    /// Leave the account's relays
    CloseRelays,
    /// The relay lists of `account`, as far as one source knows
    RelayListsLoaded { account: PublicKey, source: Source, result: Result<RelayLists, FetchError> },
    /// How the relays this is connected to are doing
    RelayUseLoaded(HashMap<RelayUrl, RelayUse>),
    /// What is typed where a relay is added to this set changed
    RelayTyped(Section, String),
    /// Add the relay typed to this set
    AddRelay(Section),
    /// Take this relay out of this set
    RemoveRelay(Section, RelayUrl),
    /// Move this relay a place up this set (`true`), or down
    ShiftRelay(Section, RelayUrl, bool),
    /// Write the relays as the page has them to the account's lists
    SaveRelays,
    /// The relay lists of `account` were changed, or weren't: each as it is now, of those there was a change to
    RelaysSaved {
        account: PublicKey,
        advertised: Option<Result<Option<Event>, ListError>>,
        listed: Vec<(Section, Result<ListedRelays, ListError>)>,
    },
    /// A relay on the relays page said what it is, or didn't
    RelaySaid { relay: RelayUrl, info: Option<RelayInfo> },

    // Media servers, and what is attached to a note
    /// Show the account's media servers
    OpenMedia,
    /// Leave the account's media servers
    CloseMedia,
    /// The media server list of `account`, as far as one source knows
    MediaServersLoaded { account: PublicKey, source: Source, result: Result<Option<Event>, FetchError> },
    /// What is typed where a media server is added changed
    MediaServerTyped(String),
    /// Add the media server typed
    AddMediaServer,
    /// Take this media server out
    RemoveMediaServer(Url),
    /// Move this media server a place up (`true`), or down
    ShiftMediaServer(Url, bool),
    /// Write the media servers as the page has them to the account's list
    SaveMediaServers,
    /// The media server list of `account` was changed, or wasn't: as it is now
    MediaServersSaved { account: PublicKey, result: Result<Option<Event>, ListError> },
    /// Pick what is to be attached to what is being written
    AttachMedia,
    /// The file chooser is done: with these files, with none, or it could not be opened
    MediaPicked(Result<Vec<PathBuf>, String>),
    /// Ctrl+V was pressed in the composer
    ComposePaste,
    /// What was on the clipboard when it was
    ComposePasted(Option<Pasted>),
    /// The file so numbered is on a media server, or isn't
    MediaUploaded { id: u64, result: Result<Attachment, AttachFailure> },
    /// Call off the file so numbered, which is on its way
    CancelUpload(u64),
    /// Time to show how far the files on their way have got
    UploadTick,

    /// Log this much from now on
    SetLogLevel(LogLevel),
    /// Set the text of notes in this size from now on
    SetNoteTextSize(u16),
    /// Keep other people's notes in the event store for this many days from now on; `0` is for good
    SetKeepDays(u32),
    /// It is time to see whether the event store is to let go of what is old
    SweepStore,

    // Composing
    /// The draft of a new note was read from where it is kept between runs
    DraftLoaded(String),
    /// The relay of this machine took this text as the draft of `account` for this target, or didn't
    DraftKept { account: PublicKey, target: Target, text: String, result: Result<(), String> },
    /// The drafts of `account` that the relay of this machine keeps, each with what it says of the files it links to
    KeptDraftsLoaded { account: PublicKey, result: Result<Vec<(Target, String, Vec<Tag>)>, String> },
    /// Write a new note
    ComposeNote,
    /// Write a reply to this note
    ComposeReply(EventId),
    /// Write a note that quotes this note
    ComposeQuote(EventId),
    /// Something was typed, selected or moved in the composer
    ComposeEdited(text_editor::Action),
    /// Leave the composer, keeping what was written
    CloseComposer,
    /// Backspace was pressed in the composer's text with nothing left in it: leave the composer
    ComposerRubbedOut,
    /// Leave the composer, and keep nothing of what was written
    DiscardDraft,
    /// Not leaving the composer after all: go on writing
    ResumeComposer,
    /// Sign what was written and send it to the relays
    Publish,
    /// What was written to `target` went out as this note, or didn't
    Published { target: Target, result: Result<Note, PublishFailure> },

    // What was made of notes
    /// How many answered, reposted, liked and zapped these notes, as far as one source knows
    /// and for the account whose likes and reposts were told apart; and which relays they were seen on
    EngagementLoaded { source: Source, account: Option<PublicKey>, result: Result<Standing, FetchError> },
    /// Offer what to react to this note with, or put the offer away if it is showing
    ToggleReactionMenu(EventId),
    /// Put that offer away
    CloseReactionMenu,
    /// Show every emoji there is to react with, in place of the few offered first
    OpenEmojiPicker,
    /// What to find an emoji by was typed
    EmojiSearchTyped(String),
    /// Show this group of emoji to pick from
    EmojiGroup(Group),
    /// React to this note: with a like, or with an emoji
    React(EventId, Reaction),
    /// Show what can be done to pass this note on, repost it or quote it, or put that away
    ToggleRepostMenu(EventId),
    /// Put away what can be done to pass a note on
    CloseRepostMenu,
    /// Repost this note
    Repost(EventId),
    /// Show who left this on this note, under the count of them, or put that away if it shows
    ShowWho(EventId, Left),
    /// Who left this on this note was read from the store, or could not be
    WhoLoaded { note: EventId, what: Left, result: Result<Vec<Doer>, FetchError> },
    /// Put away who left something on a note
    CloseWho,
    /// The account's like or repost of this note went out, or didn't
    Marked { note: EventId, mark: Mark, result: Result<(), PublishFailure> },

    // Bookmarks
    /// What `account` has bookmarked, as far as one source knows
    BookmarksLoaded { account: PublicKey, source: Source, result: Result<Bookmarks, ListError> },
    /// Bookmark this note, or take its bookmark off if it has one
    Bookmark(EventId),
    /// The bookmark list of `account` changed for this note, or didn't: what it has bookmarked now
    Bookmarked { account: PublicKey, note: EventId, result: Result<Bookmarks, ListError> },
    /// Show the notes the account has bookmarked
    OpenBookmarks,
    /// The bookmarks page with this id was scrolled near the end of the notes it shows:
    /// look up the next of them
    LoadBookmarkedNotes(u64),
    /// The notes bookmarked `from`th to `to`th were looked up for the bookmarks page with this id,
    /// as it had them the `taken`th time it took them from the list: those that were found
    BookmarkedNotesLoaded { page: u64, taken: u64, from: usize, to: usize, notes: Vec<Note> },

    // Zapping
    /// Zap this note
    ZapNote(EventId),
    /// The amount to zap was typed, or a preset chosen
    ZapAmountChanged(String),
    /// The comment to go with a zap was typed
    ZapCommentChanged(String),
    /// Ask the author's Lightning address for an invoice of the amount chosen
    RequestZapInvoice,
    /// The invoice for the zap of this note came, or why it didn't
    ZapInvoiceReady { note: EventId, result: Result<Invoice, ZapError> },
    /// Copy the invoice on show to the clipboard
    CopyZapInvoice,
    /// The wait for the receipt of this note's zap ended: with the invoice paid, or with nothing
    ZapReceiptAwaited { note: EventId, paid: Option<String> },
    /// Leave the zap view
    CloseZap,

    // Pictures
    /// A view needs this picture
    LoadImage(ImageRequest),
    /// A picture was clicked with the right button: put the picture at this address on the clipboard
    CopyPicture(String),
    /// The picture arrived, or why it didn't
    ImageLoaded(ImageRequest, Result<LoadedImage, String>),

    // Link previews
    /// A view needs what the web page at this address says of itself
    LoadPreview(String),
    /// What the page at this address says of itself, or why there is nothing to show
    PreviewLoaded(String, Result<Linked, String>),

    // Videos
    /// The video at this address was pressed: play it, or hold it if it plays
    VideoPress(String),
    /// What there is to play of the YouTube video at this address was found: the address of a stream, or why not
    VideoFound(String, Result<String, FindError>),
    /// Turn the sound of the video that plays off, or back on
    VideoMute,
    /// The slider of the sound of the video that plays is being dragged: make it this loud, from nothing (0) to whole (1)
    VideoVolume(f64),
    /// That slider was let go
    VideoVolumeSet,
    /// The pointer came over the sound controls of the video that plays (`true`), or left them
    VideoSound(bool),
    /// Open the menu of speeds a video plays at, or close it if it is open
    VideoSpeedMenu,
    /// Put that menu away
    CloseVideoSpeedMenu,
    /// Play videos at this speed from now on
    VideoSpeed(Speed),
    /// The video that plays is being dragged to this many seconds in
    VideoSeeking(f64),
    /// It was let go there
    VideoSeek,
    /// Time to look at the video that plays: for a new picture, how far in it is, whether it ended
    VideoTick,

    // Sharing
    /// Copy a link to this note that anyone can open
    CopyShareLink(EventId),

    // Notifications
    /// Show what the account is told of
    OpenNotifications,
    /// A page of what the account is told of was read from the store, or came from the relays:
    /// the newest, or, if `older`, the one before the oldest known
    NotificationsLoaded { account: PublicKey, source: Source, older: bool, result: Result<Notified, FetchError> },
    /// The list of notifications was scrolled near its end: look for older ones
    LoadOlderNotifications,
    /// Show this kind of notification
    SelectNotificationsTab(segmented_button::Entity),
    /// The account's Lightning address said which key signs its zap receipts, or could not be asked
    ZapperKnown(PublicKey, Result<Option<PublicKey>, String>),

    // Long notes
    /// Show all this note says, in its row
    ExpandNote(EventId),
    /// Show this note cut again, as a row first shows a long one
    CollapseNote(EventId),

    // Links
    /// A link in a note was clicked: open it in the browser
    OpenUrl(String),

    // Errors
    /// Dismiss the error banner
    ClearError,
}

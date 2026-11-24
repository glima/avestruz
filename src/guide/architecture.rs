// SPDX-License-Identifier: MIT

//! What the parts are, what each may know of the others, where each runs,
//! and where things are kept. Documentation only.
//!
//! 1. [In one picture](#in-one-picture)
//! 2. [The loop](#the-loop)
//! 3. [The layers, and what each may know](#the-layers-and-what-each-may-know)
//! 4. [The model](#the-model)
//! 5. [Which view is on show](#which-view-is-on-show)
//! 6. [Starting up](#starting-up)
//! 7. [The popup's life](#the-popups-life)
//! 8. [Where things run](#where-things-run)
//! 9. [Where things are kept](#where-things-are-kept)
//! 10. [What is asked of the web](#what-is-asked-of-the-web)
//! 11. [The modules](#the-modules)
//! 12. [What the program weighs](#what-the-program-weighs)
//!
//! # In one picture
//!
//! Avestruz is one process, started by the COSMIC panel, with one button
//! in the panel and one popup under it. Everything it shows is in the
//! popup; everything it knows is in one model; everything that changes
//! the model is a message.
//!
//! ```mermaid
//! flowchart TB
//!     reader(["Whoever is at the desktop"])
//!     subgraph panel["The COSMIC panel"]
//!         button["The applet's button"]
//!         popup["The popup: the whole client"]
//!     end
//!     subgraph process["Avestruz, one process"]
//!         views["views: draw the model"]
//!         model[("AppModel: all that outlives the popup")]
//!         handlers["handlers: take one message at a time"]
//!         service["NostrService: two relay pools over one store"]
//!         signer["Signer: signs, encrypts, decrypts"]
//!         fetched["ImageStore and PreviewStore"]
//!     end
//!     subgraph here["On this machine"]
//!         config[("COSMIC config")]
//!         keyring[("System keyring")]
//!         store[("Event store, LMDB")]
//!     end
//!     subgraph away["Elsewhere"]
//!         own["The account's own relays"]
//!         theirs["Other people's relays"]
//!         bunker["A remote signer"]
//!         web["Web servers"]
//!     end
//!     reader --> button --> popup
//!     popup -- "messages" --> handlers
//!     handlers -- "change" --> model
//!     model -- "is read by" --> views
//!     views -- "widgets" --> popup
//!     handlers --> service
//!     handlers --> signer
//!     handlers --> fetched
//!     handlers <--> config
//!     service <--> store
//!     service <--> own
//!     service <--> theirs
//!     signer <--> keyring
//!     signer <-. "NIP-46, by way of relays" .-> bunker
//!     fetched --> web
//! ```
//!
//! The names in the picture are the names in the code:
//! [`AppModel`](crate::state::AppModel),
//! [`Message`](crate::messages::Message), [`crate::handlers`],
//! [`crate::views`], [`NostrService`](crate::nostr::client::NostrService),
//! [`Signer`](crate::nostr::account::Signer),
//! [`ImageStore`](crate::images::ImageStore) and
//! [`PreviewStore`](crate::previews::PreviewStore).
//!
//! # The loop
//!
//! Avestruz is a libcosmic application, and libcosmic is iced: the Elm
//! architecture. There is a model, a function that draws it, and a
//! function that changes it when a message comes. Nothing else changes
//! it, and nothing is drawn from anything else.
//!
//! ```mermaid
//! sequenceDiagram
//!     actor reader as Reader
//!     participant view as views
//!     participant update as app update
//!     participant handler as A handler
//!     participant task as A task
//!     participant world as Store, relays, web
//!     reader->>view: presses, types, scrolls
//!     view->>update: a Message
//!     update->>handler: the handler for it
//!     handler->>handler: changes the model
//!     handler-->>task: starts what takes time
//!     update-->>view: the view is built again from the model
//!     task->>world: asks
//!     world-->>task: answers, or doesn't
//!     task->>update: a Message carrying the answer
//!     update->>handler: the handler for it
//!     handler->>handler: changes the model
//!     update-->>view: the view is built again from the model
//! ```
//!
//! [`AppModel::update`](crate::app) is one `match` over every
//! [`Message`](crate::messages::Message), and each arm calls a handler. A
//! handler does two things and no third: it changes the model, at once and
//! without waiting on anything, and it may return a `Task`, which is
//! whatever takes time (a question to the store, to a relay, to a web
//! server, a signature) wrapped so that its answer comes back as another
//! message. Nothing in a handler blocks, since the handler runs where the
//! window is drawn.
//!
//! Besides what the reader does, messages come from five subscriptions,
//! set up in [`AppModel::subscription`](crate::app):
//!
//! | From | Message | How often |
//! |---|---|---|
//! | A timer | `CheckForNewNotes` | Every minute while the popup is open, every five while it is closed |
//! | A timer | `SweepStore` | Every hour; the store is swept once a day (see [`crate::handlers::store`]) |
//! | COSMIC's config watch | `UpdateConfig` | When the config changes on disk |
//! | The picture requests | `LoadImage` | When a row on screen wants a picture that isn't there (see [`crate::requests`]) |
//! | The preview requests | `LoadPreview` | When a row on screen links to a page nobody asked yet |
//!
//! ## Answers that come too late
//!
//! A task's answer arrives some time after it was started, and by then
//! the model may have moved on: the profile was closed, another account
//! signed in, a newer search was made. So an answer says what it is an
//! answer *to*, and the handler that takes it looks whether that is still
//! there:
//!
//! - a list of notes has an id of its own
//!   ([`FeedState::id`](crate::state::FeedState::id)), and what was asked
//!   for it comes back with the id, to find the list or find it gone;
//! - a search has a serial number, and only the newest one's answers are
//!   taken;
//! - what belongs to an account (its bookmarks, its relay lists, its
//!   drafts) comes back with the account's public key, and is dropped if
//!   somebody else is signed in by then.
//!
//! # The layers, and what each may know
//!
//! ```mermaid
//! flowchart TB
//!     subgraph ui["What is seen"]
//!         views["views and views::components"]
//!     end
//!     subgraph core["What is known and what is done"]
//!         state["state: AppModel"]
//!         messages["messages: Message"]
//!         handlers["handlers"]
//!     end
//!     subgraph nostr["What speaks Nostr"]
//!         client["nostr::client: NostrService"]
//!         account["nostr::account: Signer"]
//!         pure["nostr::*: events made and read, no I/O"]
//!     end
//!     subgraph io["What reaches outside"]
//!         http["http, images, previews"]
//!         config["config, drafts, screen, links, logging"]
//!     end
//!     views -- "reads" --> state
//!     views -- "sends" --> messages
//!     handlers -- "changes" --> state
//!     handlers -- "takes" --> messages
//!     handlers --> client
//!     handlers --> account
//!     handlers --> http
//!     handlers --> config
//!     client --> pure
//!     client --> account
//!     pure -.-> http
//! ```
//!
//! - **Views** take `&AppModel` and return widgets. They can't change the
//!   model, and they never ask the store or a relay anything. What a view
//!   finds missing (a picture, a link's preview) it asks for through a
//!   store that hands the request on as a message: see [`crate::requests`].
//! - **Handlers** are `impl AppModel` blocks, one file for each kind of
//!   thing that can happen ([`crate::handlers`]). They are the only code
//!   that changes the model.
//! - **The `nostr` modules** know nothing of the toolkit: none of them
//!   names a widget, a message or the model. Most are pure: an event goes
//!   in and a model for the views comes out
//!   ([`content`](crate::nostr::content), [`thread`](crate::nostr::thread),
//!   [`feed`](crate::nostr::feed)), or what someone wrote goes in and an
//!   unsigned event comes out ([`compose`](crate::nostr::compose),
//!   [`contacts`](crate::nostr::contacts),
//!   [`bookmarks`](crate::nostr::bookmarks)). Two do the talking:
//!   [`client`](crate::nostr::client) to relays and the store,
//!   [`account`](crate::nostr::account) to the keyring and a remote
//!   signer. Three ask a web server one question each
//!   ([`nip05`](crate::nostr::nip05), [`zap`](crate::nostr::zap),
//!   [`relay_info`](crate::nostr::relay_info)).
//! - **Everything from the web** goes through [`crate::http`], which is
//!   where what may be fetched, and how much of it, is decided once.
//!
//! # The model
//!
//! [`AppModel`](crate::state::AppModel) is one struct. It is large because
//! of what the popup is (see [the popup's life](#the-popups-life)): nothing
//! can be left in a widget. What is in it, by what it is for:
//!
//! | What | Fields | Notes |
//! |---|---|---|
//! | The window | `core`, `popup`, `view_state`, `screen` | `popup` is `None` while it is closed |
//! | Who is signed in | `account`, `signer`, `login_*` | The account is a public key and whether it can sign; no secret is in the model |
//! | The relay client | `nostr` | `None` until the store is open |
//! | The feed | `following`, `follows`, `feeds_made`, `feed_checked_at`, `feed_modes` | A [`FeedState`](crate::state::FeedState) |
//! | What is open over the feed | `pages` | A stack of [`Page`](crate::state::Page): threads, profiles, the bookmarks, searches, the notifications |
//! | What the account is told of | `notifications`, `notification_tabs` | A [`NotificationsState`](crate::state::NotificationsState): kept up while no page shows it, for the count of what is new |
//! | Writing | `composer`, `drafts`, `drafts_kept`, `draft_path` | The editor's content is here, not in the editor |
//! | What was made of notes | `engagement`, `engagement_asked`, `pending_marks`, `seen_on` | Counts by note, and the reactions and reposts on their way |
//! | Reacting | `reaction_menu`, `reaction_suggestions` | What to react with, on offer for one note, and the emoji offered first |
//! | Bookmarks | `bookmarks`, `bookmark_pending`, `bookmark_first_list` | A [`BookmarkState`](crate::state::BookmarkState) |
//! | The account's relays | `relays` | A [`RelaysState`](crate::state::RelaysState): the lists, and what was changed and not saved |
//! | The account's media servers, and what is attached | `media` | A [`MediaState`](crate::state::MediaState): the server list, what was changed and not saved, the files on their way, and what a note says of each that arrived |
//! | People | `profiles`, `profiles_requested`, `nip05`, `follow_pending` | Everyone's profile is in one map, by public key |
//! | Pictures and previews | `images`, `previews`, and their request channels | Stores the views read and ask |
//! | Zapping | `zap` | A [`ZapState`](crate::state::ZapState) |
//! | What went wrong | `error_message` | One line, shown in a banner over whatever is on show |
//!
//! A list of notes is always the same shape, whether it is the feed or the
//! notes on a profile: its notes, how tall each row measured, how far it
//! is scrolled, and what is on its way.
//!
//! ```mermaid
//! classDiagram
//!     class AppModel {
//!         view_state
//!         following
//!         pages
//!         composer
//!         profiles
//!     }
//!     class FeedState {
//!         id
//!         feed
//!         heights
//!         position
//!         pending
//!     }
//!     class Page {
//!         <<enumeration>>
//!         Thread
//!         Profile
//!         Bookmarks
//!         Search
//!     }
//!     class ThreadState {
//!         thread
//!         heights
//!         position
//!     }
//!     class ProfileState {
//!         public_key
//!         notes
//!     }
//!     AppModel "1" *-- "1" FeedState : following
//!     AppModel "1" *-- "0..16" Page : pages
//!     Page --> ThreadState
//!     Page --> ProfileState
//!     ProfileState *-- FeedState : notes
//! ```
//!
//! # Which view is on show
//!
//! [`ViewState`](crate::state::ViewState) names the view, and the stack of
//! pages says what is under it. The feed is under everything. Threads,
//! profiles, the bookmarks and searches open over it as pages, one from
//! another; Back closes the last. The composer, the zap view, the
//! settings, the relays page and the view of what the keys do show *in
//! place of* whatever list was on show and leave it as it was, so
//! closing them goes back to the last page, or to the feed if no page is
//! open
//! ([`show_list_view`](crate::state::AppModel::show_list_view)).
//!
//! ```mermaid
//! stateDiagram-v2
//!     state "Compose, Zap, Settings, Relays or Keys" as Aside
//!     [*] --> Login: no account in the config
//!     [*] --> Feed: an account in the config
//!     Login --> Feed: signed in
//!     Feed --> Page: a thread, a profile, the bookmarks or a search opens
//!     Page --> Page: another opens from it
//!     Page --> Feed: Back, from the last page
//!     Feed --> Aside: opened
//!     Page --> Aside: opened
//!     Aside --> Feed: closed, with no page open
//!     Aside --> Page: closed, with a page open
//!     Aside --> Login: signed out, in the settings
//! ```
//!
//! No more than
//! [`MAX_OPEN_PAGES`](crate::handlers::navigation::MAX_OPEN_PAGES) pages
//! are kept: the applet runs for as long as the session does, and each
//! page holds a list of notes. Opening one more closes the first.
//!
//! # Starting up
//!
//! `main` does three things before there is a window: it names the TLS
//! crypto provider (there must be exactly one, see [`crate::http`]), sets
//! up logging so that the level can be changed while the applet runs
//! ([`crate::logging`]), and picks the language. Then
//! [`AppModel::init`](crate::app) reads the config and starts four things
//! at once, none of which waits for another:
//!
//! ```mermaid
//! sequenceDiagram
//!     participant init as init
//!     participant store as Event store
//!     participant keyring as Keyring
//!     participant relays as Relays
//!     participant model as AppModel
//!     par the store and the relays
//!         init->>store: open (NostrService::open)
//!         store-->>model: NostrReady
//!         init->>relays: connect to the config's relays, in the background
//!     and the account
//!         init->>keyring: is the secret of the config's account there?
//!         keyring-->>model: AccountRestored: full access or read-only
//!     and the draft file
//!         init-->>model: DraftLoaded
//!     and the screen
//!         init-->>model: ScreenMeasured
//!     end
//!     Note over model: load_account runs when both the store and the account are there
//!     model->>store: who does the account follow? what is its relay list?
//!     store-->>model: the feed as it was, at once
//!     model->>relays: the same questions, and the newest notes
//!     relays-->>model: answers, as they come
//! ```
//!
//! [`load_account`](crate::state::AppModel::load_account) is called both
//! when the store opens and when
//! the account is known, and does nothing until both are there. What it
//! starts: the account's outbox relays joining the pool, its contact list
//! (which fills the feed), its own profile, and, only if the popup is
//! open, its bookmarks, the relay lists that are gone by all the time, and
//! its drafts. Those three take the account's key to read, which may mean
//! asking a remote signer, which may mean asking a person: none of that
//! happens before there is someone looking at the popup.
//!
//! A feed that was read before shows before any relay has answered: what
//! the store holds is asked first and answers within the frame.
//!
//! # The popup's life
//!
//! Avestruz is two processes: the button in the panel, and the window,
//! which is everything else and is what this page is about (why two is
//! in [`crate::button`]). The window has nothing on the screen until the
//! button is pressed: then it puts up the popup, a surface of its own on
//! the compositor, against the edge the panel is on.
//!
//! The popup closes whenever something else is clicked, as a panel's own
//! popup does. When it closes, its widget tree is gone: every scroller's position, every
//! text field's content, every row's measured height. When it opens
//! again, a new tree is built from the model. So the rule is that
//! **whatever should still be there when the popup opens again is in the
//! model**, and the widgets are given it back:
//!
//! | What a widget would have kept | Where it is kept instead |
//! |---|---|
//! | How far a list is scrolled | A [`ScrollPosition`](crate::views::components::ScrollPosition) cell in the list's state, scrolled back to before the first frame |
//! | How tall each row measured | A [`RowHeights`](crate::views::components::RowHeights), by note id, in the list's state |
//! | Which long notes show in full | An [`Expanded`](crate::state::Expanded), by note id, in the list's state |
//! | What is being written | The composer's `text_editor::Content`, in the model; and every target's draft in [`Drafts`](crate::drafts::Drafts) |
//! | Which page is open over which | [`AppModel::pages`](crate::state::AppModel::pages) |
//! | What was typed in a search, and what it found | The search's [`SearchPage`](crate::state::SearchPage) |
//! | Relays changed and not saved | [`RelaysState::edited`](crate::state::RelaysState::edited) |
//! | Which row of each list the keyboard is at | `marked`, in each list's own state: [`FeedState::marked`](crate::state::FeedState::marked) and its like |
//! | Media servers changed and not saved, and files on their way to them | [`MediaState::edited`](crate::state::MediaState::edited), [`MediaState::uploads`](crate::state::MediaState::uploads) |
//!
//! ```mermaid
//! stateDiagram-v2
//!     [*] --> Closed
//!     Closed --> Open: the button is pressed
//!     Open --> Closed: the button, or a click anywhere else
//! ```
//!
//! | When | What happens |
//! |---|---|
//! | The popup opens | New notes are looked for, if that was not done in the last half minute. The bookmarks, the blocked and indexer relays and the drafts are read, each of which takes the key. The screen is measured again |
//! | The popup closes | What is in the composer is kept as its draft, and an open menu is put away |
//! | While it is closed | New notes are still looked for, every five minutes, and wait to be asked for |
//!
//! The popup may be as tall as the screen it opens on has room. A
//! program is not told how much that is, so the window keeps a surface
//! from the top of the screen to the bottom, which shows nothing, and
//! asks how tall the compositor made it ([`crate::screen`]).
//!
//! # Where things run
//!
//! | Where | What runs there |
//! |---|---|
//! | The main thread | The window's event loop: `update`, every handler, `view`, layout and drawing. The popup is drawn by the GPU (`wgpu`), and by the CPU (`tiny_skia`) where there is none to be had. |
//! | The button's process | The button in the panel and nothing else, drawn by the CPU: it starts the window, tells it of each press, and shows the dot it is told to ([`crate::button`]) |
//! | The tokio runtime libcosmic starts | Every `Task`: questions to relays and to the store, signing, everything fetched from the web. Each relay connection is a task of the relay library's. |
//! | tokio's blocking threads | Decoding a picture ([`crate::images`]), every call to the keyring ([`crate::nostr::account`]) |
//! | yt-dlp, a process of its own | Finding a stream of a YouTube video, when one is played: run, waited for off the main thread, and gone ([`crate::youtube`]) |
//! | GStreamer's threads | A video that plays: fetching it, decoding it, its sound. The main thread only takes the newest frame, on a timer ([`crate::video`]) |
//! | The event store's writer thread | Every write to the store, in the order they were asked for: events saved as relays send them, and what a sweep takes out ([`crate::nostr::sweep`]) |
//!
//! Two things follow from the first row. A handler must not wait on
//! anything, since nothing is drawn while it does. And a message must not
//! be sent *while a frame is being drawn* if the popup may be changing
//! size just then, or the CPU renderer, where it is the one drawing,
//! presents into a buffer of the old size and panics: this is why a list says it was scrolled near its end
//! in answer to an event and never during a redraw (see
//! [`crate::views::components::virtual_list`]).
//!
//! What is shared between tasks is behind a lock that is never held across
//! an `await` of something slow: the client's bookkeeping in
//! [`NostrService`](crate::nostr::client::NostrService) is behind
//! `std::sync::Mutex`es taken for a few lines at a time. Two async locks
//! are held longer, on purpose, to keep two things from happening at once:
//! connecting to a relay somebody else is already connecting to, and
//! changing whose relays are in use while they are being changed.
//!
//! # Where things are kept
//!
//! ```mermaid
//! flowchart LR
//!     subgraph disk["On disk"]
//!         config[("COSMIC config: settings, and the account's npub")]
//!         keyring[("Keyring: nsec or bunker address, client keys")]
//!         events[("Event store: every event a relay sent, and drafts")]
//!         draft[("draft.txt: one draft, when all else failed")]
//!     end
//!     subgraph memory["In memory, for as long as the applet runs"]
//!         model["The model"]
//!         pictures["Decoded pictures, up to a budget"]
//!         previews["What linked pages said"]
//!         seen["Which relay sent which note"]
//!     end
//!     model --- pictures
//!     model --- previews
//!     model --- seen
//! ```
//!
//! | What | Where | Notes |
//! |---|---|---|
//! | Settings: relays, which account, text size, how long notes are kept, how much is logged, the emoji reacted with, when the notifications were last looked at | COSMIC config, under the app's id | [`Config`](crate::config::Config). Read at start, watched for changes |
//! | An account's `nsec` or `bunker://` address | The system keyring, under its `npub` | Never in the config, the store or a log |
//! | The key a remote signer knows this client by | The keyring, one for each signer | Kept when signing out |
//! | Every event a relay sent | LMDB, under the data directory | The cache everything shows from first. Swept by age ([`crate::nostr::sweep`]) |
//! | Drafts | The same store, as NIP-37 events, encrypted | [`crate::nostr::kept_drafts`] |
//! | The draft of a new note, when it could not be kept as an event | A text file beside the store | [`crate::drafts`] |
//! | Fonts of the applet's own, if any were put there | A directory beside the store | Read at start, before the system's ([`crate::fonts`]) |
//! | Pictures | Memory only | Up to [`DEFAULT_BUDGET_BYTES`](crate::images::DEFAULT_BUDGET_BYTES) decoded; the least recently shown go first |
//! | Link previews | Memory only | Asked once in a run |
//! | The video that plays | Memory only | One at a time: its latest frame, and what GStreamer holds of it ([`crate::video`]) |
//! | Which relays a note was seen on | Memory only | [`crate::nostr::sightings`] |
//!
//! There is no database beside the event store. What Avestruz knows of the
//! account that is not a setting is an *event*: who it follows is its
//! contact list, what it bookmarked is its bookmark list, where it writes
//! is its relay list, what it left unfinished is a draft event. They are
//! read from the store like anything else, and what other clients write
//! shows up here by the same road.
//!
//! # What is asked of the web
//!
//! Relays are spoken to over WebSocket by the relay library. Everything
//! else goes over `https` through one client and one function
//! ([`crate::http`]), which is where the rules are: only `https`, redirects
//! included; a time limit; and no more read than the caller asked to
//! keep. Every address fetched was written by somebody else.
//!
//! | Who is asked | What | When | Where |
//! |---|---|---|---|
//! | Whoever hosts a picture | The picture | A row with it is on screen | [`crate::images`] |
//! | A page a note links to | Its head, for a title, a line and a picture | A row with the link is on screen | [`crate::previews`] |
//! | Whoever hosts a video | The video, fetched by GStreamer and not by [`crate::http`] | Its reader pressed play | [`crate::video`] |
//! | YouTube | What a video is called, and its picture | A row with a link to it is on screen | [`crate::previews`], [`crate::images`] |
//! | YouTube, by yt-dlp | Which streams a video has; then, by GStreamer, one of them | Its reader pressed play | [`crate::youtube`] |
//! | The domain of a NIP-05 name | Whether the name is that person's | Their profile is open, or the name was typed in a search | [`crate::nostr::nip05`] |
//! | A Lightning address's server | What it accepts, then an invoice | A note is being zapped | [`crate::nostr::zap`] |
//! | A relay, over `https` | What it says of itself (NIP-11) | The relays page shows, or its address was typed in a search | [`crate::nostr::relay_info`] |
//!
//! Each of those servers sees the request, and so learns that someone at
//! this address looked. That is why a NIP-05 name is only checked when a
//! profile is open: a feed would have a hundred domains told who reads
//! what.
//!
//! # The modules
//!
//! ```mermaid
//! flowchart LR
//!     main --> app
//!     app --> handlers
//!     app --> views
//!     app --> state
//!     app --> messages
//!     handlers --> state
//!     handlers --> nostr
//!     handlers --> images
//!     handlers --> previews
//!     handlers --> drafts
//!     handlers --> config
//!     handlers --> screen
//!     handlers --> links
//!     views --> state
//!     views --> components["views::components"]
//!     components --> images
//!     components --> previews
//!     images --> http
//!     previews --> http
//!     images --> requests
//!     previews --> requests
//!     nostr --> http
//!     state --> nostr
//!     messages --> nostr
//! ```
//!
//! | Module | What it is |
//! |---|---|
//! | [`crate::app`] | The `cosmic::Application`: init, the subscriptions, and `update`, which hands each message to its handler |
//! | [`crate::state`] | The model, and the state of each thing the model holds |
//! | [`crate::messages`] | Every message |
//! | [`crate::handlers`] | What happens on each message, one file for each kind of thing |
//! | [`crate::views`] | What each view looks like, and the components they share |
//! | [`crate::nostr`] | Everything that speaks Nostr |
//! | [`crate::config`] | The settings, as COSMIC's config store keeps them |
//! | [`crate::http`] | The one way anything is fetched from the web |
//! | [`crate::images`], [`crate::previews`], [`crate::requests`] | Pictures, what linked pages say of themselves, and how a view asks for either |
//! | [`crate::drafts`] | What is being written, by what it is written to |
//! | [`crate::screen`], [`crate::links`], [`crate::logging`], [`crate::helpers`], [`crate::i18n`] | How tall the screen is, opening a link, how much is logged, small pure helpers, the language |
//!
//! The crates underneath, and what each is for:
//!
//! | Crate | What for |
//! |---|---|
//! | `libcosmic` | The application framework, the widgets, the theme, the panel applet |
//! | `nostr-sdk` | The protocol, the relay pool, and the relay Avestruz runs itself |
//! | `nostr-lmdb` | The event store |
//! | `nostr-connect` | Signing through a remote signer (NIP-46) |
//! | `keyring` | The system keyring (Secret Service) |
//! | `reqwest`, `rustls` | Everything fetched from the web |
//! | `image` | Decoding pictures |
//! | `gstreamer`, `gstreamer-app`, `gstreamer-video` | Playing videos: fetching, decoding, sound, and the frames a row draws |
//! | `html5gum` | Reading the head of a linked page |
//! | `qrcode` | An invoice as a QR code |
//! | `tokio-tungstenite` | The WebSocket the relay Avestruz runs itself is spoken to over |
//!
//! # What the program weighs
//!
//! The program as installed is 49 MiB, of which 34.2 MiB is code (its
//! `.text` section) and most of the rest is what the code reads rather
//! than runs: tables, text, and other data that does not change. That
//! was measured in November 2026, and is a picture of that day: `just
//! bloat-check` measures it as it is now, from a build that keeps the
//! names `cargo bloat` tells one crate's code from another's by.
//!
//! The code comes from some three hundred and fifty crates, and no one
//! of them is much of it. Taken by what they are for:
//!
//! ```mermaid
//! pie showData title The program's code, by what it is for (% of .text)
//!     "Rust's standard library" : 11.9
//!     "Drawing on the GPU" : 11.5
//!     "D-Bus" : 11.1
//!     "Windows, Wayland and X11" : 10.1
//!     "Avestruz itself" : 9.3
//!     "Pictures" : 7.6
//!     "The toolkit" : 7.5
//!     "The web" : 7.4
//!     "Text" : 6.8
//!     "Nostr" : 6.0
//!     "Accessibility" : 3.1
//!     "Everything else" : 7.7
//! ```
//!
//! | What for | Share | The crates that are most of it, and the share of each |
//! |---|---|---|
//! | Rust's standard library | 11.9% | `std` |
//! | Drawing on the GPU | 11.5% | `naga` 5.5, `wgpu_core` 2.9, `wgpu_hal` 1.4, `iced_wgpu` 0.7, `wgpu` 0.4 |
//! | D-Bus: the keyring, the desktop's portals | 11.1% | `zbus` 8.6, `dbus` 0.7, `zvariant` 0.6, `ashpd` 0.4 |
//! | Windows, Wayland and X11 | 10.1% | `iced_winit` 2.2, `winit_wayland` 1.4, `winit_x11` 1.3, `wayland_client` 1.0, `wayland_protocols` 0.7, `clipboard_wayland` 0.7 |
//! | Avestruz itself | 9.3% | `cosmic_applet_avestruz` |
//! | Pictures: decoding them, and drawing SVG | 7.6% | `image_webp` 1.3, `image` 1.2, `usvg` 1.1, `tiny_skia` 1.1, `zune_jpeg` 0.5, `resvg` 0.4 |
//! | The toolkit: libcosmic and iced | 7.5% | `cosmic` 2.3, `cosmic_theme` 1.6, `iced_graphics` 0.5, `cosmic_config` 0.5 |
//! | The web: TLS, HTTP, and what runs the tasks | 7.4% | `rustls` 1.6, `tokio` 1.3, `ring` 0.9, `reqwest` 0.8, `html5gum` 0.6 |
//! | Text: fonts, and shaping | 6.8% | `harfrust` 1.3, `skrifa` 1.1, `read_fonts` 0.9, `rustybuzz` 0.8, `cosmic_text` 0.8 |
//! | Nostr | 6.0% | `nostr_sdk` 2.5, `nostr` 1.0, `nostr_lmdb` 0.7, `nostr_connect` 0.4 |
//! | Accessibility | 3.1% | `accesskit_unix` 2.5 |
//! | Everything else | 7.7% | Small crates, none over 0.8, and the 1.1 that `cargo bloat` gives to no crate |
//!
//! `just bloat-check` names the thirty largest crates; `cargo bloat
//! --profile release-bloat --crates -n 0` names them all, which is what
//! the shares above were added up from. They are `cargo bloat`'s best
//! guess, as it says itself: code made from one crate's generics for
//! another's types has no one owner.
//!
//! What Avestruz itself adds is under a tenth. Drawing on the GPU, which
//! the popup does (see [`crate::button`]), is more than that. Playing
//! videos is next to nothing, 0.1%: GStreamer is the system's, and no
//! part of the program. And about 2% is for X11 (`winit_x11`, `x11rb`,
//! `x11_dl`), which a popup that is a layer surface on a Wayland
//! compositor never runs. No feature of libcosmic's leaves it out, its
//! defaults turned off or not: libcosmic takes `iced_winit` with that
//! crate's defaults, which name X11; `iced_winit` takes `winit` with
//! its own, which do too; and the clipboard crate under both takes its
//! X11 half whatever is asked of it.

// SPDX-License-Identifier: MIT

//! What happens, step by step. Documentation only.
//!
//! Each flow here crosses several modules, and none of them is the place
//! to tell it whole. The diagrams name the parts as
//! [`crate::guide::architecture`] does; under each flow is where its code
//! is.
//!
//! 1. [Signing in](#signing-in)
//! 2. [Signing, and reading what is private](#signing-and-reading-what-is-private)
//! 3. [Filling the feed](#filling-the-feed)
//! 4. [New notes, and older ones](#new-notes-and-older-ones)
//! 5. [Opening a thread](#opening-a-thread)
//! 6. [Publishing](#publishing)
//! 7. [Reacting and reposting](#reacting-and-reposting)
//! 8. [Changing a list: follows, bookmarks, relays](#changing-a-list-follows-bookmarks-relays)
//! 9. [Searching](#searching)
//! 10. [Zapping](#zapping)
//! 11. [Drafts](#drafts)
//! 12. [Pictures and link previews](#pictures-and-link-previews)
//! 13. [Playing a video](#playing-a-video)
//! 14. [Notifications](#notifications)
//! 15. [Sweeping the store](#sweeping-the-store)
//!
//! Two patterns come back in nearly all of them, and are worth knowing
//! first:
//!
//! - **The store first, the relays after.** Whatever is asked is asked of
//!   the event store, which answers within the frame, and of the relays,
//!   which answer when they do. What the relays send goes into the store,
//!   so the second time the store has it. [`Source`] says which of the two
//!   an answer is from.
//! - **Shown at once, sent behind.** What the reader does shows at once (a
//!   reaction, a bookmark, a follow), and goes out behind it. If it
//!   doesn't go out, it is taken back, and the reason is said.
//!
//! [`Source`]: crate::nostr::client::Source
//!
//! # Signing in
//!
//! One field takes whatever identifies an account, and what was typed
//! decides how much the account can do.
//!
//! ```mermaid
//! flowchart TD
//!     typed["What is typed in the sign-in field"] --> parse{"What is it?"}
//!     parse -->|"an nsec"| secret["The key goes to the keyring, under its npub"]
//!     parse -->|"an npub"| public["Nothing is kept"]
//!     parse -->|"a bunker address"| bunker["The remote signer is connected to, as the client key it knows"]
//!     parse -->|"none of these"| wrong["Said under the field, and nothing happens"]
//!     secret -->|"the keyring took it"| full["Signed in, with full access"]
//!     secret -->|"the keyring refused"| nobody["Nobody is signed in, and the key is kept nowhere"]
//!     public --> readonly["Signed in, read-only"]
//!     bunker -->|"the signer agrees"| address["Its address goes to the keyring, without its one-time secret"]
//!     bunker -->|"it refuses, or the wait is called off"| refused["Nobody is signed in. The client key is shown, to be authorized at the signer"]
//!     address --> full
//!     full --> named["The config names the account by its npub"]
//!     readonly --> named
//! ```
//!
//! Waiting for a remote signer may be waiting for a person to press a
//! button on a phone, so it can be called off, and the view says what is
//! being waited for ([`LoginProgress`](crate::state::LoginProgress)).
//!
//! At the next start the config's `npub` is looked up in the keyring
//! ([`restore`](crate::nostr::account::restore)): if something is kept
//! for it the account has full access, and if not it is read-only.
//! Nothing is connected to and nothing is decrypted to find that out.
//!
//! Where: [`crate::nostr::account`] (the three ways in, and the keyring),
//! [`crate::handlers::auth`], [`crate::views::login`].
//!
//! # Signing, and reading what is private
//!
//! Everything that needs the account's secret key goes through one
//! [`Signer`](crate::nostr::account::Signer): signing an event, encrypting
//! to the account itself, decrypting what it encrypted. It goes one of two
//! ways, and the caller doesn't know which.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant caller as A handler
//!     participant signer as Signer
//!     participant keyring as Keyring
//!     participant remote as Remote signer
//!     caller->>signer: sign this, as the account
//!     alt the account signed in with an nsec
//!         signer->>keyring: the key
//!         keyring-->>signer: the key
//!         signer->>signer: signs, and lets go of the key
//!     else it signed in with a bunker address
//!         opt not connected yet in this run
//!             signer->>keyring: the address, and this client's key for that signer
//!             signer->>remote: connect (NIP-46)
//!         end
//!         signer->>remote: sign this
//!         Note over remote: may ask a person
//!         remote-->>signer: the signed event, or no
//!     end
//!     signer-->>caller: the event, or why not
//! ```
//!
//! With an `nsec` the key is in memory for the length of one signature.
//! With a remote signer every request may reach a person, which is the
//! reason for a rule that shows up all over this page: **what takes the
//! key is not asked for before someone is looking, and not asked for
//! twice.** The bookmarks, the private half of the relay lists and the
//! drafts are read when the popup opens, not when the applet starts, and
//! a list that was decrypted once is not decrypted again while it is the
//! newest.
//!
//! # Filling the feed
//!
//! The feed is the notes of the people the account follows, so it starts
//! with who that is: the account's contact list.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant model as AppModel
//!     participant store as Event store
//!     participant own as The account's relays
//!     participant theirs as Other people's relays
//!     model->>store: the account's contact list
//!     store-->>model: the one it has: who is followed
//!     model->>own: the account's contact list
//!     Note over model: a new answer to that starts the feed over, as a new list
//!     model->>store: the newest notes of those people
//!     store-->>model: shown at once
//!     par everyone, from the account's own relays
//!         model->>own: the newest notes of everyone followed
//!         own-->>model: notes
//!     and each, from where they write
//!         model->>theirs: the newest notes of those who write there
//!         theirs-->>model: notes
//!     end
//!     Note over model: merged, newest first. The feed has been filled, and is its reader's from here on
//!     model->>store: what was made of these notes? who wrote them?
//!     model->>own: the same
//! ```
//!
//! Which of other people's relays are asked, and how few, is in
//! [`crate::guide::relays`]. A page is
//! [`FEED_PAGE`](crate::nostr::client::FEED_PAGE) notes. The notes a note
//! quotes are looked for before the note is shown, so that a card doesn't
//! turn up later in a row that is being read.
//!
//! A profile's notes are a list like the feed's, of one person, and are
//! filled the same way by the same code: both are a
//! [`FeedState`](crate::state::FeedState).
//!
//! Where: [`crate::handlers::feed`], [`crate::nostr::feed`],
//! [`NostrService::cached_feed`](crate::nostr::client::NostrService::cached_feed)
//! and [`fetch_feed`](crate::nostr::client::NostrService::fetch_feed).
//!
//! # New notes, and older ones
//!
//! Once a list has been filled, what it shows only changes when its
//! reader says so. The feed is looked at again every minute while the
//! popup is open and every five while it is closed, and once when the
//! popup opens; what turned up waits beside the list
//! ([`Pending`](crate::nostr::feed::Pending)) and is offered by a button
//! over it.
//!
//! ```mermaid
//! stateDiagram-v2
//!     [*] --> Filling: the list is new
//!     Filling --> Settled: the relays answered once, or failed to
//!     Settled --> Offering: a look found notes the list doesn't have
//!     Offering --> Offering: another look found more
//!     Offering --> Settled: the button, or Refresh. They are shown, and the list goes to its top
//!     Settled --> Settled: scrolled near its end. An older page goes in below
//! ```
//!
//! Nothing is ever put in *above* what is being read without being asked
//! for, and an older page goes in below, so the rows on screen stay where
//! they are. That is one half of keeping a list still; the other half is
//! in how rows are measured (see [`crate::guide::decisions`]).
//!
//! A list has no holes. If more turned up than one page holds, the notes
//! that wait don't reach down to the list's newest, and showing them
//! starts the list over from them rather than leave a gap nobody could
//! see: what is older comes back as the list is scrolled to its end.
//!
//! A list is read with replies or without
//! ([`FeedMode`](crate::nostr::feed::FeedMode)). Read without, an older
//! page may bring only replies, which is nothing to show: the list then
//! looks further back on its own, up to
//! [`BARREN_PAGES_MAX`](crate::state::BARREN_PAGES_MAX) pages, and after
//! that when asked.
//!
//! # Opening a thread
//!
//! A thread opens for one note, the *focused* one, from wherever the note
//! was named: a row, a quoted card, a `nostr:` reference in a note's text,
//! an address typed in a search.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant reader as Reader
//!     participant model as AppModel
//!     participant store as Event store
//!     participant relays as Relays
//!     reader->>model: opens a note
//!     model->>model: a new page, with the note itself if it came from a row
//!     par
//!         model->>store: the note, what it answers, what answers it
//!         store-->>model: within the frame
//!     and
//!         model->>relays: the same
//!         relays-->>model: when they answer
//!     end
//!     Note over model: each answer is merged in: every note under the one it answers
//! ```
//!
//! The relays asked are the account's own, the relays the note's author
//! writes to, the relay its address named if it named one, and the relays
//! of the first few people the conversation involves
//! ([`THREAD_PEOPLE`](crate::nostr::client::THREAD_PEOPLE)): a
//! conversation is on the relays of those who had it.
//!
//! Where: [`crate::handlers::thread`], [`crate::nostr::thread`] (the
//! ordering, which is pure), and
//! [`NostrService::thread`](crate::nostr::client::NostrService::thread).
//!
//! # Publishing
//!
//! A note, a reply and a quote are written in the same composer and go
//! out the same way.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant reader as Reader
//!     participant model as AppModel
//!     participant signer as Signer
//!     participant own as The account's relays
//!     participant theirs as The relays of those it names
//!     participant store as Event store
//!     reader->>model: Publish
//!     model->>model: the text, and what it answers, made into an unsigned event
//!     model->>signer: sign
//!     signer-->>model: the event
//!     par
//!         model->>own: the event
//!         own-->>model: took it, or why not
//!     and
//!         model->>theirs: the event
//!         theirs-->>model: took it, or didn't
//!     end
//!     alt at least one relay took it
//!         model->>store: kept
//!         model->>reader: it shows: at the top of the feed, or in its place in the thread
//!         model->>model: its draft is done with
//!     else none did
//!         model->>reader: the composer stays as it was, with the reason
//!     end
//! ```
//!
//! What a reply is depends on what it answers: an answer to a text note
//! is a text note (NIP-10), an answer to anything else is a comment
//! (NIP-22). To build either, the note answered is needed whole, and it is
//! taken from the store; if the store has let go of it since it was shown
//! (see [sweeping](#sweeping-the-store)), the relays are asked for it
//! again ([`NostrService::event`](crate::nostr::client::NostrService::event)).
//!
//! Where: [`crate::handlers::compose`], [`crate::nostr::compose`] (the
//! event, which is pure), and
//! [`NostrService::publish`](crate::nostr::client::NostrService::publish).
//!
//! # Reacting and reposting
//!
//! ```mermaid
//! sequenceDiagram
//!     participant reader as Reader
//!     participant model as AppModel
//!     participant config as COSMIC config
//!     participant signer as Signer
//!     participant relays as Relays
//!     reader->>model: presses the heart under a note
//!     model->>reader: a row: a like, the emoji used most, and a way to all the others
//!     opt none of those
//!         reader->>model: asks for all the others
//!         model->>reader: every emoji, by group, and a field to find one by
//!     end
//!     reader->>model: presses one
//!     model->>config: if it is an emoji: used once more, and when
//!     model->>reader: it is in the heart's place, and the count goes up, at once
//!     model->>model: the reaction is remembered as on its way
//!     model->>signer: sign a reaction that says it
//!     signer-->>model: the event
//!     model->>relays: the event
//!     alt a relay took it
//!         model->>model: no longer on its way. The store has it, and counts it
//!     else none did, or it could not be signed
//!         model->>reader: the heart is empty again, and a banner says why
//!     end
//! ```
//!
//! What is offered to react with is over the list, as a menu is
//! ([`AppModel::reaction_menu`](crate::state::AppModel::reaction_menu)):
//! nothing under it moves, a click beside it or Escape puts it away, and
//! so does closing the popup. The emoji offered first are the ones used
//! most, and of those used as often the ones used last
//! ([`crate::emoji::suggested`]); which were used is one of the settings
//! ([`Config::reaction_emojis`](crate::config::Config::reaction_emojis)),
//! noted when an emoji is pressed, whatever comes of the reaction.
//!
//! While a reaction is on its way, a count that arrives from the relays
//! without it would take it off the screen again; remembering it
//! ([`AppModel::pending_marks`](crate::state::AppModel::pending_marks))
//! is what keeps it there. A repost carries the note it reposts whole, as
//! it was signed, and says which relay has it.
//!
//! Where: [`crate::handlers::engagement`], [`crate::nostr::engagement`],
//! [`crate::emoji`],
//! [`reaction_menu`](mod@crate::views::components::reaction_menu).
//!
//! # Changing a list: follows, bookmarks, relays
//!
//! Who an account follows, what it bookmarked and which relays are its
//! own are each a list, and a list is *replaceable*: there is one for an
//! account, and every new one replaces the last on every relay it
//! reaches. So a change is never "add this": it is the newest list,
//! written again whole with one thing different. Writing it from a copy
//! that is behind would undo whatever another client did since, and
//! writing a first one where one exists would lose everything in it. All
//! three go the same careful way.
//!
//! ```mermaid
//! flowchart TD
//!     change["A follow, a bookmark, or Save on the relays page"] --> ask["Each of the account's relays is asked for the list again"]
//!     ask --> found{"Is there a list?"}
//!     found -->|"yes"| read{"Can what it keeps to itself be read?"}
//!     found -->|"no, and every relay answered"| first{"Was the account told, and did it say again?"}
//!     found -->|"no, but a relay didn't answer"| stop1["Nothing is written: that relay may be the one that has it"]
//!     first -->|"not yet"| tell["Nothing is written. A banner says a first list would replace any kept elsewhere"]
//!     first -->|"yes"| write
//!     read -->|"yes, or it keeps nothing to itself"| write["The list is written again with the one change, everything else as it was"]
//!     read -->|"no"| stop2["Nothing is written: a list written without it would lose it"]
//!     write --> sign["Encrypted to the account if it is private, signed, and sent to the account's relays"]
//!     sign --> shown["The model takes the list that was sent as the newest"]
//! ```
//!
//! "Everything else as it was" is meant strictly: tags this client
//! doesn't know, the public half of a bookmark list (which is never read
//! here), the content older clients kept their relays in, the order.
//! Other clients keep things in these lists, and each finds what the
//! other left.
//!
//! One change is made at a time: the next is made to the list the last
//! one left.
//!
//! Where: [`crate::nostr::contacts`], [`crate::nostr::bookmarks`],
//! [`crate::nostr::relay_lists`] (the lists, pure);
//! [`NostrService::set_following`](crate::nostr::client::NostrService::set_following),
//! [`set_bookmarked`](crate::nostr::client::NostrService::set_bookmarked),
//! [`set_advertised_relays`](crate::nostr::client::NostrService::set_advertised_relays)
//! and [`set_listed_relays`](crate::nostr::client::NostrService::set_listed_relays);
//! and the handlers of [`crate::handlers::profile`],
//! [`crate::handlers::bookmarks`] and [`crate::handlers::relays`].
//!
//! # Searching
//!
//! What is typed on the search page is looked at before anything is sent
//! anywhere, because much of what can be typed there is not a search.
//!
//! ```mermaid
//! flowchart TD
//!     typed["What is typed, and Enter"] --> what{"What is it?"}
//!     what -->|"an nsec, or a bunker address"| secret["Cleared from the field, and sent nowhere"]
//!     what -->|"an npub or nprofile"| person["That person's profile opens"]
//!     what -->|"a note or nevent"| note["That note's thread opens, looked for where the address says"]
//!     what -->|"name@domain"| name["The domain is asked whose it is, and their profile opens"]
//!     what -->|"wss://..."| relay["That relay's latest notes show, with what it says of itself"]
//!     what -->|"words"| words{"Has the account search relays?"}
//!     words -->|"no"| none["The page says where to set them"]
//!     words -->|"yes"| asked["Each is asked for people and for notes, at once"]
//!     asked --> ranked["Their answers are taken a rank at a time, and people followed come first"]
//! ```
//!
//! Only the last row tells a relay anything, and the relays told are the
//! account's own choice (its kind 10007 list): there are none built in.
//! Nothing of a search is kept: no recent searches, and the words are not
//! logged, which is why the relay library's wire logging is turned down in
//! `main`.
//!
//! Where: [`crate::nostr::search`] (what was typed, and ranking; pure),
//! [`crate::handlers::search`],
//! [`NostrService::search`](crate::nostr::client::NostrService::search).
//!
//! # Zapping
//!
//! A zap is a Lightning payment that a wallet service announces on Nostr.
//! Avestruz holds no money: it gets as far as an invoice, and shows it as
//! a QR code for a wallet to pay.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant reader as Reader
//!     participant model as AppModel
//!     participant lnurl as The author's Lightning address
//!     participant wallet as A wallet, elsewhere
//!     participant relays as Relays
//!     reader->>model: zaps a note: how much, and a word
//!     model->>lnurl: what do you accept? (LNURL-pay)
//!     lnurl-->>model: how little, how much, and whether it does zaps
//!     model->>model: a zap request, signed by the account, or by a key made for it
//!     model->>lnurl: an invoice for this much, with the request
//!     lnurl-->>model: an invoice
//!     model->>model: only taken if it asks for the amount chosen
//!     model->>reader: the invoice, as a QR code
//!     reader->>wallet: scans and pays
//!     wallet-->>relays: a zap receipt (kind 9735)
//!     loop while the invoice is on show
//!         model->>relays: is there a receipt for this invoice?
//!     end
//!     relays-->>model: the receipt
//!     model->>reader: paid, and the note's count has it
//! ```
//!
//! Anyone can zap: a read-only account has no key to sign the request
//! with, so it is signed with a key made for the purpose and thrown away,
//! and the zap is anonymous. An address that doesn't do zaps is asked for
//! a plain invoice instead: the money arrives all the same.
//!
//! Where: [`crate::nostr::zap`], [`crate::handlers::zap`],
//! [`crate::views::zap`], and
//! [`NostrService::await_zap_receipt`](crate::nostr::client::NostrService::await_zap_receipt).
//!
//! # Drafts
//!
//! What is written and not published belongs to what it was written to: a
//! note of its own, a reply to one note, a quote of another. Leaving the
//! composer, or the popup closing over it, keeps it.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant model as AppModel
//!     participant signer as Signer
//!     participant relay as The relay Avestruz runs itself
//!     participant store as Event store
//!     participant file as draft.txt
//!     Note over model: the composer is left, or the popup closes
//!     model->>model: the text is that target's draft (for this run)
//!     model->>signer: encrypt the draft to the account, and sign its wrap
//!     alt signed
//!         model->>relay: the draft wrap (kind 31234), over a pipe inside the process
//!         relay->>store: kept, in place of the one before
//!     else it could not be, and it is the draft of a new note
//!         model->>file: the text
//!     end
//!     Note over model: later, the popup opens for the first time in a run
//!     model->>relay: the account's draft wraps
//!     relay-->>model: wraps
//!     model->>signer: decrypt each
//!     signer-->>model: the drafts, back where they were written
//! ```
//!
//! A draft never leaves the machine: it is sent to the one relay nothing
//! outside the process can reach, and to no other. A draft that was
//! published or cleared is kept as an empty wrap, which replaces the full
//! one.
//!
//! Where: [`crate::drafts`] (for the run), [`crate::nostr::kept_drafts`]
//! (the event), [`crate::nostr::private_relay`] (the relay),
//! [`crate::handlers::compose`].
//!
//! # Attaching a file
//!
//! A note links to a file that is somewhere: attaching one is putting it
//! there first. Where is the account's media servers
//! ([`crate::nostr::blossom`]), and the file goes as the picture and no
//! more ([`crate::media`]).
//!
//! ```mermaid
//! sequenceDiagram
//!     actor person as Someone
//!     participant model as AppModel
//!     participant chooser as The desktop's file chooser
//!     participant signer as Signer
//!     participant servers as The media servers
//!     person->>model: the paperclip, in the composer
//!     model->>model: the popup closes: the chooser is a window under it
//!     model->>chooser: open
//!     chooser-->>model: the files picked
//!     model->>model: the popup opens again, on the composer
//!     loop each file, all at once
//!         model->>model: read, a picture made bare, hashed
//!         model->>signer: sign an authorization for that hash
//!         model->>servers: the first that takes it is sent the file
//!         model->>servers: the others are asked to fetch it from there
//!         servers-->>model: where it is
//!         model->>model: its address goes at the end of the text
//!     end
//!     person->>model: Publish
//!     model->>model: the note says of each address still in its text what is at it
//! ```
//!
//! Pasting does the same without the chooser: files copied in a file
//! manager, or a picture copied as one. Text on the clipboard is pasted
//! as text, as ever.
//!
//! A file that arrives after the composer was left goes at the end of
//! the draft it was attached to. What the note says of a file is kept
//! with the draft (see **Drafts**), so that it is still said after a
//! restart.
//!
//! Where: [`crate::handlers::media`], [`crate::nostr::blossom`],
//! [`crate::media`], [`crate::views::media`].
//!
//! # Pictures and link previews
//!
//! A view can't fetch anything: it is built from a model it can't change.
//! What it finds missing, it asks for.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant view as A row, being built
//!     participant images as ImageStore
//!     participant channel as Requests
//!     participant update as app update
//!     participant web as A web server
//!     view->>images: the picture at this address
//!     alt it is there
//!         images-->>view: the picture
//!     else it is not
//!         images-->>view: loading: the row holds its room
//!         images->>channel: ask, once however often the row is built
//!         channel->>update: LoadImage
//!         update->>update: fetched now, if fewer than so many are on their way
//!         update->>web: the picture (https only, so many bytes at the most)
//!         web-->>update: bytes
//!         update->>update: decoded off the main thread, cut to its shape
//!         update->>images: kept, within the budget
//!         update-->>view: built again, with the picture
//!     end
//! ```
//!
//! Only rows on screen are ever built
//! ([`VirtualList`](crate::views::components::VirtualList)), so only what
//! is on screen is ever asked for. Link previews go the same road with
//! their own store. A picture's room is held before it arrives when the
//! note says how large it is, so the row is the same height before and
//! after.
//!
//! Where: [`crate::requests`], [`crate::images`], [`crate::previews`],
//! [`crate::handlers::images`], [`crate::handlers::previews`],
//! [`crate::views::components::media_placeholder`].
//!
//! # Playing a video
//!
//! A video is the one thing a row shows that is not fetched when the row
//! comes into view. Its room is there from the start, with a play button
//! in it; the rest waits to be pressed.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant row as The video's row
//!     participant update as app update
//!     participant videos as Videos, in the model
//!     participant gst as GStreamer: playbin
//!     participant web as The video's server
//!     row->>update: VideoPress(address)
//!     update->>videos: press
//!     videos->>gst: play this address (https only)
//!     gst->>web: the video, as much as it needs
//!     Note over update,videos: a timer runs while there is a video to look at
//!     loop thirty times a second while it plays
//!         update->>videos: VideoTick
//!         videos->>gst: the newest picture? how far in? ended, failed?
//!         gst-->>videos: an RGBA frame, the position
//!         update-->>row: built again: the frame, the bar
//!         row->>videos: shown (which keeps it playing)
//!     end
//!     row->>update: VideoSeeking, VideoSeek, VideoMute, VideoVolume, VideoSpeed, VideoPress again
//!     update->>videos: go there, mute, this loud, this fast, hold or let go
//! ```
//!
//! The row saying it is shown is what holds a video that is not: a row
//! scrolled out of view is not built, nor is the list while another view
//! is in its place, and after a few builds of the popup without it, and a
//! moment, the video is held. The popup closing holds it at once. Another
//! video pressed takes its place: one plays at a time.
//!
//! Which links are videos is settled when the note is read: by how the
//! address ends, or by what the note says of it (NIP-92). A link that
//! gives nothing away is asked for its preview like any page, and if its
//! server calls it a video, the card it gets offers to play it.
//!
//! A link to a YouTube video is a video by its address, and the one kind
//! that can't be played from it: what is there is a page. Pressed, it is
//! on its way while yt-dlp, a program of its own, is asked for a stream
//! of it; then that is played as any video. The diagram of that is in
//! [`crate::youtube`].
//!
//! Where: [`crate::video`], [`crate::youtube`],
//! [`crate::handlers::video`], [`crate::nostr::content`],
//! [`crate::views::components::video_box`](mod@crate::views::components::video_box),
//! [`crate::views::components::link_preview`](mod@crate::views::components::link_preview).
//!
//! # Notifications
//!
//! What others did that concerns the account is every event that names
//! it: found the way a feed is, and read for what it is.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant reader as Reader
//!     participant model as AppModel
//!     participant config as COSMIC config
//!     participant store as Event store
//!     participant relays as Its own relays, and the ones it reads
//!     participant wallet as Its Lightning address
//!     Note over model: the account is known
//!     model->>store: the newest events that name it
//!     store-->>model: a page, each read with the note it is about
//!     model->>relays: the same question
//!     relays-->>model: a page: what is new to the model is merged in
//!     model->>wallet: which key signs its zap receipts? Once
//!     wallet-->>model: the key: the zaps it signed show from now on
//!     loop each time the feed is looked at for new notes
//!         model->>relays: what was written since the newest known, and a while before
//!         relays-->>model: what there is
//!     end
//!     model->>reader: how many were written since the last look, on the bell, and a dot on the panel button
//!     reader->>model: opens the notifications
//!     model->>config: looked at: now
//!     model->>reader: the list, with what was new before this visit picked out
//!     reader->>model: leaves the page, or closes the popup
//!     model->>config: looked at: now, so that what arrived meanwhile is not new next time
//! ```
//!
//! A reaction or a repost names its author's key whoever it is to, so
//! the note it is about is fetched too: it is told of when that note is
//! the account's, or could not be found. A page that is as long as was
//! asked for and ends after everything known has a gap behind it: what
//! is older is let go of, and comes back when the list is scrolled to
//! its end, so that nothing is ever put in above what was read.
//!
//! Looking is having the page on show in an open popup, whatever made it
//! so. That is watched in one place, after every message
//! ([`AppModel::watch_notifications`](crate::state::AppModel::watch_notifications)),
//! and not in each of the ways a page comes and goes.
//!
//! Where: [`crate::handlers::notifications`],
//! [`crate::nostr::notifications`],
//! [`NostrService::notifications`](crate::nostr::client::NostrService::notifications),
//! [`crate::views::notifications`],
//! [`notification_row`](mod@crate::views::components::notification_row).
//!
//! # Sweeping the store
//!
//! Everything a relay sends is kept. Once a day the store lets go of what
//! is older than the settings say and can be asked for again.
//!
//! ```mermaid
//! flowchart TD
//!     tick["Every hour, when the bookmarks become known, and when the setting changes"] --> due{"Swept within the day, or set to keep everything?"}
//!     due -->|"yes"| nothing["Nothing"]
//!     due -->|"no"| known{"Is it known what stays?"}
//!     known -->|"the bookmarks were read"| sweep
//!     known -->|"read-only: there are none here"| sweep
//!     known -->|"not read yet, or unreadable"| nothing
//!     sweep["What is older than the limit, a page at a time"] --> each{"For each event"}
//!     each -->|"the account wrote it"| stays["Stays"]
//!     each -->|"replaceable or addressable: a profile, a list"| stays
//!     each -->|"the account bookmarked it"| stays
//!     each -->|"anything else"| goes["Goes"]
//! ```
//!
//! Nothing on show changes when the store is swept: the lists are in the
//! model. A note that is still on screen after it was swept can still be
//! answered or reposted, since the relays are asked for it again.
//!
//! Where: [`crate::nostr::sweep`], [`crate::handlers::store`].

// SPDX-License-Identifier: MIT

//! What was decided and why, the rules everything else keeps to, and what
//! is not there yet. Documentation only.
//!
//! Most of what is particular about this code follows from a few
//! decisions that are not visible in any one file. A change that goes
//! against one of them will compile, pass its tests, and be wrong. This
//! page is the list, each with its reason and with where it is kept, so
//! that a decision can be found before it is undone, or undone on purpose.
//!
//! 1. [The rules, at a glance](#the-rules-at-a-glance)
//! 2. [What is on show is the reader's](#what-is-on-show-is-the-readers)
//! 3. [The popup takes nothing with it](#the-popup-takes-nothing-with-it)
//! 4. [The store first, the relays after](#the-store-first-the-relays-after)
//! 5. [A secret is in the keyring, or nowhere](#a-secret-is-in-the-keyring-or-nowhere)
//! 6. [What takes the key waits for someone to look](#what-takes-the-key-waits-for-someone-to-look)
//! 7. [A list is other clients' too](#a-list-is-other-clients-too)
//! 8. [What is the account's alone stays so](#what-is-the-accounts-alone-stays-so)
//! 9. [No relay is built in for a purpose](#no-relay-is-built-in-for-a-purpose)
//! 10. [Every address is somebody else's word](#every-address-is-somebody-elses-word)
//! 11. [Pictures are shown as they are](#pictures-are-shown-as-they-are)
//! 12. [It holds no money](#it-holds-no-money)
//! 13. [How the code is written](#how-the-code-is-written)
//! 14. [What is not there yet](#what-is-not-there-yet)
//!
//! # The rules, at a glance
//!
//! ```mermaid
//! mindmap
//!   root((Avestruz))
//!     The reader
//!       What is on show stays put
//!       Nothing above what is being read
//!       A row is the same height before and after
//!     The popup
//!       Closes at any click elsewhere
//!       Everything outlives it in the model
//!     The account
//!       Secrets in the keyring only
//!       The key is asked for when someone looks
//!       Lists are never written blind
//!     Privacy
//!       Bookmarks are private
//!       Drafts stay on this machine
//!       A search leaves no trace
//!       No relay or media server built in for a purpose
//!       A picture is sent without what it says beside the picture
//!     Other people
//!       Read from where they write
//!       Few relays, chosen
//!       Their addresses are their word
//!     The machine
//!       The store is a cache
//!       Memory is budgeted
//!       Nothing blocks the window
//! ```
//!
//! # What is on show is the reader's
//!
//! **Where a list is scrolled to never changes unless its reader changes
//! it.** Not when notes arrive, not when a picture loads, not when a link's
//! preview comes in, not when the popup is closed and opened again.
//!
//! A feed that moves under the eye can't be read, and a row that moves
//! under the pointer gets the wrong one clicked. Holding to this takes
//! something in almost every part of what is shown:
//!
//! ```mermaid
//! flowchart TD
//!     still["The rows on screen stay where they are"]
//!     above["Nothing goes in above them unasked"] --> still
//!     height["No row changes height after it is shown"] --> still
//!     kept["The place is kept while the list is not on show"] --> still
//!     pending["New notes wait behind a button"] --> above
//!     older["Older pages go in below"] --> above
//!     room["A picture's room is held before it arrives"] --> height
//!     card["A link's card is as tall asked as answered"] --> height
//!     quotes["Quoted notes are looked for before the note shows"] --> height
//!     measured["Heights are remembered by note, not by index"] --> kept
//!     cell["The scroll offset is in the model"] --> kept
//!     estimate["A row never shown has an estimate that depends only on its note and the width"] --> kept
//! ```
//!
//! | What would move the list | What keeps it still | Where |
//! |---|---|---|
//! | Notes arriving at the top | They wait, and are offered by a button | [`Pending`](crate::nostr::feed::Pending) |
//! | A picture arriving in a row | The row holds its room from the start, when the note says how large it is | [`MediaPlaceholder`](crate::views::components::media_placeholder) |
//! | A link's preview arriving | Its card is the same height before, after, and for every page | [`crate::views::components::link_preview`](mod@crate::views::components::link_preview) |
//! | A video being played | Its room is settled from what the note says of it, before anything of it is fetched, and what plays is fitted inside | [`box_size`](crate::video::box_size) |
//! | A quoted note arriving | It is looked for before the note quoting it is shown at all | [`crate::views::components::quoted_note`](mod@crate::views::components::quoted_note) |
//! | The popup closing | The offset and the measured heights are in the model | [`ScrollPosition`](crate::views::components::ScrollPosition), [`RowHeights`](crate::views::components::RowHeights) |
//! | Rows never yet shown | Each has an estimate that depends on nothing but itself, so it doesn't change as others are measured | [`crate::views::components::virtual_list`] |
//! | A bookmark taken off, on the bookmarks page | The note stays in its place with an empty ribbon | [`crate::views::bookmarks`] |
//! | A long note shown in full in another list | Which notes show in full is each list's own: no row grows in a list that is not on show | [`Expanded`](crate::state::Expanded) |
//!
//! The estimate is held to by a test: the estimated height of a row has
//! to stay close to what the row measures.
//!
//! Two places still move. A picture whose note doesn't say how large it
//! is grows its row when it loads. And a link that turned out to be a
//! video only when it was asked for its preview has a preview's card,
//! which gives way to the taller room of a video when it is played: at
//! its reader's pressing, and never by itself.
//!
//! A long note is the one row that changes height at its reader's
//! asking, and by a lot. Shown in full, it grows down from where it is:
//! what is above it stays, and the list is not scrolled. Cut again, it
//! would leave its reader looking at whatever was further down, if its
//! top had been scrolled out of view: so the list is scrolled back to
//! the top of the row, and only then. Either way the row is measured
//! before the list is laid out again, in view or not
//! ([`RowHeights::measure_anew`](crate::views::components::RowHeights::measure_anew)),
//! and the scroller is left at the place it shows: one told to go past
//! the end of a list that just got shorter would keep that place, and go
//! there by itself when the list next grew
//! ([`scroll_list_within`](crate::handlers::navigation::scroll_list_within)).
//!
//! # The popup takes nothing with it
//!
//! A panel applet's popup closes when anything else is clicked. Whatever
//! was only in a widget is gone then: so **nothing that matters is only in
//! a widget.** What was typed, where each list is, which pages are open,
//! what was changed on the relays page and not saved: all of it is in
//! [`AppModel`](crate::state::AppModel), and the widgets are given it back
//! when they are built again. See [`crate::guide::architecture`].
//!
//! This is also why there is one process and one model rather than a
//! window with a life of its own: the applet runs for as long as the
//! session does, and the popup is a view of it that comes and goes.
//!
//! # The store first, the relays after
//!
//! **Whatever can be shown from the store is shown from the store, at
//! once, and the relays are asked behind it.** A relay may take seconds
//! or never answer; the store answers within the frame.
//!
//! Every event a relay sends is kept, so the store is also what makes the
//! second look at anything immediate. It is a cache and is treated as
//! one: once a day it lets go of what is old and can be asked for again
//! ([`crate::nostr::sweep`]), and anything that needs an event the store
//! may have let go of asks the relays for it.
//!
//! There is no live subscription. New notes are looked for on a timer,
//! and wait to be asked for: a feed that fills itself as it is read is
//! the thing the first rule is against.
//!
//! # A secret is in the keyring, or nowhere
//!
//! An account's secret key, or the address of its remote signer, is in
//! the system keyring. **It is never in the config, the event store, a
//! log, or the model.** The config names the account by its `npub`.
//!
//! - With an `nsec`, the key is fetched from the keyring for each
//!   signature and let go of after: the applet runs all day, and holds no
//!   secret between one signature and the next.
//! - What is typed in the sign-in field is wrapped in a type that prints
//!   nothing ([`SecretInput`](crate::messages::SecretInput)), because
//!   messages are `Debug` and what is `Debug` ends up in a log.
//! - A key typed in the *search* field is cleared and sent nowhere.
//! - A remote signer knows this client by a key of the client's own, one
//!   for each signer and the same ever after, so that a signer told about
//!   it once keeps answering.
//!
//! # What takes the key waits for someone to look
//!
//! With a remote signer, every signature and every decryption is a
//! request to another machine, and may be a prompt on somebody's phone.
//! So **nothing that takes the key happens before the popup is open, and
//! nothing is decrypted twice.**
//!
//! | What takes the key | When |
//! |---|---|
//! | Reading the bookmarks | The popup opens |
//! | Reading the private half of the blocked and indexer relays | The popup opens |
//! | Reading the drafts | The popup first opens in a run |
//! | Reading the private half of the search relays | A search page opens |
//! | Reading the private half of the other lists of relays | The relays page opens |
//! | Keeping a draft | The composer is left with something new in it |
//! | Anything published | It is published |
//!
//! A list that was decrypted is kept as read while it is the newest, and
//! is not sent to the signer again.
//!
//! # A list is other clients' too
//!
//! The contact list, the bookmark list and the lists of relays are each
//! one event that every client the account uses reads and replaces. So:
//!
//! - **A list is asked of the relays again just before it is changed**,
//!   and the change is made to the newest.
//! - **Everything in it that is not being changed is carried over as it
//!   was**, known or not.
//! - **A first list is never written without saying so**: if no relay has
//!   one, the account is told that writing one replaces any kept
//!   elsewhere, and has to ask again.
//! - **And not at all if a relay didn't answer**: that relay may be the
//!   one that has the list.
//! - **A list whose private half can't be read is never written over**:
//!   written without it, the list would lose it.
//!
//! The cost is that a change sometimes doesn't happen, and says why. The
//! other way round, a follow list of years is one bad moment from being
//! replaced by a list of one.
//!
//! # What is the account's alone stays so
//!
//! - **Bookmarks are private.** They are written to the encrypted half of
//!   the bookmark list and read from there, and the public half is carried
//!   over unread. What someone bookmarks is nobody else's to know.
//! - **Drafts stay on this machine.** They are events, sent to a relay
//!   that runs inside the process and listens nowhere, and encrypted all
//!   the same in the store.
//! - **A search leaves no trace.** Nothing typed there is kept or logged,
//!   and what names a person or a note outright is opened without any
//!   relay being told.
//! - **A name at a domain is checked only where it is looked at.** A feed
//!   would have a hundred domains told who reads what.
//! - **Once the account has said where it is, nobody else is asked about
//!   everything.** The config's relays stand down.
//! - **Nothing of a video is fetched until it is played.** A picture is
//!   small and is what the note is; a video is large, has sound, and is
//!   decoded by far more code.
//! - **When the notifications were last looked at stays on this
//!   machine.** It is one of the settings, for each account, and is in
//!   no event: when someone reads what was written to them is nobody
//!   else's to know.
//! - **Which emoji are reacted with most stays on this machine.** It is
//!   one of the settings, and is in no event: the reactions themselves
//!   are public, a tally of them need not be.
//!
//! What can't be helped is said rather than hidden: every server a
//! picture, a preview or a video is fetched from sees the request, and
//! whatever can read the user's files can read the event store.
//!
//! # No relay is built in for a purpose
//!
//! The config names three large relays to start from, since an account
//! has to be looked for somewhere. Past that, **which relays search, and
//! which are asked for profiles nobody else has, is the account's to
//! say**, in its own lists. With none set, nothing is searched, and the
//! page says where to set them.
//!
//! A relay built in for searching would be a party every user's searches
//! are sent to because a developer chose it.
//!
//! The same goes for media servers: **where an account's files are kept
//! is the account's to say**, in its server list. With none set, nothing
//! can be attached, and the page they are set on opens instead and says
//! so. No list is written that the account did not ask for.
//!
//! And **a picture is sent as the picture and no more**: where, when,
//! with what and by whom it was made is left out of the file before it
//! goes anywhere, and a picture that can't be made bare is not sent
//! ([`crate::media`]).
//!
//! One site is built in all the same: the one a note's share link leads
//! to ([`SHARE_GATEWAY`](crate::nostr::share::SHARE_GATEWAY)). A link
//! that opens in any browser has to be to somebody's site. Avestruz
//! itself asks that site nothing: it is named in a link the user copies,
//! and hears of the note when whoever is given the link opens it.
//!
//! # Every address is somebody else's word
//!
//! Every URL this client fetches, and every relay it hears of from a list
//! or a note, was written by someone else, who may have meant harm.
//!
//! | What | The rule | Where |
//! |---|---|---|
//! | Anything fetched from the web | Only `https`, redirects included. A time limit. No more read than is kept | [`crate::http`] |
//! | A picture | A cap on the download and another on how large it may decode | [`crate::images`] |
//! | A linked page | Only its head, read with a tokenizer. Nothing is run | [`crate::previews`] |
//! | A YouTube link | Nothing of it reaches yt-dlp's command line but the video's id, once that is known to be eleven letters, figures, `-` or `_`. What yt-dlp finds is played only if it is `https` | [`VideoId`](crate::youtube::VideoId) |
//! | A video | Only `https`, and only when its reader plays it. GStreamer fetches and decodes it: the one thing not fetched by [`crate::http`], and not held to its limits | [`crate::video`] |
//! | A link that is opened | Only web links are handed to the desktop | [`crate::links`] |
//! | A relay somebody names | Only `wss`, on the open internet, not blocked | [`public`](crate::nostr::outbox::public) |
//! | An invoice | Only shown if it asks for the amount that was chosen | [`crate::nostr::zap`] |
//! | A name at a domain | Not shown until the domain itself has said it is theirs | [`crate::nostr::nip05`] |
//!
//! # The button and the window are two processes
//!
//! What the panel starts is a button and nothing else; the client is a
//! second process of the same program, which the button starts and talks
//! to over a pipe ([`crate::button`]).
//!
//! It was one process, an applet whose popup the panel placed under its
//! button. An applet's surfaces are the panel's, which is a compositor of
//! its own and takes no GPU buffers from its applets: an applet is drawn
//! by the CPU, and a list of pictures scrolled on a screen of many pixels
//! is more than a CPU draws in the time a frame has. A surface the window
//! puts on the session's compositor itself is drawn by the GPU.
//!
//! What it costs is that the panel no longer places the popup: the window
//! is told which panel its button is in and which part of it, not where,
//! so the popup is put against the panel's edge at the end the button is
//! nearer, and not exactly under it. And closing on a click elsewhere,
//! which the panel did, is the window's to do: the popup has the keyboard
//! while it is open, and closes when it loses it.
//!
//! What is copied in the popup has to outlive it: a link is copied to
//! be pasted somewhere else, and the click that goes there closes the
//! popup. The toolkit keeps what was copied with the first surface a
//! process opened, so the window opens one before any popup, which
//! shows nothing and stays
//! ([`keep_what_is_copied`](crate::state::AppModel::keep_what_is_copied)).
//! The same surface reaches from the top of the screen to the bottom,
//! and is what says how much room the screen has for the popup, and at
//! what scale it is drawn ([`crate::screen`]): nothing else is asked.
//!
//! # Pictures are kept at the size they are shown at
//!
//! A picture fills the width of its row, is as tall as that makes it, and
//! is never shown wider than it is, a point to its pixel. It is kept at
//! as many of the screen's pixels as that takes, and not as it came: made
//! that size once, when it is decoded, and from then on copied into each
//! frame pixel for pixel (see [`crate::images`] and
//! [`Picture`](crate::views::components::Picture)).
//!
//! It was kept as it came at first, and scaled by the renderer in every
//! frame. Measured on a feed of real notes, in a popup of 1088 by 1530
//! pixels, that was two thirds of what a frame of a scrolling list cost,
//! and six frames in ten took longer than a screen of 60 Hz waits for:
//! the renderer was a CPU one then, a photograph has many times the
//! pixels its row has, and rounding its corners copied all of them, every
//! frame. The GPU that draws the popup now scales a picture for next to
//! nothing, and the CPU renderer still draws it where there is no GPU. Nothing shows a picture
//! larger than its row, so the original is not kept at all.
//!
//! What bounds memory is that size, a limit on how large one picture may
//! decode, and a budget on the bytes kept, with the least recently shown
//! dropped first.
//!
//! Three kinds of picture are cut to a shape, in their own pixels, when
//! they are decoded: an avatar to its middle square and round, the
//! picture across a profile to a strip, and the one beside a link's
//! preview to a thumbnail. The CPU renderer does neither reliably at draw
//! time for a picture that isn't the shape it is shown in.
//!
//! # It holds no money
//!
//! Zapping gets as far as an invoice and shows it as a QR code. There is
//! no wallet in this client and no connection to one: a key that can
//! spend is a different kind of secret from a key that can post, and a
//! panel applet is not where it should live.
//!
//! # How the code is written
//!
//! | Rule | Kept by |
//! |---|---|
//! | Nothing in `src/` may panic: no `unwrap`, `expect`, `panic!`, or indexing | Clippy lints, denied in `Cargo.toml`; tests may |
//! | No wildcard imports | The same |
//! | No dependency under a licence that would bind the whole (the project is MIT) | `cargo deny`, with the allow list in `deny.toml` |
//! | No dependency that isn't used | `cargo machete` |
//! | One TLS crypto provider | A test: with two compiled in, the first connection panics |
//! | A view takes `&AppModel` | The types |
//! | An error is a value with a reason, and the reason is said to the user in their language | `fl!`, and one function for each kind of error that turns it into words |
//! | A message that carries a secret doesn't print it | [`SecretInput`](crate::messages::SecretInput) |
//! | Every module says what it is for, in its first lines | Read by whoever opens the file |
//! | A test is named for what it holds to be so | See [`crate::guide::testing`] |
//!
//! All of the first four, the formatting, and that these pages build, are
//! one command: `just check`.
//!
//! # What is not there yet
//!
//! Decided against, for now or for good:
//!
//! - **A global feed.** The feed is who the account follows.
//! - **A wallet**, as above.
//! - **Public bookmarks.**
//! - **A standalone window.** It is a panel applet; a window of its own
//!   may come.
//!
//! Not built yet:
//!
//! - Direct messages, long-form articles.
//! - Reading the feed from the account's inbox relays; they are asked
//!   what names the account, and no more.
//! - Notifications of new followers, and from the desktop while the
//!   popup is closed, beyond the dot on the panel button.
//! - Reposts by the people followed, in the feed.
//! - A live subscription, in place of looking on a timer.
//! - Older pages of a feed read from the store when no relay answers.
//! - Pictures kept on disk; audio and live streams; animated pictures.
//! - A video larger than its row: full screen, or a window of its own.
//! - YouTube in more than its smallest size, which takes playing
//!   pictures and sound from two streams; its playlists.
//! - Hashtags that open anything.
//! - Taking what a video says of where it was made out of it before it
//!   is attached; a thumbnail for a picture attached (`thumbhash`,
//!   `blurhash`); dropping a file on the composer; deleting a file from
//!   a media server.
//! - Showing a picture the way up its EXIF says it is.
//! - Custom emoji (NIP-30), to react with or shown in a reaction; skin
//!   tones to choose from where all emoji are; taking a reaction back.
//! - Drawing the colour glyphs that are gradients ([`crate::fonts`]):
//!   where the system's emoji font has only those, emoji are in colour
//!   once a font with bitmaps is put among the applet's own.
//! - Verifying the zap receipts counted under other people's notes. The
//!   ones the account is told of are: see
//!   [`crate::nostr::notifications`].
//! - Any language but English.

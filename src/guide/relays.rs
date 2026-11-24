// SPDX-License-Identifier: MIT

//! Which relays there are, which of them is asked or told what, and whose
//! word is taken for where a relay is. Documentation only.
//!
//! A Nostr client is mostly a set of decisions about relays: which to
//! connect to, which to ask about whom, where to send what, and what to
//! make of a relay somebody else names. The code that carries those
//! decisions out is [`crate::nostr::client`], with the choosing in
//! [`crate::nostr::outbox`] and the account's lists in
//! [`crate::nostr::relay_lists`]. This page is the decisions themselves.
//!
//! 1. [Three sets of relays](#three-sets-of-relays)
//! 2. [Whose relays are the account's own](#whose-relays-are-the-accounts-own)
//! 3. [Reading other people](#reading-other-people)
//! 4. [The life of one of other people's relays](#the-life-of-one-of-other-peoples-relays)
//! 5. [Where what is written goes](#where-what-is-written-goes)
//! 6. [The account's lists](#the-accounts-lists)
//! 7. [Whose word is taken](#whose-word-is-taken)
//! 8. [What a relay says of itself](#what-a-relay-says-of-itself)
//! 9. [The numbers](#the-numbers)
//!
//! # Three sets of relays
//!
//! ```mermaid
//! flowchart TB
//!     subgraph service["NostrService"]
//!         home["The home pool"]
//!         others["The pool of other people's relays"]
//!         private["The relay Avestruz runs itself"]
//!         store[("One event store")]
//!     end
//!     config["Relays the config names"] --> home
//!     outbox["Relays the account's relay list says it writes to"] --> home
//!     theirlists["Relays other people's relay lists name"] --> others
//!     hints["Relays the address of a note names"] --> others
//!     indexers["The account's indexer relays"] --> others
//!     search["The account's search relays"] --> others
//!     home <--> store
//!     others --> store
//!     private <--> store
//!     pipe["A pipe inside the process"] --- private
//! ```
//!
//! | | The home pool | The pool of other people's | The relay it runs itself |
//! |---|---|---|---|
//! | Which relays | The account's own: the config's, then the ones its relay list says it writes to | The ones other people's relay lists name, the ones a note's address names, and the account's indexer and search relays | One, over the same store |
//! | Asked about | Everything | The people who list each, and nobody else | The account's drafts |
//! | Sent | Everything the account writes | What is meant for the people who read there | Drafts, and nothing else |
//! | Connected | For as long as the applet runs | When something is asked of them, and for a while after | It listens nowhere: a connection is made in memory |
//! | How many | The config's, and up to [`ACCOUNT_RELAYS_MAX`] of the list's | Up to [`REACH_CAP`] for one question, and as many kept | One |
//!
//! Every relay of both pools sends into one store, so a note that came
//! from anywhere is there for everything.
//!
//! # Whose relays are the account's own
//!
//! An account says where it writes in its relay list (NIP-65, kind
//! 10002). Until that list is known, and for an account that has none,
//! the relays the config names are where it is looked for.
//!
//! ```mermaid
//! stateDiagram-v2
//!     state "The config's relays are in use" as Config
//!     state "The account's outbox relays are in use" as Listed
//!     [*] --> Config: the applet starts
//!     Config --> Config: the account has no relay list, or none of its relays connects
//!     Config --> Listed: one of the relays its list says it writes to is connected
//!     Listed --> Config: signed out, or the list names none any more
//! ```
//!
//! While the account's outbox relays are in use, the config's relays
//! *stand down*: they are not asked and not written to, and are hung up
//! on a little later. Except those on this machine or this network, which
//! stay in use: no relay list would name them, and they are there to be
//! used.
//!
//! The reason for standing down is that the config's relays are a guess.
//! Once the account has said where it is, asking three large public
//! relays about everything as well is three more parties told what the
//! account reads, for notes its own relays have.
//!
//! Only the relays the list says the account *writes to* become its own.
//! The ones it says it reads are where other people send what is meant
//! for it, and are not read from here yet.
//!
//! Where: the `Own` struct in [`crate::nostr::client`], and
//! [`NostrService::use_relays_of`].
//!
//! # Reading other people
//!
//! The people an account follows don't all write to the account's relays.
//! Each says where they write in a relay list of their own, and their
//! notes are read from there as well (the *outbox model*).
//!
//! Everyone names several relays and everyone's differ, so asking all of
//! them would be a connection to half the network. [`plan`] picks few.
//!
//! ```mermaid
//! flowchart TD
//!     people["The people a question is about"] --> lists["Each one's relay list, from the store"]
//!     lists --> stale{"Asked about within the hour?"}
//!     stale -->|"no"| refresh["The account's relays, and its indexer relays, are asked for the lists"]
//!     stale -->|"yes"| filter
//!     refresh --> filter["Relays that may not be connected to are dropped"]
//!     filter --> short["Each person is to be had from 2 relays, counting the account's own"]
//!     short --> pick["Take the relay that helps the most people who are still short"]
//!     pick --> more{"Anyone still short, and fewer than 20 taken?"}
//!     more -->|"yes"| pick
//!     more -->|"no"| connect["Connect to those taken. One that doesn't connect is left alone for ten minutes"]
//!     connect --> ask["Each is asked about the people who list it, and only them"]
//! ```
//!
//! An example. Five people are followed, the account's own relay is `H`,
//! and each is to be had from two relays:
//!
//! | Person | Writes to | Short of |
//! |---|---|---|
//! | Ana | `H`, `A` | 1 |
//! | Bia | `A`, `B` | 2 |
//! | Caio | `A`, `C` | 2 |
//! | Davi | `B`, `D` | 2 |
//! | Eva | `H`, `E`, `F` | 1 |
//!
//! ```mermaid
//! flowchart LR
//!     subgraph taken["Taken, in this order"]
//!         A["A: Ana, Bia, Caio"] --> B["B: Bia, Davi"] --> C["C: Caio"] --> D["D: Davi"] --> E["E: Eva"]
//!     end
//!     subgraph not["Not taken"]
//!         F["F: Eva has her two"]
//!     end
//!     H["H: the account's own, asked about everyone anyway"]
//! ```
//!
//! `A` helps three people and goes first. `B` helps two. Then `C`, `D`
//! and `E` each help one who is still short, and `F` helps nobody: Eva
//! has `H` and `E`. Six relays were named, five are asked, and each is
//! asked only about the people who list it.
//!
//! The same people and lists always give the same relays in the same
//! order, so the choice doesn't wander from one question to the next.
//!
//! This example is also a test
//! (`the_plan_the_guide_works_through_is_the_plan_that_is_made`, in
//! [`crate::nostr::outbox`]), so that it stays what the code does.
//!
//! The same planning serves, with other lists, for everything asked about
//! other people:
//!
//! | Asked | Of the relays | Besides the account's own |
//! |---|---|---|
//! | The feed, a profile's notes | The authors write to | |
//! | A profile | Its person writes to | And the account's indexer relays |
//! | A thread | The note's author, and the first few people it names, write to or read | And the relay the note's address named |
//! | A quoted note, a bookmarked note | The author writes to, if who that is is known | And the relay its address or its bookmark named |
//! | Someone's relay list | | The account's indexer relays |
//!
//! What other people's relays send is always *on top of* what the
//! account's own had: if none of them is reached, that is no error.
//!
//! Where: [`plan`], [`NostrService::fetch_feed`], and the private
//! `relays_of`, `reach_for` and `ask_their_relays` beside it.
//!
//! # The life of one of other people's relays
//!
//! ```mermaid
//! stateDiagram-v2
//!     [*] --> Planned: a relay list or an address names it, and a plan takes it
//!     Planned --> Connected: it connects in time
//!     Planned --> Avoided: it doesn't, and is taken out of the pool again
//!     Connected --> Connected: asked, and it answers
//!     Connected --> Avoided: it says no, or doesn't answer in time
//!     Connected --> Asleep: nothing asked of it for two minutes
//!     Asleep --> Connected: something is asked of it
//!     Asleep --> [*]: unused for a quarter of an hour, or room is needed
//!     Avoided --> Planned: ten minutes on, a plan may take it again
//! ```
//!
//! A relay that is avoided is left alone for ten minutes: plans are made
//! without it, and the others are asked.
//!
//! A relay of other people's gets less time than the account's own, to
//! connect and to answer: it is asked for what may well be had already,
//! and a list of notes waits for the slowest of them.
//!
//! # Where what is written goes
//!
//! ```mermaid
//! flowchart LR
//!     event["A signed event"] --> kind{"What is it?"}
//!     kind -->|"a note, a comment, a repost, a reaction"| both["The account's relays, and the relays the people it names read"]
//!     kind -->|"a contact list, a bookmark list, a list of relays"| own["The account's relays"]
//!     kind -->|"a relay list"| wide["The account's relays, as they were before and as they are now"]
//!     kind -->|"a draft"| private["The relay Avestruz runs itself, and no other"]
//!     kind -->|"a zap request"| nowhere["No relay: it goes to the Lightning address, inside the request for an invoice"]
//! ```
//!
//! | What | Kind | Goes to |
//! |---|---|---|
//! | A note, a reply, a quote | 1 | The account's relays, and two of the relays each person it names reads |
//! | A comment | 1111 | The same |
//! | A repost | 6, 16 | The same |
//! | A reaction | 7 | The same |
//! | The contact list | 3 | The account's relays |
//! | The bookmark list | 10003 | The account's relays |
//! | The relay list | 10002 | The account's relays, the ones it wrote to before as well as the ones it writes to now |
//! | The other lists of relays | 10006, 10007, 10050, 10086, 10089 | The account's relays |
//! | A draft | 31234 | The relay Avestruz runs itself |
//!
//! Publishing succeeds when one relay took the event. Sending to the
//! people an event names is on top of that: if none of their relays takes
//! it, it is still on the account's own, where anyone can find it.
//!
//! Where: [`NostrService::publish`], with the private `send_home` and
//! `deliver` beside it.
//!
//! # The account's lists
//!
//! The relays page shows seven sets of relays, which are six lists. An
//! account's relay list holds two of them.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph k10002["Relay list, kind 10002"]
//!         outbox["Outbox: r tags marked write, or unmarked"]
//!         inbox["Inbox: r tags marked read, or unmarked"]
//!     end
//!     subgraph nip51["Lists of relay tags, public or kept private in the content"]
//!         dm["Direct messages, kind 10050"]
//!         search["Search, kind 10007"]
//!         indexer["Indexer, kind 10086"]
//!         trusted["Trusted, kind 10089"]
//!         blocked["Blocked, kind 10006"]
//!     end
//!     outbox --> use1["The account's own relays"]
//!     search --> use2["The relays a search asks"]
//!     indexer --> use3["Asked for profiles and relay lists the account's own don't have"]
//!     blocked --> use4["Never connected to on somebody else's word"]
//!     inbox --> kept["Kept in the list for other clients to go by"]
//!     dm --> kept
//!     trusted --> kept
//! ```
//!
//! | Set | List | Said to anyone? | What Avestruz does with it | What it keeps to itself is read |
//! |---|---|---|---|---|
//! | Outbox | 10002 | Yes | These are the account's own relays | It keeps nothing to itself |
//! | Inbox | 10002 | Yes | Shown and changed. Other people send here | It keeps nothing to itself |
//! | Direct messages | 10050 | Yes | Shown and changed | It keeps nothing to itself |
//! | Search | 10007 | Either | The relays a search asks | When a search opens |
//! | Indexer | 10086 | Either | Where profiles and relay lists are looked up | When the popup opens |
//! | Trusted | 10089 | Either | Shown and changed | On the relays page |
//! | Blocked | 10006 | Either | Never connected to because a list or an address names them | When the popup opens |
//!
//! "Either" is a list that names relays in its tags, for anyone, or among
//! the tags it keeps to itself, encrypted in its content. Reading the
//! second takes the account's key, which may mean asking a person, so
//! each list's private half is read when it is first needed and not
//! before.
//!
//! There are no built-in search or indexer relays. An account that has
//! set none searches nowhere, and the page says where to set them.
//!
//! Where: [`crate::nostr::relay_lists`], [`crate::handlers::relays`],
//! [`crate::views::relays`].
//!
//! # Whose word is taken
//!
//! Most relays this client hears of, it hears of from somebody else: a
//! relay list anyone could have written, the address of a note in
//! somebody's text. Connecting wherever such a thing points would let
//! anyone make this machine knock on any door.
//!
//! ```mermaid
//! flowchart TD
//!     named["A relay is named"] --> who{"By whom?"}
//!     who -->|"the config, or the account's own list"| yes["Connected to"]
//!     who -->|"someone's relay list, a note's address, a bookmark"| wss{"wss, on the open internet?"}
//!     wss -->|"no: unencrypted, on this machine or network, or an onion service"| no["Not connected to"]
//!     wss -->|"yes"| blocked{"On the account's blocked list?"}
//!     blocked -->|"yes"| no
//!     blocked -->|"no"| failed{"Failed in the last ten minutes?"}
//!     failed -->|"yes"| later["Not this time"]
//!     failed -->|"no"| yes
//! ```
//!
//! The test is [`public`]. A relay on this machine or network is refused
//! when somebody else names it, since a list could name one to have its
//! readers knock on doors at home; but one the *config* names stays in
//! use, since whoever wrote the config meant it.
//!
//! # What a relay says of itself
//!
//! A relay answers over `https`, at its own address, with what it is and
//! what it does (NIP-11). The relays page asks the relays of the sets
//! that need something of a relay, and says under each what gets in the
//! way of what its set is for ([`Section::hindrances`]):
//!
//! | Set | What gets in the way | Why |
//! |---|---|---|
//! | Search | It names the NIPs it supports, and not NIP-50 | It is asked to search |
//! | Direct messages | It names the NIPs it supports, and not NIP-42 | It should give messages only to whom they are for, which takes knowing who asks |
//! | Direct messages, Inbox | It restricts who may write, or has to be paid | Other people write there to reach the account |
//! | Inbox | Whoever connects has to say who they are first | A client that doesn't can't write there |
//! | Outbox | Whoever connects has to say who they are first | A client that doesn't can't read the account's notes there |
//!
//! This is the relay's own word. One that doesn't answer, or names no
//! NIPs, is not said anything of.
//!
//! Where: [`crate::nostr::relay_info`], [`Section::hindrances`].
//!
//! # The numbers
//!
//! Each links the constant, which is what counts.
//!
//! | What | Value | Constant |
//! |---|---|---|
//! | Notes in one page of a feed | 50 | [`FEED_PAGE`] |
//! | Relays of the account's list that join its own | 8 | [`ACCOUNT_RELAYS_MAX`] |
//! | Relays each person is read from, the account's own among them | 2 | [`RELAYS_PER_PERSON`] |
//! | Other people's relays one question reaches for | 20 | [`REACH_CAP`] |
//! | An own relay's time to answer | 10 s | [`FETCH_TIMEOUT`] |
//! | One of other people's relays' time to connect, and to answer | 2 s, 6 s | [`REACH_CONNECT_WAIT`], [`REACH_TIMEOUT`] |
//! | A relay that failed is left alone for | 10 min | [`REACH_AVOID`] |
//! | One of other people's relays is hung up on after | 2 min idle | [`REACH_SLEEP`] |
//! | And taken out of the pool after | 15 min unused | [`REACH_KEEP`] |
//! | Someone's relay list is asked for again after | 1 h | [`RELAY_LISTS_FRESH`] |
//! | A search relay's time to answer | 8 s | [`SEARCH_TIMEOUT`] |
//! | A relay's time to take what is published | 10 s | [`PUBLISH_TIMEOUT`] |
//! | People of a conversation whose relays are asked about it | 5 | [`THREAD_PEOPLE`] |
//!
//! [`plan`]: crate::nostr::outbox::plan
//! [`public`]: crate::nostr::outbox::public
//! [`Section::hindrances`]: crate::nostr::relay_lists::Section::hindrances
//! [`NostrService::use_relays_of`]: crate::nostr::client::NostrService::use_relays_of
//! [`NostrService::fetch_feed`]: crate::nostr::client::NostrService::fetch_feed
//! [`NostrService::publish`]: crate::nostr::client::NostrService::publish
//! [`FEED_PAGE`]: crate::nostr::client::FEED_PAGE
//! [`ACCOUNT_RELAYS_MAX`]: crate::nostr::client::ACCOUNT_RELAYS_MAX
//! [`RELAYS_PER_PERSON`]: crate::nostr::client::RELAYS_PER_PERSON
//! [`REACH_CAP`]: crate::nostr::client::REACH_CAP
//! [`FETCH_TIMEOUT`]: crate::nostr::client::FETCH_TIMEOUT
//! [`REACH_CONNECT_WAIT`]: crate::nostr::client::REACH_CONNECT_WAIT
//! [`REACH_TIMEOUT`]: crate::nostr::client::REACH_TIMEOUT
//! [`REACH_AVOID`]: crate::nostr::client::REACH_AVOID
//! [`REACH_SLEEP`]: crate::nostr::client::REACH_SLEEP
//! [`REACH_KEEP`]: crate::nostr::client::REACH_KEEP
//! [`RELAY_LISTS_FRESH`]: crate::nostr::client::RELAY_LISTS_FRESH
//! [`SEARCH_TIMEOUT`]: crate::nostr::client::SEARCH_TIMEOUT
//! [`PUBLISH_TIMEOUT`]: crate::nostr::client::PUBLISH_TIMEOUT
//! [`THREAD_PEOPLE`]: crate::nostr::client::THREAD_PEOPLE

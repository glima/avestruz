// SPDX-License-Identifier: MIT

//! Which NIPs are spoken and which kinds are read and written, and where
//! in the code each is. Documentation only.
//!
//! Nostr is specified in pieces, the NIPs, and a client is the set of
//! pieces it took. This page is that set for Avestruz: for someone who
//! knows the protocol and wants to find the code, or who knows the code
//! and wants to know what another client will make of what it writes.
//!
//! 1. [The kinds](#the-kinds)
//! 2. [The NIPs](#the-nips)
//! 3. [What an event goes through](#what-an-event-goes-through)
//! 4. [What is written, and what is only read](#what-is-written-and-what-is-only-read)
//! 5. [What other clients find](#what-other-clients-find)
//!
//! # The kinds
//!
//! ```mermaid
//! flowchart LR
//!     subgraph said["What people say"]
//!         k1["1: a note, a reply, a quote"]
//!         k1111["1111: a comment"]
//!     end
//!     subgraph done["What is done to what they say"]
//!         k6["6, 16: a repost"]
//!         k7["7: a reaction"]
//!         k9735["9735: a zap receipt"]
//!     end
//!     subgraph who["Who someone is, and where"]
//!         k0["0: a profile"]
//!         k3["3: who they follow"]
//!         k10002["10002: where they write and read"]
//!     end
//!     subgraph own["What the account keeps"]
//!         k10003["10003: bookmarks"]
//!         lists["10006, 10007, 10050, 10086, 10089: lists of relays"]
//!         k31234["31234: drafts"]
//!     end
//!     done --> said
//!     said --> who
//!     own --> who
//! ```
//!
//! | Kind | What | Read | Written | Where |
//! |---|---|---|---|---|
//! | 0 | A profile | Yes | No | [`Profile`](crate::nostr::models::Profile) |
//! | 1 | A text note: a note, a reply, a quote | Yes | Yes | [`crate::nostr::compose`], [`Note`](crate::nostr::models::Note) |
//! | 3 | A contact list: who an account follows | Yes | Yes | [`crate::nostr::contacts`] |
//! | 6, 16 | A repost of a note, of anything else | Counted, and told of when it is of the account's note | Yes | [`crate::nostr::engagement`], [`crate::nostr::notifications`] |
//! | 7 | A reaction | Counted, and told of when it is to the account's note | Yes: a like, or an emoji | [`crate::nostr::engagement`], [`crate::nostr::notifications`] |
//! | 1111 | A comment | Yes | Yes | [`crate::nostr::compose`] |
//! | 9734 | A zap request | No | Signed, and sent to no relay | [`crate::nostr::zap`] |
//! | 9735 | A zap receipt | Counted, and told of when the account's wallet service signed it | No | [`crate::nostr::engagement`], [`crate::nostr::notifications`] |
//! | 10002 | A relay list | Yes, everyone's | Yes | [`crate::nostr::outbox`], [`crate::nostr::relay_lists`] |
//! | 10003 | A bookmark list | Its private half | Its private half | [`crate::nostr::bookmarks`] |
//! | 10006 | Blocked relays | Yes | Yes | [`crate::nostr::relay_lists`] |
//! | 10007 | Search relays | Yes | Yes | [`crate::nostr::relay_lists`] |
//! | 10050 | The relays direct messages go to | Yes | Yes | [`crate::nostr::relay_lists`] |
//! | 10063 | A media server list (BUD-03) | Yes | Yes | [`crate::nostr::blossom`] |
//! | 10086 | Indexer relays | Yes | Yes | [`crate::nostr::relay_lists`] |
//! | 10089 | Trusted relays | Yes | Yes | [`crate::nostr::relay_lists`] |
//! | 24242 | An authorization to send a file to media servers (BUD-11) | No | Signed, and sent to no relay: it goes to the media servers, with the file | [`crate::nostr::blossom`] |
//! | 31234 | A draft wrap | Yes | Yes, to the relay it runs itself alone | [`crate::nostr::kept_drafts`] |
//!
//! A feed shows kinds 1 and 1111. Anything else that a note quotes or a
//! thread is about (a picture, an article) shows as what it says, as
//! plain text, with a comment under it being a comment.
//!
//! # The NIPs
//!
//! | NIP | What it is | What Avestruz does with it | Where |
//! |---|---|---|---|
//! | 01 | Events, filters, relays | Everything | [`crate::nostr::client`] |
//! | 02 | Contact lists | The feed is who the account follows. Following and unfollowing rewrite the list | [`crate::nostr::contacts`] |
//! | 04 | Encryption, the old way | Read, in lists older clients wrote. Never written | [`crate::nostr::account`] |
//! | 05 | Names at a domain | Checked with the domain when a profile is open, and resolved when one is typed in a search | [`crate::nostr::nip05`] |
//! | 10 | Replies to text notes | Threads are put in order by it, and replies are tagged by it | [`crate::nostr::thread`], [`crate::nostr::compose`] |
//! | 11 | What a relay says of itself | Asked on the relays page, and for a relay typed in a search | [`crate::nostr::relay_info`] |
//! | 17 | Private direct messages | Only the list of relays they go to (kind 10050) is shown and changed. Messages are not read or written | [`crate::nostr::relay_lists`] |
//! | 18 | Reposts and quotes | Reposts are counted and made. A quote is a note with another's address in it | [`crate::nostr::engagement`], [`crate::nostr::compose`] |
//! | 19 | Addresses: `npub`, `nsec`, `note`, `nevent`, `nprofile` | Read wherever one is typed or written in a note, relay hints included. Written in a quote, and in a note's share link, which names the relays it was seen on | [`crate::nostr::content`], [`crate::nostr::search`], [`crate::nostr::account`], [`crate::nostr::share`] |
//! | 21 | `nostr:` links | Read in the text of notes | [`crate::nostr::content`] |
//! | 22 | Comments | Read in threads. A reply to anything but a text note is one | [`crate::nostr::compose`] |
//! | 25 | Reactions | Counted, whatever they say. A like is a `+`; any emoji Unicode has can be reacted with. The account's own shows as what it says, if that is an emoji, and as a like otherwise: a custom emoji (NIP-30) is one of those | [`crate::nostr::engagement`], [`crate::emoji`] |
//! | 37 | Draft wraps | How drafts are kept. The list of private relays (kind 10013) is not used | [`crate::nostr::kept_drafts`] |
//! | 42 | Authentication to a relay | Avestruz does not authenticate to relays. It looks for NIP-42 in what a relay says of itself, on the relays page | [`crate::nostr::relay_lists`] |
//! | 44 | Encryption | What the account keeps to itself: private bookmarks, private relays in a list, drafts | [`crate::nostr::account`] |
//! | 46 | Remote signing | One of the three ways to sign in | [`crate::nostr::account`] |
//! | 50 | Search | The search page | [`crate::nostr::search`] |
//! | 51 | Lists | The bookmark list and the lists of relays | [`crate::nostr::bookmarks`], [`crate::nostr::relay_lists`] |
//! | 57 | Zaps | Zap requests are made, receipts are counted and waited for. A receipt for a zap to the account is checked against the key its Lightning address names, before it is told of | [`crate::nostr::zap`], [`crate::nostr::notifications`] |
//! | 65 | Relay lists | Where the account writes, where everyone else is read from, and where what names the account is looked for: the relays it reads | [`crate::nostr::outbox`] |
//! | 70 | Protected events | A note whose author asked that it go nowhere they didn't send it is reposted without being carried whole | [`crate::nostr::engagement`] |
//! | 92 | `imeta` tags | Read for a picture's size, so that its room is held before it arrives; and for which links are videos, their proportions and the picture that stands for them. Written for each file attached to a note: its kind, hash, size, and a picture's width and height | [`crate::nostr::content`], [`crate::nostr::blossom`] |
//!
//! Beside the NIPs, zapping speaks LNURL-pay to a Lightning address
//! (LUD-06 and LUD-16), over `https`.
//!
//! And attaching a file speaks Blossom to the account's media servers,
//! over `https`: the file is sent (BUD-02) after asking whether it would
//! be taken (BUD-06), the other servers are asked to fetch it (BUD-04),
//! each request is authorized by an event the account signs (BUD-11),
//! and which servers they are is the account's server list (BUD-03).
//! Media servers of other kinds (NIP-96, NIP-98) are not spoken to. See
//! [`crate::nostr::blossom`].
//!
//! # What an event goes through
//!
//! ```mermaid
//! flowchart LR
//!     relay["A relay sends an event"] --> checked["Its id and signature are checked, by the relay library"]
//!     checked --> kept[("Kept in the store")]
//!     kept --> queried["Asked for, by a filter"]
//!     queried --> model["Made into what a view shows"]
//!     model --> row["A row"]
//!     written["Something is written"] --> unsigned["An unsigned event"]
//!     unsigned --> signed["Signed: by the key from the keyring, or by a remote signer"]
//!     signed --> sent["Sent to relays"]
//!     sent -->|"once a relay took it"| kept
//! ```
//!
//! An event is never shown straight from a relay. It goes into the store
//! and what is shown is asked of the store, so what was checked once is
//! what is shown, and what is shown is there the next time.
//!
//! What a view shows is not an event either, but a model made from one:
//! a [`Note`](crate::nostr::models::Note) has its text taken apart into
//! what is plain, what is a link, who is mentioned and what is quoted
//! ([`crate::nostr::content`]), once, when it comes in. A row is built
//! from that many times a second and parses nothing.
//!
//! Going the other way, the modules that make events make them *unsigned*
//! and know nothing of keys: [`crate::nostr::compose`],
//! [`crate::nostr::contacts`], [`crate::nostr::bookmarks`],
//! [`crate::nostr::relay_lists`], [`crate::nostr::engagement`] and
//! [`crate::nostr::kept_drafts`] are pure functions from what is meant to
//! an `EventBuilder`. Signing is one place
//! ([`Signer`](crate::nostr::account::Signer)), and sending is another
//! ([`NostrService`](crate::nostr::client::NostrService)).
//!
//! # What is written, and what is only read
//!
//! Avestruz writes little, on purpose: what the account asked for, and
//! nothing on its own.
//!
//! - It never writes a profile (kind 0).
//! - It never writes a public bookmark, and never reads one: what is
//!   bookmarked here is in the half of the list that is encrypted.
//! - It never writes a list it could not read whole, and never writes a
//!   first list without saying so (see
//!   [`crate::guide::flows`]).
//! - It never sends a draft to any relay but the one it runs itself.
//! - It never sends what was typed in the sign-in field or the search
//!   field anywhere, if it is a key.
//!
//! # What other clients find
//!
//! The lists are shared with every other client the account uses, so they
//! are written the way the clients that keep the most in them write them:
//!
//! - a bookmark goes into the private half of kind 10003, after the
//!   others, as an `e` tag that may say after the id where the note is;
//! - a relay stays where its list had it, public or private, and a new
//!   one goes where lists of that kind usually keep theirs;
//! - a private half is written with NIP-44 and read with NIP-44 or NIP-04,
//!   so a list an older client left can be read, and is written back the
//!   new way;
//! - whatever a list holds that this client doesn't know is carried over
//!   to the next list untouched.

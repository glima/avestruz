// SPDX-License-Identifier: MIT

//! How it is tested, what is not, and every test by name. Documentation
//! only.
//!
//! 1. [How much](#how-much)
//! 2. [Where the tests are](#where-the-tests-are)
//! 3. [What a test is named](#what-a-test-is-named)
//! 4. [What the tests stand on](#what-the-tests-stand-on)
//! 5. [What `just check` holds to](#what-just-check-holds-to)
//! 6. [Fuzzing](#fuzzing)
//! 7. [What is not tested](#what-is-not-tested)
//! 8. [Trying it by hand](#trying-it-by-hand)
//! 9. [Every test](#every-test)
//!
//! # How much
//!
#![doc = include_str!(concat!(env!("OUT_DIR"), "/test_summary.md"))]
//! The number is counted from the sources when the crate is built (see
//! `build.rs`), so it is the number there are. Two of them are held
//! against the real system keyring, with throwaway keys that are taken
//! out again however the test ends: they run wherever a keyring answers,
//! and have nothing to hold to where none does.
//!
//! ```sh
//! just test           # all of them
//! just check          # everything else that is held to: see below
//! just coverage       # which lines the tests run, as pages (cargo-llvm-cov)
//! just fuzz           # what reads untrusted input, fed whatever a fuzzer makes up
//! ```
//!
//! # Where the tests are
//!
//! Every test is beside the code it tests, in a `tests` module at the
//! foot of the file. There is no `tests/` directory: what is tested is
//! mostly not public in any sense that matters, and a test beside the
//! code can reach what is private to it.
//!
//! ```mermaid
//! flowchart TB
//!     subgraph pure["Pure: something in, something out"]
//!         events["Events made and read: compose, content, thread, feed, bookmarks, contacts, relay lists, drafts"]
//!         choosing["Choosing: which relays, which rank, what was typed"]
//!         layout["Layout: row heights, the virtual list, what a picture takes"]
//!     end
//!     subgraph inprocess["Against something real, inside the test process"]
//!         relays["The relay client, against relays run in the test"]
//!         signer["Signing in, against a remote signer run in the test"]
//!         lmdb["The event store, in a directory of its own"]
//!     end
//!     subgraph outside["Against the machine"]
//!         keyring["The keyring round trip: wherever a keyring answers"]
//!     end
//!     pure --> inprocess --> outside
//! ```
//!
//! Most tests are of the first kind, because most of the code is: the
//! modules that make and read events do no I/O (see
//! [`crate::guide::protocol`]), so they are tested by giving them an event
//! and looking at what comes out.
//!
//! | What | How it is tested | Where |
//! |---|---|---|
//! | Making events: notes, replies, comments, quotes, lists, drafts | The unsigned event is built and its tags looked at | [`crate::nostr::compose`], [`crate::nostr::contacts`], [`crate::nostr::bookmarks`], [`crate::nostr::relay_lists`], [`crate::nostr::kept_drafts`] |
//! | Attaching a file: what a picture's file is left with, the server list, the authorization | Files are put together in the test with everything a camera and an editor leave in them, made bare, and decoded | [`crate::media`], [`crate::nostr::blossom`] |
//! | Sending a file to media servers | Servers that listen on this machine, each behaving its own way: taking the file, refusing it, not knowing the question, reading the authorization the old way, changing the file | [`crate::nostr::blossom`] |
//! | Reading events: text taken apart, threads put in order, counts | An event is made and signed in the test, and read | [`crate::nostr::content`], [`crate::nostr::models`], [`crate::nostr::thread`], [`crate::nostr::engagement`] |
//! | Lists that stay put: what waits, what joins up, what leaves a hole | Pages are offered to a list, and what it shows looked at | [`crate::nostr::feed`] |
//! | Which relays are asked | The plan for given people and lists | [`crate::nostr::outbox`] |
//! | The relay client: reading, publishing, lists changed safely | Against relays started in the test | [`crate::nostr::client`] |
//! | Signing in three ways, signing, encrypting | Against a remote signer started in the test | [`crate::nostr::account`] |
//! | The store letting go of what is old | Events saved, swept, and counted | [`crate::nostr::sweep`] |
//! | What is fetched from the web: caps, what is read of a page | Parsing, with no network | [`crate::http`], [`crate::previews`], [`crate::nostr::nip05`], [`crate::nostr::zap`], [`crate::nostr::relay_info`] |
//! | Pictures: the budget, the shapes, what is dropped first | Pictures made in the test | [`crate::images`] |
//! | Rows: how tall one is estimated, against how tall it measures | Rows laid out without a window | [`crate::views::components::note_row`](mod@crate::views::components::note_row) |
//! | The list: which rows are built, where they sit, what a jump does | The arithmetic of the list, without a window | [`crate::views::components::virtual_list`] |
//!
//! # What a test is named
//!
//! A test is named for what it holds to be so, as a sentence:
//! `a_note_the_store_let_go_of_is_asked_of_the_relays_again`,
//! `a_list_that_cannot_be_read_is_not_written_over`,
//! `the_estimate_is_close_to_the_measured_row`. Not for the function it
//! calls, and not `test_` anything.
//!
//! Read one after the other, the names are an account of what the code
//! is held to, which is how [the list at the foot of this
//! page](#every-test) is meant to be read. A name that can't be written
//! as a sentence that is true or false is usually a test of nothing in
//! particular.
//!
//! A test that fails says which sentence stopped being true.
//!
//! # What the tests stand on
//!
//! Nothing outside the test process, but for the two keyring tests. Each
//! test that needs one makes its own, and takes it away after:
//!
//! ```mermaid
//! flowchart LR
//!     test["A test of the relay client"] --> helper["with_relays: so many relays, and a client whose own relay is the first"]
//!     helper --> relays["LocalRelay, on a port of this machine, in this process"]
//!     helper --> store["An event store in a temporary directory named for the test"]
//!     helper --> client["A NostrService that takes anyone's word for a relay"]
//!     client --> relays
//!     client --> store
//! ```
//!
//! - **Relays.** `nostr-sdk` has a relay that runs in-process, the same
//!   one Avestruz runs for its drafts. A test of
//!   [`crate::nostr::client`] starts as many as it needs, puts events on
//!   them, and asks the client what it finds. What is asked of which
//!   relay is then a fact that can be looked at: which relays are in the
//!   pool afterwards, which relay holds what was published.
//! - **A remote signer.** The tests of [`crate::nostr::account`] run a
//!   NIP-46 signer in the test, over one of those relays: one that
//!   agrees, one that refuses, one that will not decrypt.
//! - **An event store.** A real LMDB store in a temporary directory with
//!   the test's name and the process's id in it, removed when the test
//!   is done.
//! - **Time.** What depends on when an event was written is given events
//!   written at chosen times. Timeouts in the client's tests are the real
//!   ones, which is why a few of those tests take a second or two.
//!
//! The client's real rule about which relays may be connected to would
//! refuse every relay a test can start, since they are all on this
//! machine. So the client takes that rule as a value
//! ([`Reach`](crate::nostr::client::Reach)), the tests give it one that
//! accepts anything, and the rule itself
//! ([`public`](crate::nostr::outbox::public)) is tested on its own.
//!
//! # What `just check` holds to
//!
//! Not everything that is held to is a test. `just check` runs, in
//! order:
//!
//! | Step | What it holds to |
//! |---|---|
//! | `cargo fmt --check` | One way of laying the code out (`rustfmt.toml`) |
//! | `cargo clippy`, warnings denied | Nothing in `src/` that can panic, no dead code, no wildcard imports (see [`crate::guide::decisions`]) |
//! | `cargo test --no-run` | That the tests compile, which clippy, run without them, doesn't see |
//! | `cargo machete` | No dependency that isn't used |
//! | `just doc` | That the documentation builds, with no link that leads nowhere |
//! | `just mermaid-check` | That every diagram in it parses |
//! | `cargo deny` | No dependency under a licence the project can't take |
//! | `cargo audit` | No dependency with a known vulnerability; what is only warned of is listed in the `justfile`, each with why it is there |
//!
//! The tools that are not part of Rust are skipped, with a line saying
//! so, where they are not installed.
//!
//! On GitHub every push to `main`, and every pull request to it, is put
//! through `just check` and `just test` (`.github/workflows/ci.yml`),
//! with all of those tools installed, so that nothing is skipped there.
//! A second job measures coverage. The two tests of the system keyring
//! find none on a runner and hold to nothing; the rest run as they do
//! here.
//!
//! # Fuzzing
//!
//! A test holds a parser to the inputs somebody thought of. What reads
//! what a stranger sent is also held to the ones nobody did: `just fuzz`
//! feeds each of these whatever [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
//! makes up, for a minute each, and a panic, or anything the address
//! sanitizer catches, fails it and leaves the input that did it under
//! `fuzz/artifacts/`.
//!
//! | Target | What it reads | Whose it is |
//! |---|---|---|
//! | `note_content` | A note's text, and an `imeta` tag ([`parse_attached`](crate::nostr::content::parse_attached), [`imeta`](crate::nostr::content::imeta)) | Anyone's who writes a note |
//! | `link_preview` | The head of a linked page ([`previews::parse`](crate::previews::parse)) | Whatever server a note links to |
//! | `relay_info` | The document a relay says what it is in ([`relay_info::parse`](crate::nostr::relay_info::parse)) | Any relay's |
//! | `attached_file` | A file to attach, taken apart to leave out what it says beside the picture ([`media::prepare`](crate::media::prepare)) | Whoever made the file |
//! | `youtube_link` | A link that may be to a YouTube video, and what a video is called ([`video_at`](crate::youtube::video_at), [`title`](crate::youtube::title)) | A note's author, and YouTube |
//!
//! ```sh
//! just fuzz                  # every target, a minute each
//! just fuzz note_content     # one target, for a minute
//! just fuzz note_content 0   # one target, until it is stopped
//! ```
//!
//! It takes a nightly toolchain (`rustup toolchain install nightly`) and
//! `cargo install cargo-fuzz`, and is no part of `just check`: it builds
//! the whole program a second time, with the sanitizer in it. The targets
//! are a crate of their own in `fuzz/`, with its own lock file, which
//! holds the dependencies to the versions the program is built with.
//!
//! What a fuzzer has found so far it keeps in `fuzz/corpus/`, on the
//! machine it ran on: it starts from nothing on another. Decoding a
//! picture is not among the targets: that is the `image` crate's, which
//! is fuzzed where it is made.
//!
//! # What is not tested
//!
//! - **The views, as pictures.** Nothing renders a view and compares it
//!   with anything. What is tested of them is their arithmetic: how tall
//!   a row is, which rows a list builds.
//! - **The handlers.** They are thin on purpose: what a handler decides
//!   is mostly in a function of [`crate::nostr`] or of the state, which is
//!   tested. The handlers themselves, and the order messages arrive in,
//!   are not.
//! - **Real relays, real signers, a real wallet.** The relay in the tests
//!   is one implementation, and a patient one.
//! - **The panel.** Whether the popup opens where it should and closes
//!   when it should is the desktop's doing.
//! - **Anything over the network**: pictures, link previews, NIP-05,
//!   LNURL. Their parsing is tested; their fetching is not.
//! - **Videos from the web, and their sound.** The tests play a second of
//!   a test picture from a file, through the same pipeline, where
//!   GStreamer and the elements for it are installed, and pass for
//!   nothing where they are not. Fetching one, the formats notes really
//!   link to, and what is heard are tried by hand.
//! - **Fonts.** What a font file is told to have of colour glyphs and of
//!   emoji is tested, on files made for the tests, and so is which fonts
//!   are then put first. What the fonts of a system draw of an emoji is
//!   looked at by hand.
//! - **yt-dlp, and YouTube.** Which links are videos, what is handed to
//!   yt-dlp and how what it writes is read are tested. Running it is not:
//!   whether it finds a video depends on the day, the machine and where
//!   the machine is.
//!
//! # Trying it by hand
//!
//! What the tests don't reach is tried by running the applet against a
//! relay on this machine, with a config and a store of its own, so that
//! nothing of the real account is touched:
//!
//! ```sh
//! # A relay to talk to (https://github.com/fiatjaf/nak).
//! nak serve --hostname 127.0.0.1 --port 10999 &
//!
//! # A throwaway account, someone for it to follow, and something to read.
//! key=$(nak key generate) other=$(nak key generate)
//! nak event --sec "$other" -c "hello" ws://127.0.0.1:10999
//! nak event --sec "$key" -k 3 -p "$(nak key public "$other")" ws://127.0.0.1:10999
//! nak encode nsec "$key"    # what to sign in with
//!
//! # A config that names only that relay, and a store nobody else uses.
//! export XDG_CONFIG_HOME=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
//! config="$XDG_CONFIG_HOME/cosmic/io.github.cosmic-applet-avestruz/v1"
//! mkdir -p "$config" && printf '["ws://127.0.0.1:10999"]' > "$config/relays"
//!
//! # Outside the panel the button is a small window of its own; the popup opens by the screen's edge.
//! RUST_LOG=cosmic_applet_avestruz=debug just run
//! ```
//!
//! Signing in with an `nsec` there writes to the real keyring, under the
//! throwaway account's `npub`: sign out again, or remove it with
//! `secret-tool clear service cosmic-applet-avestruz username <npub>`.
//!
//! A relay on this machine is used whatever the account's relay list
//! says, which is what makes this work: see [`crate::guide::relays`].
//!
//! # Every test
//!
#![doc = include_str!(concat!(env!("OUT_DIR"), "/test_inventory.md"))]

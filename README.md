# Avestruz

[![CI](https://github.com/glima/avestruz/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/glima/avestruz/actions/workflows/ci.yml?query=branch%3Amain)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/glima/avestruz/badge)](https://scorecard.dev/viewer/?uri=github.com/glima/avestruz)
[![Codecov](https://codecov.io/gh/glima/avestruz/graph/badge.svg)](https://codecov.io/gh/glima/avestruz)
[![GitHub Release](https://img.shields.io/github/release/glima/avestruz.svg)](https://github.com/glima/avestruz/releases/latest)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
[![Tip limachaves@strike.me with Lightning](https://img.shields.io/badge/tip-lightning-F7931A?logo=lightning&logoColor=white)](https://strike.me/@limachaves/)

**[Website](https://glima.github.io/avestruz/)** ·
**[Documentation](https://glima.github.io/avestruz/doc/)** ·
**[Releases](https://github.com/glima/avestruz/releases)**

A [Nostr](https://github.com/nostr-protocol/nostr) client that lives in the
[COSMIC™](https://system76.com/cosmic) desktop panel: a panel button, and a
popup that holds the whole client, drawn by the GPU.

Inspired by [Vega](https://github.com/hoornet/vega),
[Jumble](https://github.com/CodyTseng/jumble),
[Amethyst](https://github.com/vitorpamplona/amethyst) and
[Maré Player](https://github.com/glima/mare-player).
It is pure Rust on libcosmic: no webview, no Tauri.

## Status

Early. What works today:

- **Panel applet** — a button in the panel opens the popup. A view with a
  long list in it, as the feed is, makes the popup as tall as the screen
  it opens on has room: all of it, but the panel and a gap at either
  end. A view with little in it makes it as tall as that. How much room
  the screen has is asked of the compositor when the client starts and
  each time the popup opens, so a display plugged in or out is taken up
  the next time. Escape closes the popup, as a click outside it
  does, and nothing in it is lost for that; with a menu open, it puts the
  menu away first.
- **Signing in** — with an `nsec` (full access), an `npub` (read-only) or a
  `bunker://` address (full access through a remote signer, NIP-46, where
  the key stays). What is secret is kept in the system keyring (Secret
  Service) and nowhere else: the `nsec`, or the bunker's address, and the
  key each remote signer knows Avestruz by. That key stays the same for a
  signer, signing out included, so a signer that only answers clients it
  was told about is told once: when it turns a sign-in down, Avestruz shows
  the key to authorize there (for `nak bunker`, `--authorized-keys`).
- **The feed** — text notes from the people the account follows (its
  contact list), newest first, in a scroller that only ever builds the
  notes on screen. What was seen before shows at once from the local event
  store; scrolling towards the end fetches older notes. At the top,
  **Notes** shows the notes that start a conversation and **Replies** shows
  those with the replies among them; the choice is kept, and holds for the
  notes on a profile too. Comments (kind 1111, NIP-22), which answer what
  is not a text note, count as replies.
- **From the keyboard** — in the feed, in a thread, among the notes of a
  profile, the bookmarks, the notes a search found, and what the account
  is told of:

  | Key | What it does |
  |---|---|
  | `j` | The next note: marked, and put at the top of what is in view |
  | `k` | The one before |
  | Home | The very top |
  | `l` or Enter | Open the marked note: its thread |
  | `y` | Copy a share link to the marked note |
  | Space | Play the marked note's video, or hold it. It is for the first thing the note links to, pictures apart: if that is not a video, it is opened in the browser |
  | `h` or Backspace | Back, as the arrow at the top left does: out of a thread, a profile, the settings |
  | Backspace | Out of the composer too: at once with nothing written, and with something written only after asking whether to keep it as a draft (Enter), discard it (Backspace again) or go on writing (Escape). In the text it rubs out as it does anywhere, and leaves once there is nothing left to rub out and it has been let go |
  | `r` | Reply to the marked note |
  | `R` | Repost or quote the marked note: its menu opens, the arrows up and down choose which, and Enter does it |
  | `z` | Zap the marked note |
  | `b` | Bookmark the marked note, or take its bookmark off |
  | `B` | Show the bookmarks |
  | `.` | Show the notes that wait, as the button over the list does |
  | `?` | Show what the keys do |
  | Ctrl+Shift+S | Save a picture of the popup, as it shows, to your pictures folder (`avestruz-<date>_<time>.png`) |

  The note the keyboard is at has a line around it. A list keeps its
  mark: back from a thread, the note it was opened from is marked
  still, and `j` goes on from there. With nothing marked, Enter, `y`
  and Space are for the note at the top of what is in view. On a
  profile, who they are is the first place, above their first note.
  The keys are the text's where something is being typed. What the
  keys do is listed in a view of its own, which `?` opens, as
  **Keyboard** in the menu above the feed does; and said where the
  pointer rests on what a key does: the way back, a note's actions,
  and the button for new notes.
- **A key that opens it** — `cosmic-applet-avestruz --toggle` opens the
  popup, or closes it, as a press of the panel button does: it says so
  to the client that is running, at a socket in the session's runtime
  directory, and leaves. Bound to a key in COSMIC's Settings, under
  Keyboard, Shortcuts, Custom shortcuts, that key opens Avestruz from
  anywhere. In `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom`
  the same shortcut is:

  ```ron
  (modifiers: [], key: "F12", description: Some("Avestruz")): Spawn("cosmic-applet-avestruz --toggle"),
  ```

  A key bound this way is the desktop's, and no other application is
  given it.
- **Where one was** — what the feed shows, and how far it is scrolled, are
  the reader's: closing the popup and opening it again changes neither.
  The feed is looked at for new notes every minute while the popup is open,
  every five while it is closed, and when it opens. What turned up waits
  behind a **Show n new notes** button over the list, and nothing moves
  until that, or Refresh, is pressed: the new notes then show, from the top.
- **Long notes** — a note longer than six hundred characters is cut
  there and marked with an ellipsis. **Show more**, under what is shown
  of it, shows the rest where the note is: the row grows down from its
  place, and nothing above it moves. **Show less**, under all of it, cuts
  it again; if the top of the note was scrolled out of view by then, the
  list is scrolled back to it, so that what is left to see is the note.
  Pictures, videos and quoted notes are under the text either way. Which
  notes show in full is each list's own, and kept while the list is. A
  note quoted in another stays cut: its own thread has the rest.
- **Relays** — the account's own relays are those its relay list
  (NIP-65, kind 10002) says it writes to, eight of them at most. They are
  asked about everything, and what the account writes goes to them. The
  relays the config names are where an account is until then: where its
  lists are looked for, and what it reads and writes with while it has no
  relay list, or none of whose relays connects. Once one of its own is
  connected they stand down, neither asked nor written to, and are hung
  up on; except those on this machine or this network, which no relay
  list would name and which stay in use. Other people are read where they
  write as well (the outbox model): the relay lists of the people followed
  are asked for, kept in the event store and asked for again after an
  hour, and the relays they name are asked for those people's notes beside
  the account's own. Not all of them: each person from two relays, the
  account's own counting, and the relays that have the most people first,
  twenty at most for one question. The same goes for a profile and the
  notes on it, for a quoted note whose address says who wrote it or which
  relay has it, and for
  a thread, which is asked of the relays of whoever wrote the note opened
  and of the first few people it names, where they write and where they
  read. A relay somebody else names, in a relay list or in the address
  of a note, is connected to only if it is
  encrypted (`wss`) and on the open internet: not one on this machine or
  this network, and no onion service. It gets two seconds to connect and
  six to answer; one that doesn't, or says no, is left alone for ten
  minutes, so that nothing waits for it twice. Such a connection is closed
  after two minutes with nothing asked, and the relay forgotten after
  fifteen.
- **Threads** — a click on a note opens its conversation: the note in full,
  the notes it answers above it and the answers to it below, each set in
  under the one it answers. A click on another note of the thread, or on a
  reference to a note in what someone wrote, opens that note's thread over
  it; Back returns to where one was, in the thread or the feed. A comment
  opens the comments around it, under what they are about when that is an
  event: shown as its text, whatever kind of thing it is.
- **Profiles** — a note's author, by their picture or name, and the
  account's own picture above the feed, open a profile: the picture across
  its top, who they are, what they say of themselves, their website and
  Lightning address, and under that their notes. The key can be copied. A
  name at a domain (NIP-05) shows only once that domain has said it is
  theirs, which it is asked when the profile opens. A note there opens its
  thread, and Back returns to the profile where it was left.
- **Following** — with full access, a profile has Follow or Unfollow. The
  account's contact list is asked of the relays again, changed by that one
  person and nothing else, and sent out; the feed starts over with who is
  followed then. If no relay has a list of the account's, a first one is
  only written after saying so: it would replace a list kept elsewhere.
  And not at all while one of the relays doesn't answer: it may be the one
  that has the list.
- **Settings** — behind the cog above the feed, which opens a menu with
  the bookmarks, the relays and the settings: who is signed in, signing
  out, how large notes are set, for how long other people's notes are
  kept on this machine (a week, a month, three months, a year, or for
  good), how much is written to the journal, and what the keys do.
- **Your relays** — **Relays**, in the same menu, shows the relays the
  account says are its own, in seven sets. The **outbox** relays it
  writes to and the **inbox** relays it reads are its relay list (NIP-65,
  kind 10002). The others are each a list of `relay` tags (NIP-51), those
  said to anyone and those the list keeps to itself, encrypted (NIP-44):
  the relays **direct messages** go to (NIP-17, kind 10050), the
  **search** relays (kind 10007), the **indexer** relays (kind 10086),
  the **trusted** relays (kind 10089) and the **blocked** relays (kind
  10006). The list of private relays (kind 10013) is not among them, and
  is neither read nor written: what is written here and not published is
  kept on this machine. A relay is added by its address, with or without
  `wss://`, moved up or down with the arrows beside it, and taken out with the bin;
  nothing is written until **Save**, and what was changed is still there
  if the popup closes first. Under a relay this is connected to it says
  whether it is now, how much came from it and went to it since the
  applet started, how many connections worked, and how long it takes to
  answer. Four of the sets ask something of a relay that not every
  relay does. A search relay has to search (NIP-50). A relay direct
  messages are sent to should give them only to whom they are for, which
  takes knowing who is asking (NIP-17, by NIP-42). The inbox relays and
  the relays direct messages are sent to are where other people write to
  reach the account, so they have to take what anyone writes. And the
  outbox relays are where other people read the account's notes, so
  they have to let anyone read. While the page shows, the relays of
  those sets are asked what they say of themselves (NIP-11), and what
  one says that gets in the way is said under its name: that it names
  the NIPs it supports and not the one needed; for a relay others write
  to, that it takes notes only from some or has to be paid; and, for an
  inbox or an outbox relay, that whoever connects has to say who they
  are first. One that doesn't answer is not said anything of, and
  neither is a relay on this machine or network, which is not asked.
  Save asks the relays for each list again and writes it with its relays
  replaced and everything else in it as it was: a relay stays where it
  was, public or private, and a new one goes where the list keeps its
  relays, or, in a first list, where lists of its kind do. A changed
  relay list goes to the relays the account wrote to before as well as to
  those it writes to now. Nothing can be changed until the relays have
  said what the lists are, a first list is only written after saying so,
  and a list whose private part can't be read is never written over. With
  read-only access the relays show, and can't be changed.
  Four of the sets are gone by: the outbox relays are the account's own
  (see **Relays**), the search relays are the ones searched, the indexer
  relays are asked for the profiles and relay lists the account's own
  relays don't have, and a blocked relay is never connected to because a
  relay list, a note's address or a bookmark names it. The other two
  are kept for the clients that go by them. What a list keeps to itself
  takes the key to read, so it is read when it is needed: the blocked and
  indexer relays when the popup opens, the search relays when a search
  page does, the rest on this page.
- **Its own relay** — Avestruz runs a relay itself, over its event store,
  for what is the account's alone. It listens nowhere: on no port, and on
  no socket file. A connection to it is a pipe made inside Avestruz, so
  there is nothing for another program to open, and Avestruz is the only
  client it has. It takes the events of the account signed in and no
  others. That is the relay; the event store under it is a file, which
  whatever can read the user's files can read, and what is written and
  not published is encrypted in it. What it keeps there today is drafts
  (NIP-37, kind 31234): when the composer is left, what was written is
  encrypted to the account itself (NIP-44), signed, and sent to this
  relay and to no other; a draft that was published or cleared is kept
  as one that is done with.
  Drafts are read back when the popup first opens, which takes the key.
  If the relay can't be given the draft of a new note (the signer said
  no), that draft goes to a file instead, as it did before.
- **Notes kept for a while** — what the relays answer with is kept in the
  event store, which is why a feed, a thread or a profile shows at once
  the second time. Once a day the store lets go of what is older than the
  settings say, a month unless they say otherwise, going by the date
  each event was written: other people's notes, comments, reposts,
  reactions and zap receipts, which are asked of the relays again when
  they are looked at. What the account wrote stays, and so do the notes
  it bookmarked, and everything there is one of for each person, whose
  date says nothing of whether it still holds: profiles, follow lists,
  relay lists. The first time is when the bookmarks have been read, which
  is when the popup first opens; while they can't be read, nothing goes.
  The store's file doesn't get smaller for it: the room is used for
  what comes next.
- **Search** — the magnifier above the feed opens a page to search on,
  for people by what their profile says and for notes by what they say
  (NIP-50). The relays asked are the account's search relays and no
  others: there are none built in, and with none set the page says where
  to set them. People and notes are searched for at once, and the page
  shows one or the other, best match first as the relays ranked them,
  the answers of several relays taken a rank at a time; among the people,
  those the account follows come first and are marked, since a name is
  anyone's to take. A person opens their profile, a note its thread, and
  Back returns to what was found, and each person has Follow or Unfollow
  beside them. An `npub`, `nprofile`, `note` or `nevent` typed there is
  opened rather than searched for, and no relay is told. A name at a
  domain (`ana@example.com`, NIP-05) is asked of its domain, and the
  profile of whoever the domain says it is opens. The address of a relay
  said in full (`wss://…`) shows the notes that relay got last, under
  what it says of itself (NIP-11): its name, a line about it, and whether
  it searches. An `nsec` or a `bunker://` address is cleared and sent
  nowhere. Nothing of a search is kept: there are no recent searches,
  the words are not written to the journal at any level, and what was
  typed and found is gone when the page is left. What is found is kept
  in the event store, as every event received is.
- **Writing** — with full access, a new note from the feed, a reply from
  a thread and a quote from under any note, in a composer of their own;
  Ctrl+Enter publishes. A note is
  signed with the key from the keyring, which is read for each signature
  and not held, or by the remote signer, and counts as published once a
  relay has taken it; until then the text stays in the composer. Notes,
  replies, comments, reposts and reactions go to the account's own relays and,
  for the people they name, to the relays those people read, as their
  relay lists say (NIP-65): two for each. Hashtags
  and people mentioned are tagged, and a reply names its conversation as
  NIP-10 says; an answer to a comment, or to what comments are about, goes
  out as a comment (NIP-22). What is written and not published is kept:
  leaving the composer, or the popup, loses nothing, and a draft, of a
  new note, a reply or a quote, is still there after a restart: see **Its
  own relay**.
- **Attaching** — the paperclip under the text picks pictures, videos
  and sounds in the desktop's file chooser, and Ctrl+V attaches files
  copied in a file manager or a picture copied as one. The popup closes
  while the chooser is open, and opens again on the composer. Each file
  is sent to the account's media servers (Blossom): to the first that
  takes it (BUD-02), after asking whether it would (BUD-06), and the
  others are asked to fetch a copy from there (BUD-04), so that it is
  still to be found when one of them is gone. Every request carries an
  authorization the account signs for that one file (BUD-11, kind
  24242). While a file is on its way it shows under the text with how
  far it has got, and can be called off; when it is there its address
  goes at the end of the text, and the note says of it what kind of file
  it is, its hash, its size and a picture's width and height (`imeta`,
  NIP-92). Taking the address out of the text takes the file out of the
  note. Files of up to 100 MB are attached.
- **Nothing but the picture** — before a JPEG, a PNG or a WebP is sent,
  everything in the file that is not the picture is left out: where and
  when it was taken (EXIF, GPS), with what, what it was edited with, who
  it is credited to (XMP, IPTC, content credentials), comments, embedded
  thumbnails and second pictures, and whatever follows the end of the
  picture. The picture data is not touched, so nothing of the picture is
  lost; its colour profile, transparency and animation stay, and so does
  which way up it is, alone. A picture whose file can't be taken apart
  this way is not sent. **Videos and other files are sent as they are**:
  a video may say where it was taken.
- **Media servers** — **Media servers**, behind the cog above the feed,
  shows the account's server list (BUD-03, kind 10063): the one files
  are sent to first at the top. Servers are added, taken out and moved
  there, and nothing is written until Save; a first list is only written
  after saying so, since it replaces any such list kept elsewhere. No
  server is built in: with none set, attaching opens this page instead.
  Only `https` servers are used.
- **Replies, reposts, reactions and zaps** — under every note, how many
  answered it, how many reposted it, how many reacted to it and how many
  sats it was zapped, counted from what the relays send and kept in the
  event store. With full access, the speech bubble opens a reply and the
  heart offers what to react with (see **Reacting**).
- **Who they were** — a click with the right button on a note's reposts,
  its reactions or its zaps lists who they were under it: who reposted it,
  the last first; who reacted and with what; who zapped it and how many
  sats in all, the most first. A click on one of them opens their profile;
  a click anywhere else, or Escape, puts the list away. It is read from
  the event store, as the counts are, so who is listed is who was counted.
  Who zapped is who the zap request inside the receipt was signed by
  (NIP-57): a zap whose request is missing or does not hold is listed as
  someone who does not say. A hundred are shown at the most.
- **Reacting** — the heart under a note opens a row: a like, the emoji you
  react with most, and a way to all the others (NIP-25). That way opens
  every emoji there is, by group, with a field to find one by what it is
  called. The reaction shows at once, in the heart's place, and is taken
  back if no relay accepts it; a note is reacted to once. The emoji you
  use are remembered, how often and when last, and the most used are
  offered first the next time; a few common ones fill the row until there
  are enough. That is kept in the settings of this machine and published
  nowhere. Custom emoji (NIP-30) are not offered, and a reaction that is
  one counts as a like.
- **Emoji** — drawn in colour, in notes and to react with, by the
  system's emoji font, which is looked in before any text font that has a
  few emoji of its own. Colour glyphs that are bitmaps can be drawn; the
  ones made of gradients (`COLR` version 1), which is how some
  distributions ship Noto Color Emoji, can't. Such a font is left out, and
  emoji then show in one colour, if a font for that is installed (Noto
  Emoji, for one). `just emoji-font-setup` puts that right: see
  **Building**.
- **Reposting and quoting** — the two arrows under a note open a menu.
  **Repost** passes the note on as it is (NIP-18): it shows at once, as a
  like does, and is done once. The repost carries the note whole and names
  a relay that has it, which is asked for first. **Quote** opens the
  composer with the note on show under the text; what is written goes out
  as a note of the account's own with the quoted note's address under it,
  and may be nothing at all. The reposts of the people followed are
  counted, and not yet listed in the feed.
- **Quoted notes** — a note another note names (NIP-18, NIP-21) shows
  under it as a card: who wrote it and when, what it says, its images and
  the preview of its link. A click on the card opens that note's thread.
  The first three notes a note quotes are shown so. A page of notes comes
  with the notes it quotes, from the event store or else from the relays,
  so that a card is there when its row is first shown and doesn't turn up
  under what is being read. A quoted note shows the notes it quotes the
  same way, a card in its card, and there it ends: a note quoted deeper
  than that shows as a link, and so does one that none of the relays has.
  A note the account's own relays don't have is asked of the relays its
  author writes to and of the relay its address names (NIP-19), where the
  address says either; the link to one that wasn't found looks there
  again when it is clicked.
- **Notifications** — the bell above the feed opens what others did
  that concerns you, newest first: replies to you, replies in a
  conversation you are named in, mentions, quotes of your notes,
  reactions to them (NIP-25), reposts of them (NIP-18) and zaps
  (NIP-57). All of it, or one kind: **Mentions**, **Reactions**,
  **Zaps**. What is itself a note shows as the note it is, under a line
  that says which of those it is, and can be answered and reacted to
  where it stands; a reaction, a repost or a zap is one short row, with
  the start of the note it is about, which a click opens.
  They are the events that name the account, asked of its own relays and
  of the relays it reads (NIP-65), when it signs in and then as often as
  the feed is looked at for new notes; scrolling towards the end asks for
  older ones. A reaction or a repost shows when the note it is about is
  yours, where that note can be found.
  What was written since you last looked is new: the bell says how many
  there are, the panel button has a dot over its corner while there are
  any, and they are picked out in the list for as long as you look at
  it. Opening the list is looking; when that was is kept in the
  settings of this machine, for each account, and published nowhere. An
  account that never looked starts with nothing new.
  A zap shows once your own Lightning address has said which key its
  receipts are signed with, and only if that key signed it: anyone can
  write a receipt, for any amount. With no Lightning address, or one that
  can't be asked, no zap shows. Follows are not notifications here: who
  follows whom can't be told from a relay without asking for everyone's
  contact list.
- **Bookmarks** — with full access, the ribbon at the right of the line
  under a note bookmarks it, and takes the bookmark off again. Bookmarks
  are private, always: they are kept in the private items of the
  account's bookmark list (NIP-51, kind 10003), which are tags, as JSON,
  encrypted to the account itself (NIP-44), and are there for any client
  that reads the list as NIP-51 says. Content encrypted the older way
  (NIP-04) is read too. The public items of that list, its tags, are
  neither read nor changed: they go out with every new list as they came,
  and a note bookmarked only there has an empty ribbon here. Everything
  else among the private items stays as it was too. The list
  is read when the popup opens, which takes the key: from the keyring,
  or by asking the remote signer to decrypt it. It is asked of the
  relays again before every change, and, as with following, a first list
  is only written after saying so. A list whose private part can't be
  read is never written over.
- **Browsing bookmarks** — **Bookmarks**, behind the cog above the feed,
  lists the notes bookmarked, the one bookmarked last first: the private
  bookmarks, which are the only ones there are here. The notes are looked
  up by their ids, in the event store
  and on the relays, twenty at a time as the list is scrolled: the
  account's own, and for a note they don't have the relay its bookmark
  names, and the relays its author writes to if the bookmark says who
  that is. Under the last
  note it says how many bookmarks are not shown: notes no relay has, and
  the articles, hashtags and links the same list can hold. A
  click on a note opens its thread. Taking a bookmark off there leaves
  the note where it is, with an empty ribbon, until the page is opened
  again: nothing moves under the pointer.
- **Copying a picture** — a click on a picture in a note with the right
  button puts it on the clipboard, and the picture says so. What is
  copied is the picture itself, fetched again from where it is, as a
  PNG, the way up it says it is: not the smaller copy that is kept for
  showing. It is there to paste after the popup has closed; pasted
  before it has arrived, the paste waits for it. Where only text is
  taken, what is pasted is the picture's address.
- **Share links** — the chain link under a note, beside the ribbon, copies
  a link to the note that anyone can open in a browser:
  `https://njump.me/` and the note's `nevent` (NIP-19), which names the
  note, who wrote it, what kind it is, and two of the relays it was seen
  on, leaving out any that only this machine or this network can reach.
  The link turns into a tick once it is copied. Nothing is sent anywhere
  to make one: the site in it hears of the note when someone opens the
  link, and not before. Read-only accounts have it too.
- **Seen on** — at the right end of the line under a note, how many relays
  it was seen on, and which, for whoever points at it: every relay that
  sent the note since the applet started, and those that took one written
  here. Each relay is asked whether it has the notes on show.
- **Zapping** — the bolt under a note whose author has a Lightning address
  asks that address for an invoice of the amount chosen (NIP-57) and shows
  it as a QR code, to scan with a wallet, or to copy. Avestruz holds no
  money and pays nothing itself. While the invoice is on show it waits for
  the receipt, and says when the zap went through. With full access the
  zap is the account's; without, it is anonymous.
- **Authors** — names and pictures from profiles, looked up in the event
  store first and on the relays second. Until a picture loads, and for
  people who have none, a disc in a colour of their own with their initial.
- **Note content** — links open in the browser and show as host and path;
  people mentioned show by name and open their profile; hashtags are picked
  out; images show under the text, from one edge of the note to the
  other and as tall as that makes them, unless they are narrower than
  that themselves: a picture is made smaller to fit, never larger. They
  keep their room while they load when the note says how large they are.
  A picture is kept at the size it is shown at on the screen, not as it
  came: a list of them scrolls at the screen's pace, and they take a
  fraction of the memory.
  A click on a picture opens the note, as a click on its text does.
- **Link previews** — under a note that links to a web page, a card with
  what the page says of itself: the picture it is shared with, the site,
  its title and a few words about it. A click opens
  the page. The last page a note links to is the one previewed. Only the
  head of the page is fetched, and read for its Open Graph tags and its
  title; nothing of it is run. The card is always the same height, and its
  room is kept while the page is asked, so nothing moves when the preview
  arrives; a page that is gone, or says nothing of itself, gets no card.
- **Videos** — a link to a video plays where it is, in the note's row,
  through GStreamer. A link is a video if its address ends like one
  (`mp4`, `webm`, `mov`, `mkv`, `m4v`, `ogv`, `avi`) or if the note says it
  is one (the media type in its NIP-92 `imeta` tag). Nothing of a video is
  fetched until it is pressed: until then there is a play button, over the
  picture the note gives for it if it gives one. Then it plays with its
  sound, and under it are play and pause, how far in it is and of how
  long, a slider to go elsewhere in it, its sound, and the link itself, to
  open in the browser. The sound is one button: pressed, it mutes and
  unmutes; pointed at, it shows a slider beside it that sets how loud the
  video is, on the scale a mixer shows; and its picture says how loud that
  is. Moving the slider up unmutes. Both show the sound as the system has
  it: where the system remembers how an app's sound was left (PipeWire,
  PulseAudio), a video starts as the last one ended, as loud and as muted,
  and what a mixer does to it shows there too. Beside the sound is how
  fast the video plays, which opens a menu of speeds from 0.5× to 2×; its
  sound keeps its pitch. The speed chosen holds for the videos played
  after it, until the applet starts again. One video plays at a time,
  and only while it is on screen: scrolled out of view, with another view
  in the list's place or with the popup closed it is held where it was. Its room in the row is
  settled before anything of it is known (the proportions the note says,
  or 16:9, never taller than wide), so playing it moves nothing. A link
  that gives nothing away and turns out to be a video when it is asked
  for its preview gets a card that says so, and plays in its place once
  that is pressed. Which videos play is up to the GStreamer plugins on the
  machine; one that can't be played says why.
- **YouTube videos** — a link to a YouTube video (its page, a short, a
  live one, an embedded one, or the short `youtu.be` address) plays in
  its row as any video does, in the same player: not in YouTube's own,
  which is a program for a web engine, and there is none here. Under it
  is what the video is called, and over it, until it is played, the
  picture YouTube keeps for it. Pressing play asks
  [yt-dlp](https://github.com/yt-dlp/yt-dlp), which has to be installed,
  for a stream of the video, and plays that. It takes a few seconds, and
  a few more when YouTube says the stream is only good to fetch a moment
  later, which is waited for. The stream is the one YouTube keeps as a
  single file of pictures and sound, which is small (360 lines): enough
  for a row, and the link opens the video in the browser for more. A
  video that is live has no such file, and is played as it comes,
  pictures and sound, in a size near 720 lines if it comes in one: how
  long it has gone on is shown, with no slider to go elsewhere in it
  and no choice of speed. yt-dlp runs as you have set it up, its
  configuration file included, so whatever it needs to get past YouTube
  on your machine is said there; if it can't find the video, what it
  says is shown in the video's place.

Still to come: the trending and hashtag feeds,
reposts in the feed, attachments, emoji
reactions, paying zaps from a
connected wallet (NWC), direct messages, articles, audio, live streams,
and podcasts.

Pictures are fetched over `https` only, straight from wherever a profile or a
note points, and are shown at the size they come in. They are kept in memory,
within a budget, and not on disk. The pages notes link to are asked the same
way, once each for as long as the applet runs, when the note comes into
view: as with a picture, the page's server sees the request. A video is
fetched by GStreamer, over `https` only, and not before it is played: its
server sees that request, and no other. Of a YouTube video, YouTube is
asked what it is called and for its picture when the note comes into view,
as any linked page is; and when it is played, yt-dlp asks YouTube for it
and GStreamer fetches the stream from there. Lightning
addresses are asked over `https` only as well, and an invoice is shown only
if it is for the amount chosen.
So are the domains of NIP-05 names, and only for a profile that is opened.
Relays other than the ones the config names are connected to as well: those
the account's relay list names, those the relay lists of the people it
follows or reads name, the one a bookmark of its own names, and the one the
address of a quoted note names, which is whatever relay the note's writer
chose. Each of them sees the request, and where it comes from. A search goes to the
account's search relays, which see the words searched for; a name at a
domain typed there goes to that domain, and a relay's address to that
relay.

## Installing

A release has a `.deb` and an `.rpm` for x86_64 and aarch64, and a
tarball of the same files for anything else, with their checksums
(`SHA256SUMS`). The packages name what the applet needs to run, and
recommend the GStreamer plugins most videos are decoded with and yt-dlp
for YouTube; what yt-dlp itself needs to read YouTube is said under
[Building](#building). Once installed, add **Avestruz** to the panel
from COSMIC Settings → Desktop → Panel → Applets.

## Building

Dependencies are those of any libcosmic application (a Rust toolchain,
`libxkbcommon`, Wayland development files), D-Bus's development files
(`libdbus-1-dev` on Debian and its kin, `dbus-devel` on Fedora, `dbus` on
Arch), GStreamer's development files
(`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Debian and
its kin, `gstreamer1-devel` and `gstreamer1-plugins-base-devel` on Fedora,
`gstreamer` and `gst-plugins-base` on Arch), plus a Secret Service provider
at run time (gnome-keyring, KWallet) for signing in with an `nsec` or a
bunker address.

Videos play with whatever GStreamer plugins are installed. The good, bad
and libav sets (`gstreamer1.0-plugins-good`, `gstreamer1.0-plugins-bad`,
`gstreamer1.0-libav`, or what your distribution calls them) cover fetching
over `https` and the formats notes commonly link to. Without them the
applet runs all the same, and a video says why it can't be played.

YouTube videos take [yt-dlp](https://github.com/yt-dlp/yt-dlp) at run
time, on the `PATH` the panel runs with, and yt-dlp these days takes a
JavaScript runtime to read YouTube (its
[EJS page](https://github.com/yt-dlp/yt-dlp/wiki/EJS) says how). If
`yt-dlp --get-url 'https://www.youtube.com/watch?v=…'` prints an address
in a terminal, the video plays here. An applet does not have a shell's
`PATH`, so a runtime that a version manager puts on it has to be named by
its full path in yt-dlp's configuration. `just install` sees to that for
whoever installs, and `just yt-dlp-setup` does it alone: if your yt-dlp
configuration names no runtime, the first found of deno, node, bun and
quickjs is added to it by its full path (`--js-runtimes node:/…/node`),
and, unless your yt-dlp came with its solver scripts, so is
`--remote-components ejs:github`, which lets it fetch them from its own
releases. Nothing that is already there is changed, and what was added
is said. Installing into a `rootdir`, as a package does, leaves it out.

Emoji in colour take an emoji font whose colour glyphs are bitmaps. Where
the system's are gradients (`fc-list | grep -i emoji` names a file called
`COLRv1`), `just emoji-font-setup` fetches the bitmap build of Noto Color
Emoji from [its project](https://github.com/googlefonts/noto-emoji) (SIL
Open Font License) into `~/.local/share/cosmic-applet-avestruz/fonts/`.
Any font put there is read by Avestruz when it starts, before the
system's, and by nothing else: no other program draws anything
differently for it. Deleting the file undoes it.

```sh
just                # release build
just run            # run outside the panel: a small window holding the button
sudo just install   # binary, desktop entry, metainfo and icon under /usr; and your yt-dlp set up for YouTube
just yt-dlp-setup   # only that last part, as yourself
just emoji-font-setup  # an emoji font that can be drawn in colour, where the system's can't
```

Once installed, add **Avestruz** to the panel from COSMIC Settings → Desktop →
Panel → Applets.

```sh
just check          # formatting, clippy, unused dependencies, commit messages, licences, security advisories
just test           # unit tests
just coverage       # HTML and LCOV coverage reports, via cargo-llvm-cov
just coverage-summary  # coverage as a table in the terminal
just bloat-check    # where the binary's size goes, by crate and by function (cargo-bloat)
just stats          # lines of code, comments and blanks, by language (tokei)
just fuzz           # fuzz what reads untrusted input, a minute a target (nightly, cargo-fuzz)
just doc            # build the docs, private items included, diagrams drawn
just doc-open       # build the docs and open them in the browser
just mermaid-check  # parse the diagrams in the docs (needs Node.js, see below)
```

What a change has to pass, and how its commit is written, is in
[CONTRIBUTING.md](./CONTRIBUTING.md).

### Documentation

How Avestruz is put together is written where the code is, as doc
comments, and built into pages by `just doc-open`. The way in is the
`guide` module, which has no code in it:

| Page | What it answers |
|---|---|
| `guide::architecture` | What the parts are, what each may know of the others, where each runs, where things are kept |
| `guide::flows` | What happens, step by step: signing in, filling the feed, publishing, bookmarking, searching, zapping |
| `guide::relays` | Which relays there are, which is asked or told what, and whose word is taken for a relay |
| `guide::protocol` | Which NIPs and kinds are spoken, and where in the code each is |
| `guide::decisions` | What was decided and why, the rules everything keeps to, and what is not there yet |
| `guide::testing` | How it is tested, what is not, and every test by name |

The documentation as it is on `main` is published at
<https://glima.github.io/avestruz/doc/>, and the guide at
<https://glima.github.io/avestruz/doc/cosmic_applet_avestruz/guide/>.

The diagrams are Mermaid, written in the doc comments as code blocks.
`just doc` has rustdoc put a small script in each page
(`docs/rustdoc-header.html`) that draws them in the browser, in the
page's theme; a click on one shows it large. Mermaid itself is loaded
from a CDN when a page with a diagram is opened, so the diagrams need a
network the first time; without one, and under plain `cargo doc`, each
shows as the text it was written as.

rustdoc builds a diagram that doesn't parse without a word. `just
mermaid-check` parses them all, with the same Mermaid release the pages
load, and is part of `just check` where Node.js and its packages are
there:

```sh
npm ci --prefix tools/mermaid-check
```

The list of tests in `guide::testing`, and the count of them, are
written by `build.rs` from the sources each time the crate is built.

### Website

The project's site, <https://glima.github.io/avestruz/>, is `site/`: one
page that reads whole as plain HTML,
and, where the machine is up to it, a scene over its first screen to walk
about in, in which each thing stands for a piece of Nostr. `just site`
assembles it into `target/site` as GitHub Pages publishes it (it takes
`npm`), and `just site-open` serves that on this machine and opens it in
the browser with the scene on (`?mode=hifi`, which asks for the scene
whatever the machine; `just site-open ''` opens it as a visitor gets it). The scene's
props are made by scripts, with their files kept in the repository:
[site/craft/README.md](site/craft/README.md) says how. Under `doc/` the
site carries the documentation `just doc` builds, the guide included,
and each push to `main` publishes both.

### Viewing logs

As an applet, Avestruz logs to the journal through cosmic-panel:

```sh
journalctl --user -f | grep cosmic-applet-avestruz
```

How much it logs is chosen in its settings (the cog above the feed), kept
between runs, and changes at once. `RUST_LOG`, when set, has the say instead
(`RUST_LOG=debug just run`).

## Where things are kept

| What | Where |
| --- | --- |
| Settings (relays, which account is signed in, the size of notes' text, how long notes are kept, how much is logged) | COSMIC config, `io.github.cosmic-applet-avestruz` |
| An account's secret key, or the way to its remote signer | The system keyring, service `cosmic-applet-avestruz`, under the account's `npub` |
| The key a remote signer knows Avestruz by | The same, under `bunker-client:` and the signer's public key |
| Events received from relays | `~/.local/share/cosmic-applet-avestruz/events` (LMDB) |
| Drafts | The event store, encrypted, as that relay took them |
| The draft of a new note, when that relay could not be given it | `~/.local/share/cosmic-applet-avestruz/draft.txt` |
| Fonts of the applet's own, if any were put there | `~/.local/share/cosmic-applet-avestruz/fonts/` |
| When each account last looked at its notifications | COSMIC config, with the settings |

## Project Structure

```
src/
├── nostr/          # Relay client and LMDB event store, the account and its key, feed, thread, contact list, bookmarks, the account's relay lists, search, media servers (Blossom), the relay it runs itself, drafts, what the store lets go of, composing, engagement, which relays a note was seen on, zaps, NIP-05 names, note content and models
├── guide/          # Documentation only: architecture, flows, relays, protocol, decisions, testing
├── handlers/       # Message handlers: auth, feed, thread, profile, compose, engagement, bookmarks, relays, media (media servers, attaching), search, zap, images, previews, video, share, screenshot, navigation, settings, store
├── views/          # UI views
│   ├── components/ # Reusable components: VirtualList (what the lists scroll in), the scroller and its place, note row, quoted note, link preview, video box, avatar, QR code, media placeholder, icons
│   └── *.rs        # Popup, login, feed, thread, profile, compose, zap, settings, what the keys do
└── *.rs            # App model, the button in the panel (and the line between it and the window), state, messages, config, drafts, http, media (a file to attach: its kind, and a picture without what it says beside the picture), images (fetching and keeping pictures), previews (what linked pages say of themselves), video (playing the videos notes link to), youtube (which links are YouTube videos, and asking yt-dlp for one), requests (what views ask to be fetched), screen (how tall it is), links, logging, helpers, i18n
build.rs            # Writes the list of tests the docs show
.github/            # What GitHub runs: the checks and tests on every push, code scanning, dependency updates, and a release for every `vX.Y.Z` tag
fuzz/               # The fuzz targets, a crate of their own (cargo-fuzz)
docs/               # The script that draws the docs' diagrams
tools/mermaid-check # Parses the diagrams in the doc comments
```

## Key Dependencies

| Crate | Purpose |
|---|---|
| [libcosmic](https://github.com/pop-os/libcosmic) | COSMIC application framework |
| [nostr-sdk](https://github.com/rust-nostr/nostr) | Nostr protocol, relay pool, and the relay Avestruz runs itself |
| [tokio-tungstenite](https://crates.io/crates/tokio-tungstenite) | The WebSocket that relay is spoken to over, inside the process |
| [nostr-lmdb](https://github.com/rust-nostr/nostr) | Local event store |
| [nostr-connect](https://github.com/rust-nostr/nostr) | Signing, encrypting and decrypting through a remote signer (NIP-46) |
| [reqwest](https://crates.io/crates/reqwest) | Fetching avatars, images and the pages notes link to, asking Lightning addresses, sending files to media servers |
| [sha2](https://crates.io/crates/sha2), [base64](https://crates.io/crates/base64) | What a file is known by on a media server, and how the authorization to send it is put in a header |
| [html5gum](https://crates.io/crates/html5gum) | Reading the head of a linked page for its preview |
| [image](https://crates.io/crates/image) | Decoding pictures |
| [gstreamer](https://crates.io/crates/gstreamer), gstreamer-app, gstreamer-video | Playing the videos notes link to |
| [qrcode](https://crates.io/crates/qrcode) | The QR code of an invoice |
| [keyring](https://crates.io/crates/keyring) | System credential storage |

## License

[MIT](LICENSE)


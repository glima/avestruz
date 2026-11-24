// SPDX-License-Identifier: MIT

//! Notes and profiles, reduced to what the views show.

use std::borrow::Cow;
use std::collections::HashMap;

use nostr_sdk::prelude::{CommentTarget, Event, EventId, Kind, Metadata, PublicKey, Tag, Tags, ToBech32, Url, nip22};

use crate::helpers::truncate_chars;
use crate::nostr::content::{self, Content, FEED_NOTE_MAX_CHARS, MAX_NOTE_QUOTES, Quote, imeta, link_label};

/// How much of what someone says about themselves is kept. It is theirs to
/// write, at any length.
const ABOUT_MAX_CHARS: usize = 1_000;

/// Where a note stands in a conversation, when it is a reply (NIP-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyTo {
    /// The note the conversation started with.
    pub root: EventId,
    /// The note this one answers: the root itself, for a reply straight to it.
    pub parent: EventId,
}

/// What `event` says it replies to, if to anything that can be named as a
/// note is: a text note by its `e` tags (NIP-10), a comment by the tags of
/// the comment it answers and of what the comments are about (NIP-22). An
/// event of another kind replies to nothing, whatever it cites.
pub(crate) fn reply_to(event: &Event) -> Option<ReplyTo> {
    match event.kind {
        Kind::TextNote => note_reply_to(&event.tags),
        Kind::Comment => comment_reply_to(event),
        _ => None,
    }
}

/// What a comment (kind 1111) answers: the event it names as its parent,
/// under the event the comments are about. Comments on something that is
/// no event (an article by its address, a web page) have no such root; the
/// first comment then stands for it, and answers nothing.
fn comment_reply_to(comment: &Event) -> Option<ReplyTo> {
    let event = |target: Option<CommentTarget<'_>>| match target {
        Some(CommentTarget::Event { id, .. }) => Some(id),
        _ => None,
    };
    let (root, parent) = (event(nip22::extract_root(comment)), event(nip22::extract_parent(comment)));
    let parent = parent.or(root)?;
    Some(ReplyTo { root: root.unwrap_or(parent), parent })
}

/// What the `e` tags of a text note say it replies to, if to anything.
///
/// Tags marked `reply` and `root` say so themselves. Without markers it is
/// the older convention, by position: the first tag is the root and the last
/// the parent. A tag marked `mention` only cites a note, and is no part of
/// this.
fn note_reply_to(tags: &Tags) -> Option<ReplyTo> {
    let tagged: Vec<(EventId, Option<&str>)> = tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            if fields.first().map(String::as_str) != Some("e") {
                return None;
            }
            let id = EventId::from_hex(fields.get(1)?).ok()?;
            Some((id, fields.get(3).map(String::as_str).filter(|marker| !marker.is_empty())))
        })
        .filter(|(_, marker)| *marker != Some("mention"))
        .collect();
    let (first, last) = (tagged.first()?.0, tagged.last()?.0);
    let marked = |marker: &str| tagged.iter().find(|(_, found)| *found == Some(marker)).map(|(id, _)| *id);

    let root = marked("root");
    let parent = marked("reply")
        // A root marker and no reply marker: a reply straight to the root,
        // unless tags without the root marker follow the older convention.
        .or_else(|| root.map(|root| tagged.iter().rev().find(|(_, marker)| *marker != Some("root")).map_or(root, |(id, _)| *id)))
        .unwrap_or(last);
    Some(ReplyTo { root: root.unwrap_or(first), parent })
}

/// Something someone wrote, as a row shows it: a short text note (kind 1),
/// a comment (kind 1111), or, at the top of a thread of comments, whatever
/// they are about, as far as it is text. With the notes it quotes, as far
/// as they were at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: EventId,
    /// What kind of event it is.
    pub kind: Kind,
    pub author: PublicKey,
    /// When the author says it was written, in seconds since the Unix epoch.
    pub created_at: u64,
    /// What it says, parsed, and cut to what its row shows.
    pub content: Content,
    /// What it replies to. `None` for a note that starts a conversation.
    pub reply_to: Option<ReplyTo>,
    /// The notes it quotes, of those its row shows, that were at hand when
    /// it was put together: each as a row shows it, with the notes it
    /// quotes in turn, down to [`QUOTE_DEPTH`](content::QUOTE_DEPTH) notes
    /// from the note of a row. The rest of what it
    /// [quotes](Content::quotes) shows as links.
    pub quoted: Vec<Note>,
}

/// A note as a feed row shows it: a long one is cut.
impl From<Event> for Note {
    fn from(event: Event) -> Self {
        Self::new(&event, FEED_NOTE_MAX_CHARS)
    }
}

impl Note {
    /// `event` as a note, showing at most `max_chars` characters of what it
    /// says.
    fn new(event: &Event, max_chars: usize) -> Self {
        // What the note says of what it links to: how large its pictures
        // are, and which of its links are videos.
        let attached: Vec<_> = event.tags.iter().filter_map(|tag| imeta(tag.as_slice())).collect();
        Self {
            id: event.id,
            kind: event.kind,
            author: event.pubkey,
            created_at: event.created_at.as_secs(),
            content: content::parse_attached(&event.content, max_chars, &attached),
            reply_to: reply_to(event),
            quoted: Vec::new(),
        }
    }

    /// The notes it quotes that its row shows, by id: the first few.
    pub fn quotes_shown(&self) -> impl Iterator<Item = &Quote> + '_ {
        self.content.quotes.iter().take(MAX_NOTE_QUOTES)
    }

    /// This, with those of the notes its row shows it quoting that are
    /// among `found`.
    #[must_use]
    pub fn with_quoted(mut self, found: &HashMap<EventId, Note>) -> Self {
        self.quoted = self.quotes_shown().filter_map(|quote| found.get(&quote.id)).cloned().collect();
        self
    }

    /// The note `id`, if it is one this quotes and it was at hand.
    pub fn quoted(&self, id: EventId) -> Option<&Note> {
        self.quoted.iter().find(|quoted| quoted.id == id)
    }

    /// How many quoted notes it is shown with, the ones in those counted:
    /// more of them is more of the same note at hand.
    pub fn quoted_count(&self) -> usize {
        self.quoted.iter().map(|quoted| 1 + quoted.quoted_count()).sum()
    }

    /// A note with all it says: how the note a thread was opened for shows.
    pub fn in_full(event: &Event) -> Self {
        Self::new(event, usize::MAX)
    }

    /// What `author` is writing, as a row would show it in full if it went
    /// out now as it is, with `said` for what it says of the files it
    /// links to. It is no note yet, and has no id of its own; the notes it
    /// quotes are for whoever has them to add.
    pub fn being_written(text: &str, said: &[Tag], author: PublicKey, now: u64) -> Self {
        let attached: Vec<_> = said.iter().filter_map(|tag| imeta(tag.as_slice())).collect();
        Self {
            id: EventId::from_byte_array([0; 32]),
            kind: Kind::TextNote,
            author,
            created_at: now,
            content: content::parse_attached(text, usize::MAX, &attached),
            reply_to: None,
            quoted: Vec::new(),
        }
    }

    /// Whether it answers something, rather than starting a conversation.
    /// A comment always does, whatever it is about.
    pub fn is_reply(&self) -> bool {
        self.reply_to.is_some() || self.kind == Kind::Comment
    }

    /// Everyone whose profile showing this note needs: its author and the
    /// people it mentions, and those of the notes it is shown quoting, as
    /// deep as they go.
    pub fn people(&self) -> impl Iterator<Item = PublicKey> + '_ {
        let mut people: Vec<PublicKey> = std::iter::once(self.author).chain(self.content.mentions()).collect();
        for quoted in &self.quoted {
            people.extend(quoted.people());
        }
        people.into_iter()
    }
}

/// What an author published about themselves (kind 0), as far as it is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// The name to show: `display_name`, or else `name`. `None` when both are
    /// missing or blank.
    pub name: Option<String>,
    /// Where their picture is, if it is somewhere it can be fetched from: an
    /// `https` address.
    pub picture: Option<String>,
    /// Where the picture across the top of their profile is, likewise.
    pub banner: Option<String>,
    /// The name at a domain they say is theirs (NIP-05). Theirs to say:
    /// whether the domain agrees is another matter (see
    /// [`crate::nostr::nip05`]).
    pub nip05: Option<String>,
    /// Their Lightning address (`lud16`): where they can be zapped.
    pub lightning: Option<String>,
    /// What they say about themselves, cut if it runs long.
    pub about: Option<String>,
    /// Their website, if what they gave is a web address.
    pub website: Option<Website>,
}

/// A web address someone gives as theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Website {
    /// Where it is.
    pub url: String,
    /// How it shows: as links in notes do.
    pub label: String,
}

impl Website {
    /// The web address in `given`, which people write with a scheme or
    /// without one. `None` for anything that is not a web address.
    fn parse(given: &str) -> Option<Self> {
        let given = given.trim();
        let url = if given.contains("://") { Url::parse(given) } else { Url::parse(&format!("https://{given}")) }.ok()?;
        let on_the_web = matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|host| host.contains('.'));
        on_the_web.then(|| Self { label: link_label(&url), url: url.to_string() })
    }
}

impl Profile {
    /// The profile in a kind-0 event's content. Content that isn't profile
    /// JSON gives an empty profile: the author exists, and said nothing usable.
    pub fn from_content(content: &str) -> Self {
        let Ok(metadata) = Metadata::from_json(content) else {
            return Self::default();
        };
        let name = [metadata.display_name, metadata.name]
            .into_iter()
            .flatten()
            .map(|name| name.trim().to_string())
            .find(|name| !name.is_empty());
        let fetchable = |address: Option<String>| {
            address
                .and_then(|address| Url::parse(address.trim()).ok())
                .filter(|url| url.scheme() == "https")
                .map(|url| url.to_string())
        };
        let (picture, banner) = (fetchable(metadata.picture), fetchable(metadata.banner));
        let nip05 = metadata.nip05.map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
        let lightning = metadata.lud16.map(|address| address.trim().to_string()).filter(|address| !address.is_empty());
        let about = metadata
            .about
            .map(|about| truncate_chars(about.trim(), ABOUT_MAX_CHARS).into_owned())
            .filter(|about| !about.is_empty());
        let website = metadata.website.as_deref().and_then(Website::parse);
        Self { name, picture, banner, nip05, lightning, about, website }
    }
}

/// A public key short enough to stand in for a name: `npub1abcd…wxyz`.
pub fn short_npub(public_key: &PublicKey) -> String {
    let Ok(npub) = public_key.to_bech32();
    match (npub.get(..9), npub.get(npub.len().saturating_sub(4)..)) {
        (Some(head), Some(tail)) => format!("{head}\u{2026}{tail}"),
        _ => npub,
    }
}

/// The name someone goes by: the one in their profile, or a short form of
/// their public key until (and unless) they have one.
pub fn display_name<'a>(profiles: &'a HashMap<PublicKey, Profile>, public_key: &PublicKey) -> Cow<'a, str> {
    match profiles.get(public_key).and_then(|profile| profile.name.as_deref()) {
        Some(name) => Cow::Borrowed(name),
        None => Cow::Owned(short_npub(public_key)),
    }
}

#[cfg(test)]
mod tests {
    use super::{ABOUT_MAX_CHARS, Note, Profile, ReplyTo, display_name, note_reply_to, reply_to, short_npub};
    use crate::nostr::content::FEED_NOTE_MAX_CHARS;
    use nostr_sdk::prelude::{EventBuilder, EventId, FinalizeEvent, Keys, Kind, PublicKey, Tag, Tags, ToBech32};
    use std::collections::HashMap;

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    /// The reply the `e` tags `(id, marker)` make a note, an empty marker
    /// being none.
    fn reply(tags: &[(u8, &str)]) -> Option<(EventId, EventId)> {
        let tags = Tags::from_list(
            tags.iter()
                .map(|(byte, marker)| {
                    let mut fields = vec!["e".to_string(), id(*byte).to_hex()];
                    if !marker.is_empty() {
                        fields.extend([String::new(), (*marker).to_string()]);
                    }
                    Tag::parse(fields).unwrap()
                })
                .collect(),
        );
        note_reply_to(&tags).map(|ReplyTo { root, parent }| (root, parent))
    }

    #[test]
    fn what_is_being_written_reads_as_the_note_it_would_be() {
        let keys = Keys::generate();
        let text = "# gm\nlook at **this**\nhttps://cdn.example.com/a.jpg\n".to_string() + &"and more ".repeat(200);
        let said = Tag::parse(["imeta", "url https://cdn.example.com/a.jpg", "dim 800x600"]).unwrap();

        let written = Note::being_written(&text, std::slice::from_ref(&said), keys.public_key(), 7);
        let event = EventBuilder::new(Kind::TextNote, &text).tags([said]).finalize(&keys).unwrap();
        let published = Note::in_full(&event);

        assert_eq!(written.content, published.content, "as the note in full would read, once out");
        assert!(written.content.whole.is_none(), "nothing of it is cut, long as it is");
        assert_eq!(written.content.images[0].size, Some((800, 600)), "what it says of a file attached counts");
        assert_eq!((written.author, written.created_at, written.reply_to), (keys.public_key(), 7, None));
    }

    #[test]
    fn marked_tags_say_what_a_note_replies_to() {
        assert_eq!(reply(&[]), None, "no e tags: not a reply");
        assert_eq!(reply(&[(1, "root")]), Some((id(1), id(1))), "straight to the root");
        assert_eq!(reply(&[(1, "root"), (2, "reply")]), Some((id(1), id(2))));
        assert_eq!(reply(&[(2, "reply"), (1, "root")]), Some((id(1), id(2))), "whatever the order");
        assert_eq!(reply(&[(1, "root"), (3, "mention")]), Some((id(1), id(1))), "a cited note is not answered");
        assert_eq!(reply(&[(3, "mention")]), None, "citing alone is not replying");
    }

    #[test]
    fn unmarked_tags_say_it_by_position() {
        assert_eq!(reply(&[(1, "")]), Some((id(1), id(1))));
        assert_eq!(reply(&[(1, ""), (2, "")]), Some((id(1), id(2))));
        assert_eq!(reply(&[(1, ""), (2, ""), (3, "")]), Some((id(1), id(3))), "first the root, last the parent");
        assert_eq!(reply(&[(1, "root"), (2, "")]), Some((id(1), id(2))), "a marked root and a parent by position");
    }

    #[test]
    fn tags_that_name_no_event_are_passed_over() {
        let tags = Tags::from_list(vec![
            Tag::parse(["e", "not an id"]).unwrap(),
            Tag::parse(["p", &id(7).to_hex()]).unwrap(),
            Tag::parse(["e", &id(1).to_hex()]).unwrap(),
        ]);
        assert_eq!(note_reply_to(&tags), Some(ReplyTo { root: id(1), parent: id(1) }));
    }

    #[test]
    fn a_feed_row_cuts_a_long_note_and_its_own_view_does_not() {
        let long = "word ".repeat(FEED_NOTE_MAX_CHARS);
        let event = EventBuilder::new(Kind::TextNote, long.clone()).finalize(&Keys::generate()).unwrap();
        let shown = |note: &Note| note.content.line_chars.iter().sum::<usize>();

        assert_eq!(shown(&Note::in_full(&event)), long.trim().chars().count());
        assert_eq!(shown(&Note::from(event)), FEED_NOTE_MAX_CHARS);
    }

    #[test]
    fn a_note_is_shown_with_the_first_few_notes_it_quotes_that_are_at_hand() {
        let (me, them, friend) = (Keys::generate(), Keys::generate(), Keys::generate().public_key());
        let said = |text: String, keys: &Keys| Note::from(EventBuilder::new(Kind::TextNote, text).finalize(keys).unwrap());
        let quoted: Vec<Note> =
            (0..5).map(|number| said(format!("quoted {number} nostr:{}", friend.to_bech32().unwrap()), &them)).collect();
        let addresses: Vec<String> = quoted.iter().map(|note| format!("nostr:{}", note.id.to_bech32().unwrap())).collect();
        let note = said(format!("look {}", addresses.join(" ")), &me);
        assert_eq!(note.content.quotes.len(), 5);
        assert!(note.quoted.is_empty(), "nothing is at hand until it is looked for");

        // The second is not to be found, and the last two are more than a
        // row shows.
        let found: HashMap<EventId, Note> =
            [0, 2, 3, 4].into_iter().map(|number: usize| (quoted[number].id, quoted[number].clone())).collect();
        let note = note.with_quoted(&found);
        assert_eq!(note.quoted, [quoted[0].clone(), quoted[2].clone()]);
        assert_eq!(note.quoted(quoted[2].id), Some(&quoted[2]));
        assert_eq!(note.quoted(quoted[1].id), None);

        // Showing it takes knowing who wrote what it quotes, and whom
        // those mention.
        let people: Vec<PublicKey> = note.people().collect();
        assert_eq!(people, [me.public_key(), them.public_key(), friend, them.public_key(), friend]);
        assert_eq!(note.quoted_count(), 2);

        // And so on down: a quoted note that is shown with one it quotes.
        let third = Keys::generate();
        let inner = said("the first of them".to_string(), &third);
        let mut outer = note.clone();
        outer.quoted[0].quoted = vec![inner];
        assert_eq!(outer.quoted_count(), 3);
        assert_eq!(outer.people().nth(3), Some(third.public_key()), "after the note that quotes it, before the next");
    }

    #[test]
    fn someone_goes_by_their_profile_name_or_else_their_short_key() {
        let key = PublicKey::parse("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6").unwrap();
        let mut profiles = HashMap::new();
        assert_eq!(display_name(&profiles, &key), "npub180cv…h6w6");

        profiles.insert(key, Profile::default());
        assert_eq!(display_name(&profiles, &key), "npub180cv…h6w6", "a profile without a name");

        profiles.insert(key, Profile { name: Some("fiatjaf".into()), ..Profile::default() });
        assert_eq!(display_name(&profiles, &key), "fiatjaf");
    }

    #[test]
    fn a_comment_answers_what_it_names_in_small_letters_under_what_it_names_in_capitals() {
        let keys = Keys::generate();
        let comment = |tags: &[&[&str]]| {
            EventBuilder::new(Kind::Comment, "said")
                .tags(tags.iter().map(|tag| Tag::parse(tag.iter().copied()).unwrap()))
                .finalize(&keys)
                .unwrap()
        };
        let (about, answered) = (id(1).to_hex(), id(2).to_hex());

        // An answer to a comment, on an event.
        let deep = comment(&[&["E", &about], &["K", "20"], &["e", &answered], &["k", "1111"]]);
        assert_eq!(reply_to(&deep), Some(ReplyTo { root: id(1), parent: id(2) }));
        assert!(Note::from(deep).is_reply());

        // On an article, named by its address: the comment answered stands
        // for the conversation.
        let on_article = comment(&[&["A", "30023:aa:herons"], &["K", "30023"], &["e", &answered], &["k", "1111"]]);
        assert_eq!(reply_to(&on_article), Some(ReplyTo { root: id(2), parent: id(2) }));

        // On a web page: nothing to name as a note, and a reply all the same.
        let on_page = comment(&[&["I", "https://example.com/"], &["K", "web"], &["i", "https://example.com/"], &["k", "web"]]);
        assert_eq!(reply_to(&on_page), None);
        let shown = Note::from(on_page);
        assert!(shown.is_reply());
        assert_eq!(shown.kind, Kind::Comment);
    }

    #[test]
    fn only_notes_and_comments_reply_to_anything() {
        let keys = Keys::generate();
        let citing = Tag::parse(["e", &id(1).to_hex()]).unwrap();
        let article = EventBuilder::new(Kind::LongFormTextNote, "# Cites a note").tag(citing.clone()).finalize(&keys).unwrap();
        let note = EventBuilder::new(Kind::TextNote, "answers one").tag(citing).finalize(&keys).unwrap();

        assert_eq!(reply_to(&article), None);
        assert!(!Note::from(article).is_reply());
        assert_eq!(reply_to(&note), Some(ReplyTo { root: id(1), parent: id(1) }));
    }

    #[test]
    fn a_profile_prefers_the_display_name_and_skips_blank_ones() {
        let name = |content: &str| Profile::from_content(content).name;
        assert_eq!(name(r#"{"name":"fiatjaf","display_name":"Fiat Jaf"}"#).as_deref(), Some("Fiat Jaf"));
        assert_eq!(name(r#"{"name":"fiatjaf","display_name":"  "}"#).as_deref(), Some("fiatjaf"));
        assert_eq!(name(r#"{"name":" padded "}"#).as_deref(), Some("padded"));
        assert_eq!(name(r#"{"about":"no name here"}"#), None);
    }

    #[test]
    fn a_profile_picture_is_kept_only_if_it_can_be_fetched() {
        let picture = |content: &str| Profile::from_content(content).picture;
        assert_eq!(picture(r#"{"picture":" https://example.com/me.png "}"#).as_deref(), Some("https://example.com/me.png"));
        assert_eq!(picture(r#"{"picture":"http://example.com/me.png"}"#), None, "not https");
        assert_eq!(picture(r#"{"picture":"data:image/png;base64,AAAA"}"#), None);
        assert_eq!(picture(r#"{"picture":"not an address"}"#), None);
        assert_eq!(picture(r#"{"name":"nobody"}"#), None);
    }

    #[test]
    fn a_profile_banner_is_kept_as_its_picture_is() {
        let banner = |content: &str| Profile::from_content(content).banner;
        assert_eq!(banner(r#"{"banner":"https://example.com/wide.jpg"}"#).as_deref(), Some("https://example.com/wide.jpg"));
        assert_eq!(banner(r#"{"banner":"http://example.com/wide.jpg"}"#), None, "not https");
        assert_eq!(banner(r#"{"picture":"https://example.com/me.png"}"#), None);
    }

    #[test]
    fn a_profile_says_which_name_its_author_claims() {
        let nip05 = |content: &str| Profile::from_content(content).nip05;
        assert_eq!(nip05(r#"{"nip05":" ana@example.com "}"#).as_deref(), Some("ana@example.com"));
        assert_eq!(nip05(r#"{"nip05":""}"#), None);
        assert_eq!(nip05(r#"{"name":"ana"}"#), None);
    }

    #[test]
    fn a_profile_says_where_its_author_can_be_zapped() {
        let lightning = |content: &str| Profile::from_content(content).lightning;
        assert_eq!(lightning(r#"{"lud16":" ana@example.com "}"#).as_deref(), Some("ana@example.com"));
        assert_eq!(lightning(r#"{"lud16":""}"#), None);
        assert_eq!(lightning(r#"{"name":"ana"}"#), None);
    }

    #[test]
    fn a_profile_says_what_its_author_says_of_themselves_up_to_a_length() {
        let about = |content: &str| Profile::from_content(content).about;
        assert_eq!(
            about(r#"{"about":" bird watcher\nand relay operator "}"#).as_deref(),
            Some("bird watcher\nand relay operator")
        );
        assert_eq!(about(r#"{"about":"   "}"#), None);
        assert_eq!(about(r#"{"name":"ana"}"#), None);

        let long = about(&format!(r#"{{"about":"{}"}}"#, "a".repeat(ABOUT_MAX_CHARS + 50))).unwrap();
        assert_eq!(long.chars().count(), ABOUT_MAX_CHARS + 1, "cut, and marked as cut");
    }

    #[test]
    fn a_profile_website_is_kept_only_if_it_is_a_web_address() {
        let website = |content: &str| Profile::from_content(content).website.map(|website| (website.url, website.label));
        let (url, label) = website(r#"{"website":" https://www.example.com/me "}"#).unwrap();
        assert_eq!((url.as_str(), label.as_str()), ("https://www.example.com/me", "example.com/me"));
        // Written without a scheme, as people do.
        let (url, label) = website(r#"{"website":"example.com"}"#).unwrap();
        assert_eq!((url.as_str(), label.as_str()), ("https://example.com/", "example.com"));

        assert_eq!(website(r#"{"website":"javascript://example.com/%0Aalert(1)"}"#), None);
        assert_eq!(website(r#"{"website":"file:///etc/passwd"}"#), None);
        assert_eq!(website(r#"{"website":"not a website"}"#), None);
        assert_eq!(website(r#"{"website":""}"#), None);
    }

    #[test]
    fn content_that_is_not_a_profile_gives_an_empty_one() {
        assert_eq!(Profile::from_content("not json"), Profile::default());
        assert_eq!(Profile::from_content(""), Profile::default());
    }

    #[test]
    fn a_short_npub_keeps_both_ends() {
        let key = PublicKey::parse("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6").unwrap();
        assert_eq!(short_npub(&key), "npub180cv…h6w6");
    }
}

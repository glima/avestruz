// SPDX-License-Identifier: MIT

//! What someone wrote, made into an event ready to be signed: a note of its
//! own, a reply to another, or a note that quotes another.
//!
//! The text goes out as it was written. The tags say what a relay or another
//! client can't be expected to read out of the text: which conversation a
//! reply belongs to and whom it answers, who is mentioned, which notes are
//! quoted, and the hashtags.
//!
//! What a reply is depends on what it answers. An answer to a text note is
//! a text note (NIP-10). An answer to anything else — a comment, or what
//! comments are about — is a comment (NIP-22): the two don't mix.
//!
//! A quote is a note with the address of another in its text (NIP-18),
//! which [`quote`] puts under what was written. Nothing else sets it apart:
//! whatever names a note in its text quotes it.
//!
//! ```mermaid
//! flowchart TD
//!     written["What was written, and what it was written to"] --> target{"Written to?"}
//!     target -->|"nothing"| note["Kind 1, with no reply tags"]
//!     target -->|"nothing, about a note"| quoted["Kind 1. The note's address goes under the text"]
//!     target -->|"a note"| parent{"What kind is the note answered?"}
//!     parent -->|"a text note, kind 1"| reply["Kind 1. e tags: the root, and the note answered. p tags: its author, and everyone it spoke to"]
//!     parent -->|"anything else"| comment["Kind 1111. What the comments are about, in capitals, then the note answered"]
//!     note --> text
//!     quoted --> text
//!     reply --> text
//!     comment --> text["Then what the text calls for: a p for each mention, a t for each hashtag, a q for each note quoted"]
//! ```
//!
//! One kind of tag is not read out of the text, since the text doesn't
//! say it: what the note says of a file it links to that was sent to a
//! media server from here (`imeta`, NIP-92; see
//! [`crate::nostr::blossom::imeta`]). Those are added to what [`build`]
//! made, by whoever publishes it.
//!
//! Everything here is pure: text and an event in, an unsigned event out.
//! Signing and sending are elsewhere (see [`crate::guide::flows`]).

use std::collections::HashSet;

use nostr_sdk::prelude::{
    CommentTarget, Event, EventBuilder, EventId, Kind, Nip18Tag, Nip19Event, PublicKey, Tag, ToBech32, nip22,
};

use crate::nostr::content::{self, Quote, Segment};
use crate::nostr::models::{Note, reply_to};

/// What is written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// Nothing: a note of its own.
    Note,
    /// This note: a reply.
    Reply(EventId),
    /// Nothing, about this note: a note of its own that quotes it.
    Quote(EventId),
}

/// What a quote of `quoted` says, when `text` was written about it: the
/// text, and under it the note's address, where other clients show the note.
/// The address says who wrote the note and what kind of event it is, so
/// that it can be found by those who don't have it. With nothing written,
/// the address is all there is.
pub fn quote(text: &str, quoted: &Note) -> String {
    let address = Nip19Event::new(quoted.id).author(quoted.author).kind(quoted.kind).to_bech32().unwrap_or_else(|_| {
        let Ok(address) = quoted.id.to_bech32();
        address
    });
    match text {
        "" => format!("nostr:{address}"),
        text => format!("{text}\nnostr:{address}"),
    }
}

/// The tags the text itself calls for: a `p` for everyone mentioned, a `t`
/// for every hashtag, and a `q` for every note quoted, with a `p` for who
/// wrote it, as far as its address says. Each once; `author`, who writes
/// the text, is not told of quoting themselves.
fn content_tags(text: &str, author: PublicKey) -> Vec<Tag> {
    let mut seen_people = HashSet::new();
    let mut seen_hashtags = HashSet::new();
    let mut tags = Vec::new();
    let said = content::parse(text, usize::MAX);
    for segment in said.segments {
        match segment {
            Segment::Mention(person) if seen_people.insert(person) => tags.push(Tag::public_key(person)),
            // Hashtags are matched in lowercase, whatever case they were
            // written in.
            Segment::Hashtag(hashtag) if seen_hashtags.insert(hashtag.to_lowercase()) => tags.push(Tag::hashtag(hashtag)),
            _ => {}
        }
    }
    for Quote { id, author: quoted, relay, .. } in said.quotes {
        tags.push(Nip18Tag::Quote { id, relay_hint: relay, public_key: quoted }.into());
        if let Some(quoted) = quoted.filter(|quoted| *quoted != author && seen_people.insert(*quoted)) {
            tags.push(Tag::public_key(quoted));
        }
    }
    tags
}

/// The tags that make a note a reply to `parent`, written by `author`.
///
/// The conversation's root and the note answered are named with markers; a
/// reply to the root itself names it once, as the root. Everyone the parent
/// spoke to is spoken to again, and its author first: that is who hears of
/// the reply. The one replying is not told of their own reply.
fn reply_tags(parent: &Event, author: PublicKey) -> Vec<Tag> {
    let marked = |id: EventId, marker: &str| Tag::custom("e", [id.to_hex(), String::new(), marker.to_string()]);
    let mut tags = match reply_to(parent) {
        Some(above) => vec![marked(above.root, "root"), marked(parent.id, "reply")],
        None => vec![marked(parent.id, "root")],
    };
    let mut seen = HashSet::from([author]);
    tags.extend(
        std::iter::once(parent.pubkey)
            .chain(parent.tags.public_keys())
            .filter(|person| seen.insert(*person))
            .map(Tag::public_key),
    );
    tags
}

/// The tags that make a comment an answer to `parent`, which is not a text
/// note (NIP-22).
///
/// What the comments are about is named first, in capitals: what `parent`
/// says that is, if it is a comment itself, and `parent`, if it is what is
/// commented on. Then `parent`, as what is answered. Something that is
/// known by an address (an article) is named by it, and by its id as well,
/// which is the version answered.
fn comment_tags(parent: &Event) -> Vec<Tag> {
    let answered = CommentTarget::from(parent);
    let about = match parent.kind {
        Kind::Comment => nip22::extract_root(parent),
        _ => None,
    }
    .unwrap_or_else(|| answered.clone());

    let mut tags = about.as_vec(true);
    tags.extend(answered.as_vec(false));
    if parent.coordinate().is_some() {
        tags.push(Tag::event(parent.id));
    }
    tags
}

/// `text` as an event of `author`'s, yet to be signed: a note; a reply to
/// `parent`, if that is a text note; or a comment on `parent`, if it is
/// anything else. A note that quotes another is a note, whatever it quotes.
/// A tag the reply already carries is not added again for the text's sake.
pub fn build(text: &str, parent: Option<&Event>, author: PublicKey) -> EventBuilder {
    let (kind, mut tags) = match parent {
        None => (Kind::TextNote, Vec::new()),
        Some(parent) if parent.kind == Kind::TextNote => (Kind::TextNote, reply_tags(parent, author)),
        Some(parent) => (Kind::Comment, comment_tags(parent)),
    };
    for tag in content_tags(text, author) {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    EventBuilder::new(kind, text).tags(tags)
}

#[cfg(test)]
mod tests {
    use super::{build, quote};
    use crate::nostr::content;
    use crate::nostr::models::{Note, ReplyTo, reply_to};
    use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent, FromBech32, Keys, Kind, Nip19Event, Tag, ToBech32};

    fn signed(builder: EventBuilder, keys: &Keys) -> Event {
        builder.finalize(keys).unwrap()
    }

    /// The tags of `event`, each as its fields.
    fn tags(event: &Event) -> Vec<Vec<&str>> {
        event.tags.iter().map(|tag| tag.as_slice().iter().map(String::as_str).collect()).collect()
    }

    #[test]
    fn a_note_goes_out_as_written_with_no_tags_it_did_not_ask_for() {
        let me = Keys::generate();
        let note = signed(build("gm, everyone", None, me.public_key()), &me);

        assert_eq!(note.kind, Kind::TextNote);
        assert_eq!(note.content, "gm, everyone");
        assert!(note.tags.is_empty());
        assert_eq!(reply_to(&note), None);
    }

    #[test]
    fn mentions_and_hashtags_are_tagged_each_once() {
        let me = Keys::generate();
        let friend = Keys::generate().public_key();
        let npub = friend.to_bech32().unwrap();
        let text = format!("#Nostr with nostr:{npub}, again nostr:{npub} #nostr #rust");
        let note = signed(build(&text, None, me.public_key()), &me);

        let friend = friend.to_hex();
        assert_eq!(tags(&note), [vec!["t", "nostr"], vec!["p", friend.as_str()], vec!["t", "rust"]]);
        assert_eq!(note.content, text, "the text is left as it was written");
    }

    #[test]
    fn a_reply_to_a_note_that_starts_a_conversation_names_it_as_the_root() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let root = signed(build("it starts", None, them.public_key()), &them);
        let reply = signed(build("an answer", Some(&root), me.public_key()), &me);

        let (root_id, their_key) = (root.id.to_hex(), them.public_key().to_hex());
        assert_eq!(tags(&reply), [vec!["e", root_id.as_str(), "", "root"], vec!["p", their_key.as_str()]]);
        assert_eq!(reply_to(&reply), Some(ReplyTo { root: root.id, parent: root.id }));
    }

    #[test]
    fn a_reply_further_down_names_the_root_the_parent_and_everyone_spoken_to() {
        let (me, starter, answerer) = (Keys::generate(), Keys::generate(), Keys::generate());
        let root = signed(build("it starts", None, starter.public_key()), &starter);
        let parent = signed(build("an answer", Some(&root), answerer.public_key()), &answerer);
        let reply = signed(build("an answer to the answer", Some(&parent), me.public_key()), &me);

        let (root_id, parent_id) = (root.id.to_hex(), parent.id.to_hex());
        let (starter_key, answerer_key) = (starter.public_key().to_hex(), answerer.public_key().to_hex());
        assert_eq!(
            tags(&reply),
            [
                vec!["e", root_id.as_str(), "", "root"],
                vec!["e", parent_id.as_str(), "", "reply"],
                vec!["p", answerer_key.as_str()],
                vec!["p", starter_key.as_str()],
            ]
        );
        assert_eq!(reply_to(&reply), Some(ReplyTo { root: root.id, parent: parent.id }));
    }

    #[test]
    fn replying_tells_nobody_twice_and_not_oneself() {
        let (me, them) = (Keys::generate(), Keys::generate());
        // They answered me, and I answer back, mentioning them by key.
        let mine = signed(build("it starts", None, me.public_key()), &me);
        let theirs = signed(build("an answer", Some(&mine), them.public_key()), &them);
        let text = format!("thanks nostr:{}", them.public_key().to_bech32().unwrap());
        let reply = signed(build(&text, Some(&theirs), me.public_key()), &me);

        let people: Vec<Tag> = reply.tags.iter().filter(|tag| tag.kind() == "p").cloned().collect();
        assert_eq!(people, [Tag::public_key(them.public_key())]);
    }

    #[test]
    fn an_answer_to_a_comment_is_a_comment_under_what_the_comments_are_about() {
        let (me, them, artist) = (Keys::generate(), Keys::generate(), Keys::generate());
        // A picture, and a comment on it.
        let picture = signed(EventBuilder::new(Kind::Custom(20), "a heron"), &artist);
        let comment = signed(build("what a bird", Some(&picture), them.public_key()), &them);
        let (picture_id, artist_key) = (picture.id.to_hex(), artist.public_key().to_hex());

        assert_eq!(comment.kind, Kind::Comment);
        assert_eq!(
            tags(&comment),
            [
                vec!["E", picture_id.as_str(), "", artist_key.as_str()],
                vec!["P", artist_key.as_str()],
                vec!["K", "20"],
                vec!["e", picture_id.as_str(), "", artist_key.as_str()],
                vec!["p", artist_key.as_str()],
                vec!["k", "20"],
            ]
        );
        assert_eq!(reply_to(&comment), Some(ReplyTo { root: picture.id, parent: picture.id }));

        // An answer to the comment is about the picture still, and answers
        // the comment.
        let answer = signed(build("isn't it", Some(&comment), me.public_key()), &me);
        let (comment_id, their_key) = (comment.id.to_hex(), them.public_key().to_hex());

        assert_eq!(answer.kind, Kind::Comment);
        assert_eq!(
            tags(&answer),
            [
                vec!["E", picture_id.as_str(), "", artist_key.as_str()],
                vec!["P", artist_key.as_str()],
                vec!["K", "20"],
                vec!["e", comment_id.as_str(), "", their_key.as_str()],
                vec!["p", their_key.as_str()],
                vec!["k", "1111"],
            ]
        );
        assert_eq!(reply_to(&answer), Some(ReplyTo { root: picture.id, parent: comment.id }));
    }

    #[test]
    fn a_comment_on_an_article_names_it_by_its_address_and_its_id() {
        let (me, writer) = (Keys::generate(), Keys::generate());
        let article = signed(EventBuilder::new(Kind::LongFormTextNote, "# On herons").tag(Tag::identifier("herons")), &writer);
        let comment = signed(build("well put", Some(&article), me.public_key()), &me);
        let address = format!("30023:{}:herons", writer.public_key().to_hex());
        let (article_id, writer_key) = (article.id.to_hex(), writer.public_key().to_hex());

        assert_eq!(comment.kind, Kind::Comment);
        assert_eq!(
            tags(&comment),
            [
                vec!["A", address.as_str()],
                vec!["P", writer_key.as_str()],
                vec!["K", "30023"],
                vec!["a", address.as_str()],
                vec!["p", writer_key.as_str()],
                vec!["k", "30023"],
                vec!["e", article_id.as_str()],
            ]
        );
        // Under the article, as far as threads go.
        assert_eq!(reply_to(&comment), Some(ReplyTo { root: article.id, parent: article.id }));
    }

    #[test]
    fn an_answer_to_a_text_note_is_never_a_comment() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let note = signed(build("gm", None, them.public_key()), &them);
        assert_eq!(signed(build("gm!", Some(&note), me.public_key()), &me).kind, Kind::TextNote);
    }

    #[test]
    fn a_quote_names_the_note_in_its_text_and_its_tags_and_tells_its_author() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let quoted = Note::from(signed(build("worth a second look", None, them.public_key()), &them));

        let text = quote("look at this", &quoted);
        assert!(text.starts_with("look at this\nnostr:nevent1"), "{text}");
        let published = signed(build(&text, None, me.public_key()), &me);

        assert_eq!(published.kind, Kind::TextNote);
        assert_eq!(published.content, text, "the address is in the text, where other clients look for it");
        let (quoted_id, their_key) = (quoted.id.to_hex(), them.public_key().to_hex());
        assert_eq!(tags(&published), [vec!["q", quoted_id.as_str(), "", their_key.as_str()], vec!["p", their_key.as_str()]]);
        assert_eq!(reply_to(&published), None, "a quote answers nothing");

        // The address says what was quoted: who wrote it, and its kind.
        let named = content::parse(&text, usize::MAX).quotes;
        assert_eq!(
            named.iter().map(|quote| (quote.id, quote.author)).collect::<Vec<_>>(),
            [(quoted.id, Some(them.public_key()))]
        );
        let address = text.rsplit("nostr:").next().unwrap();
        assert_eq!(Nip19Event::from_bech32(address).unwrap().kind, Some(Kind::TextNote));

        // With nothing said about it, the note's address is the quote.
        let bare = signed(build(&quote("", &quoted), None, me.public_key()), &me);
        assert_eq!(bare.content, format!("nostr:{address}"));
        assert_eq!(tags(&bare), tags(&published));
    }

    #[test]
    fn quoting_tags_each_note_once_and_tells_nobody_twice_nor_oneself() {
        let (me, them) = (Keys::generate(), Keys::generate());
        let mine = Note::from(signed(build("as I said", None, me.public_key()), &me));
        let theirs = Note::from(signed(build("as they said", None, them.public_key()), &them));
        let bare = theirs.id.to_bech32().unwrap();

        let text = format!("nostr:{} said", them.public_key().to_bech32().unwrap());
        let text = quote(&quote(&quote(&text, &theirs), &theirs), &mine);
        let text = format!("{text} nostr:{bare}");
        let published = signed(build(&text, None, me.public_key()), &me);

        let (their_id, my_id) = (theirs.id.to_hex(), mine.id.to_hex());
        let (their_key, my_key) = (them.public_key().to_hex(), me.public_key().to_hex());
        assert_eq!(
            tags(&published),
            [
                vec!["p", their_key.as_str()],
                vec!["q", their_id.as_str(), "", their_key.as_str()],
                vec!["q", my_id.as_str(), "", my_key.as_str()],
            ]
        );
    }

    #[test]
    fn a_reply_that_quotes_is_a_reply_still() {
        let (me, them, third) = (Keys::generate(), Keys::generate(), Keys::generate());
        let root = signed(build("it starts", None, them.public_key()), &them);
        let elsewhere = Note::from(signed(build("said elsewhere", None, third.public_key()), &third));
        let reply = signed(build(&quote("see", &elsewhere), Some(&root), me.public_key()), &me);

        assert_eq!(reply_to(&reply), Some(ReplyTo { root: root.id, parent: root.id }));
        let quoted: Vec<&str> = reply.tags.iter().filter(|tag| tag.kind() == "q").filter_map(|tag| tag.content()).collect();
        assert_eq!(quoted, [elsewhere.id.to_hex()]);
    }
}

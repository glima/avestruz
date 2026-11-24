// SPDX-License-Identifier: MIT

//! Drafts as events (NIP-37): what is written and not published, as the
//! relay Avestruz runs itself keeps it.
//!
//! A draft is kept as a draft wrap (kind 31234): an addressable event the
//! account signs, whose content is the note the draft would be, unsigned,
//! as JSON, encrypted to the account itself (NIP-44). Its `d` tag says
//! which draft it is, so that a later one replaces it, and its `k` tag
//! what kind of event it wraps. A wrap with no content is a draft that
//! was done with: published, or cleared.
//!
//! There is one draft for each thing that can be written to, as in
//! [`crate::drafts`], and the `d` tag is which: a note of its own, a reply
//! to a note, a quote of one.
//!
//! ```mermaid
//! flowchart TD
//!     text["What was written"] --> note["The note it would be: kind 1, unsigned, as JSON"]
//!     note -- "encrypted to the account (NIP-44)" --> content
//!     subgraph wrap["A draft wrap, kind 31234, signed by the account"]
//!         direction TB
//!         d["d: which draft it is"]
//!         k["k: 1, the kind it wraps"]
//!         e["e: the note answered or quoted, if any"]
//!         content["content: the encrypted note, or nothing for a draft done with"]
//!     end
//!     wrap --> relay["The relay Avestruz runs itself, and no other"]
//! ```
//!
//! | Written to | `d` tag |
//! |---|---|
//! | A note of its own | `avestruz:note` |
//! | A reply to a note | `avestruz:reply:` and the note's id |
//! | A quote of a note | `avestruz:quote:` and the note's id |
//!
//! Wraps with any other `d` tag are another client's drafts, and are left
//! alone.

use nostr_sdk::prelude::{Event, EventBuilder, EventId, Kind, PublicKey, Tag, Timestamp};
use serde_json::{Value, json};

use crate::nostr::blossom;
use crate::nostr::compose::Target;

/// The kind of a draft wrap.
pub const DRAFT_KIND: Kind = Kind::Custom(31234);

/// What the `d` tag of a draft says, by what the draft is of: a note of
/// its own; or, before the id of the note it is about, a reply or a quote.
const NOTE: &str = "avestruz:note";
const REPLY: &str = "avestruz:reply:";
const QUOTE: &str = "avestruz:quote:";

/// What the `d` tag of the draft for `target` says.
pub fn identifier(target: Target) -> String {
    match target {
        Target::Note => NOTE.to_string(),
        Target::Reply(note) => format!("{REPLY}{}", note.to_hex()),
        Target::Quote(note) => format!("{QUOTE}{}", note.to_hex()),
    }
}

/// What the draft whose `d` tag says `identifier` is a draft of, if it is
/// one of this client's.
pub fn target_of(identifier: &str) -> Option<Target> {
    let about = |id: &str| EventId::from_hex(id).ok();
    if identifier == NOTE {
        Some(Target::Note)
    } else if let Some(id) = identifier.strip_prefix(REPLY) {
        about(id).map(Target::Reply)
    } else {
        identifier.strip_prefix(QUOTE).and_then(about).map(Target::Quote)
    }
}

/// What a draft wrap's content is before it is encrypted: the note `text`
/// would be if `account` published it at `now`, unsigned, as JSON. Of its
/// tags it has those in `said`: what it says of the files it links to
/// (NIP-92), which nothing but having sent them tells.
pub fn written(account: PublicKey, text: &str, said: &[Tag], now: Timestamp) -> String {
    let tags: Vec<&[String]> = said.iter().map(Tag::as_slice).collect();
    let note = json!({
        "kind": Kind::TextNote.as_u16(),
        "pubkey": account.to_hex(),
        "created_at": now.as_secs(),
        "tags": tags,
        "content": text,
    });
    note.to_string()
}

/// What the draft that `json` is says of the files it links to: its
/// `imeta` tags. None if it is no event, or has none.
pub fn said_of(json: &str) -> Vec<Tag> {
    let draft: Value = serde_json::from_str(json).unwrap_or_default();
    let tags = draft.get("tags").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let fields =
        |tag: &Value| -> Option<Vec<String>> { tag.as_array()?.iter().map(|field| field.as_str().map(str::to_string)).collect() };
    tags.iter()
        .filter_map(fields)
        .filter_map(|fields| Tag::parse(fields).ok())
        .filter(|tag| blossom::imeta_url(tag).is_some())
        .collect()
}

/// The text of the draft that `json` is, which is what a draft wrap's
/// content says once it is decrypted. `None` if it is no event, or says
/// nothing.
pub fn text_of(json: &str) -> Option<String> {
    let draft: Value = serde_json::from_str(json).ok()?;
    let text = draft.get("content")?.as_str()?;
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// The draft wrap for `target`, with `content` for what it wraps:
/// encrypted, or nothing at all for a draft that is done with. Written at
/// `now` or, if `earlier`, the wrap it replaces, claims to be from later,
/// after it: it has to be the newer of the two.
pub fn wrap(target: Target, content: String, earlier: Option<&Event>, now: Timestamp) -> EventBuilder {
    let mut tags = vec![Tag::identifier(identifier(target))];
    // What kind of event it wraps: a note.
    tags.extend(Tag::parse(["k", Kind::TextNote.as_u16().to_string().as_str()]).ok());
    // What it is about, for whoever reads the drafts elsewhere.
    if let Target::Reply(note) | Target::Quote(note) = target {
        tags.push(Tag::event(note));
    }
    let after = earlier.map_or(now, |earlier| now.max(Timestamp::from_secs(earlier.created_at.as_secs() + 1)));
    EventBuilder::new(DRAFT_KIND, content).tags(tags).custom_created_at(after)
}

/// What the draft wrap `event` is a draft of, and its content, which is
/// empty when the draft is done with. `None` if it is no draft wrap, or
/// not one of this client's.
pub fn kept(event: &Event) -> Option<(Target, &str)> {
    if event.kind != DRAFT_KIND {
        return None;
    }
    Some((target_of(&event.tags.identifier()?)?, event.content.as_str()))
}

#[cfg(test)]
mod tests {
    use super::{DRAFT_KIND, identifier, kept, said_of, target_of, text_of, wrap, written};
    use crate::nostr::compose::Target;
    use nostr_sdk::prelude::{EventBuilder, EventId, FinalizeEvent, Keys, Kind, Tag, Timestamp};

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    #[test]
    fn each_thing_written_to_has_a_draft_of_its_own() {
        for target in [Target::Note, Target::Reply(id(1)), Target::Quote(id(1)), Target::Reply(id(2))] {
            assert_eq!(target_of(&identifier(target)), Some(target));
        }
        assert_ne!(identifier(Target::Reply(id(1))), identifier(Target::Quote(id(1))));
        for not_ours in ["", "note", "avestruz:reply:", "avestruz:reply:xyz", "some-other-client-draft"] {
            assert_eq!(target_of(not_ours), None, "{not_ours}");
        }
    }

    #[test]
    fn a_draft_is_the_note_it_would_be_and_reads_back_as_its_text() {
        let keys = Keys::generate();
        let json = written(keys.public_key(), "half a thought\nand a line", &[], Timestamp::from_secs(100));
        assert!(json.contains(r#""kind":1"#) && json.contains(&keys.public_key().to_hex()), "{json}");
        assert_eq!(text_of(&json).as_deref(), Some("half a thought\nand a line"));
        assert!(said_of(&json).is_empty());

        // As another client may have written one, and what is no draft.
        assert_eq!(text_of(r#"{"kind":1,"content":"theirs","tags":[]}"#).as_deref(), Some("theirs"));
        for nothing in ["", "[]", r#"{"content":"  "}"#, r#"{"content":7}"#, "not json"] {
            assert_eq!(text_of(nothing), None, "{nothing}");
        }
    }

    #[test]
    fn a_draft_keeps_what_it_says_of_the_files_it_links_to() {
        let keys = Keys::generate();
        let said = Tag::parse(["imeta", "url https://cdn.example.com/abc.png", "m image/png", "dim 3x2"]).unwrap();
        let json = written(
            keys.public_key(),
            "look https://cdn.example.com/abc.png",
            std::slice::from_ref(&said),
            Timestamp::from_secs(100),
        );
        assert_eq!(said_of(&json), [said]);
        assert_eq!(text_of(&json).as_deref(), Some("look https://cdn.example.com/abc.png"));

        // Of another client's draft, only what is said of a link.
        let theirs = r#"{"kind":1,"content":"x","tags":[["t","nostr"],["imeta","m image/png"],["imeta","url https://a.example/b.jpg"],[1]]}"#;
        assert_eq!(said_of(theirs), [Tag::parse(["imeta", "url https://a.example/b.jpg"]).unwrap()]);
        for nothing in ["", "[]", r#"{"tags":7}"#, "not json"] {
            assert!(said_of(nothing).is_empty(), "{nothing}");
        }
    }

    #[test]
    fn a_draft_wrap_says_which_draft_it_is_and_replaces_the_one_before() {
        let keys = Keys::generate();
        let now = Timestamp::from_secs(200);
        let first = wrap(Target::Reply(id(3)), "encrypted".to_string(), None, now).finalize(&keys).unwrap();
        assert_eq!((first.kind, first.created_at), (DRAFT_KIND, now));
        let fields: Vec<Vec<String>> = first.tags.iter().map(|tag| tag.as_slice().to_vec()).collect();
        assert_eq!(
            fields,
            [
                vec!["d".to_string(), identifier(Target::Reply(id(3)))],
                vec!["k".into(), "1".into()],
                vec!["e".into(), id(3).to_hex()]
            ]
        );
        assert_eq!(kept(&first), Some((Target::Reply(id(3)), "encrypted")));

        // Done with: no content. Written in the same second, it is still
        // the later of the two.
        let done = wrap(Target::Reply(id(3)), String::new(), Some(&first), now).finalize(&keys).unwrap();
        assert_eq!(done.created_at.as_secs(), 201);
        assert_eq!(kept(&done), Some((Target::Reply(id(3)), "")));
        let note = wrap(Target::Note, "x".to_string(), None, now).finalize(&keys).unwrap();
        assert_eq!(note.tags.len(), 2);

        // Another client's draft, and what is no draft at all.
        let theirs = EventBuilder::new(DRAFT_KIND, "x").tag(nostr_sdk::prelude::Tag::identifier("abc")).finalize(&keys).unwrap();
        assert_eq!(kept(&theirs), None);
        assert_eq!(kept(&EventBuilder::new(Kind::TextNote, "x").finalize(&keys).unwrap()), None);
    }
}

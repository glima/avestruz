// SPDX-License-Identifier: MIT

//! A link to a note that can be given to anyone: a web address, which
//! opens in a browser whether or not whoever gets it has a Nostr client.
//!
//! The note is named in it by its `nevent` (NIP-19): its id, who wrote it,
//! what kind it is, and a couple of the relays it was seen on, so that
//! whatever opens the link knows where to look. Nothing is asked of
//! anyone to make one, and nothing is told to anyone until it is opened.
//!
//! The page it leads to is [`SHARE_GATEWAY`]'s: a site that shows a note
//! to a browser and offers to open it in a client. That is somebody
//! else's server, and the one thing in a share link that is not the
//! note's own.

use nostr_sdk::prelude::{EventId, Kind, Nip19Event, PublicKey, RelayUrl, ToBech32};

use crate::nostr::outbox::public;

/// Where a share link leads: the site that shows the note it names.
pub(crate) const SHARE_GATEWAY: &str = "https://njump.me/";

/// How many relays a share link names for its note: enough to find it by
/// if one is gone, and no more than keeps the link short.
const SHARE_HINTS: usize = 2;

/// The link to give someone to the note `id` of kind `kind`, written by
/// `author` and seen on the relays `seen_on`. Of those only relays anyone
/// can reach are named: not one on this machine or this network.
pub fn share_link(id: EventId, author: PublicKey, kind: Kind, seen_on: &[RelayUrl]) -> String {
    let hints = seen_on.iter().filter(|relay| public(relay)).take(SHARE_HINTS).cloned();
    let named = Nip19Event::new(id).author(author).kind(kind).relays(hints);
    // An address always encodes; the note's bare id stands in if it ever
    // did not.
    let address = named.to_bech32().unwrap_or_else(|_| {
        let Ok(note) = id.to_bech32();
        note
    });
    format!("{SHARE_GATEWAY}{address}")
}

#[cfg(test)]
mod tests {
    use super::{SHARE_GATEWAY, share_link};
    use nostr_sdk::prelude::{EventId, FromBech32, Keys, Kind, Nip19Event, RelayUrl};

    fn relay(address: &str) -> RelayUrl {
        RelayUrl::parse(address).unwrap()
    }

    /// The note a share link names.
    fn named(link: &str) -> Nip19Event {
        Nip19Event::from_bech32(link.strip_prefix(SHARE_GATEWAY).unwrap()).unwrap()
    }

    #[test]
    fn a_share_link_is_a_web_address_that_names_the_note_and_where_it_is() {
        let (id, author) = (EventId::from_byte_array([7; 32]), Keys::generate().public_key());
        let seen_on = [relay("wss://one.example.com"), relay("wss://two.example.com"), relay("wss://three.example.com")];

        let link = share_link(id, author, Kind::TextNote, &seen_on);
        assert!(link.starts_with("https://njump.me/nevent1"), "{link}");
        let note = named(&link);
        assert_eq!((note.event_id, note.author, note.kind), (id, Some(author), Some(Kind::TextNote)));
        // The first two it was seen on, and no more.
        assert_eq!(note.relays, seen_on[..2]);

        // Whatever kind the note is, the link says.
        assert_eq!(named(&share_link(id, author, Kind::Comment, &[])).kind, Some(Kind::Comment));
    }

    #[test]
    fn a_share_link_names_no_relay_only_this_machine_can_reach() {
        let (id, author) = (EventId::from_byte_array([7; 32]), Keys::generate().public_key());
        let seen_on = [
            relay("ws://127.0.0.1:10547"),
            relay("wss://192.168.1.20"),
            relay("ws://plain.example.com"),
            relay("wss://far.example.com"),
        ];
        assert_eq!(named(&share_link(id, author, Kind::TextNote, &seen_on)).relays, [relay("wss://far.example.com")]);

        // A note no relay was seen sending is still named, by who wrote it.
        let unseen = named(&share_link(id, author, Kind::TextNote, &[]));
        assert!(unseen.relays.is_empty());
        assert_eq!(unseen.author, Some(author));
    }
}

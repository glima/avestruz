// SPDX-License-Identifier: MIT

//! Which relays a note was seen on: every relay that sent it, for as long
//! as the applet has been running.
//!
//! A note comes from as many relays as have it, and is one note all the
//! same. Which relays those were says how far it went, and where someone
//! else can find it. The event store doesn't keep that, so it is kept
//! here, in memory: a note read from the store has been seen nowhere until
//! a relay sends it again.
//!
//! There are few relays and many notes, so each note has a bit for every
//! relay rather than a list of them.

use std::collections::HashMap;

use nostr_sdk::prelude::{EventId, RelayUrl};

/// How many relays can be told apart: as many as a note has bits. A relay
/// past that many is not kept track of.
const MAX_RELAYS: usize = u64::BITS as usize;

/// Which relays each note was seen on.
#[derive(Debug, Default)]
pub struct Sightings {
    /// The relays anything was seen on, in the order they first sent
    /// something: a relay's place here is its bit in `seen`.
    relays: Vec<RelayUrl>,
    seen: HashMap<EventId, u64>,
}

impl Sightings {
    /// The bit that stands for `relay`, which gets one if it has none and
    /// there is one left.
    fn bit(&mut self, relay: &RelayUrl) -> Option<u64> {
        let place = match self.relays.iter().position(|known| known == relay) {
            Some(place) => place,
            None if self.relays.len() < MAX_RELAYS => {
                self.relays.push(relay.clone());
                self.relays.len() - 1
            }
            None => return None,
        };
        Some(1 << place)
    }

    /// The note `id` was sent by `relay`.
    pub fn record(&mut self, id: EventId, relay: &RelayUrl) {
        if let Some(bit) = self.bit(relay) {
            *self.seen.entry(id).or_default() |= bit;
        }
    }

    /// The relays the note `id` was seen on, in the order relays were first
    /// heard from. None, for a note no relay sent.
    pub fn of(&self, id: &EventId) -> Vec<RelayUrl> {
        let seen = self.seen.get(id).copied().unwrap_or_default();
        self.relays.iter().enumerate().filter(|(place, _)| seen & (1 << place) != 0).map(|(_, relay)| relay.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_RELAYS, Sightings};
    use nostr_sdk::prelude::{EventId, RelayUrl};

    fn relay(name: &str) -> RelayUrl {
        RelayUrl::parse(&format!("wss://{name}.example.com")).unwrap()
    }

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    #[test]
    fn a_note_is_seen_on_every_relay_that_sent_it_each_once() {
        let mut sightings = Sightings::default();
        sightings.record(id(1), &relay("a"));
        sightings.record(id(2), &relay("b"));
        sightings.record(id(1), &relay("c"));
        sightings.record(id(1), &relay("a"));

        assert_eq!(sightings.of(&id(1)), [relay("a"), relay("c")]);
        assert_eq!(sightings.of(&id(2)), [relay("b")]);
        assert!(sightings.of(&id(3)).is_empty(), "a note no relay sent");
    }

    #[test]
    fn relays_past_what_can_be_told_apart_are_not_kept_track_of() {
        let mut sightings = Sightings::default();
        for number in 0..MAX_RELAYS + 5 {
            sightings.record(id(1), &relay(&format!("r{number}")));
        }
        let seen = sightings.of(&id(1));
        assert_eq!(seen.len(), MAX_RELAYS);
        assert_eq!(seen.last(), Some(&relay(&format!("r{}", MAX_RELAYS - 1))));
    }
}

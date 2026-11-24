// SPDX-License-Identifier: MIT

//! Sharing a note: copying a link to it that anyone can open.

use cosmic::iced::clipboard;
use cosmic::prelude::*;
use nostr_sdk::prelude::EventId;

use crate::messages::Message;
use crate::nostr::share::share_link;
use crate::state::AppModel;

impl AppModel {
    /// Put a link to the note `id` on the clipboard (see
    /// [`share_link`]), naming the relays it was seen on, and have its
    /// row say so. Nothing is copied of a note that is no longer at hand:
    /// its row is not there to be pressed.
    pub fn handle_copy_share_link(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        let Some(note) = self.note_at_hand(id) else {
            return Task::none();
        };
        let seen_on = self.seen_on.get(&id).map(Vec::as_slice).unwrap_or_default();
        let link = share_link(note.id, note.author, note.kind, seen_on);
        self.share_copied = Some(id);
        self.picture_copied = None;
        clipboard::write(link)
    }
}

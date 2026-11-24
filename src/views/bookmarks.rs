// SPDX-License-Identifier: MIT

//! The bookmarks view for Avestruz: the notes the account has bookmarked,
//! the one bookmarked last first.
//!
//! These are the bookmarks the account's list keeps to itself, which are
//! the only ones made here and the only ones read. A click on a note opens
//! its thread, and its ribbon takes the bookmark off, or puts it back.
//!
//! The notes are looked up a page at a time as the list is scrolled, and
//! what is on show stays as it was when the page was opened: a note whose
//! bookmark is taken off stays in its place, with an empty ribbon, so that
//! nothing moves under the pointer.

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container, text};

use crate::fl;
use crate::messages::Message;
use crate::nostr::models::Note;
use crate::state::{AppModel, BookmarkState, BookmarkedNotes, BookmarksPage};
use crate::views::components::{
    VirtualList, estimated_note_height, list_scroller, marking, note_row, opening_thread, status_line,
};
use crate::views::feed::LOAD_OLDER_WITHIN;

/// What is said under the notes of `listed` once every one of them has
/// been looked up, or in their place when there is none to show: that this
/// is all, and how many bookmarks are not shown.
fn end_text(listed: &BookmarkedNotes) -> String {
    let hidden = listed.hidden();
    match (listed.notes.is_empty(), hidden) {
        (true, 0) => fl!("bookmarks-none"),
        (true, _) => fl!("bookmarks-hidden", count = hidden),
        (false, 0) => fl!("bookmarks-end"),
        (false, _) => format!("{} {}", fl!("bookmarks-end"), fl!("bookmarks-hidden", count = hidden)),
    }
}

impl AppModel {
    /// The line above the bookmarks: the way back, and what this is.
    fn view_bookmarks_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("bookmarks-back"))
            .on_press(Message::ClosePage);
        let title = container(text(fl!("bookmarks-title")).size(16).font(cosmic::font::bold())).width(Length::Fill);
        widget::Row::new().push(back).push(title).spacing(8).align_y(Alignment::Center).padding([4, 12]).into()
    }

    /// The notes of the bookmarks page `page`, in the scroller, and under
    /// them what is going on at their end.
    fn view_bookmarked_notes<'a>(&'a self, page: &'a BookmarksPage) -> Element<'a, Message> {
        let listed = &page.notes;
        let end = if listed.all_asked() { end_text(listed) } else { fl!("bookmarks-loading") };
        if listed.notes.is_empty() {
            return status_line(end);
        }
        let context = self.row_context(&listed.expanded, listed.marked);
        let sizes = context.sizes;
        let list: VirtualList<'_, Note, _, Message, cosmic::Theme, cosmic::Renderer> = VirtualList::new(
            &listed.notes,
            &listed.heights,
            |note: &Note| note.id,
            move |note, width| estimated_note_height(note, sizes, width, context.expanded.has(&note.id)),
            move |_index, note| marking(opening_thread(note_row(note, &context), note.id, &context), note.id, &context),
        )
        .on_near_end(LOAD_OLDER_WITHIN, Message::LoadBookmarkedNotes(page.id));

        // Said once the notes are all there, or while more are on their
        // way; between pages there is nothing to say.
        let end = (listed.all_asked() || listed.loading).then(|| status_line(end));
        list_scroller(&listed.position, widget::Column::new().push(list).push_maybe(end))
    }

    /// Render the bookmarks view, of the page `page`.
    pub fn view_bookmarks<'a>(&'a self, page: &'a BookmarksPage) -> Element<'a, Message> {
        let body = match &self.bookmarks {
            BookmarkState::Unknown => status_line(fl!("bookmarks-reading")),
            BookmarkState::Unreadable(why) => status_line(fl!("note-bookmark-unreadable", error = why.as_str())),
            BookmarkState::Known(_) => self.view_bookmarked_notes(page),
        };
        widget::Column::new().push(self.view_bookmarks_header()).push(widget::divider::horizontal::default()).push(body).into()
    }
}

#[cfg(test)]
mod tests {
    use super::end_text;
    use crate::nostr::bookmarks::Bookmarks;
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::models::Note;
    use crate::state::BookmarkedNotes;
    use nostr_sdk::prelude::{EventId, Keys, Kind};

    fn id(byte: u8) -> EventId {
        EventId::from_byte_array([byte; 32])
    }

    fn e(byte: u8) -> Vec<String> {
        vec!["e".to_string(), id(byte).to_hex()]
    }

    fn note(byte: u8) -> Note {
        Note {
            id: id(byte),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse("worth finding again", FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        }
    }

    #[test]
    fn a_page_keeps_what_it_found_until_the_bookmarks_are_others() {
        let mut side = BookmarkedNotes::default();
        assert!(side.all_asked(), "nothing to ask about before the bookmarks are known");

        let bookmarks = Bookmarks::new(None, vec![e(1), e(2), e(3)]);
        assert!(side.take(&bookmarks));
        assert_eq!((side.taken, &side.ids[..]), (1, &[id(3), id(2), id(1)][..]), "the last bookmarked first");
        assert!(!side.all_asked());

        // Two of them were looked up, one found, and the list scrolled.
        side.notes.push(note(3));
        side.asked = 2;
        side.missing = 1;
        side.position.set(120.0);

        // The same bookmarks again, as when the relays have what the
        // store had: nothing starts over.
        assert!(!side.take(&bookmarks));
        assert_eq!((side.taken, side.asked, side.missing, side.notes.len()), (1, 2, 1, 1));
        assert_eq!(side.position.get(), 120.0);

        // Others: from the top, and what is asked of the old ones is
        // known for stale by the count.
        let more = Bookmarks::new(None, vec![e(1), e(2), e(3), e(4)]);
        assert!(side.take(&more));
        assert_eq!((side.taken, side.asked, side.missing, side.notes.len()), (2, 0, 0, 0));
        assert_eq!(side.ids.first(), Some(&id(4)));
        assert_eq!(side.position.get(), 0.0);
    }

    #[test]
    fn the_end_of_the_bookmarks_says_what_is_not_shown() {
        let mut side = BookmarkedNotes::default();

        // No bookmarks kept that way at all.
        side.take(&Bookmarks::default());
        assert!(side.all_asked());
        assert!(end_text(&side).starts_with("No bookmarks yet"), "{}", end_text(&side));

        // Bookmarks, none of which is a note to show.
        let article = vec!["a".to_string(), "30023:abc:post".to_string()];
        side.take(&Bookmarks::new(None, vec![article.clone()]));
        assert!(end_text(&side).starts_with("1 bookmark is not shown"), "{}", end_text(&side));

        // Two notes nobody has, and the article.
        side.take(&Bookmarks::new(None, vec![e(1), article, e(2), e(3)]));
        assert!(!side.all_asked());
        side.asked = 2;
        side.missing = 2;
        let three = end_text(&side);
        assert!(three.contains('3') && three.contains("bookmarks are not shown"), "{three}");

        // With a note to show above it, it follows what says that is all.
        side.notes.push(note(3));
        side.asked = 3;
        let after = end_text(&side);
        assert!(after.starts_with("That is all of them.") && after.contains("bookmarks are not shown"), "{after}");
        side.missing = 0;
        side.others = 0;
        assert_eq!(end_text(&side), "That is all of them.");
    }
}

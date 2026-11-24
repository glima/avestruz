// SPDX-License-Identifier: MIT

//! The thread view for Avestruz: the conversation one note is part of, in
//! reading order, in a scroller.
//!
//! Every note is under the one it answers, set in from it. The popup is
//! narrow, so only the first few answers deep are set in further; past
//! that a note keeps the place of the one above it. The note the
//! thread was opened for is picked out, and shows in full; a click on any
//! other opens that note's thread over this one.

use cosmic::Element;
use cosmic::iced::{Alignment, Background, Border, Color, Length};
use cosmic::widget::{self, button, container, text};

use crate::fl;
use crate::messages::Message;
use crate::nostr::thread::ThreadRow;
use crate::state::{AppModel, Expanded, ThreadState};
use crate::views::components::{
    RowSizes, VirtualList, estimated_note_height, list_scroller, marking, note_row, opening_thread, status_line,
};

/// How far an answer is set in from the note it answers, and how many
/// answers deep that goes on.
const THREAD_INDENT: f32 = 14.0;
const THREAD_MAX_INDENTS: usize = 3;

/// The gap between the line that marks an answer and the answer.
const THREAD_GUIDE_GAP: f32 = 7.0;

/// How strongly the accent colour tints the note the thread was opened
/// for, and how much room the tint has around the note.
const FOCUSED_TINT: f32 = 0.12;
const FOCUSED_PADDING: u16 = 6;
const FOCUSED_RADIUS: f32 = 6.0;

/// How tall a thread row will be, near enough, before it has been laid
/// out, set in `sizes` in a list `width` wide, with the long notes among
/// `expanded` in full. An answer set in is a little narrower than the
/// estimate assumes, and so sometimes a line taller: the list finds out
/// when the row is in view.
pub fn estimated_row_height(row: &ThreadRow, sizes: RowSizes, width: f32, expanded: &Expanded) -> f32 {
    estimated_note_height(&row.note, sizes, width, expanded.has(&row.note.id))
}

/// `note`, the row of an answer `depth` deep, set in under what it answers
/// with a line down its left.
fn set_in<'a>(note: Element<'a, Message>, depth: usize) -> Element<'a, Message> {
    let indents = depth.min(THREAD_MAX_INDENTS);
    if indents == 0 {
        return note;
    }
    // The line is a pixel wide: with the room before it and the gap after,
    // it takes the indent.
    let before = indents as f32 * THREAD_INDENT - THREAD_GUIDE_GAP - 1.0;
    widget::Row::new()
        .push(widget::Space::new().width(before))
        .push(widget::divider::vertical::default())
        .push(note)
        .spacing(THREAD_GUIDE_GAP)
        // A row takes after its children, and the line would fill any
        // height: said after it is pushed, so the row is as tall as the note.
        .height(Length::Shrink)
        .into()
}

/// `note`, the row of the note the thread was opened for, picked out from
/// the others by a tint of the accent colour.
fn picked_out<'a>(note: Element<'a, Message>, accent: Color) -> Element<'a, Message> {
    container(note)
        .padding([0, FOCUSED_PADDING])
        .class(cosmic::theme::Container::custom(move |_theme| container::Style {
            background: Some(Background::Color(Color { a: FOCUSED_TINT, ..accent })),
            border: Border { radius: FOCUSED_RADIUS.into(), ..Default::default() },
            ..Default::default()
        }))
        .into()
}

impl AppModel {
    /// The line above the thread: the way back, what this is, and, for an
    /// account that can publish, replying to the note the thread is for.
    fn view_thread_header(&self, state: &ThreadState) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("thread-back"))
            .on_press(Message::ClosePage);
        let title = container(text(fl!("thread-title")).size(16).font(cosmic::font::bold())).width(Length::Fill);
        let mut header = widget::Row::new().push(back).push(title);
        if self.can_publish() && state.thread.focused_row().is_some() {
            header = header.push(button::text(fl!("thread-reply")).on_press(Message::ComposeReply(state.thread.focused())));
        }
        header.spacing(8).align_y(Alignment::Center).padding([4, 12]).into()
    }

    /// The notes of `state`, in the scroller.
    fn view_thread_list<'a>(&'a self, state: &'a ThreadState) -> Element<'a, Message> {
        let context = self.row_context(&state.expanded, state.marked);
        let (accent, sizes) = (context.accent, context.sizes);
        let focused = state.thread.focused();
        let list: VirtualList<'_, ThreadRow, _, Message, cosmic::Theme, cosmic::Renderer> = VirtualList::new(
            state.thread.rows(),
            &state.heights,
            |row: &ThreadRow| row.note.id,
            move |row, width| estimated_row_height(row, sizes, width, context.expanded),
            move |_index, row| {
                let note = note_row(&row.note, &context);
                let note =
                    if row.note.id == focused { picked_out(note, accent) } else { opening_thread(note, row.note.id, &context) };
                set_in(marking(note, row.note.id, &context), row.depth)
            },
        );

        // Said only while there are no answers: under a conversation, it
        // would be about what is already there to read.
        let mut content = widget::Column::new().push(list);
        if !state.thread.has_replies() {
            content = content.push(status_line(if state.loading { fl!("thread-loading") } else { fl!("thread-no-replies") }));
        }
        list_scroller(&state.position, content)
    }

    /// Render the thread view, of the thread `state`.
    pub fn view_thread<'a>(&'a self, state: &'a ThreadState) -> Element<'a, Message> {
        let body = if state.thread.rows().is_empty() {
            status_line(if state.loading { fl!("thread-looking") } else { fl!("thread-not-found") })
        } else {
            self.view_thread_list(state)
        };

        widget::Column::new().push(self.view_thread_header(state)).push(widget::divider::horizontal::default()).push(body).into()
    }
}

#[cfg(test)]
mod tests {
    use super::{THREAD_MAX_INDENTS, picked_out, set_in};
    use crate::images::ImageStore;
    use crate::messages::Message;
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::models::Note;
    use crate::previews::PreviewStore;
    use crate::state::Expanded;
    use crate::video::Videos;
    use crate::views::components::{Bookmarking, Reacting, RowContext, RowSizes, note_row};
    use cosmic::Element;
    use cosmic::iced::core::layout;
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::{Color, Size};
    use nostr_sdk::prelude::{EventId, Keys, Kind};
    use std::collections::HashMap;

    /// How tall `dress` makes the row of a note of a few lines, in a list
    /// as wide as the popup's.
    fn height(dress: impl for<'a> Fn(Element<'a, Message>) -> Element<'a, Message>) -> f32 {
        let note = Note {
            id: EventId::from_byte_array([7; 32]),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse("one\ntwo\nthree", FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        };
        let renderer = crate::views::components::testing::renderer();
        let (profiles, images) = (HashMap::new(), ImageStore::new(1_000_000).0);
        let previews = PreviewStore::new().0;
        let engagement = HashMap::new();
        let seen_on = HashMap::new();
        let videos = Videos::default();
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(13),
        };
        let mut element = dress(note_row(&note, &context));
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(374.0, f32::INFINITY));
        element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
    }

    #[test]
    fn a_row_is_as_tall_set_in_or_picked_out_as_it_is_plain() {
        // The line down an answer's left is as tall as the answer, not as
        // tall as it is allowed to be.
        let plain = height(|note| note);
        assert!(plain > 50.0 && plain < 200.0, "{plain}");
        assert_eq!(height(|note| set_in(note, 1)), plain);
        assert_eq!(height(|note| set_in(note, THREAD_MAX_INDENTS + 5)), plain);
        assert_eq!(height(|note| picked_out(note, Color::WHITE)), plain);
    }
}

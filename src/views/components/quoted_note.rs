// SPDX-License-Identifier: MIT

//! A note another note quotes, shown under what that one says: a card with
//! who wrote the quoted note and when, and what it says, as a row would
//! show it: its text, its images, the preview of the page it links to. A
//! click opens its thread.
//!
//! A quoted note shows the notes it quotes in turn the same way, a card in
//! its card, and there it ends: quoting has no end of its own, so only so
//! many notes deep are looked for (see
//! [`QUOTE_DEPTH`](crate::nostr::content::QUOTE_DEPTH)). A quoted note that
//! is not at hand, for that or because nobody has it, shows as a link to
//! it, on a line of its own.
//!
//! Whether a quoted note is at hand is settled before the note quoting it
//! is ever shown (see [`Note::quoted`]), so a card doesn't turn up in a row
//! that is being read.

use cosmic::Element;
use cosmic::iced::widget::text::{Span, Wrapping};
use cosmic::iced::widget::{rich_text, span};
use cosmic::iced::{Alignment, Length, mouse};
use cosmic::widget::{self, container, text};
use nostr_sdk::prelude::EventId;

use crate::helpers::relative_time;
use crate::messages::Message;
use crate::nostr::content::Quote;
use crate::nostr::models::{Note, display_name};
use crate::nostr::outbox::Whereabouts;
use crate::views::components::note_row::{NOTE_SPACING, Reading, RowContext, RowSizes, line_height, said, said_height};
use crate::views::components::{CARD_BORDER, avatar, card_style};

/// The room between the card's edge and the note in it.
const QUOTE_PADDING: f32 = 8.0;

/// How wide the picture of the quoted note's author is, and the gap
/// between it and their name.
const QUOTE_AVATAR_SIZE: f32 = 20.0;
const QUOTE_AVATAR_GAP: u16 = 8;

/// How much narrower than its card a quoted note is: the card's line and
/// the room inside it, on both sides.
const QUOTE_INSET: f32 = 2.0 * (CARD_BORDER + QUOTE_PADDING);

/// How tall the card of the quoted note `note` will be, near enough, set
/// in `sizes` in a row `width` wide.
pub fn quoted_note_height(note: &Note, sizes: RowSizes, width: f32) -> f32 {
    let header = QUOTE_AVATAR_SIZE.max(line_height(sizes.name()));
    QUOTE_INSET + header + said_height(note, sizes, (width - QUOTE_INSET).max(1.0), Reading::Quoted)
}

/// The card of the quoted note `note`: who wrote it and how long ago, and
/// under that what it says. It opens the note's thread when it is clicked
/// anywhere that doesn't lead somewhere of its own; not while a menu is
/// open over the list, as a row doesn't.
pub fn quoted_note<'a>(note: &'a Note, context: &RowContext<'a>) -> Element<'a, Message> {
    let RowContext { profiles, images, now, sizes, .. } = *context;
    let name = display_name(profiles, &note.author);
    let picture = profiles.get(&note.author).and_then(|profile| profile.picture.as_deref());

    let header = widget::Row::new()
        .push(avatar(images, picture, &note.author, &name, QUOTE_AVATAR_SIZE))
        .push(
            container(text(name).size(sizes.name()).font(cosmic::font::bold()).wrapping(Wrapping::None))
                .width(Length::Fill)
                .clip(true),
        )
        .push(text(relative_time(now, note.created_at)).size(sizes.time()).wrapping(Wrapping::None))
        .spacing(QUOTE_AVATAR_GAP)
        .align_y(Alignment::Center);
    let body = said(widget::Column::new().push(header).spacing(NOTE_SPACING).width(Length::Fill), note, context, Reading::Quoted);
    let card = container(body).padding(CARD_BORDER + QUOTE_PADDING).width(Length::Fill).class(card_style());

    let area = widget::mouse_area(card);
    if context.under_a_menu() {
        area.into()
    } else {
        area.on_press(Message::OpenThread(note.id, Whereabouts::default())).interaction(mouse::Interaction::Pointer).into()
    }
}

/// A quoted note that is not at hand: the short form of its address, on a
/// line of its own, which opens its thread, where it is looked for again.
pub fn quote_link<'a>(quote: &'a Quote, context: &RowContext<'a>) -> Element<'a, Message> {
    // With where its address says it is: that is where it is looked for.
    let link = (quote.id, quote.whereabouts());
    let spans: Vec<Span<'a, (EventId, Whereabouts)>> = vec![span(quote.label.as_str()).color(context.accent).link(link)];
    rich_text(spans)
        .size(context.sizes.text())
        .wrapping(Wrapping::WordOrGlyph)
        .width(Length::Fill)
        .on_link_click(|(id, whereabouts)| Message::OpenThread(id, whereabouts))
        .into()
}

// SPDX-License-Identifier: MIT

//! How something shown inside a note is set apart from the note: a quoted
//! note, the preview of a web page. A thin rounded line around it, in the
//! colour of the lines between rows.

use cosmic::iced::Border;
use cosmic::widget::container;

/// The line around a card, and how round its corners are: as a note's
/// images are.
pub const CARD_BORDER: f32 = 1.0;
pub const CARD_RADIUS: f32 = 6.0;

/// The style of a container that is a card.
pub fn card_style() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|theme| container::Style {
        border: Border { color: theme.cosmic().bg_divider().into(), width: CARD_BORDER, radius: CARD_RADIUS.into() },
        ..container::Style::default()
    })
}

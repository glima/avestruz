// SPDX-License-Identifier: MIT

//! A line of small centred text: what a list says when it has nothing to
//! show, and under its last row.

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{container, text};

use crate::messages::Message;

/// A line of small centred text.
pub fn status_line<'a>(message: String) -> Element<'a, Message> {
    container(text(message).size(12)).width(Length::Fill).align_x(Alignment::Center).padding(16).into()
}

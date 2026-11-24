// SPDX-License-Identifier: MIT

//! A small choice button: one of a few things to choose between.

use cosmic::Element;
use cosmic::widget::{button, text};

use crate::messages::Message;

/// A small choice button, highlighted when it is the choice made.
pub fn chip<'a>(label: String, active: bool, on_press: Message) -> Element<'a, Message> {
    let class = if active { cosmic::theme::Button::Suggested } else { cosmic::theme::Button::Standard };
    button::custom(text(label).size(13)).padding([4, 12]).class(class).on_press(on_press).into()
}

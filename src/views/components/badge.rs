// SPDX-License-Identifier: MIT

//! A small mark over the corner of something: how many new things are
//! behind it, or only that there are some.

use cosmic::Element;
use cosmic::iced::{Background, Border, Color, Length};
use cosmic::widget::{container, text};

use crate::messages::Message;

/// How tall a count is, and how wide at the least: a circle around one
/// figure.
const COUNT_SIZE: f32 = 14.0;

/// How wide and tall the mark is that only says there is something.
const DOT_SIZE: f32 = 7.0;

/// `content`, on the accent colour, with its ends rounded all the way.
fn on_accent<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    height: f32,
) -> container::Container<'a, Message, cosmic::Theme> {
    container(content).height(Length::Fixed(height)).class(cosmic::theme::Container::custom(move |theme| {
        let cosmic = theme.cosmic();
        container::Style {
            text_color: Some(cosmic.on_accent_color().into()),
            background: Some(Background::Color(cosmic.accent_color().into())),
            border: Border { radius: (height / 2.0).into(), ..Default::default() },
            ..Default::default()
        }
    }))
}

/// `count`, small, on the accent colour: for over the corner of a button.
pub fn count_badge<'a>(count: String) -> Element<'a, Message> {
    let on_accent_color: Color = cosmic::theme::active().cosmic().on_accent_color().into();
    let figures = text(count).size(9).font(cosmic::font::bold()).class(cosmic::theme::Text::Color(on_accent_color));
    on_accent(figures, COUNT_SIZE).padding([0, 4]).center_x(Length::Shrink).center_y(Length::Fixed(COUNT_SIZE)).into()
}

/// A dot of the accent colour: there is something new, however much.
pub fn dot_badge<'a, Message: 'a>() -> Element<'a, Message> {
    on_accent(cosmic::widget::Space::new(), DOT_SIZE).width(Length::Fixed(DOT_SIZE)).into()
}

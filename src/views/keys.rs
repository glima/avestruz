// SPDX-License-Identifier: MIT

//! The view that says what the keys do, on a list and elsewhere (see
//! [`crate::handlers::keys`]): they are nowhere else to be found out. It
//! is what `?` opens, and Keyboard in the menu above the feed.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Border, Length};
use cosmic::widget::{self, button, container, text};

use crate::fl;
use crate::messages::Message;
use crate::state::AppModel;

/// How wide the keys are set, before what they do: the widest of them,
/// two keys and what is between them.
const KEYS_WIDTH: f32 = 150.0;

/// A key, as a key looks: its name in a box of its own.
fn key_cap<'a>(name: &str) -> Element<'a, Message> {
    container(text(name.to_string()).size(12).font(cosmic::font::mono()).wrapping(Wrapping::None))
        .padding([1, 6])
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            container::Style {
                border: Border { color: cosmic.bg_divider().into(), width: 1.0, radius: cosmic.corner_radii.radius_xs.into() },
                ..container::Style::default()
            }
        }))
        .into()
}

/// One line of what the keys do: the keys `keys`, any of which does it,
/// and what that is.
fn key_line<'a>(keys: &[&str], does: String) -> Element<'a, Message> {
    let caps = keys.iter().fold(widget::Row::new().spacing(4).align_y(Alignment::Center), |caps, key| caps.push(key_cap(key)));
    widget::Row::new()
        .push(container(caps).width(Length::Fixed(KEYS_WIDTH)))
        .push(text(does).size(12).wrapping(Wrapping::WordOrGlyph).width(Length::Fill))
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// Some of the keys, under what they have in common: `lines`, each the
/// keys that do a thing and what that is, under `heading`.
fn key_group<'a>(heading: String, lines: Vec<(&[&str], String)>) -> Element<'a, Message> {
    lines
        .into_iter()
        .fold(widget::Column::new().push(text(heading).size(14)).spacing(8), |group, (keys, does)| {
            group.push(key_line(keys, does))
        })
        .into()
}

impl AppModel {
    /// The line above the keys: the way back, and what this is.
    fn view_keys_header(&self) -> Element<'_, Message> {
        let back =
            button::icon(widget::icon::from_name("go-previous-symbolic")).tooltip(fl!("keys-back")).on_press(Message::CloseKeys);
        widget::Row::new()
            .push(back)
            .push(text(fl!("keys-title")).size(16).font(cosmic::font::bold()))
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// Render the view of what the keys do: those that go about a list
    /// and from it, those that do something to the marked note, and
    /// those that are not a list's.
    pub fn view_keys(&self) -> Element<'_, Message> {
        let right_click = fl!("keys-right-click");
        let about = key_group(
            fl!("keys-about"),
            vec![
                (&["j", "k"], fl!("keys-step")),
                (&["Home"], fl!("keys-top")),
                (&["l", "Enter"], fl!("keys-open")),
                (&["h", "Backspace"], fl!("keys-go-back")),
                (&["."], fl!("keys-new")),
                (&["B"], fl!("keys-bookmarks")),
                (&["?"], fl!("keys-help")),
            ],
        );
        let note = key_group(
            fl!("keys-note"),
            vec![
                (&["r"], fl!("keys-reply")),
                (&["R"], fl!("keys-repost")),
                (&["z"], fl!("keys-zap")),
                (&["b"], fl!("keys-bookmark")),
                (&["y"], fl!("keys-share")),
                (&["Space"], fl!("keys-play")),
            ],
        );
        let elsewhere = key_group(
            fl!("keys-elsewhere"),
            vec![
                // One key to a box: these two are pressed together.
                (&["Ctrl+Enter"], fl!("keys-publish")),
                (&["Backspace"], fl!("keys-leave-composer")),
                (&["Esc"], fl!("keys-escape")),
                (&[right_click.as_str()], fl!("keys-copy-picture")),
                (&[right_click.as_str()], fl!("keys-who")),
            ],
        );
        let body = widget::Column::new()
            .push(about)
            .push(note)
            .push(elsewhere)
            .push(text(fl!("keys-description")).size(11).wrapping(Wrapping::WordOrGlyph))
            .spacing(16);

        // In a scroller: on a small screen there is more of it than room.
        widget::Column::new()
            .push(self.view_keys_header())
            .push(widget::divider::horizontal::default())
            .push(widget::scrollable(container(body).padding([8, 12]).width(Length::Fill)))
            .into()
    }
}

// SPDX-License-Identifier: MIT

//! A small menu that opens under what was pressed: what can be done to
//! pass a note on, what the cog above the feed offers, how fast a video
//! plays.

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget::button::Catalog;
use cosmic::widget::menu::menu_button;
use cosmic::widget::{self, container, icon, text};

use crate::messages::Message;

/// How wide a menu is, the room around what it offers, and the gap between
/// what was pressed and the menu.
pub const MENU_WIDTH: f32 = 160.0;
const MENU_PADDING: u16 = 4;
pub const MENU_GAP: f32 = 2.0;

/// How tall one thing a menu offers is, at most.
const MENU_ITEM_HEIGHT: f32 = 36.0;

/// How tall a menu that offers `items` things is, at most: the room it
/// needs under what was pressed to show all of them.
pub fn menu_height(items: usize) -> f32 {
    f32::from(MENU_PADDING) * 2.0 + MENU_ITEM_HEIGHT * items as f32
}

/// A menu of `choices`, one under the other, `width` wide: each what it
/// is called, whether it is the one chosen now, which is ticked, and what
/// choosing it does.
pub fn choice_menu<'a>(choices: impl IntoIterator<Item = (String, bool, Message)>, width: f32) -> Element<'a, Message> {
    let choices = choices.into_iter().fold(widget::Column::new(), |choices, (label, chosen, on_press)| {
        let tick: Element<'a, Message> = if chosen {
            icon::from_name("object-select-symbolic").size(16).into()
        } else {
            widget::Space::new().width(16).into()
        };
        let choice = menu_button(vec![tick, widget::Space::new().width(8).into(), text(label).size(14).into()]);
        choices.push(choice.on_press(on_press))
    });
    container(choices).padding(MENU_PADDING).width(Length::Fixed(width)).class(cosmic::theme::Container::Dropdown).into()
}

/// A menu of `items`, one under the other: each a picture, what it is
/// called, and what choosing it does. One that does nothing shows, and
/// can't be chosen.
pub fn menu<'a>(items: impl IntoIterator<Item = (icon::Handle, String, Option<Message>)>) -> Element<'a, Message> {
    menu_at(items, None)
}

/// How an item of a menu looks while the keyboard is at it: as one the
/// pointer is over, wherever the pointer is.
fn at_the_keys() -> cosmic::theme::Button {
    use cosmic::theme::Button::MenuItem;
    cosmic::theme::Button::Custom {
        active: Box::new(|focused, theme| theme.hovered(focused, false, &MenuItem)),
        hovered: Box::new(|focused, theme| theme.hovered(focused, false, &MenuItem)),
        pressed: Box::new(|focused, theme| theme.pressed(focused, false, &MenuItem)),
        disabled: Box::new(|theme| theme.disabled(&MenuItem)),
    }
}

/// As [`menu`], with the keyboard at the item that is `at` in `items`, if
/// it is at one: that item is shown as one the pointer is over.
pub fn menu_at<'a>(
    items: impl IntoIterator<Item = (icon::Handle, String, Option<Message>)>,
    at: Option<usize>,
) -> Element<'a, Message> {
    let items = items.into_iter().enumerate().fold(widget::Column::new(), |items, (place, (picture, label, on_press))| {
        let item = menu_button(vec![
            icon::icon(picture).size(16).into(),
            widget::Space::new().width(8).into(),
            text(label).size(14).into(),
        ]);
        let item = if at == Some(place) { item.class(at_the_keys()) } else { item };
        items.push(item.on_press_maybe(on_press))
    });
    container(items).padding(MENU_PADDING).width(Length::Fixed(MENU_WIDTH)).class(cosmic::theme::Container::Dropdown).into()
}

// SPDX-License-Identifier: MIT

//! What a note can be reacted to with, on offer under its like.
//!
//! First a row: the like itself, the emoji this reader reacts with most
//! (see [`crate::emoji`]), and a way to all the others. That way swaps the
//! row for every emoji there is: a field to find one by what it is
//! called, the groups they come in, and the emoji of the group chosen, or
//! those found, in a grid that scrolls. Pressing any emoji reacts with it.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::widget::scrollable;
use cosmic::iced::widget::text::Shaping;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, button, container, icon, text, text_input};
use nostr_sdk::prelude::EventId;

use crate::emoji::{self, Group};
use crate::fl;
use crate::messages::Message;
use crate::nostr::engagement::Reaction;
use crate::state::{EmojiPicker, ReactionMenu};
use crate::views::components::like_icon;

/// How large an emoji to react with is set, the room each has to be
/// pressed in, and the room around them all.
const EMOJI_SIZE: u16 = 20;
const EMOJI_CELL: f32 = 34.0;
const REACTIONS_PADDING: u16 = 4;

/// How many emoji the grid of all of them has across, and how tall it is
/// before it scrolls.
const PICKER_COLUMNS: usize = 8;
const PICKER_HEIGHT: f32 = 6.0 * EMOJI_CELL;

/// The room beside the grid for the bar it scrolls by, which is drawn
/// over what it scrolls: without it the last emoji of each row is under
/// the bar.
const PICKER_SCROLLBAR_ROOM: f32 = 14.0;

/// The field an emoji is found by, by which it is told to take the
/// keyboard.
static EMOJI_SEARCH: LazyLock<widget::Id> = LazyLock::new(widget::Id::unique);

/// The id of the field an emoji is found by.
pub fn emoji_search_id() -> widget::Id {
    EMOJI_SEARCH.clone()
}

/// `shown`, in the room one thing to react with has, doing `on_press`
/// when pressed.
fn cell<'a>(shown: impl Into<Element<'a, Message>>, on_press: Message) -> Element<'a, Message> {
    button::custom(container(shown).center(Length::Fill))
        .padding(0)
        .width(Length::Fixed(EMOJI_CELL))
        .height(Length::Fixed(EMOJI_CELL))
        .class(cosmic::theme::Button::Text)
        .on_press(on_press)
        .into()
}

/// `emoji`, to be pressed: it reacts to the note `note` with itself. It
/// is set in the colour of text, as it is in a note: an emoji drawn in
/// one colour would take the colour of a button otherwise, where it is
/// what is said and not what is done.
fn emoji_cell<'a>(note: EventId, emoji: &str) -> Element<'a, Message> {
    let color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
    let shown = text(emoji.to_string()).size(EMOJI_SIZE).shaping(Shaping::Advanced).class(cosmic::theme::Text::Color(color));
    cell(shown, Message::React(note, Reaction::Emoji(emoji.to_string())))
}

/// What is offered, set apart from what it is over as a menu is.
fn offered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).padding(REACTIONS_PADDING).class(cosmic::theme::Container::Dropdown).into()
}

/// The row offered first: the like, each of `suggested`, and the way to
/// every other emoji.
fn reaction_row<'a>(note: EventId, suggested: &[String]) -> Element<'a, Message> {
    let like = widget::tooltip(
        cell(icon::icon(like_icon(false)).size(EMOJI_SIZE), Message::React(note, Reaction::Like)),
        text(fl!("note-like")).size(12),
        widget::tooltip::Position::Top,
    );
    let more = widget::tooltip(
        cell(icon::from_name("view-more-symbolic").size(16), Message::OpenEmojiPicker),
        text(fl!("note-react-more")).size(12),
        widget::tooltip::Position::Top,
    );
    let emoji = suggested.iter().map(|emoji| emoji_cell(note, emoji));
    offered(widget::Row::with_children(std::iter::once(like.into()).chain(emoji).chain([more.into()])).align_y(Alignment::Center))
}

/// Every emoji there is, to pick one from: the field to find one by, the
/// groups, and the emoji of the group chosen or those found.
fn emoji_picker<'a>(note: EventId, picker: &'a EmojiPicker) -> Element<'a, Message> {
    let across = PICKER_COLUMNS as f32 * EMOJI_CELL + PICKER_SCROLLBAR_ROOM;
    let search =
        text_input(fl!("emoji-search-placeholder"), &picker.typed).id(emoji_search_id()).on_input(Message::EmojiSearchTyped);

    // The groups, each stood for by its first emoji; the one on show is
    // marked, unless what is typed has put a search in its place.
    let searching = !picker.typed.trim().is_empty();
    let groups = Group::iter().map(|group| {
        let shown = text(emoji::standing_for(group)).size(16).shaping(Shaping::Advanced);
        let chosen = group == picker.group && !searching;
        button::custom(container(shown).center(Length::Fill))
            .padding(0)
            .width(Length::Fill)
            .height(Length::Fixed(EMOJI_CELL - 4.0))
            .class(if chosen { cosmic::theme::Button::Suggested } else { cosmic::theme::Button::Text })
            .on_press(Message::EmojiGroup(group))
            .into()
    });

    let found = emoji::found(&picker.typed, picker.group);
    let grid: Element<'a, Message> = if found.is_empty() {
        container(text(fl!("emoji-search-nothing")).size(13)).center_x(Length::Fill).padding(12).into()
    } else {
        let rows = found
            .chunks(PICKER_COLUMNS)
            .map(|row| widget::Row::with_children(row.iter().map(|emoji| emoji_cell(note, emoji))).into());
        widget::Column::with_children(rows).into()
    };
    let grid = scrollable(grid).width(Length::Fill).height(Length::Fixed(PICKER_HEIGHT));

    let picker = widget::Column::new()
        .push(search)
        .push(widget::Row::with_children(groups))
        .push(grid)
        .spacing(4)
        .width(Length::Fixed(across));
    offered(picker)
}

/// What `menu` offers to react to its note with: the row offered first,
/// or every emoji there is once those were asked for.
pub fn reaction_menu<'a>(menu: &'a ReactionMenu, suggested: &[String]) -> Element<'a, Message> {
    match &menu.picker {
        Some(picker) => emoji_picker(menu.note, picker),
        None => reaction_row(menu.note, suggested),
    }
}

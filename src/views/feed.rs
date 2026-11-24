// SPDX-License-Identifier: MIT

//! The feed view for Avestruz: the notes of the people the account follows,
//! newest first, in a scroller.
//!
//! The notes are in a [`VirtualList`], so only the rows on screen are ever
//! built, however far the feed has been scrolled into the past.
//!
//! What is on show is the reader's, and stays put. Notes that turned up
//! since are offered by a button over the list, as Jumble does.

use cosmic::Element;
use cosmic::iced::widget::Stack;
use cosmic::iced::{Alignment, Length, Point, mouse};
use cosmic::widget::popover::{Position, popover};
use cosmic::widget::{self, button, container, text};
use nostr_sdk::prelude::PublicKey;

use crate::fl;
use crate::messages::Message;
use crate::nostr::account::Account;
use crate::nostr::models::{Note, display_name};
use crate::state::{AppModel, FeedState};
use crate::views::components::{
    MENU_GAP, MENU_WIDTH, VirtualList, avatar, bell_icon, bookmark_icon, count_badge, estimated_note_height, list_scroller,
    marking, menu, menu_height, note_row, opening_thread, status_line,
};

/// How close to its end, in pixels, a list of notes is scrolled before the
/// next older page is fetched: about two popups' worth, so the page is
/// usually there before the end is.
pub(crate) const LOAD_OLDER_WITHIN: f32 = 1_000.0;

/// How wide each choice of which notes to show is, at the least: the two
/// are as wide as each other, whatever their words.
const FEED_MODE_WIDTH: u16 = 84;

/// The button that offers new notes: how many of their authors it shows,
/// how wide each picture is and how far each starts from the one before,
/// and the most notes it counts before saying only that there are more.
const NEW_NOTES_AVATARS: usize = 3;
const NEW_NOTES_AVATAR_SIZE: f32 = 20.0;
const NEW_NOTES_AVATAR_STEP: f32 = 12.0;
const NEW_NOTES_COUNTED: usize = 99;

/// The most new notifications the bell counts before saying only that
/// there are more.
const NEW_NOTIFICATIONS_COUNTED: usize = 9;

/// How wide the account's picture is above the feed: as the buttons beside
/// it are.
const ACCOUNT_AVATAR_SIZE: f32 = 32.0;

/// How wide and tall the cog above the feed is, which its menu opens
/// under.
const COG_SIZE: f32 = 32.0;

/// How many things the menu under the cog offers, and the room above and
/// below the line the cog is in.
const COG_MENU_ITEMS: usize = 5;
const HEADER_PADDING: f32 = 4.0;

impl AppModel {
    /// The account's picture, which leads to its profile.
    fn view_account_button(&self, account: &Account) -> Element<'_, Message> {
        let name = display_name(&self.profiles, &account.public_key);
        let picture = self.profiles.get(&account.public_key).and_then(|profile| profile.picture.as_deref());
        let picture = avatar(&self.images, picture, &account.public_key, &name, ACCOUNT_AVATAR_SIZE);
        let to_profile = widget::mouse_area(picture)
            .on_press(Message::OpenProfile(account.public_key))
            .interaction(mouse::Interaction::Pointer);
        widget::tooltip(to_profile, text(fl!("feed-profile")), widget::tooltip::Position::Bottom).into()
    }

    /// The choice between the notes that start a conversation and those
    /// with the replies among them, for above a list of notes. A segmented
    /// control: two choices, one of them always made, and seen at a glance.
    pub(crate) fn view_feed_modes(&self) -> Element<'_, Message> {
        widget::segmented_control::horizontal(&self.feed_modes)
            .on_activate(Message::SelectFeedMode)
            .minimum_button_width(FEED_MODE_WIDTH)
            .width(Length::Shrink)
            .into()
    }

    /// The cog above the feed, and the menu that opens under it: the
    /// account's bookmarks, for an account that keeps any, its relays, its
    /// media servers, and the settings. A click anywhere else puts the menu away.
    fn view_cog(&self) -> Element<'_, Message> {
        let cog = button::icon(widget::icon::from_name("emblem-system-symbolic"))
            .tooltip(fl!("feed-menu"))
            .on_press(Message::ToggleMenu);
        // Under the cog, its right edge at the cog's.
        let under = Point::new(COG_SIZE - MENU_WIDTH, COG_SIZE + MENU_GAP);
        let cog = popover(cog).position(Position::Point(under)).on_close(Message::CloseMenu);
        if !self.menu_open {
            return cog.into();
        }
        let settings = widget::icon::from_name("emblem-system-symbolic").handle();
        let relays = widget::icon::from_name("network-server-symbolic").handle();
        let media = widget::icon::from_name("folder-pictures-symbolic").handle();
        let keys = widget::icon::from_name("input-keyboard-symbolic").handle();
        let items: [_; COG_MENU_ITEMS] = [
            (bookmark_icon(false), fl!("menu-bookmarks"), self.can_publish().then_some(Message::OpenBookmarks)),
            (relays, fl!("menu-relays"), self.account.is_some().then_some(Message::OpenRelays)),
            (media, fl!("menu-media"), self.account.is_some().then_some(Message::OpenMedia)),
            (keys, fl!("menu-keys"), Some(Message::OpenKeys)),
            (settings, fl!("feed-settings"), Some(Message::OpenSettings)),
        ];
        cog.popup(menu(items)).into()
    }

    /// The bell above the feed: the way to what the account is told of,
    /// with how many of those are new over its corner, when any are. The
    /// count is over the bell and takes no room: the line is the same
    /// with it and without.
    fn view_bell(&self) -> Element<'_, Message> {
        let new = self.unread_notifications();
        let tooltip = if new == 0 { fl!("feed-notifications") } else { fl!("feed-notifications-new", count = new) };
        let bell = button::icon(bell_icon()).tooltip(tooltip).on_press(Message::OpenNotifications);
        if new == 0 {
            return bell.into();
        }
        let count = if new > NEW_NOTIFICATIONS_COUNTED { format!("{NEW_NOTIFICATIONS_COUNTED}+") } else { new.to_string() };
        Stack::new().push(bell).push(container(count_badge(count)).align_right(Length::Fill)).into()
    }

    /// The line above the feed: writing a note, Refresh and Search on the left; in
    /// the middle, which notes it shows; the bell, the cog, with the
    /// bookmarks and the settings behind it, and the account's picture on
    /// the right.
    fn view_feed_header(&self) -> Element<'_, Message> {
        let refresh = button::icon(widget::icon::from_name("view-refresh-symbolic"))
            .tooltip(fl!("feed-refresh"))
            // Not while one asked for by hand is on its way. A look in
            // passing for new notes doesn't grey it out.
            .on_press_maybe((!self.following.show_when_fetched && self.nostr.is_some()).then_some(Message::RefreshFeed));

        let mut left = widget::Row::new().spacing(8).align_y(Alignment::Center);
        if self.can_publish() {
            let compose = button::icon(widget::icon::from_name("edit-symbolic"))
                .tooltip(fl!("feed-compose"))
                .on_press(Message::ComposeNote);
            left = left.push(compose);
        }
        let search = button::icon(widget::icon::from_name("system-search-symbolic"))
            .tooltip(fl!("feed-search"))
            .on_press_maybe(self.account.is_some().then_some(Message::OpenSearch));
        let left = left.push(refresh).push(search);
        let mut right = widget::Row::new().spacing(8).align_y(Alignment::Center);
        if self.account.is_some() {
            right = right.push(self.view_bell());
        }
        right = right.push(self.view_cog());
        // At startup, who is signed in is known a moment after the feed
        // view shows.
        if let Some(account) = &self.account {
            right = right.push(self.view_account_button(account));
        }

        // The two sides share what the middle leaves, half each, so the
        // middle is the middle of the popup whatever is on either side.
        widget::Row::new()
            .push(container(left).width(Length::Fill).align_x(Alignment::Start))
            .push(self.view_feed_modes())
            .push(container(right).width(Length::Fill).align_x(Alignment::End))
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// The button that offers the notes waiting for the list `state`, as
    /// it is read now: who wrote the first few, and how many there are.
    /// Pressing it shows them, and goes to the top of the list.
    fn view_new_notes_button(&self, state: &FeedState) -> Option<Element<'_, Message>> {
        let waiting = state.pending.notes(self.config.feed_mode);
        if waiting.is_empty() {
            return None;
        }
        let mut authors: Vec<PublicKey> = Vec::new();
        for note in waiting {
            if authors.len() == NEW_NOTES_AVATARS {
                break;
            }
            if !authors.contains(&note.author) {
                authors.push(note.author);
            }
        }
        // Each picture half over the one before it. The first thing in a
        // stack sets its size: room for them all.
        let width = NEW_NOTES_AVATAR_SIZE + NEW_NOTES_AVATAR_STEP * authors.len().saturating_sub(1) as f32;
        let pictures = authors.iter().enumerate().fold(
            Stack::new().push(widget::Space::new().width(width).height(NEW_NOTES_AVATAR_SIZE)),
            |pictures, (place, author)| {
                let name = display_name(&self.profiles, author);
                let picture = self.profiles.get(author).and_then(|profile| profile.picture.as_deref());
                let picture = avatar(&self.images, picture, author, &name, NEW_NOTES_AVATAR_SIZE);
                pictures.push(container(picture).padding([0.0, 0.0, 0.0, NEW_NOTES_AVATAR_STEP * place as f32]))
            },
        );

        // When there may be more than wait here, there are at least so many.
        let label = match (waiting.len(), state.pending.is_detached()) {
            (1, false) => fl!("feed-new-note"),
            (count, _) if count > NEW_NOTES_COUNTED => fl!("feed-new-notes", count = format!("{NEW_NOTES_COUNTED}+")),
            (count, true) => fl!("feed-new-notes", count = format!("{count}+")),
            (count, false) => fl!("feed-new-notes", count = count.to_string()),
        };
        let content = widget::Row::new()
            .push(pictures)
            .push(text(label).size(13).font(cosmic::font::semibold()))
            .push(widget::icon::from_name("go-up-symbolic").size(14))
            .spacing(8)
            .align_y(Alignment::Center);
        let show = button::custom(content)
            .padding([6, 12, 6, 8])
            .class(cosmic::theme::Button::Suggested)
            .on_press(Message::ShowNewNotes(state.id));
        // The key that does the same, for whoever would rather.
        Some(widget::tooltip(show, text(fl!("feed-new-notes-key")).size(12), widget::tooltip::Position::Top).into())
    }

    /// `list`, the scroller of the list `state`, with the button that
    /// offers its new notes over it, at the bottom, when there are any.
    /// Over the list rather than in it: it takes no room, so its coming
    /// and going moves nothing.
    pub(crate) fn with_new_notes<'a>(&'a self, list: Element<'a, Message>, state: &'a FeedState) -> Element<'a, Message> {
        let over: Element<'a, Message> = match self.view_new_notes_button(state) {
            Some(show) => container(show)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::End)
                .padding([0, 0, 12, 0])
                .into(),
            // Nothing, in the same place: the list under it keeps its
            // widget state whether or not the button is there.
            None => widget::Row::new().into(),
        };
        Stack::new().push(list).push(over).into()
    }

    /// What is going on at the end of the list `state`, if anything is:
    /// an older page on its way, nothing older to be had, or a stop after
    /// page upon page with nothing to show, and the way on.
    pub(crate) fn view_list_end(&self, state: &FeedState) -> Option<Element<'_, Message>> {
        if state.loading_older {
            Some(status_line(fl!("feed-loading-older")))
        } else if state.end_reached {
            Some(status_line(fl!("feed-end")))
        } else if state.paused() {
            let further = button::text(fl!("feed-look-further")).on_press(Message::LoadOlderNotes(state.id));
            Some(
                widget::Column::new()
                    .push(text(fl!("feed-nothing-recent")).size(12))
                    .push(further)
                    .spacing(4)
                    .padding(12)
                    .width(Length::Fill)
                    .align_x(Alignment::Center)
                    .into(),
            )
        } else {
            None
        }
    }

    /// The notes of `state` that are to show, as a list, and under them
    /// what is going on at the list's end. With none to show, `nothing`
    /// says so in their place. For a scroller: the feed's, a profile's.
    pub(crate) fn view_notes<'a>(&'a self, state: &'a FeedState, nothing: String) -> Element<'a, Message> {
        let notes = state.feed.notes(self.config.feed_mode);
        if notes.is_empty() {
            // While it is looking further back, or stopped short of the
            // end, the list's end says what there is to say. Otherwise
            // there is nothing, and `nothing` says so.
            return match self.view_list_end(state) {
                Some(end) if !state.end_reached => end,
                _ => status_line(nothing),
            };
        }

        let context = self.row_context(&state.expanded, state.marked);
        let sizes = context.sizes;
        let list: VirtualList<'_, Note, _, Message, cosmic::Theme, cosmic::Renderer> = VirtualList::new(
            notes,
            &state.heights,
            |note: &Note| note.id,
            move |note, width| estimated_note_height(note, sizes, width, context.expanded.has(&note.id)),
            move |_index, note| marking(opening_thread(note_row(note, &context), note.id, &context), note.id, &context),
        )
        .on_near_end(LOAD_OLDER_WITHIN, Message::LoadOlderNotes(state.id));

        widget::Column::new().push(list).push_maybe(self.view_list_end(state)).into()
    }

    /// What the feed view says in place of notes, when the feed has none
    /// to show.
    fn empty_feed_text(&self) -> String {
        match &self.follows {
            _ if self.nostr.is_none() => fl!("feed-loading"),
            None => fl!("feed-following-unknown"),
            Some(follows) if follows.is_empty() => fl!("feed-following-nobody"),
            _ if self.following.refreshing => fl!("feed-loading"),
            _ => fl!("feed-empty"),
        }
    }

    /// Render the feed view.
    pub fn view_feed(&self) -> Element<'_, Message> {
        let state = &self.following;
        let list = list_scroller(&state.position, self.view_notes(state, self.empty_feed_text()));
        let body = self.with_new_notes(list, state);

        let feed = widget::Column::new().push(self.view_feed_header()).push(widget::divider::horizontal::default()).push(body);
        if !self.menu_open {
            return feed.into();
        }
        // The menu opens inside the popup, which is no taller than what
        // is in it: over a feed with nothing to show, it is made tall
        // enough for all the menu offers.
        let room = HEADER_PADDING + COG_SIZE + MENU_GAP + menu_height(COG_MENU_ITEMS) + HEADER_PADDING;
        widget::Row::new().push(widget::Space::new().height(Length::Fixed(room))).push(feed).into()
    }
}

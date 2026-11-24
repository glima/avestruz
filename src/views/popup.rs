// SPDX-License-Identifier: MIT

//! Popup window view for Avestruz.
//!
//! This module renders the popup window: the view [`ViewState`] names, under
//! an error banner when something went wrong.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Limits};
use cosmic::widget::{self, button, container, text};
use nostr_sdk::prelude::EventId;

use crate::fl;
use crate::helpers::{truncate_chars, unix_now};
use crate::messages::Message;
use crate::state::{AppModel, BookmarkState, Expanded, Page, ViewState};
use crate::views::components::{Bookmarking, POPUP_MAX_WIDTH, POPUP_MIN_WIDTH, Reacting, RowContext, RowSizes};

/// How much of an error the banner shows. Errors from the network can run
/// long; the whole of it is in the log.
const BANNER_MAX_CHARS: usize = 160;

/// A red banner across the top of the content: what went wrong, and a
/// button to dismiss it.
fn error_banner(error: &str) -> Element<'_, Message> {
    let first_line = error.lines().next().unwrap_or_default();
    let row = widget::Row::new()
        .push(
            container(text(truncate_chars(first_line, BANNER_MAX_CHARS)).size(12).wrapping(Wrapping::WordOrGlyph))
                .width(Length::Fill),
        )
        .push(
            button::icon(widget::icon::from_name("window-close-symbolic"))
                .tooltip(fl!("error-dismiss"))
                .on_press(Message::ClearError)
                .padding(2),
        )
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(row)
        .padding(8)
        .width(Length::Fill)
        .class(cosmic::theme::Container::custom(|_theme| container::Style {
            background: Some(cosmic::iced::Background::Color(cosmic::iced::Color::from_rgb(0.8, 0.2, 0.2))),
            text_color: Some(cosmic::iced::Color::WHITE),
            border: cosmic::iced::Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        }))
        .into()
}

impl AppModel {
    /// What the rows of a list are built with, as things stand now: the
    /// list whose long notes in full are `expanded`, and in which the
    /// keyboard is at the row `marked`.
    pub(crate) fn row_context<'a>(&'a self, expanded: &'a Expanded, marked: Option<EventId>) -> RowContext<'a> {
        RowContext {
            profiles: &self.profiles,
            images: &self.images,
            previews: &self.previews,
            videos: &self.videos,
            engagement: &self.engagement,
            seen_on: &self.seen_on,
            now: unix_now(),
            // The theme's accent, as text. Read once here rather than for
            // every row.
            accent: cosmic::theme::active().cosmic().accent_text_color().into(),
            can_publish: self.can_publish(),
            repost_menu: self.repost_menu,
            reacting: Reacting { menu: self.reaction_menu.as_ref(), suggested: &self.reaction_suggestions },
            expanded,
            marked,
            share_copied: self.share_copied,
            picture_copied: self.picture_copied.as_deref(),
            menu_open: self.menu_open,
            bookmarking: match &self.bookmarks {
                _ if !self.can_publish() => Bookmarking::Unavailable,
                BookmarkState::Unknown => Bookmarking::Unknown,
                BookmarkState::Known(bookmarks) => Bookmarking::Known { bookmarks, pending: self.bookmark_pending },
                BookmarkState::Unreadable(why) => Bookmarking::Unreadable(why),
            },
            sizes: RowSizes::new(self.config.note_text_size()),
        }
    }

    /// Dispatch to the appropriate view based on the current [`ViewState`].
    fn view_page_content(&self) -> Element<'_, Message> {
        match self.view_state {
            ViewState::Login => self.view_login(),
            ViewState::Feed => self.view_feed(),
            ViewState::Page => match self.pages.last() {
                Some(Page::Thread(state)) => self.view_thread(state),
                Some(Page::Profile(state)) => self.view_profile(state),
                Some(Page::Bookmarks(page)) => self.view_bookmarks(page),
                Some(Page::Search(page)) => self.view_search(page),
                Some(Page::Notifications(page)) => self.view_notifications(page),
                // Nothing is open over the feed.
                None => self.view_feed(),
            },
            ViewState::Compose => self.view_compose(),
            ViewState::Zap => self.view_zap(),
            ViewState::Settings => self.view_settings(),
            ViewState::Keys => self.view_keys(),
            ViewState::Relays => self.view_relays(),
            ViewState::Media => self.view_media(),
        }
    }

    /// Build the full content tree for the popup: the current view, under
    /// the error banner if there is something to say.
    pub fn view_content(&self) -> Element<'_, Message> {
        let page = self.view_page_content();
        match &self.error_message {
            Some(error) => widget::Column::new().push(error_banner(error)).push(page).spacing(8).into(),
            None => page,
        }
    }

    /// Render the main popup window, in the applet popup container, which
    /// sizes the popup to its content: within these limits, in place of the
    /// container's own, which are one width and no other. A list longer
    /// than the popup may be tall takes what room the limit leaves it, and
    /// scrolls.
    pub fn view_popup(&self, _id: Id) -> Element<'_, Message> {
        // A video plays while the rows built for this show it.
        self.videos.viewing();
        self.core
            .applet
            .popup_container(self.view_content())
            .limits(
                Limits::NONE
                    .min_width(POPUP_MIN_WIDTH)
                    .max_width(POPUP_MAX_WIDTH)
                    .min_height(1.0)
                    .max_height(self.popup_max_height()),
            )
            .into()
    }
}

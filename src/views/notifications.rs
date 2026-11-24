// SPDX-License-Identifier: MIT

//! The notifications view for Avestruz: what others did that concerns the
//! account, newest first, in a scroller.
//!
//! Above the list, a control chooses between all of it and one kind:
//! what was written to the account, what was made of its notes, what it
//! was zapped. What is new since the notifications were last looked at
//! is picked out, for as long as this visit lasts (see
//! [`crate::handlers::notifications`]).

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container, text};
use nostr_sdk::prelude::EventId;

use crate::fl;
use crate::messages::Message;
use crate::nostr::notifications::{Notifications, Tab};
use crate::state::{AppModel, NotificationsPage};
use crate::views::components::{
    VirtualList, estimated_notification_height, list_scroller, marking, notification_row, status_line,
};
use crate::views::feed::LOAD_OLDER_WITHIN;

/// How wide each choice of which notifications show is, at the least.
const TAB_WIDTH: u16 = 76;

impl AppModel {
    /// The line above the notifications: the way back, and what this is.
    fn view_notifications_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("notifications-back"))
            .on_press(Message::ClosePage);
        let title = container(text(fl!("notifications-title")).size(16).font(cosmic::font::bold())).width(Length::Fill);
        widget::Row::new().push(back).push(title).spacing(8).align_y(Alignment::Center).padding([4, 12]).into()
    }

    /// The choice of which notifications show, in the middle of its line.
    fn view_notification_tabs(&self) -> Element<'_, Message> {
        let tabs = widget::segmented_control::horizontal(&self.notification_tabs)
            .on_activate(Message::SelectNotificationsTab)
            .minimum_button_width(TAB_WIDTH)
            .width(Length::Shrink);
        container(tabs).center_x(Length::Fill).padding([0, 12, 6, 12]).into()
    }

    /// What is said in place of the list when the tab on show has
    /// nothing: that it is being looked for, that it could not be, or
    /// that there is nothing.
    fn empty_notifications_text(&self, tab: Tab) -> String {
        let state = &self.notifications;
        if state.failed {
            fl!("notifications-failed")
        } else if !state.settled {
            fl!("notifications-loading")
        } else {
            match tab {
                Tab::All => fl!("notifications-empty"),
                Tab::Mentions => fl!("notifications-empty-mentions"),
                Tab::Reactions => fl!("notifications-empty-reactions"),
                Tab::Zaps => fl!("notifications-empty-zaps"),
            }
        }
    }

    /// The notifications the tab on show has, in the scroller, and under
    /// them what is going on at their end.
    fn view_notifications_list<'a>(&'a self, page: &'a NotificationsPage) -> Element<'a, Message> {
        let state = &self.notifications;
        let tab = self.notifications_tab();
        let listed = state.list.listed(tab);
        if listed.is_empty() {
            return status_line(self.empty_notifications_text(tab));
        }

        let context = self.row_context(&page.expanded, page.marked);
        let sizes = context.sizes;
        let (told, seen_before) = (&state.list, state.seen_before);
        let list: VirtualList<'_, usize, _, Message, cosmic::Theme, cosmic::Renderer> = VirtualList::new(
            listed,
            &page.heights,
            // A place that holds nothing has no row, and is nobody's.
            move |place: &usize| told.get(*place).map_or(EventId::from_byte_array([0; 32]), |notification| notification.id),
            move |place, width| {
                told.get(*place).map_or(0.0, |notification| {
                    estimated_notification_height(notification, sizes, width, context.expanded.has(&notification.id))
                })
            },
            move |_index, place| match told.get(*place) {
                Some(notification) => marking(
                    notification_row(notification, &context, Notifications::is_new(notification, seen_before)),
                    notification.id,
                    &context,
                ),
                None => widget::Space::new().into(),
            },
        )
        .on_near_end(LOAD_OLDER_WITHIN, Message::LoadOlderNotifications);

        let end = if state.loading_older {
            Some(status_line(fl!("notifications-loading-older")))
        } else if state.end_reached {
            Some(status_line(fl!("notifications-end")))
        } else {
            None
        };
        list_scroller(&page.position, widget::Column::new().push(list).push_maybe(end))
    }

    /// Render the notifications view, of the page `page`.
    pub fn view_notifications<'a>(&'a self, page: &'a NotificationsPage) -> Element<'a, Message> {
        widget::Column::new()
            .push(self.view_notifications_header())
            .push(self.view_notification_tabs())
            .push(widget::divider::horizontal::default())
            .push(self.view_notifications_list(page))
            .into()
    }
}

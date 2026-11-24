// SPDX-License-Identifier: MIT

//! The media servers view for Avestruz: the servers the account keeps
//! what it attaches to notes on, the one files are sent to first at the
//! top, with a server to add under them. Save, above them, writes them
//! to the account's list.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container, text, text_input};

use crate::fl;
use crate::messages::Message;
use crate::nostr::blossom::server_name;
use crate::state::AppModel;
use crate::views::relays::small;

impl AppModel {
    /// The line above the servers: the way back, what this is, and Save.
    fn view_media_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("media-back"))
            .on_press(Message::CloseMedia);
        let label = if self.media.saving { fl!("media-saving") } else { fl!("media-save") };
        let save = button::suggested(label).on_press_maybe(self.media_can_save().then_some(Message::SaveMediaServers));
        widget::Row::new()
            .push(back)
            .push(text(fl!("media-title")).size(16).font(cosmic::font::bold()).width(Length::Fill))
            .push(save)
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// The servers, each with what it is for and the ways to move it and
    /// to take it out, and a server to add.
    fn view_media_servers(&self) -> Element<'_, Message> {
        let can_change = self.media_can_change();
        let mut shown = widget::Column::new().push(small(fl!("media-description"))).spacing(6);

        let servers = &self.media.edited;
        if servers.is_empty() && self.media.read {
            shown = shown.push(small(fl!("media-none")));
        }
        let last = servers.len().saturating_sub(1);
        for (place, server) in servers.iter().enumerate() {
            let moving = |up: bool, at_end: bool| (can_change && !at_end).then(|| Message::ShiftMediaServer(server.clone(), up));
            let up = button::icon(widget::icon::from_name("go-up-symbolic"))
                .tooltip(fl!("media-up"))
                .on_press_maybe(moving(true, place == 0));
            let down = button::icon(widget::icon::from_name("go-down-symbolic"))
                .tooltip(fl!("media-down"))
                .on_press_maybe(moving(false, place == last));
            let remove = button::icon(widget::icon::from_name("edit-delete-symbolic"))
                .tooltip(fl!("media-remove"))
                .on_press_maybe(can_change.then(|| Message::RemoveMediaServer(server.clone())));
            // A long address is cut rather than wrapped.
            let name = container(text(server_name(server)).size(13).wrapping(Wrapping::None)).clip(true);
            let what_for = if place == 0 { fl!("media-first") } else { fl!("media-copy") };
            let named = widget::Column::new()
                .push(name)
                .push(text(what_for).size(11).wrapping(Wrapping::None))
                .spacing(2)
                .width(Length::Fill);
            shown =
                shown.push(widget::Row::new().push(named).push(up).push(down).push(remove).spacing(4).align_y(Alignment::Center));
        }
        if can_change {
            let typed = text_input(fl!("media-add-placeholder"), self.media.typed.as_str())
                .on_input(Message::MediaServerTyped)
                .on_submit(|_| Message::AddMediaServer)
                .width(Length::Fill);
            let add = button::standard(fl!("media-add"))
                .on_press_maybe((!self.media.typed.trim().is_empty()).then_some(Message::AddMediaServer));
            shown = shown.push(widget::Row::new().push(typed).push(add).spacing(8).align_y(Alignment::Center));
        }
        shown.into()
    }

    /// Render the media servers view.
    pub fn view_media(&self) -> Element<'_, Message> {
        let mut body = widget::Column::new().spacing(16);
        if !self.can_publish() {
            body = body.push(small(fl!("media-read-only")));
        } else if !self.media.from_relays {
            body = body.push(small(fl!("media-reading")));
        }
        body = body.push(self.view_media_servers()).push(small(fl!("media-privacy")));

        widget::Column::new()
            .push(self.view_media_header())
            .push(widget::divider::horizontal::default())
            .push(widget::scrollable(container(body).padding([8, 12]).width(Length::Fill)))
            .into()
    }
}

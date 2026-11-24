// SPDX-License-Identifier: MIT

//! The relays view for Avestruz: the relays the account writes to, those
//! it reads and those it searches, each set under what it is for, with a
//! relay to add under it. Save, above them, writes them to the account's
//! lists.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container, text, text_input};

use nostr_sdk::prelude::RelayUrl;

use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::helpers::readable_size;
use crate::messages::Message;
use crate::nostr::relay_lists::{Hindrance, Kept, NIP_AUTH, NIP_SEARCH, Section};
use crate::state::AppModel;
use crate::views::components::note_row::relay_name;

/// What a set of relays is called, and what it is for.
fn section_texts(section: Section) -> (String, String) {
    match section {
        Section::Outbox => (fl!("relays-outbox"), fl!("relays-outbox-description")),
        Section::Inbox => (fl!("relays-inbox"), fl!("relays-inbox-description")),
        Section::Messages => (fl!("relays-messages"), fl!("relays-messages-description")),
        Section::Search => (fl!("relays-search"), fl!("relays-search-description")),
        Section::Indexer => (fl!("relays-indexer"), fl!("relays-indexer-description")),
        Section::Trusted => (fl!("relays-trusted"), fl!("relays-trusted-description")),
        Section::Blocked => (fl!("relays-blocked"), fl!("relays-blocked-description")),
    }
}

/// What a set of relays is called.
pub fn section_title(section: Section) -> String {
    section_texts(section).0
}

/// What is said under a relay of what it says of itself that gets in
/// the way of what its set is for.
fn hindrance_text(hindrance: Hindrance) -> String {
    match hindrance {
        Hindrance::Lacks(NIP_SEARCH) => fl!("relays-lacks-search"),
        Hindrance::Lacks(NIP_AUTH) => fl!("relays-lacks-auth"),
        Hindrance::Lacks(nip) => fl!("relays-lacks-nip", nip = nip),
        Hindrance::RestrictedWrites => fl!("relays-restricted-writes"),
        Hindrance::PaymentRequired => fl!("relays-payment-required"),
        Hindrance::AuthToWrite => fl!("relays-auth-to-write"),
        Hindrance::AuthToRead => fl!("relays-auth-to-read"),
    }
}

/// A line of small print, wrapped.
pub(crate) fn small<'a>(words: String) -> Element<'a, Message> {
    text(words).size(11).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).into()
}

impl AppModel {
    /// The line above the relays: the way back, what this is, and Save.
    fn view_relays_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("relays-back"))
            .on_press(Message::CloseRelays);
        let label = if self.relays.saving { fl!("relays-saving") } else { fl!("relays-save") };
        let save = button::suggested(label).on_press_maybe(self.relays_can_save().then_some(Message::SaveRelays));
        widget::Row::new()
            .push(back)
            .push(text(fl!("relays-title")).size(16).font(cosmic::font::bold()).width(Length::Fill))
            .push(save)
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// What is known of how `relay` is doing, if it is one this is
    /// connected to: under its name, in small print.
    fn view_relay_use(&self, relay: &RelayUrl) -> Option<Element<'_, Message>> {
        let doing = self.relays.in_use.get(relay)?;
        let mut said = vec![if doing.connected { fl!("relays-use-connected") } else { fl!("relays-use-disconnected") }];
        if doing.received > 0 || doing.sent > 0 {
            said.push(fl!("relays-use-traffic", down = readable_size(doing.received), up = readable_size(doing.sent)));
        }
        if doing.attempts > 1 {
            said.push(fl!("relays-use-attempts", worked = doing.succeeded, all = doing.attempts));
        }
        if let Some(latency) = doing.latency {
            said.push(fl!("relays-use-latency", ms = latency.as_millis()));
        }
        Some(text(said.join(" · ")).size(11).wrapping(Wrapping::None).into())
    }

    /// What `relay` says of itself that gets in the way of what
    /// `section` is for, if it said anything: a line each under its name,
    /// in small print that stands out.
    fn view_relay_hindrances(&self, section: Section, relay: &RelayUrl) -> Vec<Element<'_, Message>> {
        let Some(Some(info)) = self.relays.said.get(relay) else {
            return Vec::new();
        };
        let line = |hindrance| {
            text(hindrance_text(hindrance)).size(11).class(cosmic::theme::Text::Accent).wrapping(Wrapping::WordOrGlyph).into()
        };
        section.hindrances(info).into_iter().map(line).collect()
    }

    /// One set of relays: what it is for, the relays in it, each with the
    /// ways to move it and to take it out, and a relay to add.
    fn view_relay_section(&self, section: Section) -> Element<'_, Message> {
        let (title, description) = section_texts(section);
        let can_change = self.relays_can_change(section);
        let mut shown =
            widget::Column::new().push(text(title).size(14).font(cosmic::font::bold())).push(small(description)).spacing(6);

        let relays = self.relays.edited.of(section);
        if relays.is_empty() && self.relays.lists.is_some() {
            shown = shown.push(small(fl!("relays-none")));
        }
        let last = relays.len().saturating_sub(1);
        for (place, relay) in relays.iter().enumerate() {
            let moving =
                |up: bool, at_end: bool| (can_change && !at_end).then(|| Message::ShiftRelay(section, relay.clone(), up));
            let up = button::icon(widget::icon::from_name("go-up-symbolic"))
                .tooltip(fl!("relays-up"))
                .on_press_maybe(moving(true, place == 0));
            let down = button::icon(widget::icon::from_name("go-down-symbolic"))
                .tooltip(fl!("relays-down"))
                .on_press_maybe(moving(false, place == last));
            let remove = button::icon(widget::icon::from_name("edit-delete-symbolic"))
                .tooltip(fl!("relays-remove"))
                .on_press_maybe(can_change.then(|| Message::RemoveRelay(section, relay.clone())));
            // A long address is cut rather than wrapped.
            let name = container(text(relay_name(relay)).size(13).wrapping(Wrapping::None)).clip(true);
            let named = widget::Column::new()
                .push(name)
                .push_maybe(self.view_relay_use(relay))
                .extend(self.view_relay_hindrances(section, relay))
                .spacing(2)
                .width(Length::Fill);
            shown =
                shown.push(widget::Row::new().push(named).push(up).push(down).push(remove).spacing(4).align_y(Alignment::Center));
        }

        // What the list keeps to itself, when that was not read.
        let kept = self.relays.lists.as_ref().map(|lists| lists.of(section).kept().clone());
        match kept.filter(|_| Section::LISTS.contains(&section)) {
            Some(Kept::Unreadable(why)) => {
                shown = shown.push(small(fl!("relays-list-unreadable", error = sign_error_text(why))));
            }
            Some(Kept::Unread) => shown = shown.push(small(fl!("relays-list-unread"))),
            Some(Kept::Read(_)) | None => {}
        }
        if can_change {
            let typed = text_input(fl!("relays-add-placeholder"), self.relays.typed(section))
                .on_input(move |typed| Message::RelayTyped(section, typed))
                .on_submit(move |_| Message::AddRelay(section))
                .width(Length::Fill);
            let add = button::standard(fl!("relays-add"))
                .on_press_maybe((!self.relays.typed(section).trim().is_empty()).then_some(Message::AddRelay(section)));
            shown = shown.push(widget::Row::new().push(typed).push(add).spacing(8).align_y(Alignment::Center));
        }
        shown.into()
    }

    /// Render the relays view.
    pub fn view_relays(&self) -> Element<'_, Message> {
        let mut body = widget::Column::new().spacing(16);
        if !self.can_publish() {
            body = body.push(small(fl!("relays-read-only")));
        } else if !self.relays.from_relays {
            body = body.push(small(fl!("relays-reading")));
        }
        for section in Section::ALL {
            body = body.push(self.view_relay_section(section));
        }

        widget::Column::new()
            .push(self.view_relays_header())
            .push(widget::divider::horizontal::default())
            .push(widget::scrollable(container(body).padding([8, 12]).width(Length::Fill)))
            .into()
    }
}

// SPDX-License-Identifier: MIT

//! The settings view for Avestruz: who is signed in, signing out, how large
//! notes are set, how long they are kept, and how much is logged.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, button, container, text};

use crate::config::{KEEP_DAYS_CHOICES, LogLevel, NOTE_TEXT_SIZE_MAX, NOTE_TEXT_SIZE_MIN};
use crate::fl;
use crate::messages::Message;
use crate::nostr::account::Account;
use crate::nostr::models::{display_name, short_npub};
use crate::state::AppModel;
use crate::views::components::avatar;

/// How wide the account's picture is in the settings: as in a note's row.
const ACCOUNT_AVATAR_SIZE: f32 = 36.0;

impl AppModel {
    /// The line above the settings: the way back, and what this is.
    fn view_settings_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("settings-back"))
            .on_press(Message::CloseSettings);
        widget::Row::new()
            .push(back)
            .push(text(fl!("settings-title")).size(16).font(cosmic::font::bold()))
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// Who is signed in, how, and the way out.
    fn view_settings_account(&self, account: &Account) -> Element<'_, Message> {
        let name = display_name(&self.profiles, &account.public_key);
        let picture = self.profiles.get(&account.public_key).and_then(|profile| profile.picture.as_deref());
        let picture = avatar(&self.images, picture, &account.public_key, &name, ACCOUNT_AVATAR_SIZE);

        let key = if account.can_sign {
            short_npub(&account.public_key)
        } else {
            format!("{} · {}", short_npub(&account.public_key), fl!("read-only"))
        };
        // The name is cut rather than wrapped when it doesn't fit.
        let who = widget::Column::new()
            .push(container(text(name).size(13).font(cosmic::font::bold()).wrapping(Wrapping::None)).clip(true))
            .push(text(key).size(11).wrapping(Wrapping::None))
            .spacing(2)
            .width(Length::Fill);

        widget::Row::new()
            .push(picture)
            .push(who)
            .push(button::destructive(fl!("sign-out")).on_press(Message::SignOut))
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
    }

    /// How large notes are set: the size, a step at a time, and a line set
    /// in it, since the notes themselves are behind this view.
    fn view_settings_text_size(&self) -> Element<'_, Message> {
        let size = self.config.note_text_size();
        let stepper = widget::spin_button(
            size.to_string(),
            fl!("settings-text-size"),
            size,
            1,
            NOTE_TEXT_SIZE_MIN,
            NOTE_TEXT_SIZE_MAX,
            Message::SetNoteTextSize,
        );
        let chooser = widget::Row::new()
            .push(text(fl!("settings-text-size-description")).size(11).wrapping(Wrapping::WordOrGlyph).width(Length::Fill))
            .push(stepper)
            .spacing(8)
            .align_y(Alignment::Center);
        widget::Column::new()
            .push(text(fl!("settings-text-size")).size(14))
            .push(chooser)
            .push(text(fl!("settings-text-size-sample")).size(size).wrapping(Wrapping::WordOrGlyph))
            .spacing(8)
            .into()
    }

    /// For how long the event store keeps other people's notes. A number
    /// of days the config file says that is none of the choices shows as
    /// none chosen.
    fn view_settings_keeping(&self) -> Element<'_, Message> {
        let choices: Vec<String> = KEEP_DAYS_CHOICES
            .iter()
            .map(|days| match days {
                0 => fl!("settings-keep-forever"),
                days => fl!("settings-keep-days", days = days),
            })
            .collect();
        let chosen = KEEP_DAYS_CHOICES.iter().position(|days| *days == self.config.keep_days);
        widget::Column::new()
            .push(text(fl!("settings-keep")).size(14))
            .push(
                widget::dropdown(choices, chosen, |index| {
                    Message::SetKeepDays(KEEP_DAYS_CHOICES.get(index).copied().unwrap_or_default())
                })
                .width(Length::Fill),
            )
            .push(text(fl!("settings-keep-description")).size(11).wrapping(Wrapping::WordOrGlyph))
            .spacing(8)
            .into()
    }

    /// How much is logged: an applet has no command line to say so on.
    fn view_settings_logging(&self) -> Element<'_, Message> {
        let chosen = LogLevel::ALL.iter().position(|level| *level == self.config.log_level);
        widget::Column::new()
            .push(text(fl!("settings-logging")).size(14))
            .push(
                widget::dropdown(&LogLevel::ALL, chosen, |index| {
                    Message::SetLogLevel(LogLevel::ALL.get(index).copied().unwrap_or_default())
                })
                .width(Length::Fill),
            )
            .push(text(fl!("settings-logging-description")).size(11).wrapping(Wrapping::WordOrGlyph))
            .spacing(8)
            .into()
    }

    /// Render the settings view.
    pub fn view_settings(&self) -> Element<'_, Message> {
        let mut account = widget::Column::new().push(text(fl!("settings-account")).size(14)).spacing(8);
        // There is nobody to sign out until the account is known.
        if let Some(signed_in) = &self.account {
            account = account.push(self.view_settings_account(signed_in));
        }
        let body = widget::Column::new()
            .push(account)
            .push(self.view_settings_text_size())
            .push(self.view_settings_keeping())
            .push(self.view_settings_logging())
            .spacing(16);

        // In a scroller: on a small screen there is more of it than room.
        widget::Column::new()
            .push(self.view_settings_header())
            .push(widget::divider::horizontal::default())
            .push(widget::scrollable(container(body).padding([8, 12]).width(Length::Fill)))
            .into()
    }
}

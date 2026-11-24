// SPDX-License-Identifier: MIT

//! Sign-in view for Avestruz.
//!
//! One field takes an `nsec`, an `npub` or a `bunker://` address. It hides
//! what is typed, because what is typed may be a secret.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget::text_input::secure_input;
use cosmic::widget::{self, button, container, text};

use crate::fl;
use crate::messages::{Message, SecretInput};
use crate::nostr::account::CredentialError;
use crate::state::{AppModel, BunkerRefusal, LoginProgress};
use crate::views::components::app_icon;

/// Why a sign-in attempt was refused, in the user's language.
fn credential_error_text(error: CredentialError) -> String {
    match error {
        CredentialError::Empty => fl!("sign-in-error-empty"),
        CredentialError::NotAKey => fl!("sign-in-error-not-a-key"),
        CredentialError::InvalidSecret => fl!("sign-in-error-invalid-secret"),
        CredentialError::InvalidPublic => fl!("sign-in-error-invalid-public"),
        CredentialError::InvalidBunker => fl!("sign-in-error-invalid-bunker"),
    }
}

/// What is said when a remote signer turned the sign-in down: why, the key
/// to authorize there, with a button that copies it, and, of an address
/// with a secret, that the secret may have been used up.
fn refusal_notice(refusal: &BunkerRefusal) -> Element<'_, Message> {
    let small = |message: String| text(message).size(11).wrapping(Wrapping::WordOrGlyph);
    let mut notice = widget::Column::new()
        .push(
            text(fl!("sign-in-refused", error = refusal.reason.as_str()))
                .size(12)
                .class(cosmic::theme::Text::Accent)
                .wrapping(Wrapping::WordOrGlyph),
        )
        .spacing(6)
        .width(Length::Fill);
    if let Some(client) = refusal.client {
        let copy = if refusal.copied { fl!("sign-in-client-key-copied") } else { fl!("sign-in-copy-client-key") };
        notice = notice
            .push(small(fl!("sign-in-client-key-help")))
            // In full: it is what goes into the signer's configuration.
            .push(text(client.to_hex()).size(11).font(cosmic::font::mono()).wrapping(Wrapping::Glyph))
            .push(button::standard(copy).on_press(Message::CopyClientKey));
    }
    if refusal.with_secret {
        notice = notice.push(small(fl!("sign-in-secret-help")));
    }
    notice.into()
}

impl AppModel {
    /// Render the sign-in view: a field for a key or a bunker address.
    pub fn view_login(&self) -> Element<'_, Message> {
        let input = secure_input(
            fl!("sign-in-placeholder"),
            &self.login_input,
            Some(Message::ToggleLoginInputVisibility),
            self.login_input_hidden,
        )
        .on_input(|value| Message::LoginInputChanged(SecretInput(value)))
        .on_submit(|_| Message::SubmitLogin)
        .width(Length::Fill);

        let sign_in_label = match self.login_progress {
            LoginProgress::Idle => fl!("sign-in"),
            LoginProgress::Storing => fl!("signing-in"),
            LoginProgress::Connecting => fl!("sign-in-connecting"),
        };
        let sign_in = button::suggested(sign_in_label)
            .on_press_maybe((self.login_progress == LoginProgress::Idle).then_some(Message::SubmitLogin))
            .width(Length::Fill);

        let mut content = widget::Column::new()
            .push(widget::icon::icon(app_icon()).size(48))
            .push(text(fl!("app-name")).size(24))
            .push(text(fl!("sign-in-prompt")).size(14))
            .push(widget::space::vertical().height(8))
            .push(input);

        if let Some(error) = self.login_error {
            content = content.push(text(credential_error_text(error)).size(12).class(cosmic::theme::Text::Accent));
        }

        if let Some(refusal) = &self.login_refusal {
            content = content.push(refusal_notice(refusal));
        }

        let mut content = content.push(text(fl!("sign-in-help")).size(11).wrapping(Wrapping::WordOrGlyph)).push(sign_in);
        // A remote signer may wait on a person: say so, and offer a way out.
        if self.login_progress == LoginProgress::Connecting {
            content = content
                .push(text(fl!("sign-in-waiting")).size(11).wrapping(Wrapping::WordOrGlyph))
                .push(button::text(fl!("sign-in-cancel")).on_press(Message::CancelLogin));
        }
        let content = content.spacing(12).align_x(Alignment::Center).padding(20).width(Length::Fill);

        container(content).width(Length::Fill).align_x(Alignment::Center).align_y(Alignment::Center).into()
    }
}

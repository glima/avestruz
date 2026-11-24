// SPDX-License-Identifier: MIT

//! Authentication message handlers for Avestruz.
//!
//! Signing in with a key is offline: it is read from the sign-in field, and
//! a secret key is handed to the system keyring. Signing in with a bunker
//! address waits on its remote signer, which may wait on a person, and can
//! be called off meanwhile. A signer that turns it down is shown with the
//! key it would have to know this client by. Which account is signed in is
//! remembered in the config by its `npub`.

use cosmic::Application;
use cosmic::cosmic_config::CosmicConfigEntry;
use cosmic::iced::clipboard;
use cosmic::prelude::*;

use crate::config::Config;
use crate::fl;
use crate::messages::{Message, SecretInput};
use crate::nostr::account::{self, Account, Credential, SignError, SignInError};
use crate::state::{AppModel, BunkerRefusal, LoginProgress, ViewState};

/// Why an event was not signed, in the user's language.
pub(crate) fn sign_error_text(error: SignError) -> String {
    match error {
        SignError::Keyring(error) => fl!("error-keyring", error = error),
        SignError::Remote(error) => fl!("error-remote-signer", error = error),
        SignError::Cipher(error) => fl!("error-cipher", error = error),
    }
}

impl AppModel {
    /// Write the whole config to COSMIC's config store, logging (not
    /// failing) if that doesn't work. `what` names the change, for the log.
    pub(crate) fn save_config(&self, what: &str) {
        if let Ok(config_context) = cosmic::cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            && let Err(e) = self.config.write_entry(&config_context)
        {
            tracing::error!("Failed to save {what} config: {e}");
        }
    }

    /// Empty the sign-in field and forget its error. The field may hold a
    /// secret key, so it is not left lying around once it has served. A
    /// sign-in still waiting on a remote signer is called off: whoever left
    /// the sign-in view is not to be signed in a minute later.
    pub(crate) fn clear_login(&mut self) {
        self.login_input.clear();
        self.login_input_hidden = true;
        self.login_error = None;
        self.login_refusal = None;
        self.signer.cancel_sign_in();
        self.login_progress = LoginProgress::Idle;
    }

    /// The user edited the sign-in field.
    pub fn handle_login_input_changed(&mut self, input: SecretInput) {
        self.login_input = input.0;
        self.login_error = None;
        // The key shown is the one for the signer that was in the field.
        self.login_refusal = None;
    }

    /// Sign in with what is in the sign-in field.
    pub fn handle_submit_login(&mut self) -> Task<cosmic::Action<Message>> {
        if self.login_progress != LoginProgress::Idle {
            return Task::none();
        }
        match account::parse_credential(&self.login_input) {
            Ok(credential) => {
                self.login_error = None;
                self.login_refusal = None;
                self.login_progress = match credential {
                    Credential::Bunker(_) => LoginProgress::Connecting,
                    Credential::Secret(_) | Credential::Public(_) => LoginProgress::Storing,
                };
                let signer = self.signer.clone();
                Task::perform(async move { signer.sign_in(credential).await }, |result| {
                    cosmic::Action::App(Message::LoginComplete(result))
                })
            }
            Err(error) => {
                self.login_error = Some(error);
                Task::none()
            }
        }
    }

    /// `account` is the one in use from now on: who it follows, which
    /// fills its feed, is found out.
    fn enter_account(&mut self, account: Account) -> Task<cosmic::Action<Message>> {
        self.reset_following();
        self.reset_engagement();
        self.reset_bookmarks();
        self.reset_relays();
        self.reset_media();
        self.reset_kept_drafts();
        self.reset_notifications();
        self.account = Some(account);
        self.load_account()
    }

    /// Signing in finished. On success the account is remembered and its
    /// feed shows; if the keyring refused, or the remote signer did, nobody
    /// is signed in.
    pub fn handle_login_complete(&mut self, result: Result<Account, SignInError>) -> Task<cosmic::Action<Message>> {
        self.login_progress = LoginProgress::Idle;
        match result {
            Ok(account) => {
                tracing::info!(
                    "signed in as {} ({})",
                    account.npub(),
                    if account.can_sign { "full access" } else { "read-only" }
                );
                self.config.account = Some(account.npub());
                self.save_config("account");
                self.clear_login();
                self.view_state = ViewState::Feed;
                self.enter_account(account)
            }
            // Called off: what was typed is still in the field.
            Err(SignInError::Cancelled) => Task::none(),
            Err(SignInError::Keyring(e)) => {
                tracing::error!("sign-in failed at the keyring: {e}");
                self.error_message = Some(fl!("error-keyring", error = e));
                Task::none()
            }
            // Shown in the sign-in view, with the key to authorize at the
            // signer: what was typed is still in the field, to try again.
            Err(SignInError::Bunker { reason, client, with_secret }) => {
                tracing::error!("sign-in failed at the remote signer: {reason}");
                self.login_refusal = Some(BunkerRefusal { reason, client, with_secret, copied: false });
                Task::none()
            }
        }
    }

    /// Copy the key the signer that turned the sign-in down would have to
    /// know this client by.
    pub fn handle_copy_client_key(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(refusal) = &mut self.login_refusal else {
            return Task::none();
        };
        let Some(client) = refusal.client else {
            return Task::none();
        };
        refusal.copied = true;
        clipboard::write(client.to_hex())
    }

    /// The account named in the config was looked up at startup.
    pub fn handle_account_restored(&mut self, result: Result<Account, String>) -> Task<cosmic::Action<Message>> {
        match result {
            Ok(account) => {
                tracing::info!("account {} ({})", account.npub(), if account.can_sign { "full access" } else { "read-only" });
                self.enter_account(account)
            }
            Err(e) => {
                // The config names something that isn't an npub. Nobody is
                // signed in; say so by asking again.
                tracing::error!("the saved account could not be read: {e}");
                self.config.account = None;
                self.save_config("account");
                self.view_state = ViewState::Login;
                Task::none()
            }
        }
    }

    /// Sign the account out: forget it, take what it signs with out of the
    /// keyring, and let go of its remote signer.
    pub fn handle_sign_out(&mut self) -> Task<cosmic::Action<Message>> {
        self.config.account = None;
        self.save_config("account");
        self.clear_login();
        self.view_state = ViewState::Login;
        // Nobody's follows are anybody's now, and nothing stays open over
        // the feed. What was being written stays a draft: it is the text
        // of whoever sits here, not the key's.
        let kept = self.keep_draft();
        self.composer = None;
        self.zap = None;
        self.pages.clear();
        self.videos.stop();
        self.reset_following();
        self.reset_engagement();
        self.reset_bookmarks();
        self.reset_relays();
        self.reset_media();
        self.reset_kept_drafts();
        self.reset_notifications();
        let account = self.account.take();
        let signed_out = match account {
            Some(account) => {
                tracing::info!("signed out of {}", account.npub());
                let (signer, service) = (self.signer.clone(), self.nostr.clone());
                let leaving = async move {
                    // The relays its relay list named were its own, and
                    // are nobody's now.
                    if let Some(service) = service {
                        service.use_relays_of(None).await;
                    }
                    signer.sign_out(account.public_key).await
                };
                Task::perform(leaving, |result| cosmic::Action::App(Message::SignedOut(result)))
            }
            None => Task::none(),
        };
        Task::batch([kept, signed_out])
    }

    /// The secret key left the keyring, or the keyring refused. A key that
    /// is still there is worth knowing about.
    pub fn handle_signed_out(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            tracing::error!("the secret key could not be removed from the keyring: {e}");
            self.error_message = Some(fl!("error-keyring", error = e));
        }
    }
}

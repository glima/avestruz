// SPDX-License-Identifier: MIT

//! Zap message handlers for Avestruz.
//!
//! Zapping a note opens a view of its own: how much, and a word to go with
//! it. The author's Lightning address is then asked for an invoice, which
//! shows as a QR code for a wallet to scan; this client holds no money and
//! pays nothing itself.
//!
//! While the invoice is on show the relays are listened to for the receipt
//! the author's wallet service publishes once it is paid. When it comes the
//! view says so, and the note's count has it. The listening stops when the
//! view is left.
//!
//! Anyone can zap. With a secret key the zap is the account's; without one
//! the request is signed with a key made for it, and the zap is anonymous.

use cosmic::iced::clipboard;
use cosmic::prelude::*;
use nostr_sdk::prelude::{EventId, RelayUrl};

use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::messages::Message;
use crate::nostr::zap::{self, Invoice, ZapError, ZapOrder, Zapper};
use crate::state::{AppModel, ViewState, ZapStage, ZapState};
use crate::views::components::qr_code;

/// What the amount field starts at, in sats.
const DEFAULT_ZAP_SATS: u64 = 21;

/// How long an amount may be typed, in digits, and a comment, in
/// characters.
const MAX_AMOUNT_DIGITS: usize = 9;
const MAX_COMMENT_CHARS: usize = 140;

/// Why there is no invoice, in the user's language.
fn zap_error_text(error: ZapError) -> String {
    match error {
        ZapError::BadAddress => fl!("zap-error-address"),
        ZapError::Unreachable(error) => fl!("zap-error-unreachable", error = error),
        ZapError::Refused(reason) => fl!("zap-error-refused", reason = reason),
        ZapError::OutOfRange { min, max } => fl!("zap-error-range", min = min, max = max),
        ZapError::WrongAmount => fl!("zap-error-wrong-amount"),
        ZapError::Sign(error) => sign_error_text(error),
    }
}

/// What a wallet scans to pay `bolt11`: the invoice as a `lightning:`
/// address, in capitals, which a QR code holds in fewer modules.
fn wallet_address(bolt11: &str) -> String {
    format!("LIGHTNING:{}", bolt11.to_ascii_uppercase())
}

impl AppModel {
    /// Who a zap sent now would be from.
    pub(crate) fn zapper(&self) -> Zapper {
        match &self.account {
            Some(account) if account.can_sign => Zapper::Account(account.public_key),
            _ => Zapper::Anonymous,
        }
    }

    /// Open the zap view for the note `id`, which is one on show and whose
    /// author has a Lightning address. A zap of the same note that was left
    /// open comes back as it was.
    pub fn handle_zap_note(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        if self.zap.as_ref().is_none_or(|zap| zap.note != id) {
            let Some(note) = self.note_at_hand(id) else {
                return Task::none();
            };
            let Some(address) = self.profiles.get(&note.author).and_then(|profile| profile.lightning.clone()) else {
                return Task::none();
            };
            self.zap = Some(ZapState {
                note: id,
                recipient: note.author,
                address,
                amount: DEFAULT_ZAP_SATS.to_string(),
                comment: String::new(),
                stage: ZapStage::Choosing,
                error: None,
            });
        }
        self.view_state = ViewState::Zap;
        Task::none()
    }

    /// The amount was typed, or a preset chosen. Only digits are kept.
    pub fn handle_zap_amount_changed(&mut self, amount: &str) {
        if let Some(zap) = self.zap.as_mut().filter(|zap| matches!(zap.stage, ZapStage::Choosing)) {
            zap.amount = amount.chars().filter(char::is_ascii_digit).take(MAX_AMOUNT_DIGITS).collect();
            zap.error = None;
        }
    }

    /// The comment was typed.
    pub fn handle_zap_comment_changed(&mut self, comment: &str) {
        if let Some(zap) = self.zap.as_mut().filter(|zap| matches!(zap.stage, ZapStage::Choosing)) {
            zap.comment = comment.chars().take(MAX_COMMENT_CHARS).collect();
        }
    }

    /// Ask the author's Lightning address for an invoice of the amount
    /// chosen.
    pub fn handle_request_zap_invoice(&mut self) -> Task<cosmic::Action<Message>> {
        let zapper = self.zapper();
        // Where the receipt is looked for afterwards: the relays read from.
        let relays: Vec<RelayUrl> = self.config.relays.iter().filter_map(|url| RelayUrl::parse(url).ok()).collect();
        let Some(zap) = self.zap.as_mut().filter(|zap| matches!(zap.stage, ZapStage::Choosing)) else {
            return Task::none();
        };
        let Some(sats) = zap.sats() else {
            return Task::none();
        };
        zap.stage = ZapStage::Requesting;
        zap.error = None;
        let note = zap.note;
        let order = ZapOrder {
            address: zap.address.clone(),
            recipient: zap.recipient,
            note,
            sats,
            comment: zap.comment.trim().to_string(),
            relays,
            zapper,
        };
        Task::perform(zap::request_invoice(self.http.clone(), self.signer.clone(), order), move |result| {
            cosmic::Action::App(Message::ZapInvoiceReady { note, result })
        })
    }

    /// The invoice for the zap of `note` came, or didn't. One that comes
    /// after the view was left, or for another zap, is let go.
    pub fn handle_zap_invoice_ready(
        &mut self,
        note: EventId,
        result: Result<Invoice, ZapError>,
    ) -> Task<cosmic::Action<Message>> {
        let service = self.nostr.clone();
        let Some(zap) = self.zap.as_mut().filter(|zap| zap.note == note && matches!(zap.stage, ZapStage::Requesting)) else {
            return Task::none();
        };
        match result {
            Ok(invoice) => {
                tracing::info!(
                    "invoice for {} sats ({})",
                    invoice.sats,
                    if invoice.is_zap { "a zap" } else { "a plain payment" }
                );
                let qr = qr_code(&wallet_address(&invoice.bolt11));
                // Only a zap is announced; a plain payment has no receipt
                // to wait for.
                let (waiting, handle) = match service.filter(|_| invoice.is_zap) {
                    Some(service) => {
                        let bolt11 = invoice.bolt11.clone();
                        let (waiting, handle) = Task::perform(
                            async move { service.await_zap_receipt(note, bolt11.clone()).await.then_some(bolt11) },
                            move |paid| cosmic::Action::App(Message::ZapReceiptAwaited { note, paid }),
                        )
                        .abortable();
                        (waiting, Some(handle.abort_on_drop()))
                    }
                    None => (Task::none(), None),
                };
                zap.stage = ZapStage::Invoice { invoice, qr, copied: false, waiting: handle };
                waiting
            }
            Err(e) => {
                tracing::error!("no invoice: {e:?}");
                zap.stage = ZapStage::Choosing;
                zap.error = Some(zap_error_text(e));
                Task::none()
            }
        }
    }

    /// Copy the invoice on show, for a wallet on this machine.
    pub fn handle_copy_zap_invoice(&mut self) -> Task<cosmic::Action<Message>> {
        match self.zap.as_mut().map(|zap| &mut zap.stage) {
            Some(ZapStage::Invoice { invoice, copied, .. }) => {
                *copied = true;
                clipboard::write(invoice.bolt11.clone())
            }
            _ => Task::none(),
        }
    }

    /// The wait for the receipt of `note`'s zap ended: with the invoice
    /// that was paid, or with nothing when the time ran out.
    pub fn handle_zap_receipt_awaited(&mut self, note: EventId, paid: Option<String>) -> Task<cosmic::Action<Message>> {
        let Some(zap) = self.zap.as_mut().filter(|zap| zap.note == note) else {
            return Task::none();
        };
        let ZapStage::Invoice { invoice, waiting, .. } = &mut zap.stage else {
            return Task::none();
        };
        match paid {
            Some(bolt11) if bolt11 == invoice.bolt11 => {
                tracing::info!("zap of {} sats paid", invoice.sats);
                zap.stage = ZapStage::Paid { sats: invoice.sats };
                // The receipt is in the store now: count the note again.
                self.engagement_asked.remove(&note);
                self.look_up_engagement([note])
            }
            Some(_) => Task::none(),
            None => {
                *waiting = None;
                Task::none()
            }
        }
    }

    /// Leave the zap view for what it was opened from. Whatever was being
    /// waited for is let go.
    pub fn handle_close_zap(&mut self) -> Task<cosmic::Action<Message>> {
        self.zap = None;
        self.show_list_view()
    }
}

#[cfg(test)]
mod tests {
    use super::wallet_address;

    #[test]
    fn a_wallet_is_given_the_invoice_as_an_address_in_capitals() {
        assert_eq!(wallet_address("lnbc210n1pjq9xyz"), "LIGHTNING:LNBC210N1PJQ9XYZ");
    }
}

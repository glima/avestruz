// SPDX-License-Identifier: MIT

//! The zap view for Avestruz: how much to zap a note, and then the invoice
//! to pay, as a QR code for a wallet to scan.

use cosmic::Element;
use cosmic::iced::widget::image::FilterMethod;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Background, Border, Color, Length};
use cosmic::widget::{self, button, container, text, text_input};

use crate::fl;
use crate::helpers::compact_count;
use crate::messages::Message;
use crate::nostr::models::display_name;
use crate::nostr::zap::{Invoice, Zapper};
use crate::state::{AppModel, ZapStage, ZapState};
use crate::views::components::chip;

/// The amounts offered, in sats: Vega's.
const ZAP_PRESETS: [u64; 5] = [21, 100, 500, 1_000, 5_000];

/// How wide the QR code is shown, the white around it included.
const QR_SIZE: f32 = 300.0;
const QR_RADIUS: f32 = 8.0;

/// A line of small print, wrapped.
fn small_print<'a>(message: String) -> Element<'a, Message> {
    text(message).size(12).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).into()
}

/// The invoice as a QR code, on white whatever the theme, or word that
/// there is none to show.
fn invoice_code<'a>(qr: Option<&cosmic::widget::image::Handle>) -> Element<'a, Message> {
    let Some(qr) = qr else {
        return small_print(fl!("zap-no-code"));
    };
    // Scaled without smoothing: the edges of the modules stay edges.
    let picture = cosmic::widget::image(qr.clone())
        .width(Length::Fixed(QR_SIZE))
        .height(Length::Fixed(QR_SIZE))
        .filter_method(FilterMethod::Nearest);
    let code = container(picture).class(cosmic::theme::Container::custom(|_theme| container::Style {
        background: Some(Background::Color(Color::WHITE)),
        border: Border { radius: QR_RADIUS.into(), ..Default::default() },
        ..Default::default()
    }));
    container(code).width(Length::Fill).align_x(Alignment::Center).into()
}

impl AppModel {
    /// The line above the zap view: the way back, and what this is.
    fn view_zap_header(&self) -> Element<'_, Message> {
        let back =
            button::icon(widget::icon::from_name("go-previous-symbolic")).tooltip(fl!("zap-back")).on_press(Message::CloseZap);
        widget::Row::new()
            .push(back)
            .push(text(fl!("zap-title")).size(16).font(cosmic::font::bold()))
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// Choosing how much to zap, and what to say with it.
    fn view_zap_choice<'a>(&'a self, zap: &'a ZapState, name: String) -> Element<'a, Message> {
        let requesting = matches!(zap.stage, ZapStage::Requesting);
        let chosen = zap.sats();

        let presets = ZAP_PRESETS.into_iter().fold(widget::Row::new().spacing(6), |row, sats| {
            row.push(chip(compact_count(sats), chosen == Some(sats), Message::ZapAmountChanged(sats.to_string())))
        });
        let amount = widget::Row::new()
            .push(text_input(fl!("zap-amount-placeholder"), &zap.amount).on_input(Message::ZapAmountChanged).width(Length::Fill))
            .push(text(fl!("zap-sats")).size(13))
            .spacing(8)
            .align_y(Alignment::Center);
        let comment = text_input(fl!("zap-comment-placeholder"), &zap.comment)
            .on_input(Message::ZapCommentChanged)
            .on_submit(|_| Message::RequestZapInvoice)
            .width(Length::Fill);

        let mut body = widget::Column::new()
            .push(text(fl!("zap-to", name = name)).size(14).font(cosmic::font::bold()).wrapping(Wrapping::WordOrGlyph))
            .push(small_print(zap.address.clone()))
            .push(presets)
            .push(amount)
            .push(comment)
            .spacing(8);
        if self.zapper() == Zapper::Anonymous {
            body = body.push(small_print(fl!("zap-anonymous")));
        }
        if let Some(error) = &zap.error {
            body = body.push(text(error.as_str()).size(12).class(cosmic::theme::Text::Accent).wrapping(Wrapping::WordOrGlyph));
        }
        let label = if requesting { fl!("zap-requesting") } else { fl!("zap-get-invoice") };
        let ask = button::suggested(label)
            .on_press_maybe((!requesting && chosen.is_some()).then_some(Message::RequestZapInvoice))
            .width(Length::Fill);
        body.push(ask).into()
    }

    /// The invoice to pay: its QR code, what paying it does, and copying it.
    fn view_zap_invoice<'a>(
        &'a self,
        invoice: &'a Invoice,
        qr: Option<&'a cosmic::widget::image::Handle>,
        copied: bool,
        waiting: bool,
        name: String,
    ) -> Element<'a, Message> {
        let what = if !invoice.is_zap {
            fl!("zap-plain-payment", name = name.clone())
        } else if waiting {
            fl!("zap-waiting")
        } else {
            fl!("zap-not-seen")
        };
        let copy = button::standard(if copied { fl!("zap-copied") } else { fl!("zap-copy") }).on_press(Message::CopyZapInvoice);
        let buttons = widget::Row::new()
            .push(copy)
            .push(widget::space::horizontal())
            .push(button::suggested(fl!("zap-done")).on_press(Message::CloseZap))
            .align_y(Alignment::Center);

        widget::Column::new()
            .push(
                text(fl!("zap-invoice-for", sats = invoice.sats, name = name))
                    .size(14)
                    .font(cosmic::font::bold())
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .push(invoice_code(qr))
            .push(small_print(fl!("zap-scan")))
            .push(small_print(what))
            .push(buttons)
            .spacing(8)
            .into()
    }

    /// The zap went through.
    fn view_zap_paid(&self, sats: u64, name: String) -> Element<'_, Message> {
        widget::Column::new()
            .push(
                text(fl!("zap-paid", sats = sats, name = name))
                    .size(16)
                    .font(cosmic::font::bold())
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .push(button::suggested(fl!("zap-done")).on_press(Message::CloseZap))
            .spacing(12)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .into()
    }

    /// Render the zap view. With no note being zapped there is nothing to
    /// show but what is under it.
    pub fn view_zap(&self) -> Element<'_, Message> {
        let Some(zap) = &self.zap else {
            return self.view_feed();
        };
        let name = display_name(&self.profiles, &zap.recipient).into_owned();
        let body = match &zap.stage {
            ZapStage::Choosing | ZapStage::Requesting => self.view_zap_choice(zap, name),
            ZapStage::Invoice { invoice, qr, copied, waiting } => {
                self.view_zap_invoice(invoice, qr.as_ref(), *copied, waiting.is_some(), name)
            }
            ZapStage::Paid { sats } => self.view_zap_paid(*sats, name),
        };

        widget::Column::new()
            .push(self.view_zap_header())
            .push(widget::divider::horizontal::default())
            .push(container(body).padding([8, 12]).width(Length::Fill))
            .into()
    }
}

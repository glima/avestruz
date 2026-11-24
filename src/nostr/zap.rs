// SPDX-License-Identifier: MIT

//! Zapping a note (NIP-57): asking its author's Lightning address for an
//! invoice that, once paid, their wallet service announces as a zap.
//!
//! It takes two requests over `https` (LNURL-pay, LUD-06 and LUD-16). The
//! first asks the address what it accepts: how little, how much, and
//! whether it knows about zaps. The second asks for an invoice of an amount,
//! and carries a signed *zap request* naming the note and the relays the
//! receipt should go to. Paying the invoice is left to a wallet: the invoice
//! is all this client produces.
//!
//! Everything the address answers is somebody else's word. The invoice is
//! only shown if it asks for the amount that was chosen.

use nostr_sdk::prelude::{
    Event, EventBuilder, EventId, FinalizeEvent, IntoEventBuilder, Keys, PublicKey, RelayUrl, Url, ZapRequestData,
};

use crate::http;
use crate::nostr::account::{SignError, Signer};

/// How long what a Lightning address answers may be. Real answers are a few
/// hundred bytes, and an invoice a few hundred more.
const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// Why there is no invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZapError {
    /// What the profile gives as a Lightning address is not one.
    BadAddress,
    /// The address did not answer, or not with what LNURL-pay says it should.
    Unreachable(String),
    /// The address answered that it won't, and why.
    Refused(String),
    /// The amount is outside what the address accepts, in sats.
    OutOfRange { min: u64, max: u64 },
    /// The invoice that came back is not for the amount asked.
    WrongAmount,
    /// The zap request could not be signed.
    Sign(SignError),
}

/// What a Lightning address accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayInfo {
    /// Where an invoice is asked for.
    callback: Url,
    /// The least and the most it takes at once, in millisats.
    min_millisats: u64,
    max_millisats: u64,
    /// The key its receipts are signed with, if it does zaps at all.
    zapper: Option<PublicKey>,
    /// How long a comment sent with a plain payment may be, in characters.
    comment_chars: usize,
}

/// An invoice to pay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoice {
    /// The invoice itself (BOLT 11).
    pub bolt11: String,
    /// What it asks for, in sats.
    pub sats: u64,
    /// Whether paying it is a zap: announced by the wallet service, and
    /// counted under the note. If not, it is a payment and nothing more.
    pub is_zap: bool,
}

/// Who a zap is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zapper {
    /// The account, which signs the request.
    Account(PublicKey),
    /// Nobody in particular: the request is signed with a key made for it
    /// and thrown away.
    Anonymous,
}

/// What a Lightning invoice asks for, in millisats, read from the amount
/// in its human-readable part: `lnbc2500u1…` is 2500 micro-bitcoin. `None`
/// for an invoice that names no amount, or isn't one.
pub(crate) fn invoice_millisats(invoice: &str) -> Option<u64> {
    let invoice = invoice.to_ascii_lowercase();
    // The data part has no `1` in it, so the last one ends the first part.
    let (readable, _) = invoice.rsplit_once('1')?;
    let amount = readable.strip_prefix("ln")?.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let (digits, unit) = match amount.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => amount.split_at(at),
        None => (amount, ""),
    };
    let value: u64 = digits.parse().ok()?;
    // One bitcoin is 10^11 millisats.
    match unit {
        "" => value.checked_mul(100_000_000_000),
        "m" => value.checked_mul(100_000_000),
        "u" => value.checked_mul(100_000),
        "n" => value.checked_mul(100),
        "p" => Some(value / 10),
        _ => None,
    }
}

/// Where the Lightning address `name@domain` is asked what it accepts
/// (LUD-16). `None` if it isn't an address.
fn pay_info_url(address: &str) -> Option<Url> {
    let (name, domain) = address.trim().split_once('@')?;
    let name = name.to_ascii_lowercase();
    let is_name = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c));
    let path = format!("/.well-known/lnurlp/{name}");
    let url = Url::parse(&format!("https://{domain}{path}")).ok()?;
    // Whatever stood for the domain was only a domain: nothing in it went
    // into the path, the query, or before an `@` of its own.
    let only_a_domain = url.path() == path && url.query().is_none() && url.fragment().is_none() && url.username().is_empty();
    (is_name && only_a_domain).then_some(url)
}

/// The JSON object in `answer`, unless it is LNURL's way of saying no:
/// `{"status": "ERROR", "reason": "…"}`.
fn parse_answer(answer: &[u8]) -> Result<serde_json::Value, ZapError> {
    let value: serde_json::Value = serde_json::from_slice(answer).map_err(|e| ZapError::Unreachable(e.to_string()))?;
    if value.get("status").and_then(serde_json::Value::as_str).is_some_and(|status| status.eq_ignore_ascii_case("error")) {
        let reason = value.get("reason").and_then(serde_json::Value::as_str).unwrap_or_default();
        return Err(ZapError::Refused(reason.to_string()));
    }
    Ok(value)
}

/// What a Lightning address accepts, by its answer to the first request.
fn parse_pay_info(answer: &[u8]) -> Result<PayInfo, ZapError> {
    let value = parse_answer(answer)?;
    let missing = |what: &str| ZapError::Unreachable(format!("the answer has no usable `{what}`"));
    let text = |field: &str| value.get(field).and_then(serde_json::Value::as_str);
    let number = |field: &str| value.get(field).and_then(serde_json::Value::as_u64);

    if text("tag") != Some("payRequest") {
        return Err(missing("tag"));
    }
    let callback = text("callback")
        .and_then(|callback| Url::parse(callback).ok())
        .filter(|url| url.scheme() == "https")
        .ok_or(missing("callback"))?;
    let (min_millisats, max_millisats) =
        (number("minSendable").ok_or(missing("minSendable"))?, number("maxSendable").ok_or(missing("maxSendable"))?);
    let allows_nostr = value.get("allowsNostr").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let zapper = text("nostrPubkey").filter(|_| allows_nostr).and_then(|key| PublicKey::from_hex(key).ok());
    let comment_chars = number("commentAllowed").map_or(0, |chars| usize::try_from(chars).unwrap_or(usize::MAX));
    Ok(PayInfo { callback, min_millisats, max_millisats, zapper, comment_chars })
}

/// The zap request for `millisats` to the author of `note`, yet to be
/// signed: what the wallet service copies into its receipt, which is to go
/// to `relays`.
fn zap_request(recipient: PublicKey, note: EventId, millisats: u64, comment: &str, relays: Vec<RelayUrl>) -> EventBuilder {
    ZapRequestData::new(recipient, relays).amount(millisats).event_id(note).message(comment).into_event_builder()
}

/// Where an invoice for `millisats` is asked for: `pay_info`'s callback,
/// with the amount and, for a zap, the signed request. A plain payment
/// carries the comment instead, if the address takes one that long.
fn invoice_url(pay_info: &PayInfo, millisats: u64, request: Option<&Event>, comment: &str) -> Url {
    let mut url = pay_info.callback.clone();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("amount", &millisats.to_string());
        match request {
            Some(request) => {
                query.append_pair("nostr", &request.as_json());
            }
            None if !comment.is_empty() && comment.chars().count() <= pay_info.comment_chars => {
                query.append_pair("comment", comment);
            }
            None => {}
        }
    }
    url
}

/// The invoice in the answer to the second request, if it is one for
/// `millisats`.
fn parse_invoice(answer: &[u8], millisats: u64) -> Result<String, ZapError> {
    let value = parse_answer(answer)?;
    let invoice = value
        .get("pr")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ZapError::Unreachable("the answer has no invoice".to_string()))?;
    if invoice_millisats(invoice) != Some(millisats) {
        return Err(ZapError::WrongAmount);
    }
    Ok(invoice.to_string())
}

/// A zap to ask an invoice for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZapOrder {
    /// The Lightning address of the one zapped, from their profile.
    pub address: String,
    /// The one zapped: the author of the note.
    pub recipient: PublicKey,
    /// The note zapped.
    pub note: EventId,
    /// How much, in sats.
    pub sats: u64,
    /// What is said with it. May be empty.
    pub comment: String,
    /// Where the receipt is asked to be sent.
    pub relays: Vec<RelayUrl>,
    /// Who it is from.
    pub zapper: Zapper,
}

/// The key the wallet service behind the Lightning address `address`
/// signs its zap receipts with, as it says itself: none, of one that
/// doesn't do zaps. A receipt for a zap to that address counts when this
/// key signed it (NIP-57).
pub async fn zapper_key(http: reqwest::Client, address: &str) -> Result<Option<PublicKey>, ZapError> {
    let url = pay_info_url(address).ok_or(ZapError::BadAddress)?;
    let answer = http::get(&http, url.as_str(), MAX_ANSWER_BYTES).await.map_err(ZapError::Unreachable)?;
    Ok(parse_pay_info(&answer)?.zapper)
}

/// Ask a Lightning address for the invoice of `order`, with `signer`
/// signing for an account that zaps.
///
/// An address that doesn't do zaps is asked for a plain invoice instead:
/// the money arrives all the same, and the result says which it is.
pub async fn request_invoice(http: reqwest::Client, signer: Signer, order: ZapOrder) -> Result<Invoice, ZapError> {
    let ZapOrder { address, recipient, note, sats, comment, relays, zapper } = order;
    let get = |url: Url| {
        let http = http.clone();
        async move { http::get(&http, url.as_str(), MAX_ANSWER_BYTES).await.map_err(ZapError::Unreachable) }
    };
    let pay_info = parse_pay_info(&get(pay_info_url(&address).ok_or(ZapError::BadAddress)?).await?)?;

    let millisats = sats.saturating_mul(1000);
    if !(pay_info.min_millisats..=pay_info.max_millisats).contains(&millisats) {
        // Rounded inwards: what is shown as the range is inside it.
        return Err(ZapError::OutOfRange { min: pay_info.min_millisats.div_ceil(1000), max: pay_info.max_millisats / 1000 });
    }

    let request = match pay_info.zapper {
        Some(_) => {
            let unsigned = zap_request(recipient, note, millisats, &comment, relays);
            Some(match zapper {
                Zapper::Account(account) => signer.sign(account, unsigned).await.map_err(ZapError::Sign)?,
                Zapper::Anonymous => {
                    unsigned.finalize(&Keys::generate()).map_err(|e| ZapError::Sign(SignError::Keyring(e.to_string())))?
                }
            })
        }
        None => None,
    };
    let answer = get(invoice_url(&pay_info, millisats, request.as_ref(), &comment)).await?;
    let bolt11 = parse_invoice(&answer, millisats)?;
    Ok(Invoice { bolt11, sats, is_zap: request.is_some() })
}

#[cfg(test)]
mod tests {
    use super::{PayInfo, ZapError, invoice_millisats, invoice_url, parse_invoice, parse_pay_info, pay_info_url, zap_request};
    use nostr_sdk::prelude::{Event, EventId, FinalizeEvent, Keys, Kind, RelayUrl, Url};

    #[test]
    fn an_invoice_says_what_it_asks_for() {
        assert_eq!(invoice_millisats("lnbc2500u1pvjluezsp5zyg3zyg3"), Some(250_000_000));
        assert_eq!(invoice_millisats("lnbc210n1pjq9xyz"), Some(21_000), "21 sats");
        assert_eq!(invoice_millisats("lnbc1m1pabc"), Some(100_000_000));
        assert_eq!(invoice_millisats("lnbc20p1pabc"), Some(2));
        assert_eq!(invoice_millisats("LNBC210N1PJQ9XYZ"), Some(21_000), "in capitals, as in a QR code");
        assert_eq!(invoice_millisats("lntb100u1pabc"), Some(10_000_000), "another network");
        assert_eq!(invoice_millisats("lnbc1pvjluezpp5"), None, "no amount named");
        assert_eq!(invoice_millisats("not an invoice"), None);
        assert_eq!(invoice_millisats(""), None);
    }

    #[test]
    fn a_lightning_address_is_asked_at_its_domain_over_https() {
        let url = |address: &str| pay_info_url(address).map(|url| url.to_string());
        assert_eq!(url("ana@example.com").as_deref(), Some("https://example.com/.well-known/lnurlp/ana"));
        assert_eq!(url(" Ana.B+tips@Example.com ").as_deref(), Some("https://example.com/.well-known/lnurlp/ana.b+tips"));
        assert_eq!(url("ana@example.com:8443").as_deref(), Some("https://example.com:8443/.well-known/lnurlp/ana"));

        assert_eq!(url("example.com"), None, "no name");
        assert_eq!(url("@example.com"), None);
        assert_eq!(url("ana@"), None);
        assert_eq!(url("a/../b@example.com"), None, "a name is not a path");
        assert_eq!(url("ana@example.com/elsewhere?x="), None, "a domain is not a path");
        assert_eq!(url("ana@user@example.com"), None, "nor someone at a domain");
        assert_eq!(url("ana@exa mple.com"), None);
    }

    const PAY_INFO: &str = r#"{
        "callback": "https://example.com/lnurlp/ana/callback",
        "minSendable": 1000, "maxSendable": 100000000,
        "metadata": "[[\"text/plain\",\"Sats for Ana\"]]", "tag": "payRequest",
        "allowsNostr": true,
        "nostrPubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "commentAllowed": 20
    }"#;

    fn pay_info(json: &str) -> Result<PayInfo, ZapError> {
        parse_pay_info(json.as_bytes())
    }

    #[test]
    fn an_address_says_what_it_accepts_and_whether_it_does_zaps() {
        let info = pay_info(PAY_INFO).unwrap();
        assert_eq!(info.callback.as_str(), "https://example.com/lnurlp/ana/callback");
        assert_eq!((info.min_millisats, info.max_millisats, info.comment_chars), (1000, 100_000_000, 20));
        assert!(info.zapper.is_some());

        // A key and no word that zaps are allowed is not an address that does them.
        let quiet = pay_info(&PAY_INFO.replace(r#""allowsNostr": true,"#, "")).unwrap();
        assert_eq!(quiet.zapper, None);
        let no_key = pay_info(&PAY_INFO.replace("79be667e", "nonsense")).unwrap();
        assert_eq!(no_key.zapper, None);
    }

    #[test]
    fn an_answer_that_is_not_lnurl_pay_is_no_answer() {
        let unusable = |json: &str| matches!(pay_info(json), Err(ZapError::Unreachable(_)));
        assert!(unusable("<html>not found</html>"));
        assert!(unusable("{}"));
        assert!(unusable(&PAY_INFO.replace("payRequest", "withdrawRequest")));
        assert!(unusable(&PAY_INFO.replace("https://example.com/lnurlp", "http://example.com/lnurlp")), "not https");
        assert!(unusable(&PAY_INFO.replace(r#""minSendable": 1000,"#, "")));

        assert_eq!(
            pay_info(r#"{"status":"ERROR","reason":"no such user"}"#),
            Err(ZapError::Refused("no such user".into())),
            "LNURL's own way of saying no"
        );
    }

    fn signed_request(comment: &str) -> Event {
        let (me, them) = (Keys::generate(), Keys::generate());
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        zap_request(them.public_key(), EventId::from_byte_array([7; 32]), 21_000, comment, vec![relay]).finalize(&me).unwrap()
    }

    #[test]
    fn a_zap_request_names_the_note_its_author_the_amount_and_where_the_receipt_goes() {
        let request = signed_request("great note");
        let tag = |kind: &str| request.tags.iter().find(|tag| tag.kind() == kind).map(|tag| tag.as_slice().to_vec());

        assert_eq!(request.kind, Kind::ZapRequest);
        assert_eq!(request.content, "great note");
        assert_eq!(request.tags.event_ids().collect::<Vec<_>>(), [EventId::from_byte_array([7; 32])]);
        assert_eq!(request.tags.public_keys().count(), 1);
        assert_eq!(tag("amount"), Some(vec!["amount".to_string(), "21000".to_string()]));
        assert_eq!(tag("relays"), Some(vec!["relays".to_string(), "wss://relay.example.com".to_string()]));
    }

    /// The query of `url`, as pairs.
    fn query(url: &Url) -> Vec<(String, String)> {
        url.query_pairs().map(|(key, value)| (key.into_owned(), value.into_owned())).collect()
    }

    #[test]
    fn an_invoice_is_asked_for_with_the_amount_and_the_signed_request() {
        let info = pay_info(PAY_INFO).unwrap();
        let request = signed_request("great note & more");
        let url = invoice_url(&info, 21_000, Some(&request), "great note & more");

        assert_eq!(url.path(), "/lnurlp/ana/callback");
        assert_eq!(query(&url), [("amount".to_string(), "21000".to_string()), ("nostr".to_string(), request.as_json())]);
        // What arrives is the request itself, whatever was in it.
        let arrived = Event::from_json(&query(&url)[1].1).unwrap();
        assert_eq!(arrived, request);
        assert!(arrived.verify().is_ok());
    }

    #[test]
    fn a_plain_payment_carries_its_comment_only_if_the_address_takes_it() {
        let mut info = pay_info(PAY_INFO).unwrap();
        // A callback that has a query of its own keeps it.
        info.callback = Url::parse("https://example.com/pay?user=ana").unwrap();
        let pair = |key: &str, value: &str| (key.to_string(), value.to_string());

        assert_eq!(
            query(&invoice_url(&info, 5_000, None, "thanks")),
            [pair("user", "ana"), pair("amount", "5000"), pair("comment", "thanks")]
        );
        let long = "a comment longer than the twenty characters it takes";
        assert_eq!(query(&invoice_url(&info, 5_000, None, long)), [pair("user", "ana"), pair("amount", "5000")]);
        assert_eq!(query(&invoice_url(&info, 5_000, None, "")), [pair("user", "ana"), pair("amount", "5000")]);
    }

    #[test]
    fn an_invoice_is_only_taken_if_it_asks_for_what_was_chosen() {
        let answer = |invoice: &str| format!(r#"{{"pr":"{invoice}","routes":[]}}"#);
        assert_eq!(parse_invoice(answer("lnbc210n1pjq9xyz").as_bytes(), 21_000).as_deref(), Ok("lnbc210n1pjq9xyz"));
        assert_eq!(parse_invoice(answer("lnbc10u1pjq9xyz").as_bytes(), 21_000), Err(ZapError::WrongAmount), "for more");
        assert_eq!(parse_invoice(answer("lnbc1pjq9xyz").as_bytes(), 21_000), Err(ZapError::WrongAmount), "for any amount");
        assert!(matches!(parse_invoice(b"{}", 21_000), Err(ZapError::Unreachable(_))));
        assert_eq!(
            parse_invoice(br#"{"status":"ERROR","reason":"amount too small"}"#, 21_000),
            Err(ZapError::Refused("amount too small".into()))
        );
    }
}

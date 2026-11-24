// SPDX-License-Identifier: MIT

//! What a relay says of itself (NIP-11): its name, a few words about it,
//! which NIPs it supports, and whom it takes.
//!
//! A relay answers at its own address, over `https` where it is reached
//! over `wss`, to whoever asks for `application/nostr+json` instead of a
//! page. What it says is its own word, and is shown as that.

use nostr_sdk::prelude::RelayUrl;
use serde_json::Value;

use crate::http;

/// The most a relay's account of itself may weigh.
const DOCUMENT_MAX_BYTES: usize = 256 * 1024;

/// How many characters of a relay's name, and of what it says about
/// itself, are kept.
const NAME_MAX_CHARS: usize = 60;
const DESCRIPTION_MAX_CHARS: usize = 200;

/// Whom a relay says it takes: what it asks of whoever connects, or
/// writes. What it doesn't say it asks, it is taken not to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Whoever connects has to say who they are (NIP-42) before anything
    /// else.
    pub auth_required: bool,
    /// Whoever connects has to have paid.
    pub payment_required: bool,
    /// It takes events only of some: those it knows, or what it is for.
    pub restricted_writes: bool,
}

/// What a relay says of itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayInfo {
    /// What it calls itself.
    pub name: Option<String>,
    /// A few words about it: the first line of what it says.
    pub description: Option<String>,
    /// The NIPs it says it supports. `None` if it names none, which says
    /// nothing of what it does.
    pub nips: Option<Vec<u16>>,
    /// Whom it says it takes.
    pub limits: Limits,
}

impl RelayInfo {
    /// Whether it says it supports `nip`. `None` if it doesn't say what it
    /// supports.
    pub fn supports(&self, nip: u16) -> Option<bool> {
        self.nips.as_ref().map(|nips| nips.contains(&nip))
    }
}

/// Where `relay` says what it is: its own address, as a page is asked
/// for. Only for a relay reached with encryption: the other kind is on
/// this machine or this network, and is not asked over the web.
pub fn document_url(relay: &RelayUrl) -> Option<String> {
    relay.as_str().strip_prefix("wss://").map(|rest| format!("https://{rest}"))
}

/// What the document `json` says of the relay that sent it. `None` if it
/// is no such document.
pub fn parse(json: &str) -> Option<RelayInfo> {
    let document: Value = serde_json::from_str(json).ok()?;
    let document = document.as_object()?;
    let said = |field: &str, max: usize| {
        let words = document.get(field)?.as_str()?.lines().next()?.trim();
        (!words.is_empty()).then(|| words.chars().take(max).collect::<String>())
    };
    let nips: Option<Vec<u16>> = document
        .get("supported_nips")
        .and_then(Value::as_array)
        .map(|nips| nips.iter().filter_map(|nip| u16::try_from(nip.as_u64()?).ok()).collect())
        .filter(|nips: &Vec<u16>| !nips.is_empty());
    let asked =
        |limit: &str| document.get("limitation").and_then(|limits| limits.get(limit)).and_then(Value::as_bool).unwrap_or(false);
    let limits = Limits {
        auth_required: asked("auth_required"),
        payment_required: asked("payment_required"),
        restricted_writes: asked("restricted_writes"),
    };
    Some(RelayInfo { name: said("name", NAME_MAX_CHARS), description: said("description", DESCRIPTION_MAX_CHARS), nips, limits })
}

/// What `relay` says of itself, if it says: asked over `https`.
pub async fn fetch(http: &reqwest::Client, relay: &RelayUrl) -> Option<RelayInfo> {
    let url = document_url(relay)?;
    match http::get_as(http, &url, "application/nostr+json", DOCUMENT_MAX_BYTES).await {
        Ok(answer) => parse(&String::from_utf8_lossy(&answer)),
        Err(e) => {
            tracing::debug!("{relay} did not say what it is: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Limits, RelayInfo, document_url, parse};
    use nostr_sdk::prelude::RelayUrl;

    #[test]
    fn a_relay_is_asked_what_it_is_at_its_own_address() {
        let url = |relay: &str| document_url(&RelayUrl::parse(relay).unwrap());
        assert_eq!(url("wss://relay.example.com").as_deref(), Some("https://relay.example.com"));
        assert_eq!(url("wss://relay.example.com/inbox").as_deref(), Some("https://relay.example.com/inbox"));
        assert_eq!(url("ws://127.0.0.1:7777"), None);
    }

    #[test]
    fn what_a_relay_says_of_itself_is_its_name_a_line_about_it_and_what_it_supports() {
        let document =
            r#"{"name":" Example relay ","description":"A relay.\nWith rules.","supported_nips":[1,11,50],"software":"x"}"#;
        let info = parse(document).unwrap();
        assert_eq!(info.name.as_deref(), Some("Example relay"));
        assert_eq!(info.description.as_deref(), Some("A relay."));
        assert_eq!(info.nips, Some(vec![1, 11, 50]));
        assert_eq!((info.supports(50), info.supports(42)), (Some(true), Some(false)));

        assert_eq!(info.limits, Limits::default());

        // Whom it takes is what it says of that, and anyone otherwise.
        let limited = r#"{"limitation":{"auth_required":true,"payment_required":false,"restricted_writes":true,"max_limit":5}}"#;
        let limits = Limits { auth_required: true, payment_required: false, restricted_writes: true };
        assert_eq!(parse(limited), Some(RelayInfo { limits, ..RelayInfo::default() }));
        let paid = Limits { payment_required: true, ..Limits::default() };
        assert_eq!(parse(r#"{"supported_nips":[],"limitation":{"payment_required":true}}"#).unwrap().limits, paid);
        assert_eq!(parse(r#"{"limitation":{"payment_required":"yes"}}"#).unwrap().limits, Limits::default());
        assert_eq!(parse(r#"{"limitation":[true]}"#).unwrap().limits, Limits::default());

        // Little said, or said oddly, is still an answer; and what is no
        // such document is none.
        assert_eq!(parse(r#"{"supported_nips":[1],"name":""}"#), Some(RelayInfo { nips: Some(vec![1]), ..RelayInfo::default() }));
        // One that names no NIP it supports says nothing of what it does.
        for unsaid in [r#"{"supported_nips":"50","name":7}"#, r#"{"supported_nips":[]}"#, r#"{"supported_nips":["50",-1]}"#, "{}"]
        {
            assert_eq!(parse(unsaid), Some(RelayInfo::default()), "{unsaid}");
            assert_eq!(parse(unsaid).unwrap().supports(50), None);
        }
        let long = format!(r#"{{"name":"{}"}}"#, "n".repeat(500));
        assert_eq!(parse(&long).unwrap().name.unwrap().chars().count(), 60);
        assert_eq!(parse("<html></html>"), None);
        assert_eq!(parse("[]"), None);
    }
}

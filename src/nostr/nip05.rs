// SPDX-License-Identifier: MIT

//! Names at a domain (NIP-05): `ana@example.com`.
//!
//! Anyone can write any name into their profile. It is theirs only if the
//! domain says so: `https://<domain>/.well-known/nostr.json?name=<name>`
//! has to give their public key for that name. Until it has, the name is
//! not shown; a name that could be shown unchecked is a name that could be
//! borrowed.
//!
//! The domain has to answer itself. An answer from somewhere it redirects
//! to is not taken, as NIP-05 has it.

use nostr_sdk::prelude::{Nip05Address, Nip05Profile, PublicKey, verify_from_raw_json};

use crate::http;

/// The most a domain's answer may weigh. Asked for one name, a domain
/// answers with one; some answer with every name they have.
const ANSWER_MAX_BYTES: usize = 1024 * 1024;

/// What `claimed` says, if it is a name at a domain: the name in lower
/// case, which is how it is looked up, and where to ask.
fn parse(claimed: &str) -> Option<Nip05Address> {
    let claimed = claimed.trim().to_lowercase();
    let address = Nip05Address::parse(&claimed).ok()?;
    let name_is_plain =
        !address.name().is_empty() && address.name().chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    // What follows the `@` has to be a host and nothing more: anything
    // else would have the question asked of another place on it.
    let url = address.url();
    let domain_is_a_host = address.domain().contains('.')
        && url.host_str() == Some(address.domain())
        && url.path() == "/.well-known/nostr.json"
        && url.port().is_none()
        && url.username().is_empty();
    (name_is_plain && domain_is_a_host).then_some(address)
}

/// How a name shows: `ana@example.com`, or the domain alone for the name
/// `_`, which stands for the domain itself.
pub fn shown(claimed: &str) -> String {
    match parse(claimed) {
        Some(address) if address.name() == "_" => address.domain().to_string(),
        Some(address) => address.to_string(),
        None => claimed.trim().to_string(),
    }
}

/// Whether the domain in `claimed` says that name is `public_key`'s.
/// Anything short of that is no: a name that isn't one, a domain that
/// doesn't answer, an answer that names someone else.
pub async fn verify(http: &reqwest::Client, public_key: PublicKey, claimed: &str) -> bool {
    let Some(address) = parse(claimed) else {
        return false;
    };
    match http::get_in_place(http, address.url().as_str(), ANSWER_MAX_BYTES).await {
        Ok(answer) => verify_from_raw_json(&public_key, &address, &String::from_utf8_lossy(&answer)).unwrap_or(false),
        Err(e) => {
            tracing::debug!("{} did not answer for {}: {e}", address.domain(), address.name());
            false
        }
    }
}

/// Whether `typed` reads as a name at a domain, `ana@example.com`: what
/// can be asked of the domain. A domain alone does not: it reads as a
/// word, or as the address of a page.
pub fn is_name(typed: &str) -> bool {
    typed.contains('@') && parse(typed).is_some()
}

/// Whose name `claimed` is, as its domain says. Nobody's, if it is no
/// name at a domain, the domain doesn't answer, or it knows no such name.
pub async fn resolve(http: &reqwest::Client, claimed: &str) -> Option<PublicKey> {
    let address = parse(claimed)?;
    match http::get_in_place(http, address.url().as_str(), ANSWER_MAX_BYTES).await {
        Ok(answer) => Nip05Profile::from_raw_json(&address, &String::from_utf8_lossy(&answer)).ok().map(|named| named.public_key),
        Err(e) => {
            tracing::debug!("{} did not answer for {}: {e}", address.domain(), address.name());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_name, parse, shown};
    use nostr_sdk::prelude::{Keys, verify_from_raw_json};

    #[test]
    fn a_name_at_a_domain_is_told_from_a_word_and_from_a_domain() {
        assert!(is_name("ana@example.com") && is_name(" Ana@Example.com "));
        for not_a_name in ["example.com", "ana", "ana@localhost", "two words@example.com", "@example.com", ""] {
            assert!(!is_name(not_a_name), "{not_a_name}");
        }
    }

    #[test]
    fn a_name_is_asked_of_its_own_domain_and_nowhere_else() {
        let url = |claimed: &str| parse(claimed).map(|address| address.url().to_string());
        assert_eq!(url("ana@example.com").as_deref(), Some("https://example.com/.well-known/nostr.json?name=ana"));
        assert_eq!(url(" Ana@Example.com ").as_deref(), Some("https://example.com/.well-known/nostr.json?name=ana"));
        assert_eq!(url("example.com").as_deref(), Some("https://example.com/.well-known/nostr.json?name=_"));

        // Not a host, or more than a host.
        assert_eq!(url("ana@localhost"), None);
        assert_eq!(url("ana@example.com/elsewhere?x="), None);
        assert_eq!(url("ana@example.com:8443"), None);
        assert_eq!(url("ana@someone@example.com"), None);
        assert_eq!(url("ana@example.com#"), None);
        // Not a name.
        assert_eq!(url("ana&name=bruno@example.com"), None);
        assert_eq!(url("@example.com"), None);
        assert_eq!(url(""), None);
    }

    #[test]
    fn a_name_shows_whole_and_the_domains_own_name_as_the_domain() {
        assert_eq!(shown("Ana@Example.com"), "ana@example.com");
        assert_eq!(shown("_@example.com"), "example.com");
        assert_eq!(shown("example.com"), "example.com");
    }

    #[test]
    fn a_name_is_someones_only_if_the_domain_gives_their_key_for_it() {
        let (ana, bruno) = (Keys::generate().public_key(), Keys::generate().public_key());
        let address = parse("ana@example.com").unwrap();
        let answer = format!(r#"{{"names":{{"ana":"{}"}}}}"#, ana.to_hex());

        assert_eq!(verify_from_raw_json(&ana, &address, &answer).ok(), Some(true));
        assert_eq!(verify_from_raw_json(&bruno, &address, &answer).ok(), Some(false), "someone else's name");
        assert_eq!(verify_from_raw_json(&ana, &address, r#"{"names":{}}"#).ok(), Some(false));
        assert!(verify_from_raw_json(&ana, &address, "<html>").is_err());
    }
}

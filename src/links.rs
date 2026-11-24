// SPDX-License-Identifier: MIT

//! Opening a link in the system's browser.
//!
//! Every address opened comes from somebody else: whoever wrote a note, or
//! a remote signer asking for approval. Only web links are handed over.
//! `file:` and every other scheme the desktop would act on are refused
//! here, whatever the caller let through.

use nostr_sdk::prelude::Url;

/// Why a link was not opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// It is not a web address.
    NotWeb,
    /// The desktop could not open it.
    Failed(String),
}

/// Whether `url` is a web address: `http` or `https`.
fn is_web(url: &str) -> bool {
    Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
}

/// Open `url` in the system's browser, if it is a web address.
pub fn open_in_browser(url: &str) -> Result<(), OpenError> {
    if !is_web(url) {
        return Err(OpenError::NotWeb);
    }
    open::that_detached(url).map_err(|e| OpenError::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{OpenError, is_web, open_in_browser};

    #[test]
    fn only_web_addresses_are_opened() {
        assert!(is_web("https://example.com/a?b=c"));
        assert!(is_web("http://example.com"));

        for other in ["file:///etc/passwd", "javascript:alert(1)", "mailto:a@example.com", "example.com", "", "ssh://host"] {
            assert!(!is_web(other), "{other}");
            assert_eq!(open_in_browser(other), Err(OpenError::NotWeb), "{other}");
        }
    }
}

// SPDX-License-Identifier: MIT

//! Fetching from the web: the one client everything is fetched with, and
//! the one way of fetching.
//!
//! Every address comes from whoever wrote a profile or a note. Only `https`
//! is fetched, redirects included; nothing waits longer than
//! [`FETCH_TIMEOUT`]; and no more is read than the caller is ready to keep,
//! so a hostile address costs a refused request at worst. Of a web page
//! only the head is read, and that only if the server calls it a page.
//!
//! Videos are the one thing fetched that is not fetched here: see
//! [`crate::video`].

use std::time::Duration;

/// How long anything gets to arrive.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The HTTP client: `https` only, redirects included, and never waiting
/// longer than [`FETCH_TIMEOUT`].
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("avestruz/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// What is at `url`, if it answers with success and is no more than
/// `max_bytes` long.
pub async fn get(http: &reqwest::Client, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let response = http.get(url).send().await.map_err(|e| e.to_string())?;
    read(response, max_bytes).await
}

/// As [`get`], of an address that has to answer for itself: what came from
/// anywhere it sent the request on to is refused.
pub async fn get_in_place(http: &reqwest::Client, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let response = http.get(url).send().await.map_err(|e| e.to_string())?;
    if response.url().as_str() != url {
        return Err("redirected".to_string());
    }
    read(response, max_bytes).await
}

/// What `response` carries, if it is a success and no more than `max_bytes`
/// long.
async fn read(response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut response = response.error_for_status().map_err(|e| e.to_string())?;
    if response.content_length().is_some_and(|length| length > max_bytes as u64) {
        return Err(format!("larger than {max_bytes} bytes"));
    }

    // A server may not say how much it will send, or may say less than it
    // does: count what arrives.
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > max_bytes {
            return Err(format!("larger than {max_bytes} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// What `url` answers when it is asked for `accept`, up to `max_bytes`:
/// a relay's account of itself, say, which is at the relay's own address
/// for whoever asks for that and not for a page.
pub async fn get_as(http: &reqwest::Client, url: &str, accept: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let response = http.get(url).header(reqwest::header::ACCEPT, accept).send().await.map_err(|e| e.to_string())?;
    let response = response.error_for_status().map_err(|e| e.to_string())?;
    read(response, max_bytes).await
}

/// The head of a web page: where it was found, after whatever redirects,
/// and its bytes as far as they were read.
pub struct PageHead {
    pub url: reqwest::Url,
    pub bytes: Vec<u8>,
}

/// What a link turned out to be, asked for as a web page.
pub enum Linked {
    /// A page: its head.
    Page(PageHead),
    /// A video, as its server calls it. None of it was read.
    Video,
}

/// The media type in what a server calls `content_type`, without what
/// follows it.
fn media_type(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or_default().trim()
}

/// Whether what a server calls `content_type` is a web page.
fn is_html(content_type: &str) -> bool {
    let kind = media_type(content_type);
    kind.eq_ignore_ascii_case("text/html") || kind.eq_ignore_ascii_case("application/xhtml+xml")
}

/// Whether what a server, or a note, calls `content_type` is a video.
pub fn is_video(content_type: &str) -> bool {
    media_type(content_type).get(.."video/".len()).is_some_and(|start| start.eq_ignore_ascii_case("video/"))
}

/// Where the head of a page ends in `bytes`, looking from `from` on: at the
/// tag that closes it, or the one that opens the body.
fn head_end(bytes: &[u8], from: usize) -> Option<usize> {
    const ENDS: [&[u8]; 2] = [b"</head", b"<body"];
    (from..bytes.len())
        .find(|&at| ENDS.iter().any(|end| bytes.get(at..at + end.len()).is_some_and(|tag| tag.eq_ignore_ascii_case(end))))
}

/// The head of the web page at `url`: what it sends until its head ends,
/// and `max_bytes` of it at most. Unlike [`get`], a page longer than that
/// is no error: the rest is left unread. What the server doesn't call a
/// web page is refused before any of it is read; if it calls it a video,
/// that is the answer.
pub async fn get_page_head(http: &reqwest::Client, url: &str, max_bytes: usize) -> Result<Linked, String> {
    let response = http.get(url).header(reqwest::header::ACCEPT, "text/html").send().await.map_err(|e| e.to_string())?;
    let mut response = response.error_for_status().map_err(|e| e.to_string())?;
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok());
    if content_type.is_some_and(is_video) {
        return Ok(Linked::Video);
    }
    if !content_type.is_some_and(is_html) {
        return Err(format!("not a web page: {}", content_type.unwrap_or("of no kind it names")));
    }

    let url = response.url().clone();
    let mut bytes = Vec::new();
    while bytes.len() < max_bytes {
        let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? else {
            break;
        };
        // A tag may lie across two chunks: look again from a little before
        // the new one.
        let from = bytes.len().saturating_sub(8);
        let room = max_bytes - bytes.len();
        bytes.extend(chunk.iter().take(room));
        if let Some(end) = head_end(&bytes, from) {
            bytes.truncate(end);
            break;
        }
    }
    Ok(Linked::Page(PageHead { url, bytes }))
}

#[cfg(test)]
mod tests {
    use super::{head_end, is_html, is_video};

    #[test]
    fn a_web_page_is_what_the_server_calls_one() {
        assert!(is_html("text/html"));
        assert!(is_html("Text/HTML; charset=utf-8"));
        assert!(is_html("application/xhtml+xml"));
        for other in ["application/pdf", "video/mp4", "text/plain", "application/json", ""] {
            assert!(!is_html(other), "{other}");
        }
    }

    #[test]
    fn a_video_is_what_the_server_calls_one() {
        for video in ["video/mp4", "Video/WebM; codecs=vp9", " video/quicktime "] {
            assert!(is_video(video), "{video}");
        }
        for other in ["text/html", "application/octet-stream", "audio/mpeg", "image/gif", "video", ""] {
            assert!(!is_video(other), "{other}");
        }
    }

    #[test]
    fn the_head_of_a_page_ends_where_it_closes_or_the_body_opens() {
        let page = b"<html><head><title>a</title></HEAD><body>text</body>";
        assert_eq!(head_end(page, 0), Some(28));
        assert_eq!(head_end(b"<title>a</title><BODY class=x>", 0), Some(16), "a head that is never closed");
        assert_eq!(head_end(b"<html><head><title>a</title><meta", 0), None, "not there yet");
        // Looked for from where the last look left off, less a tag's length.
        assert_eq!(head_end(page, 28), Some(28));
        assert_eq!(head_end(page, 29), Some(35), "the body, having missed the end of the head");
    }

    #[test]
    fn one_tls_provider_is_compiled_in() {
        // With no provider installed by name (as `main` does), rustls has to
        // work one out from its crate features, and panics if it finds two.
        // That is what a dependency turning on a second provider looks like,
        // and it would cost a second crypto library in the binary.
        let _ = rustls::ClientConfig::builder();
        let _ = super::client();
    }
}

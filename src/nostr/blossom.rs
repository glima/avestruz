// SPDX-License-Identifier: MIT

//! Media servers (Blossom): the account's list of them, and sending a
//! file to them so that a note can link to it.
//!
//! A media server keeps files by what they are: each is known by the
//! SHA-256 of its bytes, and is at that address on every server that has
//! it. So a file that is on several servers is still to be found when one
//! of them is gone, by anyone who knows where else its owner keeps files:
//! which is what the account's server list says (BUD-03, kind 10063), one
//! `server` tag for each, the one to try first at the top.
//!
//! A file is sent to the first server of the list that takes it (BUD-02),
//! and the others are then asked to fetch it from there (BUD-04), so that
//! it is on all of them:
//!
//! ```mermaid
//! sequenceDiagram
//!     participant A as Avestruz
//!     participant F as The first server
//!     participant O as Each other server
//!     Note over A: The file is hashed, and an authorization signed for that hash
//!     A->>F: HEAD /upload: would you take this? Its hash, size and type
//!     F-->>A: Yes. Or no, and why: the next server is tried
//!     A->>F: PUT /upload: the file
//!     F-->>A: Where it is now: its address, hash, size and type
//!     A->>O: PUT /mirror: fetch it from that address
//!     O-->>A: Done, or not: the file is attached either way
//! ```
//!
//! Every request carries the same authorization (BUD-11): an event of
//! kind 24242 that the account signs, which says what may be done
//! (`t`: `upload`), to which file (`x`: its hash) and until when
//! (`expiration`). It goes in the `Authorization` header, as base64. The
//! specification says base64 for addresses, without padding; servers
//! written before it said so read only the plain kind, padded, and answer
//! the other with 400 or 401. Such an answer has the request made once
//! more, the other way.
//!
//! The server is asked first whether it would take the file (BUD-06),
//! which costs nothing; a server that says no is not sent the file to say
//! no again, and one that doesn't know the question is sent it.
//!
//! What the note then carries is the file's address, in its text, and an
//! `imeta` tag (NIP-92) that says of that address what whoever shows the
//! note would otherwise have to fetch the file to know: what kind of file
//! it is, how large, its hash, and for a picture its width and height.
//!
//! No server is built in, and no list is written that the account did not
//! ask for: where its files go is for the account to say.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use futures_util::future::join_all;
use nostr_sdk::prelude::{Event, EventBuilder, Kind, Tag, Timestamp, Url};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Method, Response, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::media::Media;

/// The name of the tag that names a media server in a server list
/// (BUD-03).
const SERVER_TAG: &str = "server";

/// The name of the tag that says of a link in a note what is at it
/// (NIP-92).
const IMETA_TAG: &str = "imeta";

/// How long an authorization holds: long enough for a large file on a slow
/// line, and not much use to whoever comes by it later.
const AUTH_SECONDS: u64 = 60 * 60;

/// How long a server gets to take a file, and how long to fetch one from
/// another server.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(AUTH_SECONDS);
const MIRROR_TIMEOUT: Duration = Duration::from_secs(120);

/// How much of a file is handed over at a time: how finely it is known
/// how much of it has gone.
const PIECE: usize = 64 * 1024;

/// How much is read of what a server answers: where the file is, or why
/// it was not taken.
const MAX_ANSWER: usize = 64 * 1024;

/// How much of a server's reason is shown, in characters.
const MAX_REASON: usize = 300;

/// What a server answers a question it doesn't know with, where the
/// question is whether it would take a file (BUD-06): the file is sent
/// without asking.
const NOT_ASKED_THAT: [StatusCode; 3] = [StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED, StatusCode::NOT_IMPLEMENTED];

/// What a server that doesn't read the authorization as it was written
/// answers with: it is written the other way and sent once more.
const NOT_READ: [StatusCode; 2] = [StatusCode::BAD_REQUEST, StatusCode::UNAUTHORIZED];

/// How much of a file has been handed over to be sent, in bytes: counted
/// by what sends it, and read by what shows how far it has got.
pub type Sent = Arc<AtomicUsize>;

/// The media server typed as `typed`, with or without `https://`. `None`
/// if it is no address, or not an `https` one: nothing else is sent to.
/// A server is where its endpoints are, which is at the root of its
/// domain (BUD-01), so whatever path was typed after it is not part of it.
pub fn typed_server(typed: &str) -> Option<Url> {
    let typed = typed.trim();
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    let url = if typed.contains("://") { Url::parse(typed) } else { Url::parse(&format!("https://{typed}")) }.ok()?;
    server_of(&url)
}

/// The server that `url` is the address of, or is an address on.
fn server_of(url: &Url) -> Option<Url> {
    if url.scheme() != "https" || url.host_str().is_none_or(|host| !host.contains('.')) {
        return None;
    }
    Url::parse(&url.origin().ascii_serialization()).ok()
}

/// What a server is called on a page: its address without the scheme.
pub fn server_name(server: &Url) -> String {
    let name = server.as_str().trim_end_matches('/');
    name.strip_prefix("https://").unwrap_or(name).to_string()
}

/// The address of `path` on `server`.
fn endpoint(server: &Url, path: &str) -> String {
    format!("{}/{path}", server.origin().ascii_serialization())
}

/// Whether `tag` names a media server.
fn names_server(tag: &Tag) -> bool {
    tag.as_slice().first().is_some_and(|name| name == SERVER_TAG)
}

/// The media servers the server list `list` names, in its order, each
/// once. None with no list.
pub fn servers(list: Option<&Event>) -> Vec<Url> {
    let mut servers = Vec::new();
    let named = list.into_iter().flat_map(|list| list.tags.iter()).filter(|tag| names_server(tag));
    for server in named.filter_map(|tag| Url::parse(tag.as_slice().get(1)?).ok()).filter_map(|url| server_of(&url)) {
        if !servers.contains(&server) {
            servers.push(server);
        }
    }
    servers
}

/// The server list (BUD-03) that replaces `earlier`, if there is one to
/// replace: with `servers` for its servers, in that order, and whatever
/// else `earlier` had in it as it was.
///
/// `None` when `earlier` names the same servers in the same order: there
/// is nothing to write.
pub fn server_list(earlier: Option<&Event>, servers_now: &[Url], now: Timestamp) -> Option<EventBuilder> {
    if earlier.is_some() && servers(earlier) == servers_now {
        return None;
    }
    let kept = earlier.into_iter().flat_map(|earlier| earlier.tags.iter()).filter(|tag| !names_server(tag)).cloned();
    let named = servers_now.iter().filter_map(|server| Tag::parse([SERVER_TAG, server.as_str().trim_end_matches('/')]).ok());
    let tags: Vec<Tag> = named.chain(kept).collect();
    let content = earlier.map(|earlier| earlier.content.clone()).unwrap_or_default();
    // Claimed to be from after the one it replaces: it has to be the newer.
    let after = earlier.map_or(now, |earlier| now.max(Timestamp::from_secs(earlier.created_at.as_secs() + 1)));
    Some(EventBuilder::new(Kind::BlossomServerList, content).tags(tags).custom_created_at(after))
}

/// What a file is known by on a media server: the SHA-256 of `bytes`, in
/// lowercase hex.
pub fn hash_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The authorization (BUD-11) to send the file called `name`, whose hash
/// is `hash`, to media servers, from `now` for [`AUTH_SECONDS`]: for the
/// account to sign.
pub fn upload_auth(name: &str, hash: &str, now: Timestamp) -> EventBuilder {
    let tags = [Tag::parse(["t", "upload"]).ok(), Tag::parse(["x", hash]).ok(), Some(Tag::expiration(now + AUTH_SECONDS))];
    EventBuilder::new(Kind::BlossomAuth, format!("Upload {name}")).tags(tags.into_iter().flatten()).custom_created_at(now)
}

/// What the `Authorization` header says for the authorization `auth`:
/// the event as base64, for addresses and unpadded as the specification
/// has it, or plain and `padded` as servers older than that read it.
fn authorization(auth: &Event, padded: bool) -> String {
    let json = auth.as_json();
    let encoded = if padded { STANDARD.encode(json) } else { URL_SAFE_NO_PAD.encode(json) };
    format!("Nostr {encoded}")
}

/// Why a server did not take a file, or did not fetch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadError {
    /// It could not be reached, or the line broke.
    Unreachable(String),
    /// It said no: with this status, and for this reason, if it gave one.
    Refused { status: u16, reason: String },
    /// It said yes, and what it said of where the file is could not be
    /// read.
    Unreadable(String),
}

/// A file as a server says it has it (BUD-02).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Where it is.
    pub url: String,
    /// The hash of what is there.
    pub hash: String,
}

/// What a server answers when it has taken a file: a blob descriptor.
#[derive(Deserialize)]
struct Descriptor {
    url: String,
    sha256: Option<String>,
}

/// What `response` says, as far as [`MAX_ANSWER`].
async fn answer(mut response: Response) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = MAX_ANSWER.saturating_sub(bytes.len());
        bytes.extend_from_slice(chunk.get(..chunk.len().min(room)).unwrap_or_default());
        if bytes.len() >= MAX_ANSWER {
            break;
        }
    }
    bytes
}

/// Why `response`, which is no success, says the request was refused: the
/// reason the server gives in its `X-Reason` header, or failing that in
/// what it answers, or failing that what the status means.
async fn refusal(response: Response) -> UploadError {
    let status = response.status();
    let header = response.headers().get("x-reason").and_then(|reason| reason.to_str().ok()).map(str::to_string);
    let reason = match header.filter(|reason| !reason.trim().is_empty()) {
        Some(reason) => reason,
        None => {
            let said = String::from_utf8_lossy(&answer(response).await).trim().to_string();
            let message = serde_json::from_str::<serde_json::Value>(&said)
                .ok()
                .and_then(|json| json.get("message").and_then(|message| message.as_str().map(str::to_string)));
            // A page of HTML is no reason.
            message.or_else(|| (!said.is_empty() && !said.starts_with('<')).then_some(said)).unwrap_or_default()
        }
    };
    let reason = if reason.trim().is_empty() { status.canonical_reason().unwrap_or_default().to_string() } else { reason };
    UploadError::Refused { status: status.as_u16(), reason: reason.trim().chars().take(MAX_REASON).collect() }
}

/// Why `error` came about, down to what was behind it: a request that
/// failed says no more of itself than that it did.
fn why(error: &dyn std::error::Error) -> String {
    let mut said = error.to_string();
    let mut behind = error.source();
    while let Some(cause) = behind {
        let cause_said = cause.to_string();
        if !said.contains(&cause_said) {
            said = format!("{said}: {cause_said}");
        }
        behind = cause.source();
    }
    said
}

/// Requests to one server, each with the authorization: written as the
/// specification has it until the server doesn't read it, and the other
/// way from then on.
struct Asking<'a> {
    http: &'a reqwest::Client,
    auth: &'a Event,
    padded: bool,
}

impl Asking<'_> {
    /// Send what `request` makes, with the authorization.
    async fn send(&mut self, request: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder) -> Result<Response, UploadError> {
        let unreachable = |e: reqwest::Error| UploadError::Unreachable(why(&e.without_url()));
        let sent = |padded: bool| request(self.http).header(AUTHORIZATION, authorization(self.auth, padded)).send();
        let response = sent(self.padded).await.map_err(unreachable)?;
        if self.padded || !NOT_READ.contains(&response.status()) {
            return Ok(response);
        }
        self.padded = true;
        sent(true).await.map_err(unreachable)
    }
}

/// `bytes` as what a request sends, a piece at a time, with `sent` kept
/// at how much of it was handed over.
fn counted(bytes: &Arc<Vec<u8>>, sent: &Sent) -> reqwest::Body {
    let (bytes, sent) = (bytes.clone(), sent.clone());
    sent.store(0, Ordering::Relaxed);
    let pieces = futures_util::stream::iter((0..bytes.len()).step_by(PIECE)).map(move |start| {
        let end = (start + PIECE).min(bytes.len());
        sent.store(end, Ordering::Relaxed);
        Ok::<_, std::io::Error>(bytes.get(start..end).unwrap_or_default().to_vec())
    });
    reqwest::Body::wrap_stream(pieces)
}

/// Send the file `bytes`, whose hash is `hash` and which is a `mime`, to
/// `server` (BUD-02), after asking whether it would take it (BUD-06),
/// with `sent` kept at how much of it has gone. Returns the file as the
/// server has it.
pub async fn upload(
    http: &reqwest::Client,
    server: &Url,
    auth: &Event,
    bytes: &Arc<Vec<u8>>,
    hash: &str,
    mime: &str,
    sent: &Sent,
) -> Result<Blob, UploadError> {
    let address = endpoint(server, "upload");
    let mut asking = Asking { http, auth, padded: false };

    let asked = asking
        .send(|http| {
            http.request(Method::HEAD, &address)
                .header("X-SHA-256", hash)
                .header("X-Content-Length", bytes.len())
                .header("X-Content-Type", mime)
        })
        .await?;
    if !asked.status().is_success() && !NOT_ASKED_THAT.contains(&asked.status()) {
        return Err(refusal(asked).await);
    }

    let taken = asking
        .send(|http| {
            http.put(&address)
                .timeout(UPLOAD_TIMEOUT)
                .header(CONTENT_TYPE, mime)
                // Said, since what is sent a piece at a time doesn't say
                // how much there is of it, and servers ask.
                .header(CONTENT_LENGTH, bytes.len())
                .header("X-SHA-256", hash)
                .body(counted(bytes, sent))
        })
        .await?;
    if !taken.status().is_success() {
        return Err(refusal(taken).await);
    }
    let said = answer(taken).await;
    let descriptor: Descriptor = serde_json::from_slice(&said).map_err(|e| UploadError::Unreadable(e.to_string()))?;
    let url = Url::parse(&descriptor.url).ok().filter(|url| matches!(url.scheme(), "https" | "http"));
    let url = url.ok_or_else(|| UploadError::Unreadable(format!("no address: {}", descriptor.url)))?;
    // A server is to keep a file as it was sent. One that didn't says
    // which file it has instead.
    let is_hash = |hash: &String| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    let kept = descriptor.sha256.map(|hash| hash.to_ascii_lowercase()).filter(is_hash).unwrap_or_else(|| hash.to_string());
    Ok(Blob { url: url.to_string(), hash: kept })
}

/// Ask `server` to fetch the file at `url`, which the authorization `auth`
/// is for, and keep it as well (BUD-04).
pub async fn mirror(http: &reqwest::Client, server: &Url, auth: &Event, url: &str) -> Result<(), UploadError> {
    let address = endpoint(server, "mirror");
    let body = serde_json::json!({ "url": url }).to_string();
    let mut asking = Asking { http, auth, padded: false };
    let fetched = asking
        .send(|http| http.put(&address).timeout(MIRROR_TIMEOUT).header(CONTENT_TYPE, "application/json").body(body.clone()))
        .await?;
    if fetched.status().is_success() { Ok(()) } else { Err(refusal(fetched).await) }
}

/// A file that is on a media server, for a note to link to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Where it is: what goes in the note's text.
    pub url: String,
    /// What the note says of that address (NIP-92).
    pub imeta: Tag,
    /// The server that took it.
    pub server: Url,
    /// The servers that fetched it from there.
    pub mirrored: Vec<Url>,
}

/// What a note says of the link `url` (NIP-92): that it is to `media`,
/// whose hash there is `hash`. Its kind, its hash, its size, and for a
/// picture that was measured its width and height.
pub fn imeta(url: &str, media: &Media, hash: &str) -> Tag {
    let mut fields =
        vec![format!("url {url}"), format!("m {}", media.mime), format!("x {hash}"), format!("size {}", media.bytes.len())];
    fields.extend(media.dim.map(|(width, height)| format!("dim {width}x{height}")));
    Tag::custom(IMETA_TAG, fields)
}

/// The link that `tag` says something of, if it is an `imeta` tag.
pub fn imeta_url(tag: &Tag) -> Option<&str> {
    let fields = tag.as_slice();
    if fields.first()? != IMETA_TAG {
        return None;
    }
    fields.iter().skip(1).find_map(|field| field.strip_prefix("url ")).map(str::trim)
}

/// Send `media`, whose hash is `hash` and which `auth` authorizes sending,
/// to the first of `servers` that takes it, and have the others fetch it
/// from there; with `sent` kept at how much of it has gone. Returns where
/// it is, or why each server did not take it.
///
/// A server that refused is not asked to fetch what it refused; and a
/// server that did not fetch it is no reason for the file not to be
/// attached. If the server that took the file changed it, which it is
/// not to, it is a different file that it has, and no other is asked for
/// that one.
pub async fn send(
    http: &reqwest::Client,
    servers: &[Url],
    auth: &Event,
    media: Media,
    hash: &str,
    sent: &Sent,
) -> Result<Attachment, Vec<(Url, UploadError)>> {
    let bytes = Arc::new(media.bytes);
    let mut refused = Vec::new();
    let mut taken = None;
    for server in servers {
        match upload(http, server, auth, &bytes, hash, media.mime, sent).await {
            Ok(blob) => {
                taken = Some((server.clone(), blob));
                break;
            }
            Err(e) => {
                tracing::warn!("{server} did not take {}: {e:?}", media.name);
                refused.push((server.clone(), e));
            }
        }
    }
    let Some((server, blob)) = taken else {
        return Err(refused);
    };
    sent.store(bytes.len(), Ordering::Relaxed);

    let others: Vec<&Url> =
        servers.iter().filter(|other| **other != server && !refused.iter().any(|(tried, _)| tried == *other)).collect();
    let mut mirrored = Vec::new();
    if blob.hash == hash {
        let fetched = join_all(others.iter().map(|other| mirror(http, other, auth, &blob.url))).await;
        for (other, result) in others.into_iter().zip(fetched) {
            match result {
                Ok(()) => mirrored.push(other.clone()),
                Err(e) => tracing::warn!("{other} did not fetch {}: {e:?}", blob.url),
            }
        }
    } else {
        tracing::warn!("{server} keeps {} as another file than was sent: {} for {hash}", media.name, blob.hash);
    }
    let media = Media { bytes: Arc::unwrap_or_clone(bytes), ..media };
    Ok(Attachment { imeta: imeta(&blob.url, &media, &blob.hash), url: blob.url, server, mirrored })
}

#[cfg(test)]
mod tests {
    use super::{
        Attachment, Sent, UploadError, authorization, hash_of, imeta, imeta_url, send, server_list, server_name, servers,
        typed_server, upload_auth,
    };
    use crate::media::Media;
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, Tag, Timestamp, Url};
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    fn url(server: &str) -> Url {
        Url::parse(server).unwrap()
    }

    fn fields(event: &Event) -> Vec<Vec<&str>> {
        event.tags.iter().map(|tag| tag.as_slice().iter().map(String::as_str).collect()).collect()
    }

    #[test]
    fn a_server_is_typed_with_or_without_its_scheme_and_is_the_root_of_its_domain() {
        let server = url("https://cdn.example.com/");
        assert_eq!(typed_server("cdn.example.com"), Some(server.clone()));
        assert_eq!(typed_server("  https://cdn.example.com/  "), Some(server.clone()));
        assert_eq!(typed_server("https://CDN.example.com/upload?x=1"), Some(server.clone()), "its endpoints are at the root");
        assert_eq!(typed_server("cdn.example.com:8443"), Some(url("https://cdn.example.com:8443/")));
        assert_eq!(server_name(&server), "cdn.example.com");

        assert_eq!(typed_server("http://cdn.example.com"), None, "nothing is sent in the clear");
        assert_eq!(typed_server("wss://relay.example.com"), None);
        for typed in ["", "   ", "two words", "nodots", "https://"] {
            assert_eq!(typed_server(typed), None, "{typed:?} is no server");
        }
    }

    #[test]
    fn a_server_list_is_written_with_its_servers_in_order_and_the_rest_as_it_was() {
        let me = Keys::generate();
        let (first, second, third) =
            (url("https://one.example.com/"), url("https://two.example.com/"), url("https://three.example.com/"));
        let now = Timestamp::from_secs(2_000);
        assert!(servers(None).is_empty());

        // A first list.
        let list = server_list(None, std::slice::from_ref(&first), now).unwrap().finalize(&me).unwrap();
        assert_eq!(list.kind, Kind::BlossomServerList);
        assert_eq!(fields(&list), [vec!["server", "https://one.example.com"]]);
        assert_eq!(servers(Some(&list)), [first.clone()]);
        // An empty one is still a list to write, where there is none.
        assert!(server_list(None, &[], now).is_some());

        // One another client wrote, at a later time than now, with a
        // server twice, something that is no server, and a tag of its own.
        let theirs = [
            Tag::parse(["server", "https://one.example.com/"]).unwrap(),
            Tag::parse(["client", "another"]).unwrap(),
            Tag::parse(["server", "https://two.example.com"]).unwrap(),
            Tag::parse(["server", "https://one.example.com"]).unwrap(),
            Tag::parse(["server", "http://clear.example.com"]).unwrap(),
            Tag::parse(["server", "not an address"]).unwrap(),
        ];
        let earlier =
            EventBuilder::new(Kind::BlossomServerList, "kept").tags(theirs).custom_created_at(Timestamp::from_secs(5_000));
        let earlier = earlier.finalize(&me).unwrap();
        assert_eq!(servers(Some(&earlier)), [first.clone(), second.clone()]);
        assert!(server_list(Some(&earlier), &[first.clone(), second.clone()], now).is_none(), "it says so already");

        let changed = server_list(Some(&earlier), &[third, second, first], now).unwrap().finalize(&me).unwrap();
        assert_eq!(
            fields(&changed),
            [
                vec!["server", "https://three.example.com"],
                vec!["server", "https://two.example.com"],
                vec!["server", "https://one.example.com"],
                vec!["client", "another"],
            ]
        );
        assert_eq!(changed.content, "kept");
        assert_eq!(changed.created_at.as_secs(), 5_001, "it is the newer of the two");
    }

    #[test]
    fn an_upload_is_authorized_for_one_file_and_for_a_while() {
        let me = Keys::generate();
        let hash = hash_of(b"a file");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash_of(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

        let auth = upload_auth("holiday.jpg", &hash, Timestamp::from_secs(1_000)).finalize(&me).unwrap();
        assert_eq!(auth.kind, Kind::BlossomAuth);
        assert_eq!(auth.content, "Upload holiday.jpg");
        assert_eq!(auth.created_at.as_secs(), 1_000);
        assert_eq!(fields(&auth), [vec!["t", "upload"], vec!["x", hash.as_str()], vec!["expiration", "4600"]]);

        // In the header it is the event, as base64 of one kind or the other.
        let said = authorization(&auth, false);
        let token = said.strip_prefix("Nostr ").unwrap();
        assert!(!token.contains(['+', '/', '=']));
        assert_eq!(Event::from_json(URL_SAFE_NO_PAD.decode(token).unwrap()).unwrap(), auth);
        let said = authorization(&auth, true);
        assert_eq!(Event::from_json(STANDARD.decode(said.strip_prefix("Nostr ").unwrap()).unwrap()).unwrap(), auth);
    }

    /// A picture large enough to be sent in several pieces.
    fn picture() -> Media {
        let bytes = (0..200_000u32).map(|nth| (nth % 251) as u8).collect();
        Media { name: "holiday.png".into(), bytes, mime: "image/png", dim: Some((640, 480)) }
    }

    #[test]
    fn a_note_says_of_a_link_what_is_at_it() {
        let tag = imeta("https://cdn.example.com/abc.png", &picture(), "abc");
        assert_eq!(
            tag.as_slice(),
            ["imeta", "url https://cdn.example.com/abc.png", "m image/png", "x abc", "size 200000", "dim 640x480"]
        );
        assert_eq!(imeta_url(&tag), Some("https://cdn.example.com/abc.png"));

        let unmeasured = imeta("https://cdn.example.com/abc.mp4", &Media { mime: "video/mp4", dim: None, ..picture() }, "abc");
        assert_eq!(unmeasured.as_slice().len(), 5, "what was not measured is not said");
        assert_eq!(imeta_url(&Tag::parse(["t", "url https://cdn.example.com/abc.png"]).unwrap()), None);
        assert_eq!(imeta_url(&Tag::parse(["imeta", "m image/png"]).unwrap()), None);
    }

    /// What a media server was asked.
    #[derive(Debug, Clone)]
    struct Asked {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    /// How a media server of the tests behaves.
    #[derive(Debug, Clone, Copy, Default)]
    struct Manner {
        /// It takes no file, and says why with this status.
        refuses: Option<u16>,
        /// It doesn't know the question of whether it would take a file.
        asked_nothing: bool,
        /// It reads the authorization only as plain base64, padded, and
        /// not as the specification writes it.
        reads_the_old_way: bool,
        /// It keeps another file than it was sent.
        changes: bool,
    }

    /// A media server of the tests: where it is, and what it was asked.
    struct Server {
        url: Url,
        asked: Arc<Mutex<Vec<Asked>>>,
    }

    impl Server {
        fn asked(&self) -> Vec<Asked> {
            self.asked.lock().unwrap().clone()
        }

        /// What it was asked, each as its method and path.
        fn requests(&self) -> Vec<String> {
            self.asked().iter().map(|asked| format!("{} {}", asked.method, asked.path)).collect()
        }
    }

    /// Read one request from `stream`.
    async fn read_request(stream: &mut TcpStream) -> Option<Asked> {
        let mut buffer = Vec::new();
        let head_end = loop {
            if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break end;
            }
            let mut chunk = [0; 16_384];
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
        let mut lines = head.lines();
        let mut first = lines.next()?.split(' ');
        let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
            .collect();
        let length: usize = headers.get("content-length").and_then(|length| length.parse().ok()).unwrap_or(0);
        let mut body = buffer[head_end + 4..].to_vec();
        while body.len() < length {
            let mut chunk = [0; 16_384];
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
        }
        Some(Asked { method, path, headers, body })
    }

    /// What a server that behaves as `manner` answers `asked` with: a status, a reason for its header, and a body.
    fn answer_to(manner: Manner, base: &Url, asked: &Asked) -> (u16, Option<&'static str>, String) {
        let Some(token) = asked.headers.get("authorization").and_then(|said| said.strip_prefix("Nostr ")) else {
            return (401, Some("no authorization"), String::new());
        };
        if manner.reads_the_old_way && STANDARD.decode(token).is_err() {
            return (401, Some("the authorization could not be read"), String::new());
        }
        let descriptor =
            |hash: &str| format!(r#"{{"url":"{base}{hash}.png","sha256":"{hash}","size":1,"type":"image/png","uploaded":1}}"#);
        match (asked.method.as_str(), asked.path.as_str(), manner.refuses) {
            ("HEAD", "/upload", _) if manner.asked_nothing => (404, None, String::new()),
            ("HEAD", "/upload", Some(status)) => (status, Some("it is too large"), String::new()),
            ("HEAD", "/upload", None) => (200, None, String::new()),
            // Said in what it answers, and not in a header.
            ("PUT", "/upload", Some(status)) => (status, None, r#"{"message":"it is too large, said again"}"#.to_string()),
            ("PUT", "/upload", None) if asked.headers.contains_key("transfer-encoding") => {
                (411, Some("no length"), String::new())
            }
            ("PUT", "/upload", None) if manner.changes => (201, None, descriptor(&hash_of(b"another file"))),
            ("PUT", "/upload", None) => (201, None, descriptor(&hash_of(&asked.body))),
            ("PUT", "/mirror", Some(status)) => (status, Some("it is too large"), String::new()),
            ("PUT", "/mirror", None) => (200, None, descriptor("mirrored")),
            _ => (404, None, String::new()),
        }
    }

    /// Start a media server that behaves as `manner`.
    async fn serve(manner: Manner) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url(&format!("http://{}/", listener.local_addr().unwrap()));
        let asked: Arc<Mutex<Vec<Asked>>> = Arc::default();
        let (base, log) = (url.clone(), asked.clone());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let Some(request) = read_request(&mut stream).await else {
                    continue;
                };
                let (status, reason, body) = answer_to(manner, &base, &request);
                let body = if request.method == "HEAD" { String::new() } else { body };
                log.lock().unwrap().push(request);
                let reason = reason.map(|reason| format!("x-reason: {reason}\r\n")).unwrap_or_default();
                let head =
                    format!("HTTP/1.1 {status} Said\r\n{reason}content-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(format!("{head}{body}").as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Server { url, asked }
    }

    /// An HTTP client that speaks to servers that are not behind TLS, as
    /// those of the tests, which are on this machine, are not.
    fn plain_http() -> reqwest::Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::new()
    }

    /// Send `media` to servers that behave as `manners`, in that order.
    /// Returns what came of it, the servers, the authorization, and how
    /// much was counted as sent.
    fn sending(media: &Media, manners: &[Manner]) -> (Result<Attachment, Vec<(Url, UploadError)>>, Vec<Server>, Event, usize) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let mut started = Vec::new();
            for manner in manners {
                started.push(serve(*manner).await);
            }
            let hash = hash_of(&media.bytes);
            let keys = Keys::generate();
            let authorizing = |name: &str| upload_auth(name, &hash, Timestamp::now()).finalize(&keys).unwrap();
            // One that is not the same written both ways: base64 is only
            // padded where what it encodes is no multiple of three long.
            let auth = Some(authorizing(&media.name)).filter(|auth| auth.as_json().len() % 3 != 0);
            let auth = auth.unwrap_or_else(|| authorizing(&format!("{}.", media.name)));
            let addresses: Vec<Url> = started.iter().map(|server| server.url.clone()).collect();
            let http = plain_http();
            let sent = Sent::default();
            let sending = send(&http, &addresses, &auth, media.clone(), &hash, &sent);
            let result = tokio::time::timeout(Duration::from_secs(30), sending).await.unwrap();
            (result, started, auth, sent.load(Ordering::Relaxed))
        })
    }

    #[test]
    fn a_file_goes_to_the_first_server_and_the_others_fetch_it_from_there() {
        let media = picture();
        let hash = hash_of(&media.bytes);
        let (result, servers, auth, sent) = sending(&media, &[Manner::default(); 3]);

        let attachment = result.unwrap();
        let address = format!("{}{hash}.png", servers[0].url);
        assert_eq!(attachment.url, address);
        assert_eq!(attachment.server, servers[0].url);
        assert_eq!(attachment.mirrored, [servers[1].url.clone(), servers[2].url.clone()]);
        assert_eq!(attachment.imeta, imeta(&address, &media, &hash));
        assert_eq!(sent, media.bytes.len(), "all of it was counted as it went");

        // The first was asked whether it would take it, and then sent it.
        assert_eq!(servers[0].requests(), ["HEAD /upload", "PUT /upload"]);
        let asked = servers[0].asked();
        let said = |request: &super::tests::Asked, name: &str| request.headers.get(name).cloned().unwrap_or_default();
        assert_eq!(said(&asked[0], "x-sha-256"), hash);
        assert_eq!(said(&asked[0], "x-content-length"), "200000");
        assert_eq!(said(&asked[0], "x-content-type"), "image/png");
        assert!(asked[0].body.is_empty());
        assert_eq!(asked[1].body, media.bytes, "the file arrived as it was");
        assert_eq!(said(&asked[1], "content-length"), "200000");
        assert_eq!(said(&asked[1], "content-type"), "image/png");
        assert_eq!(said(&asked[1], "x-sha-256"), hash);
        assert!(!asked[1].headers.contains_key("transfer-encoding"));
        // Both with the authorization, as the specification writes it.
        for request in &asked {
            assert_eq!(said(request, "authorization"), authorization(&auth, false));
        }

        // The others were asked to fetch it, with the same authorization.
        for other in &servers[1..] {
            let asked = other.asked();
            assert_eq!(other.requests(), ["PUT /mirror"]);
            assert_eq!(String::from_utf8_lossy(&asked[0].body), format!(r#"{{"url":"{address}"}}"#));
            assert_eq!(said(&asked[0], "content-type"), "application/json");
            assert_eq!(said(&asked[0], "authorization"), authorization(&auth, false));
        }
    }

    #[test]
    fn a_server_that_would_not_take_a_file_is_not_sent_it_and_the_next_one_is() {
        let refusing = Manner { refuses: Some(413), ..Manner::default() };
        let (result, servers, _, _) = sending(&picture(), &[refusing, Manner::default(), Manner::default()]);

        let attachment = result.unwrap();
        assert_eq!(attachment.server, servers[1].url);
        assert_eq!(attachment.mirrored, [servers[2].url.clone()]);
        assert_eq!(servers[0].requests(), ["HEAD /upload"], "it said no to the question, and was asked no more");
        assert_eq!(servers[1].requests(), ["HEAD /upload", "PUT /upload"]);
        assert_eq!(servers[2].requests(), ["PUT /mirror"]);
    }

    #[test]
    fn a_server_that_does_not_know_the_question_is_sent_the_file() {
        let unasked = Manner { asked_nothing: true, ..Manner::default() };
        let (result, servers, _, _) = sending(&picture(), &[unasked]);
        assert_eq!(result.unwrap().server, servers[0].url);
        assert_eq!(servers[0].requests(), ["HEAD /upload", "PUT /upload"]);

        // And says why it did not take it in what it answers.
        let (result, servers, _, _) = sending(&picture(), &[Manner { refuses: Some(402), ..unasked }]);
        let why = UploadError::Refused { status: 402, reason: "it is too large, said again".into() };
        assert_eq!(result, Err(vec![(servers[0].url.clone(), why)]));
    }

    #[test]
    fn a_server_that_reads_the_authorization_the_old_way_is_asked_again_that_way() {
        let old = Manner { reads_the_old_way: true, ..Manner::default() };
        let (result, servers, auth, _) = sending(&picture(), &[old, old]);

        let attachment = result.unwrap();
        assert_eq!(attachment.mirrored, [servers[1].url.clone()]);
        // The question twice; then the file, the way that was read.
        assert_eq!(servers[0].requests(), ["HEAD /upload", "HEAD /upload", "PUT /upload"]);
        let written: Vec<String> = servers[0].asked().iter().map(|asked| asked.headers["authorization"].clone()).collect();
        assert_eq!(written, [authorization(&auth, false), authorization(&auth, true), authorization(&auth, true)]);
        assert_eq!(servers[1].requests(), ["PUT /mirror", "PUT /mirror"]);
        assert_eq!(servers[1].asked()[1].headers["authorization"], authorization(&auth, true));
    }

    #[test]
    fn when_no_server_takes_a_file_each_says_why() {
        let refusing = Manner { refuses: Some(413), ..Manner::default() };
        // A server that is not there: nothing listens where it was.
        let gone = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            url(&format!("http://{}/", listener.local_addr().unwrap()))
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (result, there) = runtime.block_on(async {
            let there = serve(refusing).await;
            let media = picture();
            let hash = hash_of(&media.bytes);
            let auth = upload_auth(&media.name, &hash, Timestamp::now()).finalize(&Keys::generate()).unwrap();
            let servers = [gone.clone(), there.url.clone()];
            (send(&plain_http(), &servers, &auth, media, &hash, &Sent::default()).await, there)
        });

        let refused = result.unwrap_err();
        assert_eq!(refused.len(), 2);
        assert!(matches!(&refused[0], (server, UploadError::Unreachable(_)) if *server == gone), "{refused:?}");
        assert_eq!(refused[1], (there.url.clone(), UploadError::Refused { status: 413, reason: "it is too large".into() }));
    }

    #[test]
    fn a_file_the_server_changed_is_another_file_and_no_other_server_is_asked_for_it() {
        let changing = Manner { changes: true, ..Manner::default() };
        let media = picture();
        let (result, servers, _, _) = sending(&media, &[changing, Manner::default()]);

        let attachment = result.unwrap();
        let kept = hash_of(b"another file");
        assert_eq!(attachment.imeta, imeta(&attachment.url, &media, &kept), "the note says which file is there");
        assert!(attachment.mirrored.is_empty());
        assert!(servers[1].requests().is_empty());
    }
}

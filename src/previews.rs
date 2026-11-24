// SPDX-License-Identifier: MIT

//! What a web page says of itself, for showing under a note that links to
//! it: its title, a few words about it, and a picture.
//!
//! Pages say this in the `meta` tags of their head, most of them as Open
//! Graph (`og:title`, `og:description`, `og:image`), which is what other
//! clients and every chat program read. Only the head is fetched and read,
//! with a tokenizer: nothing of the page is run, laid out or kept.
//!
//! A link that answers as a video, where a page was asked for, is one the
//! address did not give away: the answer is [`Linked::Video`], and the
//! row offers to play it (see [`crate::video`]).
//!
//! A YouTube video is asked what it is called the short way (see
//! [`Video::about`](crate::youtube::Video::about)): its page says so too,
//! most of a megabyte in.
//!
//! A view asks the [`PreviewStore`] for the preview of an address, as it
//! asks for pictures (see [`Requests`]): the app hears of it as a
//! [`Message::LoadPreview`](crate::messages::Message::LoadPreview). A page
//! is asked once in a run, whatever it answers. There is no disk cache.
//!
//! Addresses come from whoever wrote a note. They are fetched as everything
//! is (see [`crate::http`]): the page's server sees the request, as the
//! server of a note's picture does.

use std::collections::HashMap;

use html5gum::{DefaultEmitter, Token, Tokenizer};
use reqwest::Url;
use tokio::sync::mpsc;

use crate::http;
use crate::requests::Requests;
use crate::youtube;

/// How much of a page is read looking for the end of its head. Most heads
/// end within a few thousand bytes; some sites put a megabyte of script
/// before what they say of themselves.
const MAX_HEAD_BYTES: usize = 2 * 1024 * 1024;

/// How much of what YouTube answers about a video is read: it is a few
/// hundred bytes.
const MAX_ABOUT_BYTES: usize = 64 * 1024;

/// How many pages may be on their way at once, and how many requests may
/// wait between a view asking and the app answering.
const MAX_IN_FLIGHT: usize = 4;
const REQUEST_QUEUE: usize = 32;

/// How many characters of a title and of a description are kept: more than
/// a preview has room to show.
const TITLE_MAX_CHARS: usize = 200;
const DESCRIPTION_MAX_CHARS: usize = 300;

/// What a link a note took for a page turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Linked {
    /// A page, and what it says of itself.
    Page(LinkPreview),
    /// A video: that is what its server calls it.
    Video,
}

/// What a web page says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    /// What it is called.
    pub title: String,
    /// A few words about it, if it has any.
    pub description: Option<String>,
    /// The address of the picture it is shared with, if it has one that can
    /// be fetched.
    pub image: Option<String>,
    /// The site it is on: its host, without a leading `www.`.
    pub site: String,
}

/// `text` as one line: runs of blank space and line breaks made one space,
/// none at either end, and `max_chars` characters at most. `None` if that
/// leaves nothing, or if the text was not read as it was written: a page
/// that is not in UTF-8 says nothing rather than something garbled.
fn one_line(text: &str, max_chars: usize) -> Option<String> {
    if text.contains(char::REPLACEMENT_CHARACTER) {
        return None;
    }
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = if line.chars().count() > max_chars {
        let head: String = line.chars().take(max_chars).collect();
        format!("{}\u{2026}", head.trim_end())
    } else {
        line
    };
    (!line.is_empty()).then_some(line)
}

/// What the head of a page says of the page, as it was found: each thing
/// said the way most trusted to be meant for sharing. A lower `rank` is
/// more trusted, and among equals the first one said stands.
#[derive(Default)]
struct Said {
    title: Option<(u8, String)>,
    description: Option<(u8, String)>,
    image: Option<(u8, String)>,
}

impl Said {
    /// Keep `value` in `slot` if nothing as trusted is there.
    fn keep(slot: &mut Option<(u8, String)>, rank: u8, value: String) {
        if slot.as_ref().is_none_or(|(kept, _)| rank < *kept) {
            *slot = Some((rank, value));
        }
    }

    /// Take in a `meta` tag that names `key` and says `content`.
    fn meta(&mut self, key: &str, content: String) {
        match key {
            "og:title" => Self::keep(&mut self.title, 0, content),
            "twitter:title" => Self::keep(&mut self.title, 1, content),
            "og:description" => Self::keep(&mut self.description, 0, content),
            "twitter:description" => Self::keep(&mut self.description, 1, content),
            "description" => Self::keep(&mut self.description, 2, content),
            "og:image:secure_url" => Self::keep(&mut self.image, 0, content),
            "og:image" | "og:image:url" => Self::keep(&mut self.image, 1, content),
            "twitter:image" | "twitter:image:src" => Self::keep(&mut self.image, 2, content),
            _ => {}
        }
    }
}

/// What the page at `url` says of itself in `html`, which is its head, or
/// starts with it. `None` if it gives itself no title: there is nothing to
/// call it by.
pub fn parse(html: &str, url: &Url) -> Option<LinkPreview> {
    let mut said = Said::default();
    // The text between `<title>` and `</title>`, while between them.
    let mut title: Option<String> = None;

    // Told to treat what is in a `title` or a `script` as text, which is
    // what a browser does: a tag written inside one is not a tag.
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for Ok(token) in Tokenizer::new_with_emitter(html, emitter) {
        match token {
            Token::StartTag(tag) => match tag.name.as_slice() {
                b"meta" => {
                    let value =
                        |name: &str| tag.attributes.get(name.as_bytes()).map(|value| String::from_utf8_lossy(value).into_owned());
                    // Open Graph says `property`, the rest say `name`, and
                    // pages mix the two up.
                    if let (Some(key), Some(content)) = (value("property").or_else(|| value("name")), value("content")) {
                        said.meta(key.trim().to_ascii_lowercase().as_str(), content);
                    }
                }
                b"title" => title = Some(String::new()),
                b"body" => break,
                _ => {}
            },
            Token::String(text) => {
                if let Some(title) = &mut title {
                    title.push_str(&String::from_utf8_lossy(&text));
                }
            }
            Token::EndTag(tag) => match tag.name.as_slice() {
                b"title" => {
                    if let Some(title) = title.take() {
                        Said::keep(&mut said.title, 2, title);
                    }
                }
                b"head" => break,
                _ => {}
            },
            _ => {}
        }
    }

    let host = url.host_str().unwrap_or_default();
    Some(LinkPreview {
        title: said.title.and_then(|(_, title)| one_line(&title, TITLE_MAX_CHARS))?,
        description: said.description.and_then(|(_, description)| one_line(&description, DESCRIPTION_MAX_CHARS)),
        // Relative to the page, as pages write it; and fetched only if it
        // is where anything is fetched from.
        image: said
            .image
            .and_then(|(_, image)| url.join(image.trim()).ok())
            .filter(|image| image.scheme() == "https")
            .map(String::from),
        site: host.strip_prefix("www.").unwrap_or(host).to_string(),
    })
}

/// Fetch the head of the page at `url` and read what the page says of
/// itself; or hear that it is a video, and fetch none of it.
pub async fn fetch(http: reqwest::Client, url: String) -> Result<Linked, String> {
    if let Some(video) = youtube::video_at(&url) {
        let answer = http::get(&http, &video.about(), MAX_ABOUT_BYTES).await?;
        let title = youtube::title(&answer).and_then(|title| one_line(&title, TITLE_MAX_CHARS));
        let title = title.ok_or_else(|| "YouTube does not say what it is called".to_string())?;
        return Ok(Linked::Page(LinkPreview { title, description: None, image: None, site: "youtube.com".to_string() }));
    }
    match http::get_page_head(&http, &url, MAX_HEAD_BYTES).await? {
        http::Linked::Video => Ok(Linked::Video),
        http::Linked::Page(head) => parse(&String::from_utf8_lossy(&head.bytes), &head.url)
            .map(Linked::Page)
            .ok_or_else(|| "it gives itself no title".to_string()),
    }
}

/// What the store has to say about the preview of a page.
#[derive(Debug, PartialEq, Eq)]
pub enum PreviewState<'a> {
    Ready(&'a LinkPreview),
    /// Not a page at all, but a video.
    Video,
    /// Asked for, or about to be.
    Loading,
    /// The page could not be fetched or says nothing of itself, and won't
    /// be asked again this run.
    Nothing,
}

/// What the pages asked about so far say of themselves, by address. See the
/// [module docs](self).
pub struct PreviewStore {
    /// The links that answered, and (`None`) those that had nothing to say.
    known: HashMap<String, Option<Linked>>,
    /// The asking for those not known.
    requests: Requests<String>,
}

impl PreviewStore {
    /// An empty store, and the receiving end of its requests.
    pub fn new() -> (Self, mpsc::Receiver<String>) {
        let (requests, receiver) = Requests::new(REQUEST_QUEUE, MAX_IN_FLIGHT);
        (Self { known: HashMap::new(), requests }, receiver)
    }

    /// What the page at `url` says of itself, if that is known; otherwise
    /// ask for it.
    pub fn get(&self, url: &str) -> PreviewState<'_> {
        match self.known.get(url) {
            Some(Some(Linked::Page(preview))) => PreviewState::Ready(preview),
            Some(Some(Linked::Video)) => PreviewState::Video,
            Some(None) => PreviewState::Nothing,
            None => {
                self.requests.ask(&url.to_string());
                PreviewState::Loading
            }
        }
    }

    /// A request reached the app. Returns whether to start fetching the
    /// page: not if it is known or on its way already, and not while as
    /// many as may be are.
    pub fn begin(&mut self, url: &String) -> bool {
        let wanted = !self.known.contains_key(url);
        self.requests.begin(url, wanted)
    }

    /// The fetch of `url` ended: with what was found there, or with
    /// nothing to show for it.
    pub fn finish(&mut self, url: String, found: Option<Linked>) {
        self.requests.end(&url);
        self.known.insert(url, found);
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkPreview, Linked, PreviewState, PreviewStore, parse};
    use reqwest::Url;

    fn read(html: &str) -> Option<LinkPreview> {
        parse(html, &Url::parse("https://www.example.com/a/page?x=1").unwrap())
    }

    #[test]
    fn a_page_is_shown_as_it_says_it_wants_to_be_shared() {
        let preview = read(
            r#"<!doctype html><html><head>
            <meta charset="utf-8">
            <title>Lastbook</title>
            <meta name="description" content="For search engines.">
            <meta property="og:title" content="Lastbook &mdash; Everything you need to survive &amp; rebuild society.">
            <meta property="og:description" content="Free, open-source survival library.
                Works offline.">
            <meta property="og:image" content="https://cdn.example.com/card.png">
            </head><body><h1>Not this</h1></body></html>"#,
        )
        .unwrap();

        assert_eq!(
            preview,
            LinkPreview {
                title: "Lastbook — Everything you need to survive & rebuild society.".into(),
                description: Some("Free, open-source survival library. Works offline.".into()),
                image: Some("https://cdn.example.com/card.png".into()),
                site: "example.com".into(),
            }
        );
    }

    #[test]
    fn what_is_meant_for_sharing_is_taken_before_what_is_not_wherever_it_stands() {
        let preview = read(
            r#"<head>
            <meta name="twitter:title" content="For Twitter">
            <meta name="twitter:image" content="/twitter.png">
            <meta property="og:title" content="For sharing">
            <meta property="og:title" content="For sharing, again">
            <title>For the tab</title>
            <meta name="twitter:description" content="About it, for Twitter">
            </head>"#,
        )
        .unwrap();

        assert_eq!(preview.title, "For sharing", "Open Graph first, and the first of it");
        assert_eq!(preview.description.as_deref(), Some("About it, for Twitter"));
        assert_eq!(preview.image.as_deref(), Some("https://www.example.com/twitter.png"), "relative to the page");
    }

    #[test]
    fn a_page_with_only_a_title_is_called_by_it() {
        let preview = read("<html><head><title>\n  Just a\n  title </title></head></html>").unwrap();
        assert_eq!(
            preview,
            LinkPreview { title: "Just a title".into(), description: None, image: None, site: "example.com".into() }
        );

        // Pages name what they say with `name` where `property` belongs,
        // and in whatever case.
        assert_eq!(read(r#"<meta name="OG:Title" content="Mixed up">"#).unwrap().title, "Mixed up");
    }

    #[test]
    fn a_page_that_gives_itself_no_title_has_no_preview() {
        assert_eq!(read(""), None);
        assert_eq!(read("<html><head><title>  </title></head></html>"), None);
        assert_eq!(read(r#"<head><meta property="og:description" content="About nothing"></head>"#), None);
        assert_eq!(read("plain text, not a page at all"), None);
    }

    #[test]
    fn only_the_head_is_read_and_what_is_written_in_a_script_is_not_a_tag() {
        let preview = read(
            r#"<head>
            <script>var page = '<title>From a script</title><meta property="og:title" content="From a script">';</script>
            <title>The page</title>
            </head>
            <body><svg><title>A drawing</title></svg><meta property="og:title" content="From the body"></body>"#,
        )
        .unwrap();
        assert_eq!(preview.title, "The page");
    }

    #[test]
    fn a_picture_that_would_not_be_fetched_is_left_out() {
        let picture =
            |address: &str| read(&format!(r#"<title>t</title><meta property="og:image" content="{address}">"#)).unwrap().image;
        assert_eq!(picture("http://example.com/card.png"), None, "only https is fetched");
        assert_eq!(picture("data:image/png;base64,AAAA"), None);
        assert_eq!(picture("//cdn.example.com/card.png").as_deref(), Some("https://cdn.example.com/card.png"));
        assert_eq!(picture("card.png").as_deref(), Some("https://www.example.com/a/card.png"));
    }

    #[test]
    fn long_texts_are_cut_and_garbled_ones_dropped() {
        let long = "word ".repeat(200);
        let preview = read(&format!(r#"<title>{long}</title><meta name="description" content="{long}">"#)).unwrap();
        assert_eq!(preview.title.chars().count(), 200);
        assert!(preview.title.ends_with("word…"), "{}", preview.title);
        assert!(preview.description.unwrap().chars().count() <= 301);

        // Read as UTF-8 from a page written as something else.
        let garbled = String::from_utf8_lossy(b"<title>caf\xe9</title><meta name=description content='fine'>").into_owned();
        assert_eq!(read(&garbled), None, "no title that can be read");
    }

    #[test]
    fn a_page_is_asked_about_once_whatever_it_answers() {
        let (mut store, mut requests) = PreviewStore::new();
        let (answers, silent) = ("https://example.com/answers".to_string(), "https://example.com/silent".to_string());

        for _ in 0..3 {
            assert_eq!(store.get(&answers), PreviewState::Loading);
        }
        assert_eq!(requests.try_recv().ok().as_ref(), Some(&answers));
        assert!(requests.try_recv().is_err(), "asked once");
        assert!(store.begin(&answers));
        assert!(!store.begin(&answers), "on its way");

        let preview = LinkPreview { title: "Answers".into(), description: None, image: None, site: "example.com".into() };
        store.finish(answers.clone(), Some(Linked::Page(preview.clone())));
        assert_eq!(store.get(&answers), PreviewState::Ready(&preview));

        // A link that turned out to be a video is known as one.
        let video = "https://example.com/video".to_string();
        assert!(store.begin(&video));
        store.finish(video.clone(), Some(Linked::Video));
        assert_eq!(store.get(&video), PreviewState::Video);

        assert_eq!(store.get(&silent), PreviewState::Loading);
        assert!(store.begin(&silent));
        store.finish(silent.clone(), None);
        assert_eq!(store.get(&silent), PreviewState::Nothing);
        assert!(!store.begin(&silent), "not asked again");
        let _ = requests.try_recv();
        assert!(requests.try_recv().is_err());
    }
}

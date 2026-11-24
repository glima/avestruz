// SPDX-License-Identifier: MIT

//! What a note says, taken apart for showing: plain text, links, mentions of
//! people, hashtags, the images and the videos it carries, the notes it
//! quotes, and the web page it is shown a preview of.
//!
//! Images, videos and quoted notes are set aside from the text: what stands
//! for them in it is an address, and they are shown under it for what they
//! are.
//!
//! What a link is is told by how its address ends, and by what the note
//! says of it in a NIP-92 `imeta` tag (see [`Attachment`]): a link the note
//! calls a video is one, whatever its address looks like. A link to a
//! YouTube video is a video too (see [`crate::youtube`]), though what is
//! at its address is a page.
//!
//! The text is split by `nostr`'s own parser, so what counts as a URL, a
//! `nostr:` URI or a hashtag is what the rest of the ecosystem agrees on.
//!
//! Before that, what the note marks as Markdown is taken for what it is
//! (see [`markdown`]): the text comes here as runs, each in one style,
//! and each run is read for its links, mentions and hashtags. A note that
//! marks nothing is one run, and is read as it always was.

use nostr_sdk::prelude::{EventId, Nip21, NostrParser, PublicKey, RelayUrl, ToBech32, Token, Url};

use crate::http;
use crate::nostr::markdown::{self, Block, Run, Style};
use crate::nostr::outbox::Whereabouts;
use crate::youtube;

/// How much of a note a feed row shows, in characters, until it is asked
/// for the rest. A long note is cut here, and what it says in full is
/// kept beside what is shown of it (see [`Content::whole`]).
pub const FEED_NOTE_MAX_CHARS: usize = 600;

/// How long a link's label may be before it is cut, as in Vega.
const LINK_LABEL_MAX_CHARS: usize = 50;

/// How many characters of a `note1…`/`naddr1…` stand for the whole of it.
const REFERENCE_CHARS: usize = 12;

/// How many of a note's images a feed row shows.
const MAX_NOTE_IMAGES: usize = 4;

/// How many of a note's videos a feed row gives room to.
const MAX_NOTE_VIDEOS: usize = 4;

/// How many of the notes a note quotes are shown under it.
pub const MAX_NOTE_QUOTES: usize = 3;

/// How many notes deep quoting is shown: a note in a note in the note of a
/// row, and no further. A note quoted deeper than that shows as a link:
/// quoting has no end of its own.
pub const QUOTE_DEPTH: usize = 2;

/// The file extensions of the pictures that can be decoded and shown.
const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// The file extensions of the videos that are played in their row: files
/// of pictures and sound, whatever is in them. Whether this machine can
/// decode one is found out when it is played.
const VIDEO_EXTENSIONS: [&str; 7] = ["mp4", "webm", "mov", "mkv", "m4v", "ogv", "avi"];

/// The file extensions of what is plainly not a web page: nothing with a
/// title to preview it by, and not worth asking.
const FILE_EXTENSIONS: [&str; 16] =
    ["svg", "avif", "heic", "tiff", "m3u8", "mp3", "m4a", "ogg", "opus", "wav", "flac", "pdf", "zip", "gz", "json", "txt"];

/// How wide a mention is counted as, when counting what a note shows: about
/// what a name takes, whoever it turns out to be.
const MENTION_CHARS: usize = 16;

/// How many characters of text four of code take the room of: its
/// letters are all as wide as the widest.
const CODE_CHARS_PER_FOUR: usize = 5;

/// One piece of a note, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Plain text, line breaks included.
    Text(String),
    /// Text the note marks as something else than plain: bold, slanted,
    /// struck out, code, or quoted (see [`markdown`]). Without its marks.
    Styled { text: String, style: Style },
    /// A web link: where it goes, and the shorter text that stands for it.
    Link { url: String, label: String },
    /// Someone mentioned by key (`nostr:npub1…`, `nostr:nprofile1…`). Shown
    /// by name once their profile is known.
    Mention(PublicKey),
    /// A hashtag, without its `#`.
    Hashtag(String),
    /// An article or another addressable event (`naddr1…`), as the short
    /// form of its address. Not something this client opens yet.
    Reference(String),
    /// What stands apart from the text, between what is before it and
    /// what is after: lines of code, a rule, a table, whose cells are
    /// pieces like these (see [`markdown`]).
    Block(Block<Vec<Segment>>),
}

impl Segment {
    /// What this piece says as plain words, `named` saying who a mention
    /// is of. What stands apart has a space on either side.
    pub fn words(&self, named: &dyn Fn(&PublicKey) -> String) -> String {
        match self {
            Self::Text(text) | Self::Styled { text, .. } => text.clone(),
            Self::Link { label, .. } | Self::Reference(label) => label.clone(),
            Self::Mention(person) => named(person),
            Self::Hashtag(tag) => format!("#{tag}"),
            Self::Block(Block::Code(code)) => format!(" {code} "),
            Self::Block(Block::Rule) => " ".to_string(),
            Self::Block(Block::Table(table)) => {
                let cells =
                    table.rows.iter().flatten().map(|cell| cell.iter().map(|piece| piece.words(named)).collect::<String>());
                format!(" {} ", cells.collect::<Vec<_>>().join(" "))
            }
        }
    }
}

/// Every piece of `segments` that stands in a line of text: those in the
/// cells of its tables too, each where it stands.
pub fn inline(segments: &[Segment]) -> impl Iterator<Item = &Segment> {
    segments.iter().flat_map(|segment| -> Box<dyn Iterator<Item = &Segment>> {
        match segment {
            Segment::Block(Block::Table(table)) => Box::new(table.rows.iter().flatten().flatten()),
            other => Box::new(std::iter::once(other)),
        }
    })
}

/// Another note a note quotes: one it names by its address in the text
/// (`nostr:note1…`, `nevent1…`). Shown under the text, as the note it is
/// when that is at hand, and as a link to it when it isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    /// Which note.
    pub id: EventId,
    /// Who wrote it and a relay that has it, when the address says: what
    /// finds a note that is not at hand.
    pub author: Option<PublicKey>,
    pub relay: Option<RelayUrl>,
    /// The short form of its address, which stands for it where the note
    /// itself can't.
    pub label: String,
}

impl Quote {
    /// Where the note is, as far as its address says, for looking where
    /// the account's own relays don't have it.
    pub fn whereabouts(&self) -> Whereabouts {
        Whereabouts { author: self.author, relay: self.relay.clone() }
    }
}

/// An image a note carries: a link to a picture, shown under the text
/// rather than in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteImage {
    pub url: String,
    /// The short text that stands for the address, if the picture can't be
    /// shown.
    pub label: String,
    /// Its size in pixels, when the note says (NIP-92 `imeta`). Lets the row
    /// keep room for it before it has loaded.
    pub size: Option<(u32, u32)>,
}

/// A video a note carries: a link to one, played under the text rather
/// than followed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteVideo {
    pub url: String,
    /// The short text that stands for the address, beside its play button.
    pub label: String,
    /// Its size in pixels, when the note says: the proportions of the room
    /// it is given.
    pub size: Option<(u32, u32)>,
    /// The pictures that may stand for it until it plays, the likeliest
    /// first: the one the note names, or those YouTube keeps. The first
    /// that can be had does.
    pub posters: Vec<String>,
    /// It is a page that shows a video (YouTube's), not a file: what it
    /// is called is the page's to say, and what there is to play of it
    /// has to be found.
    pub page: bool,
}

/// What a note says of something it links to, in a NIP-92 `imeta` tag:
/// `["imeta", "url https://…", "m video/mp4", "dim 1920x1080", …]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachment {
    /// The address it is said of, as it stands in the note.
    pub url: String,
    /// What kind of thing is there, as a media type (`m`).
    pub kind: Option<String>,
    /// Its size in pixels (`dim`).
    pub size: Option<(u32, u32)>,
    /// A picture that stands for it (`image`, or else `thumb`), if it is
    /// one that would be fetched.
    pub picture: Option<String>,
}

/// The text of a note that was cut, all of it: what its row shows once
/// it is asked for the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Whole {
    pub segments: Vec<Segment>,
    /// As [`Content::line_chars`], of all the text.
    pub line_chars: Vec<usize>,
}

/// A note's content, parsed, and cut to what a feed row shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Content {
    /// The text, cut if it is long.
    pub segments: Vec<Segment>,
    /// All of the text, if `segments` is cut. What is set aside from the
    /// text is the same either way: none of it is cut.
    pub whole: Option<Whole>,
    /// The images, in the order they were linked. Their addresses are not in
    /// `segments`.
    pub images: Vec<NoteImage>,
    /// The videos, likewise.
    pub videos: Vec<NoteVideo>,
    /// The notes it quotes, in the order they were named, each once. Their
    /// addresses are not in `segments` either.
    pub quotes: Vec<Quote>,
    /// How many characters each line of the shown text has, links and
    /// mentions counted as shown. What a row's height is estimated from
    /// before it has been laid out.
    pub line_chars: Vec<usize>,
    /// The web page shown a preview of under the note: the last one it
    /// links to, as in Jumble. Its link stays where it is in the text.
    pub preview: Option<String>,
    /// The first address it links to that is not a picture's: a video's,
    /// or anything else on the web. What the key that plays a note's
    /// video is for (see [`crate::handlers::keys`]): a picture is shown
    /// where it is, and has no use for a key.
    pub first_link: Option<String>,
}

/// The text that stands for a link: its host and path, without the scheme,
/// the query or a leading `www.`, cut if it is still long.
pub(crate) fn link_label(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    let path = if url.path() == "/" { "" } else { url.path() };
    let label = format!("{host}{path}");
    match label.char_indices().nth(LINK_LABEL_MAX_CHARS) {
        Some(_) => {
            let head: String = label.chars().take(LINK_LABEL_MAX_CHARS - 1).collect();
            format!("{head}\u{2026}")
        }
        None => label,
    }
}

/// What stands for a link the note gives a `label` of its own: the
/// label, and after it where the link goes, unless the label says so
/// itself. A label can say anything, and a click on it opens the address:
/// which site that is on is not left to the label. With no label, the
/// address made short, as for any link.
fn labelled(url: &Url, label: Option<&str>) -> String {
    let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) else {
        return link_label(url);
    };
    let host = url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    if label.to_lowercase().contains(&host.to_lowercase()) { label.to_string() } else { format!("{label} ({host})") }
}

/// Whether the path of `url` ends in one of `extensions`.
fn has_extension(url: &Url, extensions: &[&str]) -> bool {
    url.path().rsplit_once('.').is_some_and(|(_, extension)| extensions.iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

/// Whether `url` is a picture this client can fetch and show: an `https`
/// address (nothing else is fetched) whose path ends like a picture's.
fn is_image(url: &Url) -> bool {
    url.scheme() == "https" && has_extension(url, &IMAGE_EXTENSIONS)
}

/// Whether `url` is a video this client can fetch and play: an `https`
/// address (nothing else is fetched) whose path ends like a video's, that
/// the note calls one (`said`), or that is a YouTube video's.
fn is_video(url: &Url, said: Option<&Attachment>) -> bool {
    let called_one = said.and_then(|said| said.kind.as_deref()).is_some_and(http::is_video);
    url.scheme() == "https" && (has_extension(url, &VIDEO_EXTENSIONS) || called_one || youtube::video(url).is_some())
}

/// Whether `url` may be a web page that can be asked what it says of
/// itself: an `https` address (nothing else is fetched) whose path doesn't
/// end like a file's.
fn may_be_page(url: &Url) -> bool {
    url.scheme() == "https"
        && ![&IMAGE_EXTENSIONS[..], &VIDEO_EXTENSIONS, &FILE_EXTENSIONS].iter().any(|extensions| has_extension(url, extensions))
}

/// The first characters of a bech32 address, standing for all of it.
fn short_reference(bech32: &str) -> String {
    let head: String = bech32.chars().take(REFERENCE_CHARS).collect();
    format!("{head}\u{2026}")
}

/// The note `id` as one quoted, written by `author` and to be found at
/// `relay`, as far as its address says.
fn quote(id: EventId, author: Option<PublicKey>, relay: Option<RelayUrl>) -> Quote {
    // The note's address, as everywhere else: not the start of the longer
    // `nevent1…`, which is the same note with hints.
    let Ok(note) = id.to_bech32();
    Quote { id, author, relay, label: short_reference(&note) }
}

/// What a `nostr:` URI stands for in a note.
enum Named {
    /// Something that shows in the text, where it was written.
    Inline(Segment),
    /// Another note, which shows under the text.
    Quote(Quote),
}

/// What the `nostr:` URI `uri` stands for.
fn named(uri: Nip21) -> Named {
    match uri {
        Nip21::Pubkey(public_key) => Named::Inline(Segment::Mention(public_key)),
        Nip21::Profile(profile) => Named::Inline(Segment::Mention(profile.public_key)),
        Nip21::EventId(id) => Named::Quote(quote(id, None, None)),
        Nip21::Event(event) => Named::Quote(quote(event.event_id, event.author, event.relays.into_iter().next())),
        Nip21::Coordinate(coordinate) => Named::Inline(Segment::Reference(
            coordinate.to_bech32().map_or_else(|_| "naddr1\u{2026}".to_string(), |naddr| short_reference(&naddr)),
        )),
    }
}

/// Builds a [`Content`], counting what it shows and stopping at a limit.
struct Builder<'a> {
    content: Content,
    /// What the note says of what it links to.
    attached: &'a [Attachment],
    /// Characters still allowed.
    room: usize,
    /// The limit was reached: nothing more is added.
    full: bool,
    /// Something was just set aside. The blank space after it closes the
    /// gap it left, rather than adding to the blank space before it.
    gap: bool,
}

impl<'a> Builder<'a> {
    fn new(max_chars: usize, attached: &'a [Attachment]) -> Self {
        Self {
            content: Content { line_chars: vec![0], ..Content::default() },
            attached,
            room: max_chars,
            full: false,
            gap: false,
        }
    }

    /// Whether `space`, the blank space or the line break that follows
    /// something set aside, would only repeat what stands before it: the
    /// text so far ends the same way, or there is none.
    fn repeats(&self, space: char) -> bool {
        match self.content.segments.last() {
            Some(Segment::Text(before) | Segment::Styled { text: before, .. }) => before.ends_with(space),
            Some(Segment::Block(_)) | None => true,
            Some(_) => false,
        }
    }

    /// What the note says of `url`, if it says anything.
    fn said_of(&self, url: &Url) -> Option<&'a Attachment> {
        self.attached.iter().find(|said| said.url == url.as_str())
    }

    /// Set an image aside. Images take none of the room text does, so a
    /// picture at the end of a long note is still shown.
    fn image(&mut self, url: &Url) {
        if self.content.images.len() < MAX_NOTE_IMAGES {
            let size = self.said_of(url).and_then(|said| said.size);
            self.content.images.push(NoteImage { url: url.to_string(), label: link_label(url), size });
        }
    }

    /// Set a video aside, as an image is.
    fn video(&mut self, url: &Url) {
        if self.content.videos.len() >= MAX_NOTE_VIDEOS {
            return;
        }
        let (said, hosted) = (self.said_of(url), youtube::video(url));
        let (url, label) = (url.to_string(), link_label(url));
        self.content.videos.push(match hosted {
            // What YouTube shows is YouTube's to describe.
            Some(hosted) => NoteVideo { url, label, size: hosted.size, posters: hosted.posters(), page: true },
            None => NoteVideo {
                url,
                label,
                size: said.and_then(|said| said.size),
                posters: said.and_then(|said| said.picture.clone()).into_iter().collect(),
                page: false,
            },
        });
    }

    /// Set a quoted note aside, unless it is already. Like images, quotes
    /// take none of the room text does.
    fn quote(&mut self, quote: Quote) {
        if !self.content.quotes.iter().any(|known| known.id == quote.id) {
            self.content.quotes.push(quote);
        }
    }

    /// The content, without the blank space the address of an image or of
    /// a quoted note left at either end of the text.
    fn finish(mut self) -> Content {
        if let Some(Segment::Text(first)) = self.content.segments.first_mut() {
            *first = first.trim_start().to_string();
        }
        if let Some(Segment::Text(last)) = self.content.segments.last_mut() {
            last.truncate(last.trim_end().len());
        }
        self.content.segments.retain(|segment| !matches!(segment, Segment::Text(text) if text.is_empty()));
        while self.content.line_chars.len() > 1 && self.content.line_chars.last() == Some(&0) {
            self.content.line_chars.pop();
        }
        while self.content.line_chars.len() > 1 && self.content.line_chars.first() == Some(&0) {
            self.content.line_chars.remove(0);
        }
        self.content
    }

    /// Count `chars` more characters on the current line.
    fn count(&mut self, chars: usize) {
        if let Some(line) = self.content.line_chars.last_mut() {
            *line += chars;
        }
        self.room = self.room.saturating_sub(chars);
    }

    /// Add text in `style` to the last segment if that is text in the
    /// same, or start one.
    fn append(&mut self, text: &str, style: Style) {
        match self.content.segments.last_mut() {
            Some(Segment::Text(last)) if style.is_plain() => last.push_str(text),
            Some(Segment::Styled { text: last, style: same }) if *same == style => last.push_str(text),
            _ if style.is_plain() => self.content.segments.push(Segment::Text(text.to_string())),
            _ => self.content.segments.push(Segment::Styled { text: text.to_string(), style }),
        }
    }

    /// Mark the note as cut: an ellipsis, and nothing after it.
    fn cut(&mut self) {
        if let Some(Segment::Text(last)) = self.content.segments.last_mut() {
            last.truncate(last.trim_end().len());
        }
        // Under a block, it is a line of its own.
        if matches!(self.content.segments.last(), Some(Segment::Block(_))) {
            self.count(1);
        }
        self.append("\u{2026}", Style::default());
        self.full = true;
    }

    /// What one run of the note comes to.
    fn run(&mut self, run: Run) {
        match run {
            Run::Text { text, style } => self.read(&text, style),
            Run::Code { .. } if self.full => {}
            Run::Code { text, style } => {
                self.gap = false;
                self.text(&text, style);
            }
            Run::Link { url, label } => self.link(url, Some(&label), Style::default()),
            Run::Block(block) => self.block(block),
        }
    }

    /// What stands apart from the text: a rule; lines of code, as many as
    /// there is room for; a table, whole or not at all.
    fn block(&mut self, block: Block<Vec<Run>>) {
        self.gap = false;
        let chars = |runs: &[Run]| -> usize {
            runs.iter()
                .map(|run| match run {
                    Run::Text { text, .. } | Run::Code { text, .. } => text.chars().count(),
                    Run::Link { label, .. } => label.chars().count(),
                    Run::Block(_) => 0,
                })
                .sum()
        };
        match block {
            Block::Rule if !self.full => self.stand_apart(Block::Rule),
            Block::Code(code) if !self.full => {
                let wanted = code.chars().count();
                let fits: String = code.chars().take(self.room).collect();
                let fits = fits.trim_end();
                if !fits.is_empty() {
                    self.stand_apart(Block::Code(fits.to_string()));
                }
                if wanted > self.room {
                    self.cut();
                }
                self.room = self.room.saturating_sub(wanted);
            }
            Block::Rule | Block::Code(_) => {}
            Block::Table(table) => {
                let wanted: usize = table.rows.iter().flatten().map(|cell| chars(cell)).sum();
                let fits = !self.full && wanted <= self.room;
                if !fits && !self.full {
                    self.cut();
                }
                // Its cells are read as the text is, for what is in them
                // and what they set aside, with all the room they want:
                // whether it fits was settled above.
                let room = std::mem::replace(&mut self.room, usize::MAX);
                let block = Block::Table(table).map(|cell| self.cell(cell));
                self.room = room.saturating_sub(usize::MAX - self.room);
                if fits {
                    self.stand_apart(block);
                }
            }
        }
    }

    /// The pieces the runs of one cell of a table come to. Past the cut,
    /// none, and only what they set aside.
    fn cell(&mut self, runs: Vec<Run>) -> Vec<Segment> {
        let text = std::mem::take(&mut self.content.segments);
        let lines = std::mem::replace(&mut self.content.line_chars, vec![0]);
        self.gap = false;
        for run in runs {
            self.run(run);
        }
        self.gap = false;
        self.content.line_chars = lines;
        let mut cell = std::mem::replace(&mut self.content.segments, text);
        if let Some(Segment::Text(first)) = cell.first_mut() {
            *first = first.trim_start().to_string();
        }
        if let Some(Segment::Text(last)) = cell.last_mut() {
            last.truncate(last.trim_end().len());
        }
        cell.retain(|piece| !matches!(piece, Segment::Text(text) if text.is_empty()));
        cell
    }

    /// Set `block` down after the text so far, which ends where it
    /// starts: the blank space before it is left out, and what follows it
    /// starts a line of its own.
    fn stand_apart(&mut self, block: Block<Vec<Segment>>) {
        while let Some(Segment::Text(last) | Segment::Styled { text: last, .. }) = self.content.segments.last_mut() {
            last.truncate(last.trim_end().len());
            if !last.is_empty() {
                break;
            }
            self.content.segments.pop();
        }
        while self.content.line_chars.len() > 1 && self.content.line_chars.last() == Some(&0) {
            self.content.line_chars.pop();
        }
        self.content.segments.push(Segment::Block(block));
        if self.content.line_chars.last().is_some_and(|chars| *chars > 0) {
            self.content.line_chars.push(0);
        }
    }

    /// Add text in `style`, as much of it as there is room for.
    fn text(&mut self, text: &str, style: Style) {
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.append("\n", style);
                self.content.line_chars.push(0);
            }
            let chars = line.chars().count();
            if chars > self.room {
                let fits: String = line.chars().take(self.room).collect();
                self.append(&fits, style);
                self.count(self.room);
                self.cut();
                return;
            }
            self.append(line, style);
            // Code is wider than it is long.
            self.count(if style.code { (chars * CODE_CHARS_PER_FOUR).div_ceil(4) } else { chars });
        }
    }

    /// Read a run of the note's text, set in `style`, for what is in it:
    /// links, the people and the notes it names, hashtags, and the text
    /// between them.
    fn read(&mut self, text: &str, style: Style) {
        for token in NostrParser::new().parse(text) {
            let token = match token {
                Token::Url(url) => {
                    self.link(url, None, style);
                    continue;
                }
                token => token,
            };
            // What is set aside leaves its place in the text empty: the
            // space or the line breaks on either side of it are left once,
            // not twice.
            let was_gap = std::mem::take(&mut self.gap);
            match token {
                Token::Nostr(uri) => match named(uri) {
                    Named::Quote(quote) => {
                        self.quote(quote);
                        self.gap = true;
                    }
                    // Past the cut, only what is set aside is still looked
                    // for: images, videos, quoted notes, and the page to
                    // preview.
                    Named::Inline(_) if self.full => {}
                    Named::Inline(segment) => {
                        let chars = match &segment {
                            Segment::Reference(label) => label.chars().count(),
                            _ => MENTION_CHARS,
                        };
                        self.whole(segment, chars);
                    }
                },
                _ if self.full => {}
                Token::Whitespace if was_gap && self.repeats(' ') => self.gap = true,
                Token::LineBreak if was_gap && self.repeats('\n') => self.gap = true,
                Token::Text(text) => self.text(text, style),
                Token::Whitespace => self.text(" ", style),
                Token::LineBreak => self.text("\n", style),
                Token::Hashtag(tag) => {
                    let tag = tag.trim_start_matches('#');
                    self.whole(Segment::Hashtag(tag.to_string()), tag.chars().count() + 1);
                }
                // Links were seen to above.
                Token::Url(_) => {}
            }
        }
    }

    /// A link: to a video or a picture, which is set aside, or to
    /// anywhere else, which stands in the text as its address made short,
    /// or as the `label` the note gives it. Where it stands among text in
    /// `style`, an address that is not followed is set so too.
    fn link(&mut self, url: Url, label: Option<&str>, style: Style) {
        let video = is_video(&url, self.said_of(&url));
        // The last page linked to is the one previewed, wherever in the
        // note it is.
        if !video && may_be_page(&url) {
            self.content.preview = Some(url.to_string());
        }
        // The first link is the first of all of the note, past where its
        // text is cut too: a video, or what a link in the text may be.
        if self.content.first_link.is_none() && (video || (!is_image(&url) && matches!(url.scheme(), "http" | "https"))) {
            self.content.first_link = Some(url.to_string());
        }
        self.gap = false;
        if video {
            self.video(&url);
            self.gap = true;
        } else if is_image(&url) {
            self.image(&url);
            self.gap = true;
        } else if self.full {
        } else if matches!(url.scheme(), "http" | "https") {
            let label = labelled(&url, label);
            let chars = label.chars().count();
            self.whole(Segment::Link { url: url.to_string(), label }, chars);
        } else {
            // Other schemes are not followed, so they are not dressed as links.
            self.text(url.as_str(), style);
        }
    }

    /// Add a segment that is shown whole or not at all: a link cut in half
    /// would go somewhere else.
    fn whole(&mut self, segment: Segment, chars: usize) {
        if chars > self.room {
            self.cut();
        } else {
            self.content.segments.push(segment);
            self.count(chars);
        }
    }
}

/// Parse a note's `content`, cutting its text at `max_chars` characters of
/// what it shows. Of a note that is cut, all of the text is kept as well.
pub fn parse(content: &str, max_chars: usize) -> Content {
    parse_attached(content, max_chars, &[])
}

/// As [`parse`], of a note that says `attached` of what it links to.
pub fn parse_attached(content: &str, max_chars: usize, attached: &[Attachment]) -> Content {
    let (mut cut, was_cut) = parse_up_to(content, max_chars, attached);
    if was_cut {
        // Read once more without a limit, for the text alone: what is
        // set aside is found past the cut the first time already.
        let (whole, _) = parse_up_to(content, usize::MAX, attached);
        cut.whole = Some(Whole { segments: whole.segments, line_chars: whole.line_chars });
    }
    cut
}

/// `content`, with at most `max_chars` characters of its text, and
/// whether there was more.
fn parse_up_to(content: &str, max_chars: usize, attached: &[Attachment]) -> (Content, bool) {
    let mut builder = Builder::new(max_chars, attached);
    for run in markdown::runs(content.trim()) {
        builder.run(run);
    }
    let was_cut = builder.full;
    (builder.finish(), was_cut)
}

/// What an `imeta` tag's `dim` field says: `"1920x1080"`.
fn parse_dimensions(dim: &str) -> Option<(u32, u32)> {
    let (width, height) = dim.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?)).filter(|(width, height)| *width > 0 && *height > 0)
}

/// What one NIP-92 `imeta` tag says, and of which address. `None` if it is
/// not such a tag, or names no address.
pub fn imeta(tag: &[String]) -> Option<Attachment> {
    let mut fields = tag.iter();
    if fields.next().map(String::as_str) != Some("imeta") {
        return None;
    }
    let https = |value: &str| Url::parse(value.trim()).ok().filter(|url| url.scheme() == "https").map(String::from);
    let (mut url, mut said, mut thumb) = (None, Attachment::default(), None);
    for field in fields {
        match field.split_once(' ') {
            Some(("url", value)) => url = Url::parse(value.trim()).ok(),
            Some(("m", value)) => said.kind = Some(value.trim().to_string()),
            Some(("dim", value)) => said.size = parse_dimensions(value.trim()),
            Some(("image", value)) => said.picture = https(value),
            Some(("thumb", value)) => thumb = https(value),
            _ => {}
        }
    }
    Some(Attachment { url: url?.to_string(), picture: said.picture.or(thumb), ..said })
}

impl Content {
    /// The text and how long its lines are: all of it if `in_full`, and
    /// as a row first shows it otherwise.
    pub fn text(&self, in_full: bool) -> (&[Segment], &[usize]) {
        match &self.whole {
            Some(whole) if in_full => (&whole.segments, &whole.line_chars),
            _ => (&self.segments, &self.line_chars),
        }
    }

    /// The people mentioned, in order, possibly more than once: in all of
    /// the text, so that the rest of a note that was cut names them too
    /// once it is shown.
    pub fn mentions(&self) -> impl Iterator<Item = PublicKey> + '_ {
        inline(self.text(true).0).filter_map(|segment| match segment {
            Segment::Mention(public_key) => Some(*public_key),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Attachment, Content, FEED_NOTE_MAX_CHARS, NoteImage, NoteVideo, Quote, Segment, imeta, parse, parse_attached};
    use crate::nostr::markdown::{Block, Style};
    use nostr_sdk::prelude::{EventId, Keys, Nip19Event, Nip19Profile, RelayUrl, ToBech32};

    fn text(text: &str) -> Segment {
        Segment::Text(text.to_string())
    }

    fn link(url: &str, label: &str) -> Segment {
        Segment::Link { url: url.to_string(), label: label.to_string() }
    }

    fn segments(content: &str) -> Vec<Segment> {
        parse(content, 10_000).segments
    }

    #[test]
    fn plain_text_stays_one_piece_with_its_line_breaks() {
        let content = parse("  gm nostr\n\nhow is everyone?  ", 10_000);
        assert_eq!(content.segments, [text("gm nostr\n\nhow is everyone?")]);
        assert_eq!(content.line_chars, [8, 0, 16]);
    }

    #[test]
    fn a_link_is_labelled_by_host_and_path_and_keeps_its_address() {
        assert_eq!(
            segments("read https://www.example.com/a/page?utm=1, then reply"),
            [text("read "), link("https://www.example.com/a/page?utm=1", "example.com/a/page"), text(", then reply")]
        );
        assert_eq!(segments("https://example.com"), [link("https://example.com/", "example.com")]);
    }

    /// `text` as the note marks it: bold, slanted, or code.
    fn styled(text: &str, style: Style) -> Segment {
        Segment::Styled { text: text.to_string(), style }
    }

    const BOLD: Style = Style { bold: true, italic: false, strike: false, code: false, quiet: false, heading: 0 };
    const ITALIC: Style = Style { italic: true, bold: false, ..BOLD };
    const CODE: Style = Style { code: true, bold: false, ..BOLD };

    #[test]
    fn what_a_note_marks_is_set_so_and_read_for_links_and_hashtags_like_the_rest() {
        assert_eq!(
            segments("**gm** #nostr, *see https://example.com/a now* and **#bitcoin**"),
            [
                styled("gm", BOLD),
                text(" "),
                Segment::Hashtag("nostr".into()),
                text(", "),
                styled("see ", ITALIC),
                link("https://example.com/a", "example.com/a"),
                styled(" now", ITALIC),
                text(" and "),
                Segment::Hashtag("bitcoin".into()),
            ]
        );
    }

    #[test]
    fn code_is_shown_as_written_and_read_for_nothing() {
        let content = parse("run `curl https://example.com/x #now` first", 10_000);
        assert_eq!(content.segments, [text("run "), styled("curl https://example.com/x #now", CODE), text(" first")]);
        assert_eq!((content.preview, content.first_link), (None, None), "what is in code is no link");
        // Code is counted wider than it is long: its letters are.
        assert_eq!(parse("`abcdefgh`", 10_000).line_chars, [10]);
    }

    #[test]
    fn a_link_the_note_labels_says_its_label_and_where_it_goes() {
        let labelled = |written: &str| match segments(written).as_slice() {
            [Segment::Link { label, .. }] => label.clone(),
            other => panic!("not one link: {other:?}"),
        };
        // Where it goes is said beside what the note calls it,
        assert_eq!(labelled("[the docs](https://docs.rs/nostr)"), "the docs (docs.rs)");
        // whatever the note calls it,
        assert_eq!(labelled("[paypal.com](https://evil.example/paypal.com)"), "paypal.com (evil.example)");
        // unless the label says so itself.
        assert_eq!(labelled("[docs.rs/nostr](https://www.docs.rs/nostr)"), "docs.rs/nostr");
        // One with no label is as any other link.
        assert_eq!(labelled("[](https://example.com/a/page)"), "example.com/a/page");

        // It is a link like any other: the page to preview, the first link.
        let content = parse("see [this](https://example.com/page)", 10_000);
        assert_eq!(content.preview.as_deref(), Some("https://example.com/page"));
        assert_eq!(content.first_link.as_deref(), Some("https://example.com/page"));
        // And a picture written as one is set aside as any other.
        let content = parse("look ![a cat](https://example.com/cat.png) there", 10_000);
        assert_eq!(content.segments, [text("look there")]);
        assert_eq!(content.images.iter().map(|image| image.url.as_str()).collect::<Vec<_>>(), ["https://example.com/cat.png"]);
    }

    #[test]
    fn a_note_cut_inside_what_it_marks_is_cut_as_any_other() {
        let content = parse(&format!("**{}**", "bold ".repeat(40).trim_end()), 12);
        assert_eq!(content.segments, [styled("bold bold bo", BOLD), text("\u{2026}")]);
        assert!(content.whole.is_some());
    }

    #[test]
    fn a_table_stands_apart_and_its_cells_are_read_as_the_text_is() {
        let friend = Keys::generate().public_key();
        let written = format!(
            "Who:\n\n| Who | Where |\n|---|---|\n| nostr:{} | #nostr at https://example.com/page |\n| a cat | https://example.com/cat.png |\n\nso",
            friend.to_bech32().unwrap()
        );
        let content = parse(&written, 10_000);
        let [Segment::Text(before), Segment::Block(Block::Table(table)), Segment::Text(after)] = content.segments.as_slice()
        else {
            panic!("text, a table, text: {:?}", content.segments);
        };
        assert_eq!((before.as_str(), after.as_str()), ("Who:", "so"), "no line break is left on either side");
        assert_eq!(
            table.rows,
            [
                vec![vec![text("Who")], vec![text("Where")]],
                vec![
                    vec![Segment::Mention(friend)],
                    vec![Segment::Hashtag("nostr".into()), text(" at "), link("https://example.com/page", "example.com/page")]
                ],
                // A picture in a cell is set aside, as anywhere.
                vec![vec![text("a cat")], vec![]],
            ]
        );
        assert_eq!(content.images.iter().map(|image| image.url.as_str()).collect::<Vec<_>>(), ["https://example.com/cat.png"]);
        assert_eq!(content.preview.as_deref(), Some("https://example.com/page"));
        assert_eq!(content.mentions().collect::<Vec<_>>(), [friend], "who is named in a cell is named");
        // The lines of text are the two around it: its own height is its own.
        assert_eq!(content.line_chars, [4, 2]);
    }

    #[test]
    fn a_table_is_shown_whole_or_not_at_all_and_code_as_far_as_there_is_room() {
        let table = "| a | b |\n|---|---|\n| one | two |\n| three | https://example.com/cat.png |";
        let content = parse(&format!("see\n{table}"), 12);
        assert_eq!(content.segments, [text("see\u{2026}")]);
        assert_eq!(content.images.len(), 1, "what a cell sets aside is found past the cut");
        let whole = content.whole.expect("it was cut");
        assert!(matches!(whole.segments.as_slice(), [Segment::Text(_), Segment::Block(Block::Table(_))]), "{:?}", whole.segments);

        let content = parse("```\nlet a = 1;\nlet b = 2;\n```", 12);
        assert_eq!(content.segments, [Segment::Block(Block::Code("let a = 1;\nl".into())), text("\u{2026}")]);
        assert!(content.whole.is_some());
    }

    #[test]
    fn what_stands_apart_reads_as_words_where_there_is_one_line_for_a_note() {
        let content = parse("Dams\n|Where|Count|\n|-|-|\n|U.S.|92,000+|\n---\n```\ncode\n```", 10_000);
        let said: String = content.segments.iter().map(|piece| piece.words(&|_| String::new())).collect();
        assert_eq!(said.split_whitespace().collect::<Vec<_>>().join(" "), "Dams Where Count U.S. 92,000+ code");
    }

    #[test]
    fn a_long_link_gets_a_short_label() {
        let url = format!("https://example.com/{}", "x".repeat(100));
        let parsed = segments(&url);
        let [Segment::Link { url: target, label }] = &parsed[..] else {
            panic!("one link");
        };
        assert_eq!(target, &url, "the address is whole");
        assert_eq!(label.chars().count(), 50);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn only_web_links_are_links() {
        // Nothing but http(s) is ever handed to the system to open.
        for content in ["file:///etc/passwd", "javascript://alert(1)", "ftp://example.com/file"] {
            assert!(
                segments(content).iter().all(|segment| matches!(segment, Segment::Text(_))),
                "{content} must stay text: {:?}",
                segments(content)
            );
        }
    }

    #[test]
    fn people_are_mentioned_by_key_whichever_way_the_key_is_written() {
        let key = Keys::generate().public_key();
        let npub = key.to_bech32().unwrap();
        assert_eq!(segments(&format!("hey nostr:{npub}!")), [text("hey "), Segment::Mention(key), text("!")]);

        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        let nprofile = Nip19Profile::new(key, [relay]).to_bech32().unwrap();
        let content = parse(&format!("nostr:{nprofile}"), 10_000);
        assert_eq!(content.segments, [Segment::Mention(key)]);
        assert_eq!(content.mentions().collect::<Vec<_>>(), [key]);
    }

    #[test]
    fn another_note_is_referred_to_by_the_start_of_its_address() {
        let id = EventId::from_byte_array([7; 32]);
        let note = id.to_bech32().unwrap();
        let label = format!("{}…", &note[..12]);
        let content = parse(&format!("look at this\n\nnostr:{note}"), 10_000);
        assert_eq!(content.quotes, [Quote { id, author: None, relay: None, label: label.clone() }]);
        // Set aside: its address is not in the text, and leaves no gap.
        assert_eq!((content.segments, content.line_chars), (vec![text("look at this")], vec![12]));

        // The longer address is the same note, with who wrote it and where
        // it is.
        let author = Keys::generate().public_key();
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        let nevent = Nip19Event::new(id).author(author).relays([relay.clone()]).to_bech32().unwrap();
        let quoted = Quote { id, author: Some(author), relay: Some(relay), label };
        assert_eq!(parse(&format!("nostr:{nevent}"), 10_000).quotes, [quoted.clone()]);

        // Named twice, it is quoted once, as it was first named; and one
        // named past where the note is cut is quoted all the same.
        let other = EventId::from_byte_array([8; 32]);
        let long = format!("{} nostr:{nevent} nostr:{note} nostr:{}", "word ".repeat(50), other.to_bech32().unwrap());
        let content = parse(&long, 20);
        assert_eq!(content.segments, [text("word word word word…")]);
        assert_eq!(content.quotes.iter().map(|quote| quote.id).collect::<Vec<_>>(), [id, other]);
        assert_eq!(content.quotes[0], quoted);
    }

    #[test]
    fn hashtags_lose_their_hash() {
        assert_eq!(segments("#nostr is #fun"), [Segment::Hashtag("nostr".into()), text(" is "), Segment::Hashtag("fun".into())]);
    }

    #[test]
    fn a_long_note_is_cut_at_the_limit_and_marked() {
        let content = parse("one two three four five", 9);
        assert_eq!(content.segments, [text("one two t…")]);

        // Cut on a word boundary, the space before the mark goes too.
        assert_eq!(parse("one two three", 8).segments, [text("one two…")]);

        // Under the limit, nothing is marked.
        assert_eq!(parse("one two", 7).segments, [text("one two")]);
    }

    #[test]
    fn a_note_that_is_cut_keeps_all_of_its_text_beside_what_is_shown() {
        let someone = Keys::generate().public_key();
        let said = format!("one two three\nfour https://example.com/page nostr:{} five", someone.to_bech32().unwrap());
        let content = parse(&said, 9);
        assert_eq!(content.segments, [text("one two t…")]);
        let whole = content.whole.clone().expect("it was cut");
        let all = parse(&said, 10_000);
        assert_eq!((&whole.segments, &whole.line_chars), (&all.segments, &all.line_chars));
        assert_eq!(whole.line_chars.len(), 2);

        // Either is there to show, and the one who is named past the cut
        // is among those mentioned.
        assert_eq!(content.text(false), (&content.segments[..], &content.line_chars[..]));
        assert_eq!(content.text(true), (&whole.segments[..], &whole.line_chars[..]));
        assert_eq!(content.mentions().collect::<Vec<_>>(), [someone]);

        // A note that is all there has nothing beside it, and shows the
        // same either way.
        assert_eq!(all.whole, None);
        assert_eq!(all.text(true), all.text(false));
        // To the last character, and no further.
        assert_eq!(parse("one two", 7).whole, None);
        assert!(parse("one two", 6).whole.is_some());

        // What is set aside is the same, cut or not.
        let with_picture = format!("{} https://example.com/end.png", "word ".repeat(50));
        let cut = parse(&with_picture, 20);
        assert_eq!(image_urls(&cut), ["https://example.com/end.png"]);
        assert_eq!(cut.whole.map(|whole| whole.segments), Some(parse(&with_picture, 10_000).segments));
    }

    #[test]
    fn a_link_is_shown_whole_or_not_at_all() {
        // "see " is 4 characters; the label "example.com/page" is 16.
        let content = parse("see https://example.com/page now", 10);
        assert_eq!(content.segments, [text("see…")]);

        // With room for it, the link is there, and the cut comes after it.
        let content = parse("see https://example.com/page now", 20);
        assert_eq!(content.segments, [text("see "), link("https://example.com/page", "example.com/page"), text("…")]);
        assert_eq!(parse("see https://example.com/page now", 22).segments.last(), Some(&text(" n…")));
    }

    #[test]
    fn an_empty_note_has_one_empty_line() {
        assert_eq!(parse("   ", 100), Content { line_chars: vec![0], ..Content::default() });
    }

    fn image_urls(content: &Content) -> Vec<&str> {
        content.images.iter().map(|image| image.url.as_str()).collect()
    }

    #[test]
    fn a_picture_is_set_aside_and_leaves_no_gap_in_the_text() {
        let content = parse("look at this\n\nhttps://example.com/cat.JPG?size=large\n", 10_000);
        assert_eq!(content.segments, [text("look at this")]);
        assert_eq!(content.line_chars, [12]);
        assert_eq!(
            content.images,
            [NoteImage { url: "https://example.com/cat.JPG?size=large".into(), label: "example.com/cat.JPG".into(), size: None }]
        );

        // In the middle of the text, it leaves the space or the line
        // breaks around it once.
        let between = |content: &str| (parse(content, 10_000).segments, parse(content, 10_000).line_chars);
        assert_eq!(between("before https://example.com/cat.png after"), (vec![text("before after")], vec![12]));
        assert_eq!(between("before\nhttps://example.com/cat.png\nafter"), (vec![text("before\nafter")], vec![6, 5]));
        assert_eq!(between("before\n\nhttps://example.com/cat.png\n\nafter"), (vec![text("before\n\nafter")], vec![6, 0, 5]));
        // A line break is not blank space, and stays.
        assert_eq!(between("before https://example.com/cat.png\nafter"), (vec![text("before \nafter")], vec![7, 5]));
        let note = EventId::from_byte_array([7; 32]).to_bech32().unwrap();
        assert_eq!(between(&format!("see nostr:{note} and https://example.com/a.png then")).0, [text("see and then")]);

        // A picture first leaves no blank line above the text either.
        let content = parse("https://example.com/cat.png\ncaption", 10_000);
        assert_eq!((content.segments, content.line_chars), (vec![text("caption")], vec![7]));

        // A note that is only a picture has no text at all.
        let content = parse("https://example.com/cat.png", 10_000);
        assert!(content.segments.is_empty());
        assert_eq!(image_urls(&content), ["https://example.com/cat.png"]);
    }

    #[test]
    fn only_what_can_be_fetched_and_decoded_is_a_picture() {
        // Not https: never fetched, so it stays a link.
        assert!(matches!(&segments("http://example.com/cat.png")[..], [Segment::Link { .. }]));
        // Not a format that is decoded.
        for url in ["https://example.com/song.mp3", "https://example.com/drawing.svg", "https://example.com/page"] {
            let content = parse(url, 10_000);
            assert!(content.images.is_empty(), "{url}");
            assert!(matches!(&content.segments[..], [Segment::Link { .. }]), "{url}");
        }
    }

    #[test]
    fn a_picture_after_the_cut_is_still_shown_and_only_so_many_are() {
        let note = format!("{} https://example.com/end.png", "word ".repeat(50));
        let content = parse(&note, 20);
        assert_eq!(content.segments, [text("word word word word…")]);
        assert_eq!(image_urls(&content), ["https://example.com/end.png"]);

        let many: String = (0..6).map(|n| format!("https://example.com/{n}.png ")).collect();
        assert_eq!(parse(&many, 10_000).images.len(), 4);
    }

    #[test]
    fn the_last_page_a_note_links_to_is_the_one_previewed() {
        let preview = |content: &str| parse(content, 10_000).preview;
        assert_eq!(preview("no links here"), None);
        assert_eq!(
            preview("first https://example.com/one then https://example.org/two?x=1 and a word").as_deref(),
            Some("https://example.org/two?x=1")
        );
        // Its link stays in the text.
        assert!(matches!(&segments("see https://example.com/one")[..], [Segment::Text(_), Segment::Link { .. }]));

        // What is not fetched, and what is plainly a file, is not a page.
        assert_eq!(preview("http://example.com/plain"), None);
        assert_eq!(preview("https://example.com/cat.png https://example.com/clip.MP4 https://example.com/paper.pdf"), None);
        assert_eq!(preview("https://example.com/page https://example.com/cat.png").as_deref(), Some("https://example.com/page"));

        // A page linked to past where the note is cut is still previewed.
        let long = format!("{} https://example.com/at-the-end", "word ".repeat(50));
        let content = parse(&long, 20);
        assert_eq!(content.segments, [text("word word word word…")]);
        assert_eq!(content.preview.as_deref(), Some("https://example.com/at-the-end"));
    }

    fn tag(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|field| (*field).to_string()).collect()
    }

    #[test]
    fn the_first_link_of_a_note_is_its_first_that_is_no_picture() {
        let first = |said: &str| parse(said, FEED_NOTE_MAX_CHARS).first_link;
        let (picture, video, page) = ("https://example.com/a.jpg", "https://example.com/b.mp4", "https://example.com/read");
        assert_eq!(first(&format!("{picture} {video} {page}")).as_deref(), Some(video));
        assert_eq!(first(&format!("{picture} {page} {video}")).as_deref(), Some(page));
        assert_eq!(first(&format!("only {picture}")), None);
        assert_eq!(first("only words, and ftp://example.com/not-followed"), None);
        // What is not a page to preview is a link all the same.
        assert_eq!(first("https://example.com/paper.pdf").as_deref(), Some("https://example.com/paper.pdf"));
        // Past where the text is cut, it is still the note's first.
        let long = format!("{} {page}", "word ".repeat(FEED_NOTE_MAX_CHARS));
        assert_eq!(first(&long).as_deref(), Some(page));
    }

    #[test]
    fn imeta_tags_say_what_a_link_is() {
        let said = imeta(&tag(&[
            "imeta",
            "url https://example.com/clip",
            "m video/mp4",
            "dim 1920x1080",
            "thumb https://example.com/small.jpg",
            "image https://example.com/poster.jpg",
            "blurhash eVF$^OI:${M{o#*0-nNFxakD",
        ]))
        .unwrap();
        assert_eq!(
            said,
            Attachment {
                url: "https://example.com/clip".into(),
                kind: Some("video/mp4".into()),
                size: Some((1920, 1080)),
                picture: Some("https://example.com/poster.jpg".into()),
            }
        );

        let of = |fields: &[&str]| imeta(&tag(fields));
        let bare = Attachment { url: "https://example.com/cat.png".into(), ..Attachment::default() };
        assert_eq!(of(&["imeta", "url https://example.com/cat.png"]), Some(bare.clone()), "only what it says");
        assert_eq!(of(&["imeta", "url https://example.com/cat.png", "dim 0x800"]), Some(bare.clone()));
        assert_eq!(of(&["imeta", "url https://example.com/cat.png", "dim wide"]), Some(bare.clone()));
        // The smaller picture stands in where there is no other, and one
        // that would not be fetched is not kept.
        let pictured = |fields: &[&str]| of(fields).unwrap().picture;
        assert_eq!(
            pictured(&["imeta", "url https://example.com/a", "thumb https://example.com/small.jpg"]).as_deref(),
            Some("https://example.com/small.jpg")
        );
        assert_eq!(pictured(&["imeta", "url https://example.com/a", "image http://example.com/poster.jpg"]), None);
        assert_eq!(of(&["imeta", "m video/mp4", "dim 1x1"]), None, "of no address");
        assert_eq!(of(&["e", "url https://example.com/cat.png", "dim 1x1"]), None, "not an imeta tag");
    }

    #[test]
    fn imeta_tags_give_pictures_their_size() {
        let known = imeta(&tag(&["imeta", "url https://example.com/cat.png", "m image/png", "dim 1200x800"])).unwrap();
        let content = parse_attached("https://example.com/cat.png https://example.com/dog.png", 100, &[known]);
        assert_eq!(content.images[0].size, Some((1200, 800)));
        assert_eq!(content.images[1].size, None);
    }

    #[test]
    fn a_video_is_set_aside_as_a_picture_is() {
        let content = parse("watch this\n\nhttps://example.com/clip.MP4?download=1\n\nand say what you think", 10_000);
        assert_eq!(content.segments, [text("watch this\n\nand say what you think")]);
        assert_eq!(
            content.videos,
            [NoteVideo {
                url: "https://example.com/clip.MP4?download=1".into(),
                label: "example.com/clip.MP4".into(),
                size: None,
                posters: Vec::new(),
                page: false
            }]
        );
        // It is nothing else as well: no picture, and no page to preview.
        assert!(content.images.is_empty());
        assert_eq!(content.preview, None);

        for played in ["mp4", "webm", "mov", "mkv", "m4v", "ogv", "avi"] {
            assert_eq!(parse(&format!("https://example.com/clip.{played}"), 100).videos.len(), 1, "{played}");
        }
        // Not https: never fetched, so it stays a link. Sound alone, and a
        // list of parts, are not played either.
        for link in ["http://example.com/clip.mp4", "https://example.com/song.mp3", "https://example.com/live.m3u8"] {
            let content = parse(link, 100);
            assert!(content.videos.is_empty(), "{link}");
            assert!(matches!(&content.segments[..], [Segment::Link { .. }]), "{link}");
        }

        // One past the cut is still given its room, and only so many are.
        let long = format!("{} https://example.com/end.webm", "word ".repeat(50));
        let content = parse(&long, 20);
        assert_eq!(content.segments, [text("word word word word…")]);
        assert_eq!(content.videos.len(), 1);
        let many: String = (0..6).map(|n| format!("https://example.com/{n}.mp4 ")).collect();
        assert_eq!(parse(&many, 10_000).videos.len(), 4);
    }

    #[test]
    fn a_youtube_link_is_a_video_and_no_page_to_preview() {
        let content = parse("worth an evening\nhttps://youtu.be/aqz-KE-bpKQ?si=Wh0Sh4redIt\nall of it", 10_000);
        assert_eq!(content.segments, [text("worth an evening\nall of it")]);
        assert_eq!(
            content.videos,
            [NoteVideo {
                // As it was written: that is what opens in the browser.
                url: "https://youtu.be/aqz-KE-bpKQ?si=Wh0Sh4redIt".into(),
                label: "youtu.be/aqz-KE-bpKQ".into(),
                size: None,
                posters: vec![
                    "https://i.ytimg.com/vi/aqz-KE-bpKQ/maxresdefault.jpg".into(),
                    "https://i.ytimg.com/vi/aqz-KE-bpKQ/mqdefault.jpg".into()
                ],
                page: true,
            }]
        );
        assert_eq!(content.preview, None);

        // A short is given the room of something upright.
        let short = parse("https://www.youtube.com/shorts/aqz-KE-bpKQ", 100);
        assert_eq!(short.videos[0].size, Some((1080, 1920)));
        // What on YouTube is not a video stays a link, and a page.
        let channel = parse("https://www.youtube.com/@BlenderOfficial", 100);
        assert!(channel.videos.is_empty());
        assert_eq!(channel.preview.as_deref(), Some("https://www.youtube.com/@BlenderOfficial"));
    }

    #[test]
    fn a_link_the_note_calls_a_video_is_one_whatever_its_address() {
        let said = |fields: &[&str]| imeta(&tag(fields)).unwrap();
        let address = "https://blossom.example.com/b1674191a88ec5cdd733e4240a81803105dc412d6c6708d53ab94fc248f4f553";
        let note = format!("look {address} at this https://example.com/page");

        // Left to its address, it is a link like any other, and the page
        // after it is the one previewed.
        let plain = parse(&note, 10_000);
        assert!(plain.videos.is_empty());
        assert!(matches!(&plain.segments[..], [Segment::Text(_), Segment::Link { .. }, Segment::Text(_), Segment::Link { .. }]));

        let attached = [said(&[
            "imeta",
            &format!("url {address}"),
            "m Video/Quicktime",
            "dim 1080x1920",
            "image https://blossom.example.com/poster.jpg",
        ])];
        let content = parse_attached(&note, 10_000, &attached);
        assert_eq!(
            content.videos,
            [NoteVideo {
                url: address.into(),
                label: "blossom.example.com/b1674191a88ec5cdd733e4240a818…".into(),
                size: Some((1080, 1920)),
                posters: vec!["https://blossom.example.com/poster.jpg".into()],
                page: false,
            }]
        );
        assert_eq!(content.segments, [text("look at this "), link("https://example.com/page", "example.com/page")]);
        assert_eq!(content.preview.as_deref(), Some("https://example.com/page"));
        // As the last link of a note, it is not taken for a page.
        assert_eq!(parse_attached(address, 100, &attached).preview, None);

        // A video by its address has what the note says of it too.
        let sized = [said(&["imeta", "url https://example.com/clip.mp4", "dim 640x360"])];
        assert_eq!(parse_attached("https://example.com/clip.mp4", 100, &sized).videos[0].size, Some((640, 360)));
        // What the note calls something else stays what its address makes
        // it; and http is not fetched, whatever it is called.
        let other = [said(&["imeta", &format!("url {address}"), "m application/pdf"])];
        assert!(parse_attached(address, 100, &other).videos.is_empty());
        let insecure = [said(&["imeta", "url http://example.com/clip", "m video/mp4"])];
        assert!(parse_attached("http://example.com/clip", 100, &insecure).videos.is_empty());
    }
}

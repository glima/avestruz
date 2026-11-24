// SPDX-License-Identifier: MIT

//! One note as a row of a list: who wrote it and when, what it says, its
//! images, its videos, the notes it quotes, and a preview of the web page
//! it links to.
//! The feed, a thread and the notes on a profile are lists of these.
//!
//! The author's picture sits beside their name, and what they wrote runs
//! under both, the full width of the list. Set in beside the picture, a
//! note would leave a margin as wide as the picture empty all the way down.
//!
//! How large a row is set is the reader's to say, in the settings: see
//! [`RowSizes`].
//!
//! A long note is cut, and says so with a button under its text that
//! shows the rest where the note is, in its row: nothing above the row
//! moves, and what is under it makes room. Under all of the text the
//! button shows less again. See [`Reading`].

use std::cell::LazyCell;
use std::collections::HashMap;

use cosmic::Element;
use cosmic::iced::font::{Style as FontStyle, Weight};
use cosmic::iced::widget::Stack;
use cosmic::iced::widget::text::{Shaping, Span, Wrapping};
use cosmic::iced::widget::{rich_text, span, svg};
use cosmic::iced::{Alignment, Background, Border, Color, Font, Length, Point, border, mouse};
use cosmic::widget::menu::menu_button;
use cosmic::widget::popover::{Position, popover};
use cosmic::widget::{self, button, container, icon, text};
use nostr_sdk::prelude::{EventId, PublicKey, RelayUrl};

use crate::fl;
use crate::helpers::{compact_count, relative_time, truncate_chars};
use crate::images::{ImageShape, ImageState, ImageStore};
use crate::messages::Message;
use crate::nostr::bookmarks::Bookmarks;
use crate::nostr::content::{NoteImage, Segment, link_label};
use crate::nostr::engagement::{Did, Engagement, Left, Reaction};
use crate::nostr::markdown::Style;
use crate::nostr::models::{Note, Profile, display_name};
use crate::nostr::outbox::Whereabouts;
use crate::previews::{PreviewState, PreviewStore};
use crate::state::{Expanded, Passing, ReactionMenu, RepostMenu, WHO_SHOWN, WhoMenu};
use crate::video::Videos;
use crate::views::components::{
    MENU_GAP, MediaPlaceholder, POPUP_MAX_WIDTH, Picture, ScrollPosition, avatar, bookmark_icon, fold_icon, like_icon, link_icon,
    link_preview, link_preview_height, menu_at, quote_link, quoted_note, quoted_note_height, reaction_menu, relays_icon,
    reply_icon, repost_icon, shown_size, video_box, video_height, zap_icon,
};

/// The sizes a row is set in, all from one: the size of a note's text,
/// which the settings keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSizes {
    text: u16,
}

impl RowSizes {
    /// The sizes that go with note text of size `text`.
    pub fn new(text: u16) -> Self {
        Self { text }
    }

    /// What a note says.
    pub(super) fn text(self) -> u16 {
        self.text
    }

    /// Who wrote it: as large as what they wrote, and bold.
    pub(super) fn name(self) -> u16 {
        self.text
    }

    /// How long ago: small print.
    pub(super) fn time(self) -> u16 {
        self.text.saturating_sub(2)
    }

    /// The count beside an action.
    fn action_text(self) -> u16 {
        self.text.saturating_sub(1)
    }

    /// The icon of an action.
    fn action_icon(self) -> u16 {
        self.text + 1
    }

    /// The site a previewed page is on: small print, as the time is.
    pub(super) fn preview_site(self) -> u16 {
        self.time()
    }

    /// The title of a previewed page: as large as what the note says, and
    /// bold.
    pub(super) fn preview_title(self) -> u16 {
        self.text
    }

    /// The few words about a previewed page: a little smaller than what
    /// the note says.
    pub(super) fn preview_description(self) -> u16 {
        self.text.saturating_sub(1)
    }
}

/// What a row is told of the account's bookmarks.
#[derive(Clone, Copy)]
pub enum Bookmarking<'a> {
    /// The account keeps none: nobody is signed in with a key, which
    /// reading and writing them takes.
    Unavailable,
    /// What it has bookmarked is not known yet.
    Unknown,
    /// What it has bookmarked, and the one note whose bookmark is being
    /// put on (`true`) or taken off just now.
    Known { bookmarks: &'a Bookmarks, pending: Option<(EventId, bool)> },
    /// Its bookmarks could not be read: why, in the user's language.
    Unreadable(&'a str),
}

impl Bookmarking<'_> {
    /// Whether the note `id` shows as bookmarked: as the change on its way
    /// will leave it, if one is, and as the bookmarks have it otherwise.
    /// `None` for an account that keeps no bookmarks.
    fn shows(self, id: &EventId) -> Option<bool> {
        match self {
            Self::Unavailable => None,
            Self::Unknown | Self::Unreadable(_) => Some(false),
            Self::Known { pending: Some((changing, to)), .. } if changing == *id => Some(to),
            Self::Known { bookmarks, .. } => Some(bookmarks.has(id)),
        }
    }
}

/// What a row is told of reacting to notes.
#[derive(Clone, Copy)]
pub struct Reacting<'a> {
    /// What to react with is on offer, for one note, if it is.
    pub menu: Option<&'a ReactionMenu>,
    /// The emoji offered first: the ones the reader reacts with most.
    pub suggested: &'a [String],
}

impl Reacting<'_> {
    /// Nothing on offer, and nothing to offer.
    pub const NONE: Self = Self { menu: None, suggested: &[] };
}

/// What every row of a list is built with, beside its own note.
#[derive(Clone, Copy)]
pub struct RowContext<'a> {
    /// What is known about note authors.
    pub profiles: &'a HashMap<PublicKey, Profile>,
    /// Avatars and the images in notes.
    pub images: &'a ImageStore,
    /// What the web pages notes link to say of themselves.
    pub previews: &'a PreviewStore,
    /// The video that plays, if one does.
    pub videos: &'a Videos,
    /// What was made of the notes seen.
    pub engagement: &'a HashMap<EventId, Engagement>,
    /// The relays each note was seen on, for the notes seen on any.
    pub seen_on: &'a HashMap<EventId, Vec<RelayUrl>>,
    /// Now, in seconds since the Unix epoch: what "5m" is counted from.
    pub now: u64,
    /// The colour links, mentions and the account's own likes and reposts
    /// are set in.
    pub accent: Color,
    /// Whether the account can publish: reply to a note, repost it, like
    /// it.
    pub can_publish: bool,
    /// The repost menu that is open, if one is.
    pub repost_menu: Option<RepostMenu>,
    /// Who left something on a note, if that is on show.
    pub who: Option<&'a WhoMenu>,
    /// What there is to react with, and whether it is on offer.
    pub reacting: Reacting<'a>,
    /// The long notes whose rows show all they say.
    pub expanded: &'a Expanded,
    /// The row the keyboard is at, in this list, if it is at any: the one
    /// that is marked.
    pub marked: Option<EventId>,
    /// The note a share link to which was copied just now, if one was.
    pub share_copied: Option<EventId>,
    /// The picture that was copied just now, if one was: where it is.
    pub picture_copied: Option<&'a str>,
    /// A menu that is no row's own is open over the list: the one behind
    /// the cog.
    pub menu_open: bool,
    /// What the account has bookmarked.
    pub bookmarking: Bookmarking<'a>,
    /// How large the row is set.
    pub sizes: RowSizes,
}

impl RowContext<'_> {
    /// Whether a menu is open over the list: a row's own, or the one
    /// behind the cog. A click beside it is to put it away, and no more.
    pub(super) fn under_a_menu(&self) -> bool {
        self.repost_menu.is_some()
            || self.who.is_some()
            || self.reacting.menu.is_some()
            || self.menu_open
            || self.videos.speed_menu()
    }
}

/// Space above a row's content, and between its parts.
pub(super) const NOTE_PADDING_TOP: u16 = 8;
pub(super) const NOTE_SPACING: u16 = 4;

/// How wide the author's picture is, and the gap between it and their
/// name.
pub(super) const NOTE_AVATAR_SIZE: f32 = 32.0;
pub(super) const NOTE_AVATAR_GAP: u16 = 8;

/// A note's image is shown as wide as the note, from one edge to the
/// other, unless it is narrower than that itself (see [`shown_size`]), with
/// its corners rounded this much. Until it has loaded it is given the room
/// it will take, if the note says its size, and this height if not.
const NOTE_IMAGE_RADIUS: f32 = 6.0;
const NOTE_IMAGE_FALLBACK_HEIGHT: f32 = 180.0;

/// How much of the text's colour what a note quotes is set in, and how
/// much of it stands behind code.
const QUIET_TEXT_ALPHA: f32 = 0.65;
const BEHIND_CODE_ALPHA: f32 = 0.12;

/// The corners of what stands behind code, and how far it reaches past
/// the code at either side. It takes no room: the text is where it would
/// be without it.
const CODE_RADIUS: f32 = 3.0;
const CODE_PADDING: [f32; 2] = [0.0, 2.0];

/// The line of actions under a note: how much room each action has around
/// it.
const NOTE_ACTION_PADDING: [u16; 2] = [2, 6];

/// How wide each action is, count and all, so that the same action sits at
/// the same place in every row whatever the counts are. Narrow enough
/// that the four of them and what is at the right end of the line all
/// have their room in the narrowest popup, at the largest size.
const NOTE_ACTION_WIDTH: f32 = 60.0;

/// How much dimmer than the text an action is that hasn't been taken.
pub(super) const NOTE_ACTION_DIM: f32 = 0.6;

/// The gap between an action's icon and the count beside it.
const NOTE_ACTION_GAP: f32 = 4.0;

/// How wide a figure of a count is, as a share of the count's size, at
/// the most.
const FIGURE_WIDTH_PER_SIZE: f32 = 0.65;

/// How much of why the bookmarks could not be read is said to whoever
/// points at the bookmark action.
const BOOKMARK_REASON_MAX_CHARS: usize = 120;

/// The thickness of the line under a row.
pub(super) const NOTE_DIVIDER_HEIGHT: f32 = 1.0;

/// The button under a long note's text, which shows more of it or less:
/// the room above and under its label, the room before the label and
/// after the arrows beside it, and the gap between the two. Less room
/// after than before: the arrows are drawn well inside the square they
/// are given, and what shows on either side is then the same.
const FOLD_PADDING_Y: u16 = 3;
const FOLD_PADDING_BEFORE: u16 = 12;
const FOLD_PADDING_AFTER: u16 = 8;
const FOLD_GAP: f32 = 4.0;

/// How much of what a long note says is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reading {
    /// Cut, and no more to be had here: a note quoted in another, whose
    /// own view has the rest.
    Quoted,
    /// Cut, with a button under it that shows the rest: a row, as it
    /// first shows.
    Cut,
    /// All of it, with a button under it that cuts it again: a row that
    /// was asked for the rest.
    Whole,
}

impl Reading {
    /// How the row of a note is read: all of it if `in_full`.
    fn of_row(in_full: bool) -> Self {
        if in_full { Self::Whole } else { Self::Cut }
    }
}

/// What [`estimated_note_height`] assumes of text it hasn't seen laid out:
/// how tall a line is and how wide a character is on average, both as a
/// share of the text's size. Read off rows laid out with the real text
/// renderer; `the_estimate_is_close_to_the_measured_row` holds them to it.
const LINE_HEIGHT_PER_SIZE: f32 = 1.4;
const CHAR_WIDTH_PER_SIZE: f32 = 0.48;

/// How wide a list is taken to be before it has been laid out once: as
/// wide as the popup opens, less the room around the list.
const UNMEASURED_LIST_WIDTH: f32 = POPUP_MAX_WIDTH - 26.0;

/// How tall a line of text of `size` is.
pub(super) fn line_height(size: u16) -> f32 {
    f32::from(size) * LINE_HEIGHT_PER_SIZE
}

/// How tall the line of actions under a note is: its icons or its counts,
/// whichever are taller, and the room around them.
fn action_height(sizes: RowSizes) -> f32 {
    f32::from(sizes.action_icon()).max(line_height(sizes.action_text())) + 2.0 * f32::from(NOTE_ACTION_PADDING[0])
}

/// How tall the button under a long note's text is: its label, and the
/// room above and under it.
fn fold_height(sizes: RowSizes) -> f32 {
    line_height(sizes.action_text()).max(f32::from(sizes.action_icon())) + 2.0 * f32::from(FOLD_PADDING_Y)
}

/// How tall what `note` says will be, near enough, set in `sizes` in room
/// `width` wide and read as `reading` says, with the space above each
/// part of it: each line of its text wrapped at a typical line length,
/// the button under a long one, each image at the height it will be
/// shown, each video in the room it has, each note it quotes as its card
/// or its link, and the preview of a page it links to, which is the
/// height it is.
pub(super) fn said_height(note: &Note, sizes: RowSizes, width: f32, reading: Reading) -> f32 {
    let spacing = f32::from(NOTE_SPACING);

    let chars_per_line = ((width / (f32::from(sizes.text()) * CHAR_WIDTH_PER_SIZE)) as usize).max(1);
    let (segments, line_chars) = note.content.text(reading == Reading::Whole);
    let lines: usize = match segments.is_empty() {
        true => 0,
        false => line_chars.iter().map(|chars| chars.div_ceil(chars_per_line).max(1)).sum(),
    };
    let text = if lines == 0 { 0.0 } else { spacing + lines as f32 * line_height(sizes.text()) };
    let fold = if note.content.whole.is_some() && reading != Reading::Quoted { spacing + fold_height(sizes) } else { 0.0 };
    let images: f32 = note
        .content
        .images
        .iter()
        .map(|image| spacing + image.size.map_or(NOTE_IMAGE_FALLBACK_HEIGHT, |size| shown_size(size, width).height))
        .sum();
    let videos: f32 = note.content.videos.iter().map(|video| spacing + video_height(video.size, width)).sum();
    let quotes: f32 = note
        .quotes_shown()
        .map(|quote| {
            spacing + note.quoted(quote.id).map_or(line_height(sizes.text()), |quoted| quoted_note_height(quoted, sizes, width))
        })
        .sum();
    let preview = note.content.preview.as_ref().map_or(0.0, |_| spacing + link_preview_height(sizes));
    text + fold + images + videos + quotes + preview
}

/// How tall a note's row will be, near enough, before it has been laid out,
/// set in `sizes` in a list `width` wide: what the note says, all of it
/// if a long one shows `in_full`, and around it the room above, the
/// author's picture or their name beside it, whichever is taller, the
/// line of actions, and the line under the row. The list replaces this
/// with the measured height once the row has been in view.
pub fn estimated_note_height(note: &Note, sizes: RowSizes, width: f32, in_full: bool) -> f32 {
    let width = if width > 0.0 { width } else { UNMEASURED_LIST_WIDTH };
    let spacing = f32::from(NOTE_SPACING);
    let header = NOTE_AVATAR_SIZE.max(line_height(sizes.name()));
    f32::from(NOTE_PADDING_TOP)
        + header
        + said_height(note, sizes, width, Reading::of_row(in_full))
        + spacing
        + action_height(sizes)
        + spacing
        + NOTE_DIVIDER_HEIGHT
}

/// A link on a line of its own, in text of `size`: a picture that could
/// not be shown, a website.
pub fn link_line<'a>(url: &'a str, label: &'a str, accent: Color, size: u16) -> Element<'a, Message> {
    let spans: Vec<Span<'a, String>> = vec![span(label).color(accent).underline(true).link(url.to_string())];
    rich_text(spans).size(size).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).on_link_click(Message::OpenUrl).into()
}

/// One of a note's images: the picture, once it is here; the room it will
/// take, until then; and if it can't be had, the link it came as.
fn note_image<'a>(image: &'a NoteImage, context: &RowContext<'a>) -> Element<'a, Message> {
    let RowContext { images, accent, sizes, .. } = *context;
    match images.get(&image.url, ImageShape::Original) {
        ImageState::Ready(loaded) => {
            // As wide as the room there is, and no wider than it is.
            // A click on a picture is a click on the row: it opens the
            // note, as one on its text does. The picture takes none
            // itself but one with the right button, which copies it.
            let picture = Picture::new(loaded).radius(NOTE_IMAGE_RADIUS).on_right_press(Message::CopyPicture(image.url.clone()));
            if context.picture_copied != Some(image.url.as_str()) {
                return picture.into();
            }
            // Said over its corner, where it takes no room: the left one,
            // which is over the picture however narrow that is.
            let said = container(text(fl!("picture-copied")).size(sizes.time())).padding([2, 8]).class(
                cosmic::theme::Container::custom(|theme| {
                    let cosmic = theme.cosmic();
                    container::Style {
                        background: Some(Background::Color(cosmic.accent_color().into())),
                        text_color: Some(cosmic.on_accent_color().into()),
                        border: Border { radius: cosmic.corner_radii.radius_s.into(), ..Border::default() },
                        ..container::Style::default()
                    }
                }),
            );
            Stack::new().push(picture).push(container(said).padding(8)).into()
        }
        ImageState::Loading => MediaPlaceholder::new(image.size, NOTE_IMAGE_FALLBACK_HEIGHT).radius(NOTE_IMAGE_RADIUS).into(),
        ImageState::Failed => link_line(&image.url, &image.label, accent, sizes.text()),
    }
}

/// Where a click on part of a note's text leads.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NoteLink {
    /// To a web page, in the browser.
    Web(String),
    /// To someone mentioned, on their profile.
    Profile(PublicKey),
}

impl NoteLink {
    /// Where a click on `segment` leads, if anywhere.
    fn of(segment: &Segment) -> Option<Self> {
        match segment {
            Segment::Link { url, .. } => Some(Self::Web(url.clone())),
            Segment::Mention(public_key) => Some(Self::Profile(*public_key)),
            Segment::Text(_) | Segment::Styled { .. } | Segment::Hashtag(_) | Segment::Reference(_) => None,
        }
    }

    /// What following the link asks of the app.
    fn follow(self) -> Message {
        match self {
            Self::Web(url) => Message::OpenUrl(url),
            Self::Profile(public_key) => Message::OpenProfile(public_key),
        }
    }
}

/// What text a note marks is set in, beside the size and the face all
/// its text has: the faces of what is stressed and of code, and the
/// colours of what is quiet and of what is behind code.
struct Faces {
    text: Font,
    code: Font,
    quiet: Color,
    behind_code: Color,
}

impl Faces {
    /// As the theme has them now. Asked for once for a note that marks
    /// something, and not at all for one that doesn't.
    fn of_the_theme() -> Self {
        let theme = cosmic::theme::active();
        let on_background: Color = theme.cosmic().on_bg_color().into();
        Self {
            text: cosmic::font::default(),
            code: cosmic::font::mono(),
            quiet: Color { a: QUIET_TEXT_ALPHA, ..on_background },
            behind_code: Color { a: BEHIND_CODE_ALPHA, ..on_background },
        }
    }

    /// `text` as a span set in `style`.
    fn styled<'a>(&self, text: &'a str, style: Style) -> Span<'a, NoteLink> {
        let mut font = if style.code { self.code } else { self.text };
        if style.bold {
            font.weight = Weight::Bold;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        let shown = span(text).font(font).strikethrough(style.strike).color_maybe(style.quiet.then_some(self.quiet));
        if style.code {
            shown.background(self.behind_code).border(border::rounded(CODE_RADIUS)).padding(CODE_PADDING)
        } else {
            shown
        }
    }
}

/// What a note says, the text `segments` of it, as rich text: links and
/// mentions can be clicked, and they and hashtags are set in `accent`;
/// what the note marks is set as it marks it.
fn note_text<'a>(segments: &'a [Segment], context: &RowContext<'a>) -> Element<'a, Message> {
    let RowContext { profiles, accent, sizes, .. } = *context;
    let faces = LazyCell::new(Faces::of_the_theme);
    let spans: Vec<Span<'a, NoteLink>> = segments
        .iter()
        .map(|segment| {
            let shown = match segment {
                Segment::Text(text) => span(text.as_str()),
                Segment::Styled { text, style } => faces.styled(text, *style),
                Segment::Link { label, .. } => span(label.as_str()).color(accent).underline(true),
                Segment::Mention(public_key) => span(format!("@{}", display_name(profiles, public_key))).color(accent),
                Segment::Hashtag(tag) => span(format!("#{tag}")).color(accent),
                Segment::Reference(reference) => span(reference.as_str()).color(accent),
            };
            match NoteLink::of(segment) {
                Some(link) => shown.link(link),
                None => shown,
            }
        })
        .collect();

    rich_text(spans).size(sizes.text()).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).on_link_click(NoteLink::follow).into()
}

/// The button under the text of the long note `id`: it shows the rest of
/// the note where it is, in a row that shows it cut, and cuts it again in
/// one that shows it `in_full`. In the middle of the row, as tall at
/// every size as [`fold_height`] says; the way to more stands out as a
/// button does, and the way back is no louder than the actions under it.
fn fold_button<'a>(id: EventId, in_full: bool, context: &RowContext<'a>) -> Element<'a, Message> {
    let sizes = context.sizes;
    let (label, message) = match in_full {
        true => (fl!("note-show-less"), Message::CollapseNote(id)),
        false => (fl!("note-show-more"), Message::ExpandNote(id)),
    };
    let mut label = text(label).size(sizes.action_text()).wrapping(Wrapping::None);
    let mut arrows = icon::icon(fold_icon(in_full)).size(sizes.action_icon());
    if in_full {
        let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
        let dim = Color { a: NOTE_ACTION_DIM, ..text_color };
        label = label.class(cosmic::theme::Text::Color(dim));
        arrows = arrows.class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(dim) }));
    }
    // As tall as the button, so that the label is in the middle of it: a
    // button puts what is in it at its top.
    let content = widget::Row::new().push(label).push(arrows).spacing(FOLD_GAP).height(Length::Fill).align_y(Alignment::Center);
    let fold = button::custom(content)
        .padding([0, FOLD_PADDING_AFTER, 0, FOLD_PADDING_BEFORE])
        .height(Length::Fixed(fold_height(sizes)))
        .class(if in_full { cosmic::theme::Button::Text } else { cosmic::theme::Button::Standard })
        .on_press(message);
    container(fold).center_x(Length::Fill).into()
}

/// `row`, the row of the note `id`, opening that note's thread when it is
/// clicked anywhere that doesn't lead somewhere of its own (a link, a
/// picture, the author). Not while a menu is open over the list: a click on
/// a row beside the menu was meant to put the menu away, and that is all
/// it does.
pub fn opening_thread<'a>(row: Element<'a, Message>, id: EventId, context: &RowContext<'_>) -> Element<'a, Message> {
    let area = widget::mouse_area(row);
    if context.under_a_menu() {
        area.into()
    } else {
        area.on_press(Message::OpenThread(id, Whereabouts::default())).interaction(mouse::Interaction::Pointer).into()
    }
}

/// How round the corners of the line around the marked row are.
const MARK_RADIUS: f32 = 8.0;

/// `row`, the row told apart by `id`, with a line around it in the accent
/// colour if it is the one the keyboard is at. The line is drawn over the
/// row's edge and takes no room: a row is the same size marked and not.
pub fn marking<'a>(row: Element<'a, Message>, id: EventId, context: &RowContext<'_>) -> Element<'a, Message> {
    if context.marked != Some(id) {
        return row;
    }
    let accent = context.accent;
    container(row)
        .class(cosmic::theme::Container::custom(move |_theme| container::Style {
            border: Border { color: accent, width: 1.0, radius: MARK_RADIUS.into() },
            ..container::Style::default()
        }))
        .into()
}

/// One of a note's actions, to be pressed: its icon in `color`, the count
/// beside it when there is something to count, and what pressing it does,
/// if anything.
fn action_button<'a>(
    icon_handle: icon::Handle,
    count: u64,
    color: Color,
    tooltip: String,
    on_press: Option<Message>,
    sizes: RowSizes,
) -> Element<'a, Message> {
    let picture = icon::icon(icon_handle)
        .size(sizes.action_icon())
        .class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(color) }));
    action_showing(picture.into(), count, color, tooltip, on_press, sizes)
}

/// As [`action_button`], with `picture` where its icon would be: what is
/// no icon, as the emoji a note was reacted to with.
fn action_showing<'a>(
    picture: Element<'a, Message>,
    count: u64,
    color: Color,
    tooltip: String,
    on_press: Option<Message>,
    sizes: RowSizes,
) -> Element<'a, Message> {
    let mut content = widget::Row::new().push(picture).spacing(NOTE_ACTION_GAP).align_y(Alignment::Center);
    if count > 0 {
        content = content.push(text(compact_count(count)).size(sizes.action_text()).class(cosmic::theme::Text::Color(color)));
    }
    let action = button::custom(content).padding(NOTE_ACTION_PADDING).class(cosmic::theme::Button::Text).on_press_maybe(on_press);
    widget::tooltip(action, text(tooltip).size(12), widget::tooltip::Position::Top).into()
}

/// `action`, one of a note's actions, in the room every action is given.
fn action_cell<'a>(action: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(action).width(Length::Fixed(NOTE_ACTION_WIDTH)).into()
}

/// What the repost action offers for the note `id`: reposting it as it is,
/// which is done once (`reposted`) and not again, or quoting it in a note
/// of one's own. The one the keyboard is `at`, if it is at one, is shown
/// as one the pointer is over.
fn repost_menu<'a>(id: EventId, reposted: bool, at: Option<Passing>) -> Element<'a, Message> {
    let offered = [Passing::Repost, Passing::Quote];
    menu_at(
        [
            (repost_icon(), fl!("note-repost"), (!reposted).then_some(Message::Repost(id))),
            (icon::from_name("edit-symbolic").handle(), fl!("note-quote"), Some(Message::ComposeQuote(id))),
        ],
        at.and_then(|at| offered.iter().position(|way| *way == at)),
    )
}

/// How wide the list of who left something on a note is, how tall one of
/// them is in it, and how tall the list is at the most: more of them are
/// scrolled to.
const WHO_WIDTH: f32 = 240.0;
const WHO_ROW_HEIGHT: f32 = 30.0;
const WHO_MAX_HEIGHT: f32 = 8.0 * WHO_ROW_HEIGHT;

/// How wide the picture of one of them is.
const WHO_AVATAR_SIZE: f32 = 20.0;

/// What one of those who left something on a note left, at the right end
/// of their line: the emoji they reacted with, or a heart; how much they
/// zapped; and nothing for a repost, which is all the list says.
fn who_did<'a>(did: &Did, context: &RowContext<'a>) -> Option<Element<'a, Message>> {
    let small = context.sizes.action_text();
    match did {
        Did::Reposted => None,
        Did::Reacted(Reaction::Emoji(emoji)) => Some(text(emoji.clone()).size(small).shaping(Shaping::Advanced).into()),
        Did::Reacted(Reaction::Like) => {
            let accent = context.accent;
            let heart = icon::icon(like_icon(true))
                .size(small)
                .class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(accent) }));
            Some(heart.into())
        }
        Did::Zapped(sats) => Some(text(fl!("who-sats", sats = compact_count(*sats))).size(small).wrapping(Wrapping::None).into()),
    }
}

/// Who left something on a note, under the count of them: what the list
/// is of, and each of them on a line, with their picture, their name and
/// what they left. A press on one opens their profile. Until the store
/// has been read it says so, and if nobody did, that.
fn who_menu<'a>(menu: &'a WhoMenu, context: &RowContext<'a>) -> Element<'a, Message> {
    let title = match menu.what {
        Left::Reposts => fl!("who-reposted"),
        Left::Reactions => fl!("who-reacted"),
        Left::Zaps => fl!("who-zapped"),
    };
    let note = |said: String| -> Element<'a, Message> {
        container(text(said).size(12).wrapping(Wrapping::WordOrGlyph)).padding([4, 8]).width(Length::Fill).into()
    };
    let body: Element<'a, Message> = match &menu.people {
        None => note(fl!("who-reading")),
        Some(people) if people.is_empty() => note(fl!("who-nobody")),
        Some(people) => {
            let mut lines = people.iter().take(WHO_SHOWN).fold(widget::Column::new(), |lines, doer| {
                let (name, picture) = match &doer.who {
                    Some(who) => (
                        display_name(context.profiles, who),
                        Some(avatar(
                            context.images,
                            context.profiles.get(who).and_then(|profile| profile.picture.as_deref()),
                            who,
                            "",
                            WHO_AVATAR_SIZE,
                        )),
                    ),
                    None => (fl!("who-unsaid").into(), None),
                };
                let picture = picture.unwrap_or_else(|| widget::Space::new().width(Length::Fixed(WHO_AVATAR_SIZE)).into());
                let mut line = vec![
                    picture,
                    widget::Space::new().width(8).into(),
                    container(text(name).size(13).wrapping(Wrapping::None)).width(Length::Fill).clip(true).into(),
                ];
                line.extend(who_did(&doer.did, context));
                let line = menu_button(line).height(Length::Fixed(WHO_ROW_HEIGHT)).padding([2, 8]);
                lines.push(line.on_press_maybe(doer.who.map(Message::OpenProfile)))
            });
            let more = people.len().saturating_sub(WHO_SHOWN);
            if more > 0 {
                lines = lines.push(note(fl!("who-more", count = more)));
            }
            container(widget::scrollable(lines)).max_height(WHO_MAX_HEIGHT).into()
        }
    };
    let title = container(text(title).size(12).font(cosmic::font::bold()).wrapping(Wrapping::None)).padding([4, 8]);
    container(widget::Column::new().push(title).push(body))
        .padding(4)
        .width(Length::Fixed(WHO_WIDTH))
        .class(cosmic::theme::Container::Dropdown)
        .into()
}

/// `action`, one of a note's actions that counts what others left on it,
/// with who they were under it for whoever asks: a click on it with the
/// right button shows them, and one anywhere else puts them away. `own`
/// is what the action itself opens under it, if it opens something and
/// that is open, and what closing it says.
fn counted<'a>(
    action: Element<'a, Message>,
    note: &Note,
    what: Left,
    context: &RowContext<'a>,
    own: Option<(Option<Element<'a, Message>>, Message)>,
) -> Element<'a, Message> {
    // Under the action, from its left edge; over it, where the popup has
    // no room left under.
    let under = Point::new(0.0, action_height(context.sizes) + MENU_GAP);
    let action = widget::mouse_area(action).on_right_press(Message::ShowWho(note.id, what));
    let who = context.who.filter(|menu| menu.note == note.id && menu.what == what);
    let (own, own_closes) = own.unwrap_or((None, Message::CloseWho));
    let action = popover(action).position(Position::Point(under));
    match (who, own) {
        (Some(who), _) => action.on_close(Message::CloseWho).popup(who_menu(who, context)).into(),
        (None, Some(own)) => action.on_close(own_closes).popup(own).into(),
        (None, None) => action.on_close(own_closes).into(),
    }
}

/// The reaction action of `note`: a heart, with how many reacted. Pressed,
/// it offers what to react with under it (see
/// [`reaction_menu`](mod@crate::views::components::reaction_menu)), over
/// it where there is no room under, and a click anywhere else puts the
/// offer away. Once the account has reacted it shows what with, in
/// `accent`: the heart filled, or the emoji in the icon's place, set no
/// taller than the line. Once is enough: it can't be pressed again.
fn reaction_action<'a>(note: &Note, engagement: &Engagement, context: &RowContext<'a>, dim: Color) -> Element<'a, Message> {
    let sizes = context.sizes;
    let count = engagement.likes as u64;
    let react = (context.can_publish && engagement.reaction.is_none()).then_some(Message::ToggleReactionMenu(note.id));
    let action = match &engagement.reaction {
        Some(Reaction::Emoji(emoji)) => {
            let shown = text(emoji.clone())
                .size(sizes.action_text())
                .shaping(Shaping::Advanced)
                .class(cosmic::theme::Text::Color(context.accent));
            action_showing(shown.into(), count, context.accent, fl!("note-reacted"), react, sizes)
        }
        Some(Reaction::Like) => action_button(like_icon(true), count, context.accent, fl!("note-reacted"), react, sizes),
        None => action_button(like_icon(false), count, dim, fl!("note-react"), react, sizes),
    };

    let menu = context.reacting.menu.filter(|menu| menu.note == note.id);
    let menu = menu.map(|menu| reaction_menu(menu, context.reacting.suggested));
    counted(action, note, Left::Reactions, context, Some((menu, Message::CloseReactionMenu)))
}

/// What a relay is called: its address without the scheme, which is the
/// same for nearly all of them, and without a slash at its end.
pub fn relay_name(relay: &RelayUrl) -> &str {
    let address = relay.as_str();
    address.split_once("://").map_or(address, |(_, name)| name).trim_end_matches('/')
}

/// How many relays a note was seen on, for the right end of its line of
/// actions: nothing to press, and which relays they were for whoever
/// points at it. Set as an action is, so that it lines up with them.
fn seen_on<'a>(relays: &[RelayUrl], color: Color, sizes: RowSizes) -> Element<'a, Message> {
    let picture = icon::icon(relays_icon())
        .size(sizes.action_icon())
        .class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(color) }));
    let count = text(compact_count(relays.len() as u64)).size(sizes.action_text()).class(cosmic::theme::Text::Color(color));
    let shown = container(widget::Row::new().push(picture).push(count).spacing(NOTE_ACTION_GAP).align_y(Alignment::Center))
        .padding(NOTE_ACTION_PADDING);

    let named = relays.iter().map(|relay| format!("\n{}", relay_name(relay)));
    let said = std::iter::once(fl!("note-seen-on", count = relays.len())).chain(named).collect::<String>();
    widget::tooltip(shown, text(said).size(12), widget::tooltip::Position::Top).into()
}

/// How wide the room is that is kept at the right end of a line of
/// actions for how many relays the note was seen on: its icon and a count
/// of two figures. Kept before that is known as after, so that the
/// bookmark beside it stays where it is when it comes.
fn seen_on_width(sizes: RowSizes) -> f32 {
    let count = 2.0 * f32::from(sizes.action_text()) * FIGURE_WIDTH_PER_SIZE;
    2.0 * f32::from(NOTE_ACTION_PADDING[1]) + f32::from(sizes.action_icon()) + NOTE_ACTION_GAP + count
}

/// The share action of `note`: two links of a chain, which copy a link to
/// the note that anyone can open. Once that is done it is a tick, in
/// `accent`, until the popup closes or another note's link is copied.
fn share_action<'a>(note: &Note, context: &RowContext<'a>, dim: Color) -> Element<'a, Message> {
    let press = Some(Message::CopyShareLink(note.id));
    if context.share_copied == Some(note.id) {
        let tick = icon::from_name("object-select-symbolic").handle();
        action_button(tick, 0, context.accent, fl!("note-share-copied"), press, context.sizes)
    } else {
        action_button(link_icon(), 0, dim, fl!("note-share"), press, context.sizes)
    }
}

/// The bookmark action of `note`: a ribbon, filled and in `accent` when the
/// account has the note bookmarked, which pressing it changes. It shows
/// the change at once, while the changed list is on its way. `None` for an
/// account that keeps no bookmarks.
///
/// It can always be pressed, and what a press does, if anything, is the
/// handler's to say: a press that found nothing to press would open the
/// note's thread instead, as a click anywhere else on the row does.
fn bookmark_action<'a>(note: &Note, context: &RowContext<'a>, dim: Color) -> Option<Element<'a, Message>> {
    let bookmarked = context.bookmarking.shows(&note.id)?;
    let tooltip = match context.bookmarking {
        Bookmarking::Unreadable(why) => {
            let why = truncate_chars(why.lines().next().unwrap_or_default(), BOOKMARK_REASON_MAX_CHARS);
            fl!("note-bookmark-unreadable", error = why.as_ref())
        }
        _ if bookmarked => fl!("note-bookmark-remove"),
        _ => fl!("note-bookmark"),
    };
    let color = if bookmarked { context.accent } else { dim };
    Some(action_button(bookmark_icon(bookmarked), 0, color, tooltip, Some(Message::Bookmark(note.id)), context.sizes))
}

/// The line of actions under a note: answer it, repost or quote it, react
/// to it, zap it. Each says how many did (for zaps, how many sats); the
/// account's own repost and like are in `accent`. Reposting opens a menu
/// under the action, which a click anywhere else puts away. An account that
/// can't publish sees the counts of the first three and can press none.
/// Anyone can zap an author who has a Lightning address. At the right end,
/// copying a link to the note, bookmarking it, and, once that is known,
/// how many relays it was seen on.
fn note_actions<'a>(note: &Note, context: &RowContext<'a>) -> Element<'a, Message> {
    let engagement = context.engagement.get(&note.id).cloned().unwrap_or_default();
    let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
    let dim = Color { a: NOTE_ACTION_DIM, ..text_color };
    let can = |message: Message| context.can_publish.then_some(message);
    let sizes = context.sizes;

    let repost = action_button(
        repost_icon(),
        engagement.reposts as u64,
        if engagement.reposted { context.accent } else { dim },
        fl!("note-repost-how"),
        can(Message::ToggleRepostMenu(note.id)),
        sizes,
    );
    // Its menu opens under it, and so does who reposted.
    let menu = context.repost_menu.filter(|menu| menu.note == note.id);
    let menu = menu.map(|menu| repost_menu(note.id, engagement.reposted, menu.at));
    let repost = counted(repost, note, Left::Reposts, context, Some((menu, Message::CloseRepostMenu)));

    let can_be_zapped = context.profiles.get(&note.author).is_some_and(|profile| profile.lightning.is_some());
    widget::Row::new()
        .push(action_cell(action_button(
            reply_icon(),
            engagement.replies as u64,
            dim,
            fl!("note-reply"),
            can(Message::ComposeReply(note.id)),
            sizes,
        )))
        .push(action_cell(repost))
        .push(action_cell(reaction_action(note, &engagement, context, dim)))
        .push(action_cell(counted(
            action_button(
                zap_icon(),
                engagement.zap_sats,
                dim,
                if can_be_zapped { fl!("note-zap") } else { fl!("note-zap-unavailable") },
                can_be_zapped.then_some(Message::ZapNote(note.id)),
                sizes,
            ),
            note,
            Left::Zaps,
            context,
            None,
        )))
        .push(widget::Space::new().width(Length::Fill))
        .push(share_action(note, context, dim))
        .push_maybe(bookmark_action(note, context, dim))
        .push(
            container(widget::Row::new().push_maybe(
                context.seen_on.get(&note.id).filter(|relays| !relays.is_empty()).map(|relays| seen_on(relays, dim, sizes)),
            ))
            .width(Length::Fixed(seen_on_width(sizes)))
            .align_x(Alignment::End),
        )
        // As tall with counts as without: a count is taller than an icon,
        // and the first one to arrive must not make the row grow.
        .height(Length::Fixed(action_height(sizes)))
        .align_y(Alignment::Center)
        .into()
}

/// `part`, a part of a note's row that stands for its author, opening
/// their profile when it is clicked.
pub(super) fn opening_profile<'a>(part: impl Into<Element<'a, Message>>, author: PublicKey) -> Element<'a, Message> {
    widget::mouse_area(part).on_press(Message::OpenProfile(author)).interaction(mouse::Interaction::Pointer).into()
}

/// `column`, with what `note` says under what is in it already: its text,
/// as much of it as `reading` says, with the button that shows more of a
/// long one or less; its images, its videos, the notes it quotes (each as
/// a card if it is at hand, and as a link if not), and the preview of the
/// page it links to.
///
/// A page that turned out to be a video, and was played, has the room of
/// a video from then on in place of its card: the one time a row changes
/// height under its reader, and at their pressing.
pub(super) fn said<'a>(
    mut column: widget::Column<'a, Message, cosmic::Theme>,
    note: &'a Note,
    context: &RowContext<'a>,
    reading: Reading,
) -> widget::Column<'a, Message, cosmic::Theme> {
    let in_full = reading == Reading::Whole;
    let (segments, _) = note.content.text(in_full);
    if !segments.is_empty() {
        column = column.push(note_text(segments, context));
    }
    if note.content.whole.is_some() && reading != Reading::Quoted {
        column = column.push(fold_button(note.id, in_full, context));
    }
    for image in &note.content.images {
        column = column.push(note_image(image, context));
    }
    for video in &note.content.videos {
        // A page that shows a video is called what it calls itself, once
        // it has said; a file by its address.
        let label = match video.page.then(|| context.previews.get(&video.url)) {
            Some(PreviewState::Ready(page)) => page.title.as_str(),
            _ => video.label.as_str(),
        };
        column = column.push(video_box(&video.url, label, video.size, &video.posters, context));
    }
    for quote in note.quotes_shown() {
        column = column.push(match note.quoted(quote.id) {
            Some(quoted) => quoted_note(quoted, context),
            None => quote_link(quote, context),
        });
    }
    if let Some(page) = &note.content.preview {
        column = column.push_maybe(if context.videos.opened.contains(page) {
            let label = reqwest::Url::parse(page).map_or_else(|_| page.clone(), |url| link_label(&url));
            Some(video_box(page, label, None, &[], context))
        } else {
            link_preview(page, context)
        });
    }
    column
}

/// One row: the author's picture, and beside it who they are and how long
/// ago they wrote; under those, as wide as the row, what the note says and
/// the line of actions; and under it all a line that sets the row apart
/// from the next. The picture and the name lead to the author's profile.
///
/// The row of a long note that shows in full keeps how far its top is
/// above what is in view, for showing less of it to scroll back by (see
/// [`Expanded`]).
pub fn note_row<'a>(note: &'a Note, context: &RowContext<'a>) -> Element<'a, Message> {
    let RowContext { profiles, images, now, sizes, expanded, .. } = *context;
    let above = expanded.above(&note.id);
    let name = display_name(profiles, &note.author);
    let picture = profiles.get(&note.author).and_then(|profile| profile.picture.as_deref());
    let author_picture = opening_profile(avatar(images, picture, &note.author, &name, NOTE_AVATAR_SIZE), note.author);

    let header = widget::Row::new()
        .push(author_picture)
        // The name takes the room the time leaves, and is cut rather than
        // wrapped when it doesn't fit.
        .push(
            container(opening_profile(
                text(name).size(sizes.name()).font(cosmic::font::bold()).wrapping(Wrapping::None),
                note.author,
            ))
            .width(Length::Fill)
            .clip(true),
        )
        .push(text(relative_time(now, note.created_at)).size(sizes.time()).wrapping(Wrapping::None))
        .spacing(NOTE_AVATAR_GAP)
        .align_y(Alignment::Center);

    let reading = Reading::of_row(above.is_some());
    let body = said(widget::Column::new().push(header).spacing(NOTE_SPACING).width(Length::Fill), note, context, reading)
        .push(note_actions(note, context));

    let row = widget::Column::new()
        .push(body)
        .push(widget::divider::horizontal::default())
        .spacing(NOTE_SPACING)
        .padding([NOTE_PADDING_TOP, 0, 0, 0])
        .width(Length::Fill);
    match above {
        // Over the row and not in it: what is in the row has space
        // between its parts, and this is no part to set apart.
        Some(above) => widget::Column::new().push(ScrollPosition::new(above)).push(row).into(),
        None => row.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bookmarking, NOTE_ACTION_WIDTH, NOTE_SPACING, NoteLink, Reacting, RowContext, RowSizes, estimated_note_height,
        fold_height, note_row, opening_thread, relay_name, seen_on_width,
    };
    use crate::emoji::Group;
    use crate::images::{ImageRequest, ImageShape, ImageStore, LoadedImage};
    use crate::messages::Message;
    use crate::nostr::bookmarks::Bookmarks;
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::engagement::{Did, Doer, Engagement, Left, Reaction};
    use crate::nostr::models::Note;
    use crate::nostr::outbox::Whereabouts;
    use crate::previews::{LinkPreview, Linked, PreviewStore};
    use crate::state::{EmojiPicker, Expanded, ReactionMenu, RepostMenu, WhoMenu};
    use crate::video::{Playback, Videos};
    use crate::views::components::{LIST_MAX_WIDTH, link_preview_height, shown_size, video_height};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{clipboard, layout};
    use cosmic::iced::runtime::user_interface::{Cache, UserInterface};
    use cosmic::iced::widget::text::Wrapping;
    use cosmic::iced::{Color, Event, Length, Point, Rectangle, Size, Vector, mouse};
    use cosmic::widget::image::Handle;
    use nostr_sdk::prelude::{EventId, Keys, Kind, Nip19Event, RelayUrl, ToBech32};
    use std::collections::HashMap;

    #[test]
    fn a_click_on_a_mention_or_a_link_leads_where_it_says() {
        let someone = Keys::generate().public_key();
        let said = note(&format!("hey nostr:{} look at https://example.com/page #birds", someone.to_bech32().unwrap()));

        let led: Vec<String> = said
            .content
            .segments
            .iter()
            .filter_map(NoteLink::of)
            .map(|link| match link.follow() {
                Message::OpenProfile(public_key) => format!("profile {}", public_key.to_hex()),
                Message::OpenUrl(url) => format!("web {url}"),
                other => panic!("nowhere a note leads: {other:?}"),
            })
            .collect();

        // The plain text and the hashtag lead nowhere.
        assert_eq!(led, [format!("profile {}", someone.to_hex()), "web https://example.com/page".to_string()]);
    }

    fn note(text: &str) -> Note {
        Note {
            id: EventId::from_byte_array([7; 32]),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse(text, FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        }
    }

    /// A store with nothing in it: every picture is still loading.
    fn no_images() -> ImageStore {
        ImageStore::new(1_000_000).0
    }

    /// How wide the list is in a popup at its widest, and the size note
    /// text is set in unless the settings say otherwise.
    const WIDTH: f32 = LIST_MAX_WIDTH;
    const SIZE: u16 = 13;

    /// How tall the real row for `note` lays out, in a list as wide as the
    /// popup's, with the real text renderer.
    fn measured_height(note: &Note) -> f32 {
        measured_height_with(note, &no_images(), SIZE, WIDTH)
    }

    fn measured_height_with(note: &Note, images: &ImageStore, size: u16, width: f32) -> f32 {
        measured_height_in(note, images, &no_previews(), size, width)
    }

    /// A store that was asked about no page yet: every preview is still
    /// loading.
    fn no_previews() -> PreviewStore {
        PreviewStore::new().0
    }

    /// A store that knows what the page at `url` says of itself, which may
    /// be nothing.
    fn previews_knowing(url: &str, preview: Option<LinkPreview>) -> PreviewStore {
        let mut previews = no_previews();
        assert!(previews.begin(&url.to_string()));
        previews.finish(url.to_string(), preview.map(Linked::Page));
        previews
    }

    fn measured_height_in(note: &Note, images: &ImageStore, previews: &PreviewStore, size: u16, width: f32) -> f32 {
        measured_height_playing(note, images, previews, &Videos::default(), size, width)
    }

    /// As [`measured_height_in`], with `videos` as the model has them.
    fn measured_height_playing(
        note: &Note,
        images: &ImageStore,
        previews: &PreviewStore,
        videos: &Videos,
        size: u16,
        width: f32,
    ) -> f32 {
        measured_height_among(note, images, previews, videos, &Expanded::default(), size, width)
    }

    /// How tall the row of `note` lays out with the note shown in full,
    /// if it is a long one.
    fn measured_height_in_full(note: &Note, size: u16, width: f32) -> f32 {
        let mut expanded = Expanded::default();
        expanded.expand(note.id);
        measured_height_among(note, &no_images(), &no_previews(), &Videos::default(), &expanded, size, width)
    }

    /// As [`measured_height_playing`], with the long notes among
    /// `expanded` in full.
    fn measured_height_among(
        note: &Note,
        images: &ImageStore,
        previews: &PreviewStore,
        videos: &Videos,
        expanded: &Expanded,
        size: u16,
        width: f32,
    ) -> f32 {
        let renderer = crate::views::components::testing::renderer();
        let profiles = HashMap::new();
        let engagement = HashMap::new();
        let seen_on = HashMap::new();
        let context = RowContext {
            profiles: &profiles,
            images,
            previews,
            videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(size),
        };
        let mut element = note_row(note, &context);
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
    }

    /// How tall `said` lays out on its own, in text of `size`, `width` wide.
    fn text_height(said: &str, size: u16, width: f32) -> f32 {
        let renderer = crate::views::components::testing::renderer();
        let mut element: cosmic::Element<'_, Message> =
            cosmic::widget::text(said).size(size).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).into();
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
    }

    #[test]
    fn a_note_is_as_wide_as_its_row() {
        // What a note says wraps as it would with the whole width of the
        // row to itself: nothing is set in beside the author's picture.
        let said = "wide ".repeat(60);
        let row = |text: &str| measured_height_with(&note(text), &no_images(), SIZE, 300.0);
        let alone = |text: &str| text_height(text.trim(), SIZE, 300.0);

        assert!(alone(&said) > 3.0 * alone("wide"), "several lines: {}", alone(&said));
        let (in_row, by_itself) = (row(&said) - row("wide"), alone(&said) - alone("wide"));
        assert!((in_row - by_itself).abs() < 0.01, "{in_row} in the row, {by_itself} alone");
        // A little narrower, and it would take another line.
        assert!(text_height(said.trim(), SIZE, 300.0 - 40.0) > alone(&said));
    }

    #[test]
    fn larger_text_makes_a_taller_row() {
        let said = note(&"the quick brown fox jumps over the lazy dog ".repeat(6));
        let heights: Vec<f32> =
            [10, 13, 18, 24].into_iter().map(|size| measured_height_with(&said, &no_images(), size, WIDTH)).collect();
        assert!(heights.windows(2).all(|pair| pair[0] < pair[1]), "{heights:?}");
    }

    #[test]
    fn a_longer_note_makes_a_taller_row() {
        let short = note("gm");
        let long = note(&"the quick brown fox jumps over the lazy dog ".repeat(12));
        let lines = note("one\ntwo\nthree\nfour\nfive");

        assert!(measured_height(&long) > measured_height(&short) + 60.0);
        assert!(measured_height(&lines) > measured_height(&short) + 50.0);
    }

    #[test]
    fn a_very_long_note_is_cut_in_the_feed() {
        let long = note(&"word ".repeat(200));
        let endless = note(&"word ".repeat(5_000));
        assert_eq!(measured_height(&long), measured_height(&endless));
    }

    /// What comes of clicking the row of `note` at `point`, in a list as
    /// wide as the popup's with room under the row: the row as a list
    /// shows it, opening the note's thread, with the repost menu open on
    /// the note or not, for an account that has `reposted` it or not, and
    /// with `previews` of the pages it links to.
    fn clicked(note: &Note, previews: &PreviewStore, menu_open: bool, reposted: bool, point: Point) -> Vec<Message> {
        clicked_by(note, previews, menu_open, reposted, Bookmarking::Unavailable, &HashMap::new(), point)
    }

    /// As [`clicked`], by an account that keeps bookmarks as `bookmarking`
    /// says, in a row whose note was seen on the relays `seen_on` says.
    fn clicked_by(
        note: &Note,
        previews: &PreviewStore,
        menu_open: bool,
        reposted: bool,
        bookmarking: Bookmarking<'_>,
        seen_on: &HashMap<EventId, Vec<RelayUrl>>,
        point: Point,
    ) -> Vec<Message> {
        let (profiles, images) = (HashMap::new(), no_images());
        let made = Engagement { reposts: usize::from(reposted), reposted, ..Engagement::default() };
        let engagement = HashMap::from([(note.id, made)]);
        let videos = Videos::default();
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews,
            videos: &videos,
            engagement: &engagement,
            seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: menu_open.then_some(RepostMenu::of(note.id)),
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        clicked_in(note, &context, point)
    }

    /// What comes of clicking the row of `note`, built with `context`, at
    /// `point`.
    fn clicked_in(note: &Note, context: &RowContext<'_>, point: Point) -> Vec<Message> {
        let mut renderer = crate::views::components::testing::renderer();
        let row = opening_thread(note_row(note, context), note.id, context);
        let mut interface = UserInterface::build(row, Size::new(WIDTH, 300.0), Cache::new(), &mut renderer);

        let mut messages = Vec::new();
        for event in [
            mouse::Event::CursorMoved { position: point },
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            let _ = interface.update(
                &[Event::Mouse(event)],
                mouse::Cursor::Available(point),
                &mut renderer,
                &mut clipboard::Null,
                &mut messages,
            );
        }
        messages
    }

    /// What comes of a click at each of a column of points `x` across, top
    /// to bottom, told in brief; the same thing twice running is told once.
    fn clicked_down(x: f32, click: impl Fn(Point) -> Vec<Message>) -> Vec<String> {
        let mut told: Vec<String> = Vec::new();
        for y in (0..300).step_by(4) {
            let came: Vec<&str> = click(Point::new(x, y as f32))
                .iter()
                .map(|message| match message {
                    Message::OpenProfile(_) => "profile",
                    Message::OpenThread(..) => "thread",
                    Message::OpenUrl(_) => "page",
                    Message::ToggleRepostMenu(_) => "menu",
                    Message::CloseRepostMenu => "close",
                    Message::Repost(_) => "repost",
                    Message::ComposeQuote(_) => "quote",
                    Message::Bookmark(_) => "bookmark",
                    Message::ComposeReply(_) => "reply",
                    Message::ToggleReactionMenu(_) => "reactions",
                    Message::CloseReactionMenu => "close",
                    Message::React(_, Reaction::Like) => "like",
                    Message::React(_, Reaction::Emoji(_)) => "emoji",
                    Message::OpenEmojiPicker => "more",
                    Message::EmojiGroup(_) => "group",
                    Message::CopyShareLink(_) => "share",
                    Message::ExpandNote(_) => "show more",
                    Message::CollapseNote(_) => "show less",
                    Message::VideoPress(_) => "play",
                    Message::VideoMute => "mute",
                    Message::VideoSpeedMenu => "speeds",
                    Message::CloseVideoSpeedMenu => "close",
                    Message::VideoSpeed(_) => "speed",
                    Message::VideoVolume(_) => "volume",
                    Message::VideoVolumeSet => "set",
                    Message::VideoSound(true) => "over",
                    Message::VideoSound(false) => "left",
                    Message::VideoSeeking(_) => "seeking",
                    Message::VideoSeek => "seek",
                    other => panic!("nothing a click here asks for: {other:?}"),
                })
                .collect();
            let came = came.join("+");
            if told.last() != Some(&came) {
                told.push(came);
            }
        }
        told
    }

    /// What comes of clicks down the middle of the repost action of `note`.
    fn clicked_down_the_repost_action(note: &Note, menu_open: bool, reposted: bool) -> Vec<String> {
        // The action is the second of the line, and its button is at the
        // left of the room it has.
        clicked_down(NOTE_ACTION_WIDTH + 12.0, |point| clicked(note, &no_previews(), menu_open, reposted, point))
    }

    /// What a page says of itself, for a card: with a picture or not.
    fn page(description: Option<&str>, image: Option<&str>) -> LinkPreview {
        LinkPreview {
            title: "A page worth the click, with a title long enough to be cut where the card ends, and then some".into(),
            description: description.map(String::from),
            image: image.map(String::from),
            site: "example.com".into(),
        }
    }

    #[test]
    fn a_row_keeps_its_height_when_the_preview_of_its_link_arrives() {
        let url = "https://example.com/a/page";
        let linking = note(&format!("worth reading\n{url}"));
        // The same, as far as its text goes, with nothing to preview.
        let plain = note("worth reading\nexample.com/a/page");
        let height = |images: &ImageStore, previews: &PreviewStore| measured_height_in(&linking, images, previews, SIZE, WIDTH);

        // While the page is asked, room is kept for its card.
        let waiting = height(&no_images(), &no_previews());
        let card = link_preview_height(RowSizes::new(SIZE)) + f32::from(NOTE_SPACING);
        assert!((waiting - measured_height(&plain) - card).abs() < 0.01, "{waiting}");

        // Whatever the page says, the card is that tall: a title alone, a
        // description of one line or of many, a picture or none.
        let picture = "https://example.com/card.png";
        for said in [
            page(None, None),
            page(Some("Short."), None),
            page(Some(&"A description that goes on and on. ".repeat(20)), None),
            page(Some("With a picture on its way."), Some(picture)),
        ] {
            assert_eq!(height(&no_images(), &previews_knowing(url, Some(said.clone()))), waiting, "{said:?}");
        }

        // And as tall when the picture is there, or can't be had.
        let previews = previews_knowing(url, Some(page(Some("With a picture."), Some(picture))));
        let request = ImageRequest { url: picture.to_string(), shape: ImageShape::Thumbnail };
        let (mut arrived, mut failed) = (no_images(), no_images());
        arrived.begin(&request);
        arrived.finish(
            request.clone(),
            Ok(LoadedImage { handle: Handle::from_rgba(160, 90, vec![0; 160 * 90 * 4]), width: 160, height: 90 }),
        );
        failed.begin(&request);
        failed.finish(request, Err("404".into()));
        assert_eq!(height(&arrived, &previews), waiting);
        assert_eq!(height(&failed, &previews), waiting);
    }

    #[test]
    fn a_page_with_nothing_to_say_gets_no_card_and_leaves_no_gap() {
        let url = "https://example.com/a/page";
        let linking = note(&format!("worth reading\n{url}"));
        let plain = note("worth reading\nexample.com/a/page");
        let silent = measured_height_in(&linking, &no_images(), &previews_knowing(url, None), SIZE, WIDTH);
        assert_eq!(silent, measured_height(&plain));
    }

    #[test]
    fn a_click_on_the_preview_opens_the_page_and_not_the_thread() {
        let url = "https://example.com/a/page";
        let linking = note(&format!("worth reading\n{url}"));
        let down_the_middle =
            |previews: &PreviewStore| clicked_down(WIDTH / 2.0, |point| clicked(&linking, previews, false, false, point));

        // Down the middle of the row: the note, the card, the line of
        // actions beside the actions, and nothing under the row.
        let shown = previews_knowing(url, Some(page(Some("About it."), None)));
        assert_eq!(down_the_middle(&shown), ["thread", "page", "thread", ""]);
        let opened = clicked(&linking, &shown, false, false, Point::new(WIDTH / 2.0, 120.0));
        assert!(matches!(&opened[..], [Message::OpenUrl(opened)] if opened == url), "{opened:?}");

        // While the page is asked there is nothing to click but the row.
        assert_eq!(down_the_middle(&no_previews()), ["thread", ""]);
    }

    /// The videos of a model in which `playing` plays, or did last, and
    /// the links `opened` were played as the videos they turned out to be.
    fn videos_having(playing: Option<Playback>, opened: &[&str]) -> Videos {
        let mut videos = Videos::default();
        videos.playing = playing;
        videos.opened = opened.iter().map(|url| (*url).to_string()).collect();
        videos
    }

    /// What the model has of the video at `url` once it was played: a
    /// picture of it, how long it is, and whether it plays just now.
    fn videos_with(url: &str, playing: bool) -> Videos {
        let mut playback = Playback::held(url);
        playback.frame = Some(Handle::from_rgba(64, 36, vec![0; 64 * 36 * 4]));
        playback.duration = Some(90.0);
        playback.playing = playing;
        videos_having(Some(playback), &[])
    }

    #[test]
    fn a_video_has_the_same_room_before_while_and_after_it_plays() {
        let url = "https://example.com/clip.mp4";
        let plain = measured_height(&note("watch this"));
        // Not said, wide, upright and small: the room kept is the room
        // taken, and the room the estimate counts.
        for size in [None, Some((1920, 1080)), Some((1080, 1920)), Some((320, 240))] {
            let mut with_video = note(&format!("watch this\n{url}"));
            with_video.content.videos[0].size = size;
            let height =
                |videos: &Videos| measured_height_playing(&with_video, &no_images(), &no_previews(), videos, SIZE, WIDTH);

            let unplayed = height(&Videos::default());
            let room = video_height(size, WIDTH) + f32::from(NOTE_SPACING);
            assert!((unplayed - plain - room).abs() < 0.01, "room is kept for a video of {size:?}: {unplayed}");
            let estimated = |note: &Note| estimated_note_height(note, RowSizes::new(SIZE), WIDTH, false);
            assert!((estimated(&with_video) - estimated(&note("watch this")) - room).abs() < 0.01);

            assert_eq!(height(&videos_with(url, true)), unplayed, "playing");
            assert_eq!(height(&videos_with(url, false)), unplayed, "held");
            let mut with_slider = videos_with(url, true);
            with_slider.sound_over(true);
            assert_eq!(height(&with_slider), unplayed, "with the slider of its sound showing");
            assert_eq!(height(&videos_with("https://example.com/another.mp4", true)), unplayed, "another one playing");
            // On its way, with nothing to show yet; and failed, with a
            // lot to say about why.
            let mut loading = Playback::held(url);
            loading.playing = true;
            assert_eq!(height(&videos_having(Some(loading), &[])), unplayed, "loading");
            let mut failed = Playback::held(url);
            failed.error = Some("no decoder for this, and a long account of which one. ".repeat(20));
            assert_eq!(height(&videos_having(Some(failed), &[])), unplayed, "failed");
        }
    }

    /// What comes of clicks down the row of `note`, `x` across, with
    /// `videos` and `previews` as the model has them.
    fn clicked_down_with(note: &Note, previews: &PreviewStore, videos: &Videos, x: f32) -> Vec<String> {
        let (profiles, images, engagement, seen_on) = (HashMap::new(), no_images(), HashMap::new(), HashMap::new());
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews,
            videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        clicked_down(x, |point| clicked_in(note, &context, point))
    }

    #[test]
    fn a_click_on_a_video_plays_it_and_its_bar_does_what_it_says() {
        let url = "https://example.com/clip.mp4";
        let mut with_video = note(&format!("watch this\n{url}"));
        // Low and wide, so that the whole row is within reach.
        with_video.content.videos[0].size = Some((640, 100));
        let down = |videos: &Videos, x: f32| clicked_down_with(&with_video, &no_previews(), videos, x);
        let (left, middle, right) = (16.0, WIDTH / 2.0, WIDTH - 16.0);

        // Not played yet: the picture and the bar play it, wherever they
        // are clicked, but for the end of the bar, which opens the link.
        // The rest of the row opens the thread, as ever.
        let unplayed = Videos::default();
        let beside_the_actions = ["thread", "profile", "thread", "play", "thread", "reply", "thread", ""];
        assert_eq!(down(&unplayed, middle), ["thread", "play", "thread", ""]);
        assert_eq!(down(&unplayed, left), beside_the_actions);
        assert_eq!(down(&unplayed, right), ["thread", "play", "page", "thread", ""]);

        // Playing: the picture and the button hold it, the middle of the
        // bar is where it is dragged, and beside the way out is its sound.
        let playing = videos_with(url, true);
        assert_eq!(down(&playing, left), beside_the_actions);
        assert_eq!(down(&playing, middle), ["thread", "play", "seeking+seek", "play", "thread", ""]);
        // Its sound: the pointer coming over the button is told, for the
        // slider to show, and a press turns the sound off.
        let (at_speed, at_sound) = (right - 48.0, right - 92.0);
        assert_eq!(down(&playing, at_sound), ["thread", "play", "over+over+mute", "thread", ""]);
        // With the slider showing, left of the button, that is where
        // it is dragged. A press over or under it, in the room the two
        // have, is neither the slider's nor the video's: nothing plays
        // or stops for it. The bar is as it was beside them.
        let mut open = videos_with(url, true);
        open.sound_over(true);
        let on_the_slider = down(&open, at_sound - 60.0);
        assert_eq!(on_the_slider, ["thread", "play", "over+over+volume+set", "over+over+over", "thread", ""]);
        assert_eq!(down(&open, at_sound), ["thread", "play", "over+over+mute", "thread", ""]);
        assert_eq!(down(&open, middle), down(&playing, middle));

        // How fast it plays, between its sound and the way out, opens the
        // menu of speeds.
        // (Under it, in the line of actions, is the note's share link.)
        assert_eq!(down(&playing, at_speed), ["thread", "play", "speeds", "play", "thread", "share", "thread", ""]);
        let mut choosing = videos_with(url, true);
        choosing.toggle_speed_menu();
        // With it open, a speed is there to choose; and a click beside
        // the menu puts it away, and neither plays nor holds the video,
        // nor opens the thread. What is the bar's own still does its own.
        let over_the_menu = down(&choosing, at_speed);
        assert!(over_the_menu.iter().any(|came| came == "speed"), "{over_the_menu:?}");
        assert_eq!(down(&choosing, middle), ["close", "seeking+close+seek", "close"]);
        for x in [left, middle, at_sound, at_speed, right] {
            let came = down(&choosing, x);
            assert!(came.iter().all(|came| came != "play" && !came.contains("thread")), "{x}: {came:?}");
        }
        assert_eq!(down(&playing, right), ["thread", "play", "page", "thread", ""]);

        // Another video playing leaves this one as it was.
        assert_eq!(down(&videos_with("https://example.com/another.mp4", true), middle), ["thread", "play", "thread", ""]);
    }

    #[test]
    fn a_youtube_video_has_a_videos_room_whatever_is_known_of_it() {
        let link = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
        let linking = note(&format!("worth an evening\n{link}"));
        let height = |images: &ImageStore, previews: &PreviewStore, videos: &Videos| {
            measured_height_playing(&linking, images, previews, videos, SIZE, WIDTH)
        };
        let room = video_height(None, WIDTH) + f32::from(NOTE_SPACING);
        let unknown = height(&no_images(), &no_previews(), &Videos::default());
        assert!((unknown - measured_height(&note("worth an evening")) - room).abs() < 0.01, "{unknown}");

        // Its title come, or not to be had; its picture here, the large
        // one or, that one not to be had, the small.
        let titled = previews_knowing(link, Some(page(None, None)));
        assert_eq!(height(&no_images(), &titled, &Videos::default()), unknown);
        assert_eq!(height(&no_images(), &previews_knowing(link, None), &Videos::default()), unknown);
        let (large, small) = (&linking.content.videos[0].posters[0], &linking.content.videos[0].posters[1]);
        assert_eq!(height(&images_with(large, (1280, 720)), &titled, &Videos::default()), unknown);
        let mut only_small = images_with(small, (320, 180));
        let gone = ImageRequest { url: large.clone(), shape: ImageShape::Original };
        only_small.begin(&gone);
        only_small.finish(gone, Err("404".into()));
        assert_eq!(height(&only_small, &titled, &Videos::default()), unknown);

        // On its way, while what there is to play of it is looked for;
        // and not found.
        let mut videos = Videos::default();
        assert!(videos.press(link).is_some());
        assert_eq!(height(&no_images(), &titled, &videos), unknown, "on its way");
        videos.found(link, Err("yt-dlp says: sign in to confirm you are not a bot. ".repeat(10)));
        assert_eq!(height(&no_images(), &titled, &videos), unknown, "not found");

        // A click on it plays it, as any video. (The row is taller than
        // the clicks go down.)
        let clicked = clicked_down_with(&linking, &titled, &Videos::default(), WIDTH / 2.0);
        assert_eq!(clicked.get(..2), Some(&["thread".to_string(), "play".to_string()][..]), "{clicked:?}");
    }

    #[test]
    fn a_link_that_turned_out_to_be_a_video_offers_to_play_it_and_then_has_a_videos_room() {
        let url = "https://example.com/a/page";
        let linking = note(&format!("worth watching\n{url}"));
        let mut found = no_previews();
        assert!(found.begin(&url.to_string()));
        found.finish(url.to_string(), Some(Linked::Video));
        let height = |previews: &PreviewStore, videos: &Videos| {
            measured_height_playing(&linking, &no_images(), previews, videos, SIZE, WIDTH)
        };

        // The offer is the card a page would have had, and a click on it
        // plays the video.
        let waiting = height(&no_previews(), &Videos::default());
        assert_eq!(height(&found, &Videos::default()), waiting);
        assert_eq!(clicked_down_with(&linking, &found, &Videos::default(), WIDTH / 2.0), ["thread", "play", "thread", ""]);

        // Played once, it has the room of a video in place of the card,
        // whether or not it is still the one that plays.
        let played = videos_having(None, &[url]);
        let grown = video_height(None, WIDTH) - link_preview_height(RowSizes::new(SIZE));
        assert!((height(&found, &played) - waiting - grown).abs() < 0.01);
        // Another link is no video for that.
        let other = videos_having(None, &["https://example.com/another"]);
        assert_eq!(height(&found, &other), waiting);
    }

    /// What comes of pressing `button` at each of a column of points `x`
    /// across the row of `note`, top to bottom, with `who` on show.
    fn pressed_down(note: &Note, who: Option<&WhoMenu>, button: mouse::Button, x: f32) -> Vec<Message> {
        let (profiles, images, previews) = (HashMap::new(), no_images(), no_previews());
        let (engagement, seen_on) = (HashMap::new(), HashMap::new());
        let (videos, expanded) = (Videos::default(), Expanded::default());
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        let mut renderer = crate::views::components::testing::renderer();
        let row = opening_thread(note_row(note, &context), note.id, &context);
        let mut interface = UserInterface::build(row, Size::new(WIDTH, 300.0), Cache::new(), &mut renderer);
        let mut messages = Vec::new();
        for y in (0..300).step_by(4) {
            let point = Point::new(x, y as f32);
            for event in [
                mouse::Event::CursorMoved { position: point },
                mouse::Event::ButtonPressed(button),
                mouse::Event::ButtonReleased(button),
            ] {
                let _ = interface.update(
                    &[Event::Mouse(event)],
                    mouse::Cursor::Available(point),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
            }
        }
        messages
    }

    #[test]
    fn a_right_click_on_a_count_asks_who_they_were_and_they_are_listed_under_it() {
        let said = note("worth a look");
        // The actions stand in a line, each in as much room as the next:
        // answering, reposting, reacting, zapping.
        let across = |place: f32| place * NOTE_ACTION_WIDTH + 12.0;
        let asked = |place: f32| -> Vec<Left> {
            let mut asked: Vec<Left> = pressed_down(&said, None, mouse::Button::Right, across(place))
                .into_iter()
                .map(|message| match message {
                    Message::ShowWho(note, what) if note == said.id => what,
                    other => panic!("nothing a right click here asks for: {other:?}"),
                })
                .collect();
            asked.dedup();
            asked
        };
        assert_eq!(asked(0.0), [], "who answered is what the thread shows");
        assert_eq!(asked(1.0), [Left::Reposts]);
        assert_eq!(asked(2.0), [Left::Reactions]);
        assert_eq!(asked(3.0), [Left::Zaps]);

        // Listed under the count: a press on one of them opens who they
        // are, one on whoever does not say opens nothing, and one
        // anywhere else puts the list away.
        let someone = Keys::generate().public_key();
        let people = vec![Doer { who: Some(someone), did: Did::Zapped(21) }, Doer { who: None, did: Did::Zapped(5) }];
        let listed = WhoMenu { note: said.id, what: Left::Zaps, people: Some(people) };
        let came = pressed_down(&said, Some(&listed), mouse::Button::Left, across(3.0) + 40.0);
        let opened: Vec<_> = came
            .iter()
            .filter_map(|message| match message {
                Message::OpenProfile(who) => Some(who),
                _ => None,
            })
            .collect();
        assert!(!opened.is_empty() && opened.iter().all(|who| **who == someone), "{came:?}");
        assert!(came.iter().any(|message| matches!(message, Message::CloseWho)), "{came:?}");

        // Until they have been read, and when there are none, it says so
        // and has nobody to press.
        for people in [None, Some(Vec::new())] {
            let empty = WhoMenu { note: said.id, what: Left::Zaps, people };
            let came = pressed_down(&said, Some(&empty), mouse::Button::Left, across(3.0) + 40.0);
            assert!(!came.iter().any(|message| matches!(message, Message::OpenProfile(_))), "{came:?}");
        }
    }

    #[test]
    fn the_repost_action_opens_a_menu_that_reposts_or_quotes() {
        let said = note("worth passing on");

        // With no menu open: the row leads to the thread, the author's name
        // to their profile, and the action opens the menu. Under the row
        // there is nothing.
        assert_eq!(clicked_down_the_repost_action(&said, false, false), ["thread", "profile", "thread", "menu", "thread", ""]);

        // With it open: under the action, reposting and then quoting. The
        // action puts the menu away, and so does a click anywhere else. On
        // the row that is all a click does: the thread is not opened. What
        // leads somewhere of its own, as the author's name does, still does.
        let open = clicked_down_the_repost_action(&said, true, false);
        let beside_closing: Vec<&str> = open.iter().map(String::as_str).filter(|came| !came.ends_with("close")).collect();
        assert_eq!(beside_closing, ["menu", "repost", "quote"], "{open:?}");
        assert_eq!(open[..3], ["close", "profile+close", "close"], "{open:?}");
        assert_eq!(open.last().map(String::as_str), Some("close"), "under the menu, as beside it: {open:?}");

        // Reposted already: once is enough, and quoting is still there.
        let again = clicked_down_the_repost_action(&said, true, true);
        assert!(again.iter().all(|came| came != "repost"), "{again:?}");
        assert!(again.iter().any(|came| came == "quote"), "{again:?}");
    }

    #[test]
    fn the_bookmark_action_is_at_the_right_of_the_line_and_stays_there_when_the_relays_come() {
        let said = note("worth finding again");
        let (none, nowhere) = (Bookmarks::default(), HashMap::new());
        let known = Bookmarking::Known { bookmarks: &none, pending: None };
        // The action is at the left of the room kept for the relays.
        let x = WIDTH - seen_on_width(RowSizes::new(SIZE)) - 14.0;
        let down = |bookmarking: Bookmarking<'_>, seen_on: &HashMap<EventId, Vec<RelayUrl>>| {
            clicked_down(x, |point| clicked_by(&said, &no_previews(), false, false, bookmarking, seen_on, point))
        };

        let relay = |name: &str| RelayUrl::parse(&format!("wss://{name}.example.com")).unwrap();
        let before = down(known, &nowhere);
        assert_eq!(before.iter().filter(|came| *came == "bookmark").count(), 1, "pressed, and nothing else: {before:?}");
        let everywhere = HashMap::from([(said.id, vec![relay("r1"), relay("r2")])]);
        assert_eq!(down(known, &everywhere), before, "it is where it was");

        // It can be pressed whatever is known: what a press does is not
        // the row's to say, and it is never the thread that opens.
        let changing = Bookmarking::Known { bookmarks: &none, pending: Some((said.id, true)) };
        for bookmarking in [Bookmarking::Unknown, Bookmarking::Unreadable("the signer said no"), changing] {
            assert_eq!(down(bookmarking, &nowhere), before);
        }
        // An account without its key keeps no bookmarks, and has no action.
        let without = down(Bookmarking::Unavailable, &nowhere);
        assert!(without.iter().all(|came| came != "bookmark"), "{without:?}");
    }

    #[test]
    fn the_share_action_is_beside_the_bookmark_and_says_when_it_has_copied() {
        let said = note("worth passing around");
        let none = Bookmarks::default();
        let sizes = RowSizes::new(SIZE);
        // Left of the bookmark, which is left of the room kept for the
        // relays; and at the left of that room for an account that keeps
        // no bookmarks.
        let beside_bookmark = WIDTH - seen_on_width(sizes) - 14.0 - f32::from(sizes.action_icon()) - 12.0;
        let alone = WIDTH - seen_on_width(sizes) - 14.0;
        let down = |bookmarking: Bookmarking<'_>, copied: Option<EventId>, x: f32| {
            let (profiles, images, previews, videos) = (HashMap::new(), no_images(), no_previews(), Videos::default());
            let (engagement, seen_on) = (HashMap::new(), HashMap::new());
            let expanded = Expanded::default();
            let context = RowContext {
                profiles: &profiles,
                images: &images,
                previews: &previews,
                videos: &videos,
                engagement: &engagement,
                seen_on: &seen_on,
                now: 0,
                accent: Color::WHITE,
                can_publish: true,
                repost_menu: None,
                who: None,
                reacting: Reacting::NONE,
                expanded: &expanded,
                marked: None,
                share_copied: copied,
                picture_copied: None,
                bookmarking,
                menu_open: false,
                sizes,
            };
            clicked_down(x, |point| clicked_in(&said, &context, point))
        };

        let known = Bookmarking::Known { bookmarks: &none, pending: None };
        let pressed = down(known, None, beside_bookmark);
        assert_eq!(pressed.iter().filter(|came| *came == "share").count(), 1, "pressed, and nothing else: {pressed:?}");
        assert!(pressed.iter().all(|came| came != "bookmark"), "{pressed:?}");
        assert_eq!(down(Bookmarking::Unavailable, None, alone), pressed, "read-only accounts share too");

        // Having copied, it is where it was, and can be pressed again;
        // another note's having been copied changes nothing here.
        assert_eq!(down(known, Some(said.id), beside_bookmark), pressed);
        assert_eq!(down(known, Some(EventId::from_byte_array([9; 32])), beside_bookmark), pressed);
    }

    /// Where every part of the row of `note` is laid out, left to right:
    /// where it starts and how wide it is. Set in text of `size` in a list
    /// `width` wide, for an account that keeps bookmarks, the note having
    /// been made much of and seen on a dozen relays.
    fn parts_across(note: &Note, size: u16, width: f32) -> Vec<(f32, f32)> {
        fn gather(node: &layout::Node, origin: Vector, found: &mut Vec<(f32, f32)>) {
            let bounds = node.bounds() + origin;
            found.push((bounds.x, bounds.width));
            for child in node.children() {
                gather(child, Vector::new(bounds.x, bounds.y), found);
            }
        }
        let renderer = crate::views::components::testing::renderer();
        let (profiles, images, previews, videos) = (HashMap::new(), no_images(), no_previews(), Videos::default());
        let made = Engagement { replies: 1_200, reposts: 1_200, likes: 1_200, zap_sats: 1_200_000, ..Engagement::default() };
        let engagement = HashMap::from([(note.id, made)]);
        let relays = (0..12).map(|number| RelayUrl::parse(&format!("wss://r{number}.example.com")).unwrap()).collect();
        let seen_on = HashMap::from([(note.id, relays)]);
        let none = Bookmarks::default();
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Known { bookmarks: &none, pending: None },
            menu_open: false,
            sizes: RowSizes::new(size),
        };
        let mut element = note_row(note, &context);
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        let mut found = Vec::new();
        gather(&element.as_widget_mut().layout(&mut tree, &renderer, &limits), Vector::ZERO, &mut found);
        found
    }

    #[test]
    fn the_line_of_actions_has_room_for_all_of_them_at_every_size() {
        let said = note("worth a look");
        // At every size the settings offer, in a popup at its narrowest
        // and at its widest.
        for (size, width) in [(10, 394.0), (13, 394.0), (18, 394.0), (24, 394.0), (13, 614.0), (24, 614.0)] {
            let parts = parts_across(&said, size, width);
            let at = format!("at size {size}, {width} wide");
            // Nothing reaches past the row's right edge.
            assert!(parts.iter().all(|(x, across)| x + across <= width + 0.01), "{at}");
            // The room kept for the relays is at that edge, whole: nothing
            // before it has pushed it in.
            let room = seen_on_width(RowSizes::new(size));
            let kept = |(x, across): &(f32, f32)| (across - room).abs() < 0.01 && (x + across - width).abs() < 0.01;
            assert!(parts.iter().any(kept), "{at}, the relays' room of {room} is squeezed");
            // And the four actions at the left each have theirs.
            for place in 0..4 {
                let x = place as f32 * NOTE_ACTION_WIDTH;
                assert!(parts.iter().any(|part| *part == (x, NOTE_ACTION_WIDTH)), "{at}, action {place}");
            }
        }
    }

    #[test]
    fn a_note_shows_as_bookmarked_as_the_change_on_its_way_will_leave_it() {
        let (kept, other) = (EventId::from_byte_array([1; 32]), EventId::from_byte_array([2; 32]));
        let bookmarks = Bookmarks::new(None, vec![vec!["e".to_string(), kept.to_hex()]]);
        let known = |pending| Bookmarking::Known { bookmarks: &bookmarks, pending };

        assert_eq!(known(None).shows(&kept), Some(true));
        assert_eq!(known(None).shows(&other), Some(false));
        // Being taken off, being put on, and a change to another note.
        assert_eq!(known(Some((kept, false))).shows(&kept), Some(false));
        assert_eq!(known(Some((other, true))).shows(&other), Some(true));
        assert_eq!(known(Some((other, true))).shows(&kept), Some(true));

        assert_eq!(Bookmarking::Unknown.shows(&kept), Some(false));
        assert_eq!(Bookmarking::Unreadable("why").shows(&kept), Some(false));
        assert_eq!(Bookmarking::Unavailable.shows(&kept), None);
    }

    /// A note saying `text` and quoting `quoted`, which is at hand or not.
    fn quoting(text: &str, quoted: &Note, at_hand: bool) -> Note {
        let mut note = note(&format!("{text}\nnostr:{}", quoted.id.to_bech32().unwrap()));
        if at_hand {
            note.quoted = vec![quoted.clone()];
        }
        note
    }

    /// A note to quote: not the one `note` makes, which quotes it.
    fn quotable(text: &str) -> Note {
        Note { id: EventId::from_byte_array([8; 32]), ..note(text) }
    }

    #[test]
    fn a_quoted_note_shows_as_a_card_when_it_is_at_hand_and_as_a_link_when_not() {
        let said = "what was said before, at some length, so that it is worth quoting at all";
        let quoted = quotable(said);
        let plain = measured_height(&note("look at this"));
        let line = text_height("note1…", SIZE, WIDTH);

        // Not at hand: one more line, and what is under it.
        let linked = measured_height(&quoting("look at this", &quoted, false));
        assert!((linked - plain - line - f32::from(NOTE_SPACING)).abs() < 0.01, "{linked} against {plain}");

        // At hand: the card, with who wrote the note above what it says.
        let shown = measured_height(&quoting("look at this", &quoted, true));
        assert!(shown > linked + 40.0, "{shown} against {linked}");

        // What the quoted note quotes in turn is a link in the card, one
        // more line there, when it is not at hand; and a card in the card
        // when it is.
        let first = quotable("the first of them");
        let deeper = |at_hand: bool| Note { id: EventId::from_byte_array([6; 32]), ..quoting(said, &first, at_hand) };
        let nested_link = measured_height(&quoting("look at this", &deeper(false), true));
        assert!((nested_link - shown - line - f32::from(NOTE_SPACING)).abs() < 0.01, "{nested_link} against {shown}");
        let nested_card = measured_height(&quoting("look at this", &deeper(true), true));
        assert!(nested_card > nested_link + 40.0, "{nested_card} against {nested_link}");
    }

    #[test]
    fn a_click_on_a_quoted_note_opens_its_thread_and_not_the_one_of_the_note_quoting_it() {
        let quoted = quotable("what was said before");
        let down_the_middle = |note: &Note, menu_open: bool| -> Vec<Vec<EventId>> {
            let previews = no_previews();
            let mut opened: Vec<Vec<EventId>> = Vec::new();
            for y in (0..300).step_by(4) {
                let threads: Vec<EventId> = clicked(note, &previews, menu_open, false, Point::new(WIDTH / 2.0, y as f32))
                    .into_iter()
                    .filter_map(|message| match message {
                        Message::OpenThread(id, _) => Some(id),
                        _ => None,
                    })
                    .collect();
                if opened.last() != Some(&threads) {
                    opened.push(threads);
                }
            }
            opened
        };

        // Down the middle of the row: the note, the card, the note again
        // (its line of actions), and nothing under the row.
        let shown = quoting("look at this", &quoted, true);
        assert_eq!(down_the_middle(&shown, false), [vec![shown.id], vec![quoted.id], vec![shown.id], vec![]]);
        // With a menu open over the list the card leads nowhere, as the
        // row doesn't.
        assert_eq!(down_the_middle(&shown, true), [vec![]]);

        // A card in the card leads to its own note, and the card around it
        // to its own still, above it and under it.
        let inner = Note { id: EventId::from_byte_array([6; 32]), ..quotable("where it started") };
        let outer = Note { quoted: vec![inner.clone()], ..quoting("what was said before", &inner, false) };
        let outer = Note { id: quoted.id, ..outer };
        let nested = quoting("look at this", &outer, true);
        assert_eq!(
            down_the_middle(&nested, false),
            [vec![nested.id], vec![outer.id], vec![inner.id], vec![outer.id], vec![nested.id], vec![]]
        );

        // Not at hand: the link, at the left, leads to the quoted note, to
        // be looked for again; beside it the row is the row.
        let linked = quoting("look at this", &quoted, false);
        assert_eq!(down_the_middle(&linked, false), [vec![linked.id], vec![]]);
        let on_the_link: Vec<Vec<Message>> =
            (60..90).step_by(4).map(|y| clicked(&linked, &no_previews(), false, false, Point::new(20.0, y as f32))).collect();
        assert!(
            on_the_link.iter().any(|came| matches!(&came[..], [Message::OpenThread(id, _)] if *id == quoted.id)),
            "{on_the_link:?}"
        );

        // With where the address said the note is, to look for it there.
        let (author, relay) = (Keys::generate().public_key(), RelayUrl::parse("wss://relay.example.com").unwrap());
        let address = Nip19Event::new(quoted.id).author(author).relays([relay.clone()]).to_bech32().unwrap();
        let addressed = note(&format!("look at this\nnostr:{address}"));
        let there = Whereabouts { author: Some(author), relay: Some(relay) };
        let on_the_link: Vec<Vec<Message>> =
            (60..90).step_by(4).map(|y| clicked(&addressed, &no_previews(), false, false, Point::new(20.0, y as f32))).collect();
        assert!(
            on_the_link
                .iter()
                .any(|came| matches!(&came[..], [Message::OpenThread(id, said)] if *id == quoted.id && *said == there)),
            "{on_the_link:?}"
        );
    }

    #[test]
    fn a_relay_is_called_by_its_address_without_the_scheme() {
        let name = |address: &str| relay_name(&RelayUrl::parse(address).unwrap()).to_string();
        assert_eq!(name("wss://relay.damus.io"), "relay.damus.io");
        assert_eq!(name("wss://nos.lol/"), "nos.lol");
        assert_eq!(name("ws://127.0.0.1:10999"), "127.0.0.1:10999");
        assert_eq!(name("wss://relay.example.com/inbox"), "relay.example.com/inbox");
    }

    #[test]
    fn a_row_is_as_tall_with_counts_under_it_as_without() {
        // What was made of a note arrives after the note: a count beside
        // an action is taller than the action's icon alone, and the row
        // has to have been that tall already.
        let said = note("worth passing on");
        let height = |made: Engagement, size: u16| {
            let renderer = crate::views::components::testing::renderer();
            let (profiles, images, previews, seen_on) = (HashMap::new(), no_images(), no_previews(), HashMap::new());
            let engagement = HashMap::from([(said.id, made)]);
            let videos = Videos::default();
            let expanded = Expanded::default();
            let context = RowContext {
                profiles: &profiles,
                images: &images,
                previews: &previews,
                videos: &videos,
                engagement: &engagement,
                seen_on: &seen_on,
                now: 0,
                accent: Color::WHITE,
                can_publish: true,
                repost_menu: None,
                who: None,
                reacting: Reacting::NONE,
                expanded: &expanded,
                marked: None,
                share_copied: None,
                picture_copied: None,
                bookmarking: Bookmarking::Unavailable,
                menu_open: false,
                sizes: RowSizes::new(size),
            };
            let mut element = note_row(&said, &context);
            let mut tree = Tree::new(element.as_widget());
            let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
            element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
        };
        let counted = Engagement {
            replies: 3,
            reposts: 12,
            reposted: true,
            likes: 1_500,
            reaction: Some(Reaction::Like),
            zap_sats: 21_000,
        };
        // The account's own reaction is an emoji in the heart's place,
        // which is a glyph of text where the heart is a picture.
        let with_emoji = Engagement { reaction: Some(Reaction::Emoji("🤌".into())), ..counted.clone() };
        let only_emoji = Engagement { likes: 1, reaction: Some(Reaction::Emoji("🧨".into())), ..Engagement::default() };
        for size in [10, 13, 18, 24] {
            let plain = height(Engagement::default(), size);
            for made in [&counted, &with_emoji, &only_emoji] {
                assert_eq!(height(made.clone(), size), plain, "at size {size}: {made:?}");
            }
        }
    }

    /// What comes of clicks down the row of `note`, `x` across, for an
    /// account that reacted to it with `reaction`, if it did, with what
    /// to react with on offer or not.
    fn clicked_down_reacting(note: &Note, reaction: Option<Reaction>, menu: Option<&ReactionMenu>, x: f32) -> Vec<String> {
        let (profiles, images, previews, videos) = (HashMap::new(), no_images(), no_previews(), Videos::default());
        let made = Engagement { likes: usize::from(reaction.is_some()), reaction, ..Engagement::default() };
        let (engagement, seen_on) = (HashMap::from([(note.id, made)]), HashMap::new());
        let suggested: Vec<String> = ["👍", "❤️", "😂"].map(String::from).into();
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting { menu, suggested: &suggested },
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        clicked_down(x, |point| clicked_in(note, &context, point))
    }

    #[test]
    fn the_heart_offers_what_to_react_with_and_an_emoji_pressed_reacts() {
        let said = note("worth a reaction");
        // The action is the third of the line; what it offers opens under
        // it, from its left edge, each thing offered a cell wide.
        let heart = 2.0 * NOTE_ACTION_WIDTH + 12.0;
        let cell = |place: f32| 2.0 * NOTE_ACTION_WIDTH + 4.0 + 34.0 * place + 17.0;

        // With nothing on offer: the heart offers, and under the row
        // there is nothing.
        assert_eq!(clicked_down_reacting(&said, None, None, heart), ["thread", "profile", "thread", "reactions", "thread", ""]);

        // On offer: the like first, then the emoji used most, then the
        // way to the others. The heart puts the offer away, and so does a
        // click anywhere else, which does nothing more.
        let menu = ReactionMenu { note: said.id, picker: None };
        let offered = |x: f32| clicked_down_reacting(&said, None, Some(&menu), x);
        let pressed = |x: f32| offered(x).into_iter().filter(|came| !came.ends_with("close")).collect::<Vec<_>>();
        assert_eq!(pressed(cell(0.0)), ["reactions", "like"]);
        assert_eq!(pressed(cell(1.0)), ["emoji"]);
        assert_eq!(pressed(cell(3.0)), ["emoji"]);
        assert_eq!(pressed(cell(4.0)), ["more"]);
        assert!(pressed(cell(6.0)).is_empty(), "past the end of what is offered");
        assert!(offered(WIDTH / 2.0 + 150.0).iter().all(|came| came == "close"), "beside it, the row does nothing");

        // For another note it is not this row's to show.
        let another = ReactionMenu { note: EventId::from_byte_array([9; 32]), picker: None };
        assert!(clicked_down_reacting(&said, None, Some(&another), cell(1.0)).iter().all(|came| came != "emoji"));

        // Every emoji there is, once asked for: the groups, and under
        // them the emoji, each of which reacts.
        let picking = ReactionMenu { note: said.id, picker: Some(EmojiPicker { typed: String::new(), group: Group::Flags }) };
        let picked = clicked_down_reacting(&said, None, Some(&picking), cell(1.0));
        assert!(picked.iter().any(|came| came == "group") && picked.iter().any(|came| came == "emoji"), "{picked:?}");

        // Having reacted, with a like or an emoji: once is enough.
        for reaction in [Reaction::Like, Reaction::Emoji("🤌".into())] {
            let reacted = clicked_down_reacting(&said, Some(reaction), None, heart);
            assert!(reacted.iter().all(|came| came != "reactions"), "{reacted:?}");
        }
    }

    #[test]
    fn an_open_menu_and_the_relays_a_note_was_seen_on_take_no_room_in_its_row() {
        let said = note("worth passing on");
        let relay = |name: &str| RelayUrl::parse(&format!("wss://{name}.example.com")).unwrap();
        let height = |menu_open: bool, seen: usize| {
            let renderer = crate::views::components::testing::renderer();
            let (profiles, images, previews, engagement) = (HashMap::new(), no_images(), no_previews(), HashMap::new());
            let seen_on = HashMap::from([(said.id, (0..seen).map(|number| relay(&format!("r{number}"))).collect())]);
            let videos = Videos::default();
            let expanded = Expanded::default();
            let context = RowContext {
                profiles: &profiles,
                images: &images,
                previews: &previews,
                videos: &videos,
                engagement: &engagement,
                seen_on: &seen_on,
                now: 0,
                accent: Color::WHITE,
                can_publish: true,
                repost_menu: menu_open.then_some(RepostMenu::of(said.id)),
                who: None,
                reacting: Reacting::NONE,
                expanded: &expanded,
                marked: None,
                share_copied: None,
                picture_copied: None,
                bookmarking: Bookmarking::Unavailable,
                menu_open: false,
                sizes: RowSizes::new(SIZE),
            };
            let mut element = note_row(&said, &context);
            let mut tree = Tree::new(element.as_widget());
            let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
            element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
        };
        let plain = height(false, 0);
        assert_eq!(height(true, 0), plain, "nothing in the list moves when the menu opens");
        // Nor when it is found out where a note is.
        assert_eq!(height(false, 3), plain);
        assert_eq!(height(false, 12), plain);
    }

    /// A store in which the picture at `url`, of `size` pixels, has arrived.
    fn images_with(url: &str, (width, height): (u32, u32)) -> ImageStore {
        let mut images = no_images();
        let request = ImageRequest { url: url.to_string(), shape: ImageShape::Original };
        images.begin(&request);
        let pixels = vec![0; width as usize * height as usize * 4];
        images.finish(request, Ok(LoadedImage { handle: Handle::from_rgba(width, height, pixels), width, height }));
        images
    }

    /// Where every part of the row of `note` that is `height` tall is laid
    /// out, in a list as wide as the popup's: a picture and what holds it.
    fn parts_as_tall_as(note: &Note, images: &ImageStore, height: f32) -> Vec<Rectangle> {
        fn gather(node: &layout::Node, origin: Vector, height: f32, found: &mut Vec<Rectangle>) {
            let bounds = node.bounds() + origin;
            if (bounds.height - height).abs() < 0.01 {
                found.push(bounds);
            }
            for child in node.children() {
                gather(child, Vector::new(bounds.x, bounds.y), height, found);
            }
        }
        let renderer = crate::views::components::testing::renderer();
        let (profiles, previews, engagement, seen_on) = (HashMap::new(), no_previews(), HashMap::new(), HashMap::new());
        let videos = Videos::default();
        let expanded = Expanded::default();
        let context = RowContext {
            profiles: &profiles,
            images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        let mut element = note_row(note, &context);
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
        let node = element.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let mut found = Vec::new();
        gather(&node, Vector::ZERO, height, &mut found);
        found
    }

    #[test]
    fn a_picture_reaches_from_one_edge_of_its_row_to_the_other_unless_it_is_narrower_itself() {
        let url = "https://example.com/cat.png";
        let with_picture = note(&format!("a cat\n{url}"));
        // Wide, tall, a little wider than the row, and exactly as wide: all
        // fill the row, and are as tall as that makes them, however tall.
        for (size, tall) in [((1200, 800), 614.0 * 800.0 / 1200.0), ((1000, 3000), 1842.0), ((615, 100), 614.0 * 100.0 / 615.0)] {
            let parts = parts_as_tall_as(&with_picture, &images_with(url, size), tall);
            assert!(!parts.is_empty(), "a picture of {size:?} is {tall} tall");
            for part in parts {
                assert_eq!((part.x, part.width), (0.0, WIDTH), "a picture of {size:?} touches both edges");
            }
        }
        // Narrower than the row: as large as it is, at the left, and not
        // made any larger.
        for size in [(200, 100), (613, 900), (40, 300)] {
            let parts = parts_as_tall_as(&with_picture, &images_with(url, size), size.1 as f32);
            assert!(!parts.is_empty(), "a picture of {size:?} keeps its height");
            for part in parts {
                assert_eq!((part.x, part.width), (0.0, size.0 as f32), "a picture of {size:?} keeps its width");
            }
        }
    }

    #[test]
    fn a_row_keeps_its_height_when_a_picture_of_known_size_arrives() {
        let url = "https://example.com/cat.png";
        let plain = measured_height(&note("a cat"));
        // Wide, tall and small: the room kept is the room taken.
        for size in [(1200, 800), (1000, 3000), (200, 100), (614, 614)] {
            let mut with_picture = note(&format!("a cat\n{url}"));
            with_picture.content.images[0].size = Some(size);

            let loading = measured_height_with(&with_picture, &no_images(), SIZE, WIDTH);
            let room = shown_size(size, WIDTH).height + f32::from(NOTE_SPACING);
            assert!((loading - plain - room).abs() < 0.01, "room is kept for a picture of {size:?}: {loading}");

            let arrived = measured_height_with(&with_picture, &images_with(url, size), SIZE, WIDTH);
            assert!((arrived - loading).abs() < 0.01, "nothing moves when a picture of {size:?} arrives: {arrived}");
        }
    }

    #[test]
    fn a_click_on_a_picture_opens_the_note_as_one_on_its_text_does() {
        let url = "https://example.com/cat.png";
        let with_picture = note(&format!("a cat\n{url}"));
        let (profiles, previews, videos) = (HashMap::new(), no_previews(), Videos::default());
        let (engagement, seen_on, expanded) = (HashMap::new(), HashMap::new(), Expanded::default());
        let down_the_middle = |images: &ImageStore| {
            let context = RowContext {
                profiles: &profiles,
                images,
                previews: &previews,
                videos: &videos,
                engagement: &engagement,
                seen_on: &seen_on,
                now: 0,
                accent: Color::WHITE,
                can_publish: true,
                repost_menu: None,
                who: None,
                reacting: Reacting::NONE,
                expanded: &expanded,
                marked: None,
                share_copied: None,
                picture_copied: None,
                bookmarking: Bookmarking::Unavailable,
                menu_open: false,
                sizes: RowSizes::new(SIZE),
            };
            clicked_down(WIDTH / 2.0, |point| clicked_in(&with_picture, &context, point))
        };
        // Down the middle of the row, text and picture alike: the note,
        // and nothing else, least of all the picture in the browser.
        assert_eq!(down_the_middle(&images_with(url, (1200, 300))), ["thread", ""]);
        // The same until the picture is here.
        assert_eq!(down_the_middle(&no_images()), ["thread", ""]);
    }

    #[test]
    fn a_picture_that_failed_leaves_a_link_and_no_gap() {
        let url = "https://example.com/gone.png";
        let with_picture = note(&format!("a cat\n{url}"));
        let mut images = no_images();
        let request = ImageRequest { url: url.to_string(), shape: ImageShape::Original };
        images.begin(&request);
        images.finish(request, Err("404".into()));

        let failed = measured_height_with(&with_picture, &images, SIZE, WIDTH);
        assert!(failed < measured_height(&note("a cat")) + 40.0, "one line of link, not a picture's room: {failed}");
    }

    #[test]
    fn the_estimate_is_close_to_the_measured_row() {
        // The estimate only has to be near: the list corrects it when the row
        // comes into view, and what is on screen moves by the difference when
        // scrolling back up after a jump.
        for text in [
            "gm".to_string(),
            "the quick brown fox jumps over the lazy dog ".repeat(3),
            "the quick brown fox jumps over the lazy dog ".repeat(12),
            "one\ntwo\nthree\nfour\nfive".to_string(),
            "a paragraph of ordinary words follows.\n\nand another one, a little longer than the first, to wrap.".to_string(),
            "links count as shown: https://example.com/some/long/path/that/is/cut/to/fifty/characters/at/most and #tags too"
                .to_string(),
            "a picture of unknown size\nhttps://example.com/cat.png".to_string(),
            "https://example.com/only-a-picture.png".to_string(),
            "a video, of no size it says\nhttps://example.com/clip.mp4".to_string(),
            "and one of YouTube's\nhttps://youtu.be/aqz-KE-bpKQ".to_string(),
            // Marked: a heading, what is stressed, code in a line and in
            // lines of its own, something quoted, items, a rule.
            "## What is new\nIt is **fully** keyboard-navigable, *at last*, and has a `--toggle` flag.\n\n\
             > notes and other stuff\n* a feed\n* threads\n---\n```\nfn main() {\n    println!(\"gm\");\n}\n```"
                .to_string(),
            "**bold words all the way along the line and round to the next one, twice over** ".repeat(3),
            // Long enough to be cut, several times over, in lines and in
            // paragraphs.
            "the quick brown fox jumps over the lazy dog ".repeat(60),
            "a paragraph of ordinary words, as long as they come in a note that has a lot to say, and then says it.\n\n\
             and a shorter one after it.\n\n"
                .repeat(20),
        ] {
            let quoted = quotable(&text);
            let plain = note(&text);
            let nested = Note { id: EventId::from_byte_array([6; 32]), ..quoting(&text, &quoted, true) };
            for note in [
                plain.clone(),
                quoting("see", &quoted, false),
                quoting("see", &quoted, true),
                quoting(&text, &plain, true),
                quoting("see", &nested, true),
            ] {
                // At every size the settings offer, in a popup at its narrowest
                // and at its widest.
                for (size, width) in [(10, 394.0), (13, 394.0), (24, 394.0), (10, 614.0), (13, 614.0), (18, 614.0), (24, 614.0)] {
                    // As a row first shows it, and in full, which is
                    // another thing only of a long note.
                    for in_full in [false, true] {
                        let estimated = estimated_note_height(&note, RowSizes::new(size), width, in_full);
                        let measured = match in_full {
                            true => measured_height_in_full(&note, size, width),
                            false => measured_height_with(&note, &no_images(), size, width),
                        };
                        assert!(
                            (estimated - measured).abs() <= measured * 0.3,
                            "estimated {estimated}, measured {measured} at size {size}, width {width}, in full: {in_full}, \
                             for {text:?}, quoting {}",
                            note.quoted.len()
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn what_stands_behind_code_takes_no_room() {
        // Code is set in letters of another width, so a line of it may
        // break elsewhere; what is drawn behind it adds nothing to that.
        let row = |text: &str| measured_height_with(&note(text), &no_images(), SIZE, WIDTH);
        assert_eq!(row("run `this` now"), row("run this now"));
        assert_eq!(row("*slanted*, **bold**, ~~struck~~"), row("slanted, bold, struck"));
    }

    /// A note a little longer than a row shows: cut, and not much taller
    /// in full.
    fn long_note() -> Note {
        let long = note(&"the quick brown fox jumps over the lazy dog ".repeat(20));
        assert!(long.content.whole.is_some(), "it is cut");
        long
    }

    /// What comes of clicks down the middle of the row of `note`, shown
    /// in full or not.
    fn clicked_down_the_middle(note: &Note, in_full: bool) -> Vec<String> {
        let (profiles, images, previews, videos) = (HashMap::new(), no_images(), no_previews(), Videos::default());
        let (engagement, seen_on) = (HashMap::new(), HashMap::new());
        let mut expanded = Expanded::default();
        if in_full {
            expanded.expand(note.id);
        }
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        // Under the row a click comes to nothing, and is not told.
        let mut told = clicked_down(WIDTH / 2.0, |point| clicked_in(note, &context, point));
        told.retain(|came| !came.is_empty());
        told
    }

    #[test]
    fn a_long_note_has_a_button_under_its_text_that_shows_the_rest_and_one_that_shows_less() {
        let long = long_note();
        // Down the middle of the row: the note, then the button, then the
        // note again down to the room between the actions.
        assert_eq!(clicked_down_the_middle(&long, false), ["thread", "show more", "thread"]);
        assert_eq!(clicked_down_the_middle(&long, true), ["thread", "show less", "thread"]);

        // A note that is all there has neither, whatever is asked of it.
        let short = note("gm");
        assert_eq!(clicked_down_the_middle(&short, false), ["thread"]);
        assert_eq!(clicked_down_the_middle(&short, true), ["thread"]);

        // A long note quoted in another is cut, and that is that: its
        // own view has the rest.
        let quoted = Note { id: EventId::from_byte_array([8; 32]), ..long_note() };
        assert_eq!(clicked_down_the_middle(&quoting("see", &quoted, true), false), ["thread"]);
    }

    #[test]
    fn the_button_under_a_long_note_takes_its_room_and_no_more_in_full() {
        for size in [10, 13, 18, 24] {
            let long = long_note();
            let room = fold_height(RowSizes::new(size)) + f32::from(NOTE_SPACING);

            // Cut: as tall as a note that says what is shown of it, and
            // the button under that.
            let as_shown = Note { content: content::Content { whole: None, ..long.content.clone() }, ..long.clone() };
            let cut = measured_height_with(&long, &no_images(), size, WIDTH);
            let plain = measured_height_with(&as_shown, &no_images(), size, WIDTH);
            assert!((cut - plain - room).abs() < 0.01, "at size {size}: {cut} against {plain}");

            // In full: as tall as a note that says all of it, and the
            // button under that. What keeps the row's place takes none.
            let whole = long.content.whole.clone().unwrap();
            let all =
                content::Content { segments: whole.segments, line_chars: whole.line_chars, whole: None, ..long.content.clone() };
            let in_full = measured_height_in_full(&long, size, WIDTH);
            let plain = measured_height_with(&Note { content: all, ..long.clone() }, &no_images(), size, WIDTH);
            assert!((in_full - plain - room).abs() < 0.01, "at size {size}: {in_full} against {plain}");
            assert!(in_full > cut, "there is more to read");
        }
    }

    #[test]
    fn the_row_of_a_note_in_full_keeps_how_far_its_top_is_above_what_is_in_view() {
        let long = long_note();
        let (profiles, images, previews, videos) = (HashMap::new(), no_images(), no_previews(), Videos::default());
        let (engagement, seen_on) = (HashMap::new(), HashMap::new());
        let mut expanded = Expanded::default();
        expanded.expand(long.id);
        let context = RowContext {
            profiles: &profiles,
            images: &images,
            previews: &previews,
            videos: &videos,
            engagement: &engagement,
            seen_on: &seen_on,
            now: 0,
            accent: Color::WHITE,
            can_publish: true,
            repost_menu: None,
            who: None,
            reacting: Reacting::NONE,
            expanded: &expanded,
            marked: None,
            share_copied: None,
            picture_copied: None,
            bookmarking: Bookmarking::Unavailable,
            menu_open: false,
            sizes: RowSizes::new(SIZE),
        };
        let renderer = crate::views::components::testing::renderer();
        // The row as a list shows it: in what opens its thread.
        let mut row = opening_thread(note_row(&long, &context), long.id, &context);
        let mut tree = Tree::new(row.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
        // The row starts 1000 down the list.
        let node = row.as_widget_mut().layout(&mut tree, &renderer, &limits).move_to(Point::new(0.0, 1000.0));
        let mut seen_from = |view_top: f32| {
            let mut messages = Vec::new();
            row.as_widget_mut().update(
                &mut tree,
                &Event::Mouse(mouse::Event::CursorMoved { position: Point::ORIGIN }),
                layout::Layout::new(&node),
                mouse::Cursor::Unavailable,
                &renderer,
                &mut clipboard::Null,
                &mut cosmic::iced::core::Shell::new(&mut messages),
                &Rectangle::new(Point::new(0.0, view_top), Size::new(WIDTH, 500.0)),
            );
            expanded.above(&long.id).map(std::cell::Cell::get)
        };
        // Its top in view, at the top of the view, and scrolled out.
        assert_eq!(seen_from(800.0), Some(0.0));
        assert_eq!(seen_from(1000.0), Some(0.0));
        assert_eq!(seen_from(1130.0), Some(130.0));
    }
}

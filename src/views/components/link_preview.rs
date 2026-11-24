// SPDX-License-Identifier: MIT

//! The preview of a web page a note links to: a card under the note with
//! the picture the page is shared with, the site it is on, its title and a
//! few words about it. A click opens the page.
//!
//! The card is as tall for one page as for another, and as tall while the
//! page is being asked as when it has answered: a row doesn't change height
//! because a preview arrived, or its picture did. A page with nothing to
//! say for itself gets no card, and the room kept for one is given back.
//!
//! A link that answered as a video is no page: its card offers to play it,
//! and pressed, gives way to the video (see
//! [`video_box`](mod@crate::views::components::video_box)).

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Color, ContentFit, Length, mouse};
use cosmic::widget::{self, container, text};

use crate::fl;
use crate::images::{ImageShape, ImageState, THUMBNAIL_ASPECT};
use crate::messages::Message;
use crate::previews::PreviewState;
use crate::views::components::note_row::{RowContext, RowSizes, line_height};
use crate::views::components::{CARD_BORDER, CARD_RADIUS, MediaPlaceholder, card_style};

/// The room between the card's edge and what it says, and between the
/// lines it says it in.
const PREVIEW_PADDING: f32 = 8.0;
const PREVIEW_SPACING: f32 = 2.0;

/// How many lines of a page's description the card shows.
const PREVIEW_DESCRIPTION_LINES: f32 = 2.0;

/// How much dimmer than the text the site and the description are.
const PREVIEW_DIM: f32 = 0.7;

/// How tall the preview of a link is in a row set in `sizes`, whatever the
/// page says: the site, the title and the lines of description, and the
/// room around them.
pub fn link_preview_height(sizes: RowSizes) -> f32 {
    2.0 * (CARD_BORDER + PREVIEW_PADDING)
        + line_height(sizes.preview_site())
        + line_height(sizes.preview_title())
        + PREVIEW_DESCRIPTION_LINES * line_height(sizes.preview_description())
        + 2.0 * PREVIEW_SPACING
}

/// One line of the card, cut where the card ends rather than wrapped.
fn cut_line<'a>(line: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(line).width(Length::Fill).clip(true).into()
}

/// The card of a link that turned out to be a video: as tall as any
/// preview, saying what it is and where, and playing it when it is
/// pressed.
fn video_offer<'a>(url: &'a str, context: &RowContext<'a>) -> Element<'a, Message> {
    let sizes = context.sizes;
    let inside = link_preview_height(sizes) - 2.0 * CARD_BORDER;
    let play = container(widget::icon::from_name("media-playback-start-symbolic").size(24))
        .center_x(Length::Fixed(inside))
        .center_y(Length::Fixed(inside));

    let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
    let dim = cosmic::theme::Text::Color(Color { a: PREVIEW_DIM, ..text_color });
    let site = reqwest::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default();
    let site = text(site).size(sizes.preview_site()).class(dim).wrapping(Wrapping::None);
    let title = text(fl!("video-offer")).size(sizes.preview_title()).font(cosmic::font::bold()).wrapping(Wrapping::None);
    let words = widget::Column::new().push(cut_line(site)).push(cut_line(title)).spacing(PREVIEW_SPACING);
    let words = container(words).padding(PREVIEW_PADDING).width(Length::Fill).center_y(Length::Fixed(inside));

    let card = container(widget::Row::new().push(play).push(words))
        .padding(CARD_BORDER)
        .width(Length::Fill)
        .height(Length::Fixed(link_preview_height(sizes)))
        .class(card_style());
    widget::mouse_area(card).on_press(Message::VideoPress(url.to_string())).interaction(mouse::Interaction::Pointer).into()
}

/// The preview of the page at `url`, for under the note that links to it:
/// the card, once the page has said what it is; the room the card will
/// take, until then; and nothing, if the page has nothing to say.
pub fn link_preview<'a>(url: &'a str, context: &RowContext<'a>) -> Option<Element<'a, Message>> {
    let RowContext { previews, images, sizes, .. } = *context;
    let height = link_preview_height(sizes);
    let preview = match previews.get(url) {
        PreviewState::Ready(preview) => preview,
        PreviewState::Loading => return Some(MediaPlaceholder::new(None, height).radius(CARD_RADIUS).into()),
        PreviewState::Video => return Some(video_offer(url, context)),
        PreviewState::Nothing => return None,
    };

    // The picture fills the card's left end, top to bottom, cut to the
    // proportions it is asked for in. A picture that can't be had leaves
    // the card to the words.
    let inside = height - 2.0 * CARD_BORDER;
    let across = (inside * THUMBNAIL_ASPECT.0 as f32 / THUMBNAIL_ASPECT.1 as f32).round();
    let left_corners = [CARD_RADIUS - CARD_BORDER, 0.0, 0.0, CARD_RADIUS - CARD_BORDER];
    let picture: Option<Element<'a, Message>> =
        preview.image.as_deref().and_then(|image| match images.get(image, ImageShape::Thumbnail) {
            ImageState::Ready(loaded) => Some(
                widget::image(loaded.handle.clone())
                    .width(Length::Fixed(across))
                    .height(Length::Fixed(inside))
                    .content_fit(ContentFit::Fill)
                    .border_radius(left_corners)
                    .into(),
            ),
            ImageState::Loading => Some(container(MediaPlaceholder::new(None, inside)).width(Length::Fixed(across)).into()),
            ImageState::Failed => None,
        });

    let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
    let dim = cosmic::theme::Text::Color(Color { a: PREVIEW_DIM, ..text_color });
    let site = text(preview.site.as_str()).size(sizes.preview_site()).class(dim).wrapping(Wrapping::None);
    let title = text(preview.title.as_str()).size(sizes.preview_title()).font(cosmic::font::bold()).wrapping(Wrapping::None);
    let mut words = widget::Column::new().push(cut_line(site)).push(cut_line(title)).spacing(PREVIEW_SPACING);
    if let Some(description) = &preview.description {
        // As many lines as fit the room it is given, and no part of the
        // next.
        let lines = PREVIEW_DESCRIPTION_LINES * line_height(sizes.preview_description());
        let description = text(description.as_str()).size(sizes.preview_description()).class(dim).wrapping(Wrapping::WordOrGlyph);
        words = words.push(container(description).width(Length::Fill).height(Length::Fixed(lines)).clip(true));
    }
    let words = container(words).padding(PREVIEW_PADDING).width(Length::Fill).height(Length::Fixed(inside));

    let card = container(widget::Row::new().push_maybe(picture).push(words))
        .padding(CARD_BORDER)
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .class(card_style());
    Some(widget::mouse_area(card).on_press(Message::OpenUrl(url.to_string())).interaction(mouse::Interaction::Pointer).into())
}

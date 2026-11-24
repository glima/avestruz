// SPDX-License-Identifier: MIT

//! A video a note links to, in the note's row: the room it plays in, and
//! under that a bar to play it by.
//!
//! Until it is played there is a play button, over the picture the note
//! gave for it if it gave one, or one of YouTube's if it is there, and
//! nothing of the video has been asked for.
//! Pressed, it plays where it is (see [`crate::video`]): the bar then has
//! play and pause, how far in it is and of how long, a slider to go
//! elsewhere in it, and its sound: a button that turns it off and on and
//! says how loud it is, and beside it, while the pointer is over either,
//! a slider that sets that. A click on the picture holds it and lets it
//! go, as the button does. Beside the sound is how fast it plays, which
//! opens a menu of speeds. At the end of the bar, either way, the
//! link it came as opens in the browser.
//!
//! The room is the same before, while and after: what the note says the
//! video's proportions are, or the usual ones, across the whole row (see
//! [`box_size`]). What plays is fitted inside it, between bars if it is
//! another shape, so nothing under it moves because it was played.

use std::borrow::Cow;

use cosmic::Element;
use cosmic::iced::core::widget::{Operation, Tree};
use cosmic::iced::core::{Clipboard, Layout, Rectangle, Shell, Size, Vector, Widget, layout, overlay, renderer};
use cosmic::iced::widget::svg;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Background, Border, Color, ContentFit, Event, Length, Point, mouse};
use cosmic::widget::popover::{Position, popover};
use cosmic::widget::{self, button, container, icon, text};

use crate::fl;
use crate::helpers::truncate_chars;
use crate::images::{ImageShape, ImageState};
use crate::messages::Message;
use crate::video::{Loudness, Playback, Speed, box_size, clock};
use crate::views::components::note_row::RowContext;
use crate::views::components::{CARD_BORDER, CARD_RADIUS, MENU_GAP, card_style, choice_menu};

/// How tall the bar under a video is, and the room at its ends and
/// between what is on it.
const VIDEO_BAR_HEIGHT: f32 = 36.0;
const VIDEO_BAR_PADDING: u16 = 4;
const VIDEO_BAR_SPACING: u16 = 6;

/// The play button over a video that is not playing: how large its icon
/// is, the room around it, and how dark the disc it sits on.
const PLAY_ICON_SIZE: u16 = 32;
const PLAY_PADDING: u16 = 12;
const PLAY_DISC: f32 = 0.6;

/// How wide the slider is that sets how loud a video is.
const VOLUME_WIDTH: f32 = 64.0;

/// The room the speed a video plays at has on the bar, whichever speed it
/// is, so that nothing beside it moves when another is chosen; and how
/// wide the menu of speeds is.
const SPEED_SIZE: (f32, f32) = (44.0, 24.0);
const SPEED_MENU_WIDTH: f32 = 112.0;

/// How much of why a video can't be played is said in its place.
const VIDEO_REASON_MAX_CHARS: usize = 160;

/// How tall a video is in a row `width` wide, bar and all, when its note
/// says it is `size` pixels, or doesn't say.
pub fn video_height(size: Option<(u32, u32)>, width: f32) -> f32 {
    box_size(size, (width - 2.0 * CARD_BORDER).max(1.0)).height + VIDEO_BAR_HEIGHT + 2.0 * CARD_BORDER
}

/// What is in it, in the room a video of `size` pixels is given: as wide
/// as there is room, and as tall as [`box_size`] makes that.
struct Screen<'a> {
    size: Option<(u32, u32)>,
    content: Element<'a, Message>,
}

impl Widget<Message, cosmic::Theme, cosmic::Renderer> for Screen<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &cosmic::Renderer, limits: &layout::Limits) -> layout::Node {
        let size = box_size(self.size, limits.max().width);
        let within = layout::Limits::new(Size::ZERO, size);
        let content = tree.children.first_mut().map(|tree| self.content.as_widget_mut().layout(tree, renderer, &within));
        layout::Node::with_children(size, content.into_iter().collect())
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &cosmic::Renderer, operation: &mut dyn Operation) {
        if let (Some(tree), Some(content)) = (tree.children.first_mut(), layout.children().next()) {
            self.content.as_widget_mut().operate(tree, content, renderer, operation);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &cosmic::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let (Some(tree), Some(content)) = (tree.children.first_mut(), layout.children().next()) {
            self.content.as_widget_mut().update(tree, event, content, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut cosmic::Renderer,
        theme: &cosmic::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let (Some(tree), Some(content)) = (tree.children.first(), layout.children().next()) {
            self.content.as_widget().draw(tree, renderer, theme, style, content, cursor, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &cosmic::Renderer,
    ) -> mouse::Interaction {
        match (tree.children.first(), layout.children().next()) {
            (Some(tree), Some(content)) => self.content.as_widget().mouse_interaction(tree, content, cursor, viewport, renderer),
            _ => mouse::Interaction::None,
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &cosmic::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, cosmic::Theme, cosmic::Renderer>> {
        let (tree, content) = (tree.children.first_mut()?, layout.children().next()?);
        self.content.as_widget_mut().overlay(tree, content, renderer, viewport, translation)
    }
}

/// An icon in `color`, `size` across.
fn tinted<'a>(name: &'static str, size: u16, color: Color) -> Element<'a, Message> {
    icon::icon(icon::from_name(name).handle())
        .size(size)
        .class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(color) }))
        .into()
}

/// The play button over a video: white, on a dark disc, so that it shows
/// on whatever picture is under it.
fn play_button<'a>() -> Element<'a, Message> {
    let disc = f32::from(PLAY_ICON_SIZE + 2 * PLAY_PADDING);
    on_dark(tinted("media-playback-start-symbolic", PLAY_ICON_SIZE, Color::WHITE), disc)
}

/// `shown`, in the middle of the room a video has.
fn centred<'a>(shown: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(shown).center(Length::Fill).into()
}

/// A picture fitted into the room a video has, whole.
fn fitted<'a>(handle: widget::image::Handle) -> Element<'a, Message> {
    widget::image(handle).width(Length::Fill).height(Length::Fill).content_fit(ContentFit::Contain).into()
}

/// The picture that stands for a video until it plays: the first of
/// `posters` that can be had, once it is here. The next is only asked
/// for when the one before it could not be had.
fn poster(posters: &[String], context: &RowContext<'_>) -> Option<widget::image::Handle> {
    let first = posters.iter().find_map(|poster| match context.images.get(poster, ImageShape::Original) {
        ImageState::Ready(loaded) => Some(Some(loaded.handle.clone())),
        ImageState::Loading => Some(None),
        ImageState::Failed => None,
    });
    first.flatten()
}

/// `shown`, on a dark ground that shows on whatever picture is under it,
/// as the play button's does.
fn on_dark<'a>(shown: impl Into<Element<'a, Message>>, radius: f32) -> Element<'a, Message> {
    container(shown)
        .padding(PLAY_PADDING)
        .class(cosmic::theme::Container::custom(move |_theme| container::Style {
            background: Some(Background::Color(Color { a: PLAY_DISC, ..Color::BLACK })),
            border: Border { radius: radius.into(), ..Border::default() },
            ..container::Style::default()
        }))
        .into()
}

/// What a video shows where it plays: its latest picture, or before it has
/// one a picture that stands for it; and over that the play button while
/// it is held, that it is on its way while it is, or why it can't be
/// played, which is said on its own.
fn screen<'a>(playback: Option<&'a Playback>, posters: &'a [String], context: &RowContext<'a>) -> Element<'a, Message> {
    let failed = playback.is_some_and(|playback| playback.error.is_some());
    let picture = match playback.and_then(|playback| playback.frame.clone()) {
        _ if failed => None,
        Some(frame) => Some(frame),
        None => poster(posters, context),
    };
    let said = |said: String| text(said).size(context.sizes.time()).align_x(Alignment::Center).wrapping(Wrapping::WordOrGlyph);
    let over: Option<Element<'a, Message>> = match playback {
        Some(Playback { error: Some(why), .. }) => {
            let why = truncate_chars(why.lines().next().unwrap_or_default(), VIDEO_REASON_MAX_CHARS);
            Some(container(said(fl!("video-failed", error = why.as_ref()))).padding(PLAY_PADDING).into())
        }
        Some(playback) if playback.loading() => Some(on_dark(said(fl!("video-loading")), CARD_RADIUS)),
        Some(playback) if playback.playing => None,
        _ => Some(play_button()),
    };

    let layers = cosmic::iced::widget::stack(picture.map(fitted).into_iter().chain(over.map(centred)));
    container(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(cosmic::theme::Container::custom(|_theme| container::Style {
            background: Some(Background::Color(Color::BLACK)),
            text_color: Some(Color::WHITE),
            border: Border {
                radius: [CARD_RADIUS - CARD_BORDER, CARD_RADIUS - CARD_BORDER, 0.0, 0.0].into(),
                ..Border::default()
            },
            ..container::Style::default()
        }))
        .into()
}

/// A button of the bar: an icon, what it does for whoever points at it,
/// and what pressing it asks of the app.
fn bar_button<'a>(name: &'static str, tooltip: String, on_press: Message) -> Element<'a, Message> {
    button::icon(icon::from_name(name)).extra_small().tooltip(tooltip).on_press(on_press).into()
}

/// The sound of a video that was played: a button that turns it off and
/// on, with a picture of how loud it is; and before it, while the pointer
/// is over either or the slider is being dragged, a slider that sets how
/// loud. The slider takes its room
/// from the one beside it, so nothing else on the bar moves.
fn sound<'a>(playback: &'a Playback) -> Element<'a, Message> {
    let (picture, tooltip) = match playback.loudness() {
        Loudness::Off if playback.muted => ("audio-volume-muted-symbolic", fl!("video-unmute")),
        Loudness::Off => ("audio-volume-muted-symbolic", fl!("video-mute")),
        Loudness::Low => ("audio-volume-low-symbolic", fl!("video-mute")),
        Loudness::Medium => ("audio-volume-medium-symbolic", fl!("video-mute")),
        Loudness::High => ("audio-volume-high-symbolic", fl!("video-mute")),
    };
    let slider = playback.sound_shows().then(|| {
        widget::slider(0.0..=1.0, playback.volume, Message::VideoVolume)
            .step(0.01)
            .on_release(Message::VideoVolumeSet)
            .width(Length::Fixed(VOLUME_WIDTH))
    });
    let controls = widget::Row::new()
        .push_maybe(slider)
        .push(bar_button(picture, tooltip, Message::VideoMute))
        .spacing(VIDEO_BAR_SPACING)
        .align_y(Alignment::Center);
    // A press between the two is nobody's: it is not the video's to play
    // or hold.
    widget::mouse_area(controls)
        .on_enter(Message::VideoSound(true))
        .on_exit(Message::VideoSound(false))
        .on_press(Message::VideoSound(true))
        .into()
}

/// How fast videos play, to press: it opens a menu of the speeds there
/// are, under it, or over it where there is no room under. A speed that
/// is not the usual one is set in the accent colour: it holds for the
/// next video too.
fn speed<'a>(context: &RowContext<'a>) -> Element<'a, Message> {
    let (chosen, small) = (context.videos.speed(), context.sizes.time());
    let label = text(chosen.label()).size(small).wrapping(Wrapping::None);
    let label = if chosen.is_usual() { label } else { label.class(cosmic::theme::Text::Color(context.accent)) };
    let (across, down) = SPEED_SIZE;
    let shown = button::custom(container(label).center(Length::Fill))
        .padding(0)
        .width(Length::Fixed(across))
        .height(Length::Fixed(down))
        .class(cosmic::theme::Button::Text)
        .on_press(Message::VideoSpeedMenu);
    let shown = widget::tooltip(shown, text(fl!("video-speed")).size(12), widget::tooltip::Position::Top);

    let under = Point::new(0.0, down + MENU_GAP);
    let mut shown = popover(shown).position(Position::Point(under)).on_close(Message::CloseVideoSpeedMenu);
    if context.videos.speed_menu() {
        let speeds = Speed::ALL.into_iter().map(|speed| (speed.label(), speed == chosen, Message::VideoSpeed(speed)));
        shown = shown.popup(choice_menu(speeds, SPEED_MENU_WIDTH));
    }
    shown.into()
}

/// The bar under a video: play or pause; then, of a video that was
/// played, how far in it is, a slider to go elsewhere in it once it is
/// known how long it is, its sound, and, with the slider, how fast it
/// plays: a live one has no length, so neither; of one that
/// wasn't, the link it came as. At the end, opening that link in the browser.
fn bar<'a>(url: &'a str, label: Cow<'a, str>, playback: Option<&'a Playback>, context: &RowContext<'a>) -> Element<'a, Message> {
    let playing = playback.is_some_and(|playback| playback.playing);
    let toggle = if playing {
        bar_button("media-playback-pause-symbolic", fl!("video-pause"), Message::VideoPress(url.to_string()))
    } else {
        bar_button("media-playback-start-symbolic", fl!("video-play"), Message::VideoPress(url.to_string()))
    };
    let mut bar = widget::Row::new().push(toggle);

    let small = context.sizes.time();
    match playback.filter(|playback| playback.error.is_none()) {
        Some(playback) => {
            let at = playback.seeking.unwrap_or(playback.position);
            let time = match playback.duration {
                Some(all) => format!("{} / {}", clock(at), clock(all)),
                None => clock(at),
            };
            bar = bar.push(text(time).size(small).wrapping(Wrapping::None));
            bar = match playback.duration {
                Some(all) => bar.push(
                    widget::slider(0.0..=all, at.min(all), Message::VideoSeeking)
                        .step(0.1)
                        .on_release(Message::VideoSeek)
                        .width(Length::Fill),
                ),
                None => bar.push(widget::Space::new().width(Length::Fill)),
            };
            bar = bar.push(sound(playback));
            // One with no length is live: it plays at the speed it comes.
            if playback.duration.is_some() {
                bar = bar.push(speed(context));
            }
        }
        None => bar = bar.push(container(text(label).size(small).wrapping(Wrapping::None)).width(Length::Fill).clip(true)),
    }

    bar.push(bar_button("window-pop-out-symbolic", fl!("video-open"), Message::OpenUrl(url.to_string())))
        .spacing(VIDEO_BAR_SPACING)
        .padding([0, VIDEO_BAR_PADDING])
        .height(Length::Fixed(VIDEO_BAR_HEIGHT))
        .align_y(Alignment::Center)
        .into()
}

/// The video at `url`, for under the note that links to it: where it
/// plays, in the proportions of `size`, with the first of `posters` that
/// can be had standing for it until it does, and the bar it is played by, on which `label` stands for
/// its address. A click anywhere on it that is not on something of the
/// bar plays it, or holds it.
pub fn video_box<'a>(
    url: &'a str,
    label: impl Into<Cow<'a, str>>,
    size: Option<(u32, u32)>,
    posters: &'a [String],
    context: &RowContext<'a>,
) -> Element<'a, Message> {
    // Being built is being on screen: asking for it here is what keeps it
    // playing.
    let playback = context.videos.shown(url);
    let screen: Element<'a, Message> = Element::new(Screen { size, content: screen(playback, posters, context) });
    let card = container(widget::Column::new().push(screen).push(bar(url, label.into(), playback, context)))
        .padding(CARD_BORDER)
        .width(Length::Fill)
        .class(card_style());
    // Not while a menu is open over the list: a click beside the menu is
    // to put it away, and no more.
    let area = widget::mouse_area(card);
    if context.under_a_menu() {
        area.into()
    } else {
        area.on_press(Message::VideoPress(url.to_string())).interaction(mouse::Interaction::Pointer).into()
    }
}

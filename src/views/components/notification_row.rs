// SPDX-License-Identifier: MIT

//! One thing the account is told of, as a row of the notifications list.
//!
//! What is itself a note (a reply, a mention, a quote) shows as the note
//! it is, under a line that says which of those it is: it can be read in
//! full, answered and reacted to where it stands. What is only about a
//! note (a reaction, a repost, a zap) is one short row: what was done,
//! who did it and when, what was said with a zap, and the start of the
//! note it is about, which a click on the row opens.
//!
//! A row that is new, written since the notifications were last looked
//! at, has a tint of the accent colour behind it. The tint takes no room:
//! a row is as tall new as old.
//!
//! Every line of a short row is one line, cut where it runs out of room,
//! so that the row is as tall as [`estimated_notification_height`] says
//! before it is ever laid out.

use cosmic::Element;
use cosmic::iced::widget::svg;
use cosmic::iced::widget::text::{Shaping, Wrapping};
use cosmic::iced::{Alignment, Background, Color, Length, mouse};
use cosmic::widget::{self, container, icon, text};

use crate::fl;
use crate::helpers::relative_time;
use crate::messages::Message;
use crate::nostr::engagement::Reaction;
use crate::nostr::models::display_name;
use crate::nostr::notifications::{Notification, What};
use crate::nostr::outbox::Whereabouts;
use crate::views::components::note_row::{
    NOTE_ACTION_DIM, NOTE_AVATAR_GAP, NOTE_AVATAR_SIZE, NOTE_DIVIDER_HEIGHT, NOTE_PADDING_TOP, RowContext, RowSizes,
    estimated_note_height, line_height, note_row, opening_profile, opening_thread,
};
use crate::views::components::{avatar, like_icon, reply_icon, repost_icon, zap_icon};

/// The room above the line that says what a note is to the account.
const REASON_PADDING_TOP: f32 = 6.0;

/// How wide the room is for what was done in a short row: an emoji, or
/// an icon.
const GLYPH_WIDTH: f32 = 22.0;

/// The gap between the lines of a short row, and the room under them,
/// above the line that ends the row.
const LINE_GAP: f32 = 2.0;
const SHORT_PADDING_BOTTOM: f32 = 8.0;

/// How strongly the accent colour tints a row that is new.
const NEW_TINT: f32 = 0.10;

/// How tall the line above a note is, which says what the note is to the
/// account, with the room above it.
fn reason_height(sizes: RowSizes) -> f32 {
    REASON_PADDING_TOP + line_height(sizes.time())
}

/// The lines a short row has under who did it: what was said with a zap,
/// if anything was, and the start of the note it is about, if that is
/// at hand.
fn lines_under(notification: &Notification) -> (Option<&str>, Option<&str>) {
    let comment = match &notification.what {
        What::Zap { comment, .. } if !comment.is_empty() => Some(comment.as_str()),
        _ => None,
    };
    (comment, notification.preview.as_deref().filter(|preview| !preview.is_empty()))
}

/// How tall a short row is: its lines, or the picture of who did it,
/// whichever is taller, and the room around them.
fn short_height(notification: &Notification, sizes: RowSizes) -> f32 {
    let (comment, preview) = lines_under(notification);
    let mut lines = line_height(sizes.name());
    if comment.is_some() {
        lines += LINE_GAP + line_height(sizes.text());
    }
    if preview.is_some() {
        lines += LINE_GAP + line_height(sizes.time());
    }
    f32::from(NOTE_PADDING_TOP) + NOTE_AVATAR_SIZE.max(lines) + SHORT_PADDING_BOTTOM + NOTE_DIVIDER_HEIGHT
}

/// How tall the row of `notification` will be, set in `sizes` in a list
/// `width` wide: near enough, for one that is a note (all of it, if a
/// long one shows `in_full`), and exactly, for a short one.
pub fn estimated_notification_height(notification: &Notification, sizes: RowSizes, width: f32, in_full: bool) -> f32 {
    match &notification.note {
        Some(note) => reason_height(sizes) + estimated_note_height(note, sizes, width, in_full),
        None => short_height(notification, sizes),
    }
}

/// What a note is to the account, in a few words.
fn reason(what: &What) -> String {
    match what {
        What::Reply => fl!("notification-reply"),
        What::ThreadReply => fl!("notification-thread-reply"),
        What::Quote => fl!("notification-quote"),
        _ => fl!("notification-mention"),
    }
}

/// What someone did about a note of the account's, after their name.
fn deed(what: &What) -> String {
    match what {
        What::Reaction(_) => fl!("notification-reaction"),
        What::Repost => fl!("notification-repost"),
        What::Zap { sats, .. } => fl!("notification-zap", sats = sats.to_string()),
        _ => String::new(),
    }
}

/// One line of `said`, of `size`, in `color`: cut where the room ends,
/// and as tall as a line whatever it says.
fn one_line<'a>(said: impl Into<String>, size: u16, color: Color) -> Element<'a, Message> {
    let said = text(said.into()).size(size).wrapping(Wrapping::None).shaping(Shaping::Advanced);
    container(said.class(cosmic::theme::Text::Color(color)))
        .width(Length::Fill)
        .height(Length::Fixed(line_height(size)))
        .align_y(Alignment::Center)
        .clip(true)
        .into()
}

/// What was done, as the picture a short row starts with: the emoji
/// reacted with, a heart, the arrows of a repost, the bolt of a zap.
fn glyph<'a>(what: &What, accent: Color) -> Element<'a, Message> {
    let picture = |handle: icon::Handle| -> Element<'a, Message> {
        icon::icon(handle).size(18).class(cosmic::theme::Svg::custom(move |_theme| svg::Style { color: Some(accent) })).into()
    };
    let shown = match what {
        What::Reaction(Reaction::Emoji(emoji)) => text(emoji.clone()).size(18).shaping(Shaping::Advanced).into(),
        What::Reaction(Reaction::Like) => picture(like_icon(true)),
        What::Repost => picture(repost_icon()),
        What::Zap { .. } => picture(zap_icon()),
        _ => picture(reply_icon()),
    };
    container(shown).center_x(Length::Fixed(GLYPH_WIDTH)).center_y(Length::Fixed(NOTE_AVATAR_SIZE)).into()
}

/// The short row of `notification`, which is about a note and is not one.
/// A click on it opens that note's thread, or the profile of whoever did
/// it, of a zap that is for no note; their picture and their name lead to
/// their profile.
fn short_row<'a>(notification: &'a Notification, context: &RowContext<'a>) -> Element<'a, Message> {
    let RowContext { profiles, images, now, sizes, accent, .. } = *context;
    let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
    let dim = Color { a: NOTE_ACTION_DIM, ..text_color };
    let name = display_name(profiles, &notification.who);
    let picture = profiles.get(&notification.who).and_then(|profile| profile.picture.as_deref());
    let picture = opening_profile(avatar(images, picture, &notification.who, &name, NOTE_AVATAR_SIZE), notification.who);

    // Their name, as wide as it is; what they did, in what room is left;
    // and when, which is never cut.
    let who = opening_profile(
        text(name.into_owned()).size(sizes.name()).font(cosmic::font::bold()).wrapping(Wrapping::None),
        notification.who,
    );
    let head = widget::Row::new()
        .push(container(who).max_width(160.0).clip(true))
        .push(one_line(deed(&notification.what), sizes.time(), dim))
        .push(text(relative_time(now, notification.at)).size(sizes.time()).wrapping(Wrapping::None))
        .spacing(6)
        .height(Length::Fixed(line_height(sizes.name())))
        .align_y(Alignment::Center);

    let (comment, preview) = lines_under(notification);
    let lines = widget::Column::new()
        .push(head)
        .push_maybe(comment.map(|comment| one_line(comment, sizes.text(), text_color)))
        .push_maybe(preview.map(|preview| one_line(preview, sizes.time(), dim)))
        .spacing(LINE_GAP)
        .width(Length::Fill);

    let body = widget::Row::new()
        .push(glyph(&notification.what, accent))
        .push(picture)
        .push(lines)
        .spacing(NOTE_AVATAR_GAP)
        .align_y(Alignment::Start)
        // As tall as it is said to be, whichever of the two is taller.
        .height(Length::Fixed(short_height(notification, sizes) - f32::from(NOTE_PADDING_TOP) - NOTE_DIVIDER_HEIGHT));
    let row = widget::Column::new()
        .push(body)
        .push(widget::divider::horizontal::default())
        .padding([NOTE_PADDING_TOP, 0, 0, 0])
        .width(Length::Fill);

    let area = widget::mouse_area(row);
    if context.under_a_menu() {
        return area.into();
    }
    let leads = match notification.about {
        Some(about) => Message::OpenThread(about, Whereabouts::default()),
        None => Message::OpenProfile(notification.who),
    };
    area.on_press(leads).interaction(mouse::Interaction::Pointer).into()
}

/// The row of `notification`: the note it is, under what that note is to
/// the account, or the short row of what was done about a note. With a
/// tint of the accent colour behind it if it `is_new`.
pub fn notification_row<'a>(notification: &'a Notification, context: &RowContext<'a>, is_new: bool) -> Element<'a, Message> {
    let sizes = context.sizes;
    let row: Element<'a, Message> = match &notification.note {
        Some(note) => {
            let text_color: Color = cosmic::theme::active().cosmic().on_bg_color().into();
            let dim = Color { a: NOTE_ACTION_DIM, ..text_color };
            let reason = container(one_line(reason(&notification.what), sizes.time(), dim))
                .padding([REASON_PADDING_TOP, 0.0, 0.0, 0.0])
                .height(Length::Fixed(reason_height(sizes)));
            // No space between the two: the note brings its own.
            widget::Column::new().push(reason).push(opening_thread(note_row(note, context), note.id, context)).into()
        }
        None => short_row(notification, context),
    };
    if !is_new {
        return row;
    }
    let tint = Color { a: NEW_TINT, ..context.accent };
    container(row)
        .class(cosmic::theme::Container::custom(move |_theme| container::Style {
            background: Some(Background::Color(tint)),
            ..Default::default()
        }))
        .into()
}

#[cfg(test)]
mod tests {
    use super::{estimated_notification_height, notification_row, reason_height, short_height};
    use crate::images::ImageStore;
    use crate::messages::Message;
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::engagement::Reaction;
    use crate::nostr::models::Note;
    use crate::nostr::notifications::{Notification, What};
    use crate::previews::PreviewStore;
    use crate::state::Expanded;
    use crate::video::Videos;
    use crate::views::components::{Bookmarking, Reacting, RowContext, RowSizes, note_row, opening_thread};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{clipboard, layout};
    use cosmic::iced::runtime::user_interface::{Cache, UserInterface};
    use cosmic::iced::{Color, Event, Point, Size, mouse};
    use nostr_sdk::prelude::{EventId, Keys, Kind, PublicKey};
    use std::collections::HashMap;

    const WIDTH: f32 = 394.0;

    /// A notification of `what`, about a note the start of which is
    /// `preview`, if it is at hand.
    fn told(what: What, preview: Option<&str>) -> Notification {
        Notification {
            id: EventId::from_byte_array([1; 32]),
            at: 0,
            who: Keys::generate().public_key(),
            what,
            note: None,
            about: Some(EventId::from_byte_array([2; 32])),
            preview: preview.map(String::from),
        }
    }

    /// A reply that says `text`, as a notification.
    fn reply(text: &str) -> Notification {
        let note = Note {
            id: EventId::from_byte_array([3; 32]),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse(text, FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        };
        Notification { id: note.id, who: note.author, what: What::Reply, note: Some(note), ..told(What::Repost, None) }
    }

    /// What `with` makes of a row context of text of `size`.
    fn with_context<T>(size: u16, with: impl FnOnce(&RowContext<'_>) -> T) -> T {
        let (profiles, images, previews, videos) =
            (HashMap::new(), ImageStore::new(1_000_000).0, PreviewStore::new().0, Videos::default());
        let (engagement, seen_on, expanded) = (HashMap::new(), HashMap::new(), Expanded::default());
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
        with(&context)
    }

    /// How tall the row of `notification` lays out, new or not.
    fn measured(notification: &Notification, size: u16, is_new: bool) -> f32 {
        with_context(size, |context| {
            let renderer = crate::views::components::testing::renderer();
            let mut row = notification_row(notification, context, is_new);
            let mut tree = Tree::new(row.as_widget());
            let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
            row.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
        })
    }

    #[test]
    fn a_short_row_is_exactly_as_tall_as_it_is_said_to_be_whatever_it_says() {
        let long = "what was said, at a length no line of a row this narrow has the room for, and then some more of it";
        let zap = |comment: &str| What::Zap { sats: 21, comment: comment.into(), issuer: Keys::generate().public_key() };
        for size in [10, 13, 18, 24] {
            let sizes = RowSizes::new(size);
            for notification in [
                told(What::Repost, None),
                told(What::Reaction(Reaction::Like), Some("a note")),
                told(What::Reaction(Reaction::Emoji("🤙".into())), Some(long)),
                told(zap(""), Some(long)),
                told(zap(long), None),
                told(zap(long), Some(long)),
            ] {
                let said = short_height(&notification, sizes);
                assert_eq!(estimated_notification_height(&notification, sizes, WIDTH, false), said);
                for is_new in [false, true] {
                    let height = measured(&notification, size, is_new);
                    assert!((height - said).abs() < 0.01, "at size {size}: {height} against {said} for {notification:?}");
                }
            }
            // Never shorter than who did it is pictured, and taller for
            // every line there is no room for beside the picture.
            let (bare, full) = (told(zap(""), None), told(zap("thanks"), Some("a note")));
            assert!(short_height(&full, sizes) > short_height(&bare, sizes));
            assert!(short_height(&told(zap("thanks"), None), sizes) >= short_height(&bare, sizes));
        }
    }

    #[test]
    fn a_note_is_under_what_it_is_to_the_account_and_as_tall_new_as_old() {
        for size in [10, 13, 24] {
            let notification = reply("an answer, of a line or two, as answers are when there is not much to add");
            let note = notification.note.as_ref().unwrap();
            let alone = with_context(size, |context| {
                let renderer = crate::views::components::testing::renderer();
                let mut row = opening_thread(note_row(note, context), note.id, context);
                let mut tree = Tree::new(row.as_widget());
                let limits = layout::Limits::new(Size::ZERO, Size::new(WIDTH, f32::INFINITY));
                row.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
            });
            let height = measured(&notification, size, false);
            assert!((height - alone - reason_height(RowSizes::new(size))).abs() < 0.01, "at size {size}: {height}");
            assert_eq!(measured(&notification, size, true), height, "the tint takes no room");
            let estimated = estimated_notification_height(&notification, RowSizes::new(size), WIDTH, false);
            assert!((estimated - height).abs() <= height * 0.3, "estimated {estimated}, measured {height}");
        }
    }

    /// What comes of a click on the row of `notification` at `point`.
    fn clicked(notification: &Notification, point: Point) -> Vec<Message> {
        with_context(13, |context| {
            let mut renderer = crate::views::components::testing::renderer();
            let row = notification_row(notification, context, false);
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
        })
    }

    #[test]
    fn a_click_on_a_short_row_opens_the_note_it_is_about_and_who_did_it_by_their_picture() {
        let liked = told(What::Reaction(Reaction::Like), Some("a note of mine"));
        let opened = |messages: Vec<Message>| -> Vec<(Option<EventId>, Option<PublicKey>)> {
            messages
                .into_iter()
                .map(|message| match message {
                    Message::OpenThread(id, _) => (Some(id), None),
                    Message::OpenProfile(who) => (None, Some(who)),
                    other => panic!("nothing a click here asks for: {other:?}"),
                })
                .collect()
        };
        // On the line under their name: the note.
        assert_eq!(opened(clicked(&liked, Point::new(300.0, 36.0))), [(liked.about, None)]);
        // On their picture, which is after what was done: their profile.
        assert_eq!(opened(clicked(&liked, Point::new(50.0, 24.0))), [(None, Some(liked.who))]);

        // A zap for no note has only who sent it to lead to.
        let issuer = Keys::generate().public_key();
        let zapped = Notification { about: None, ..told(What::Zap { sats: 21, comment: String::new(), issuer }, None) };
        assert_eq!(opened(clicked(&zapped, Point::new(300.0, 24.0))), [(None, Some(zapped.who))]);
    }
}

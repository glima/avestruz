// SPDX-License-Identifier: MIT

//! The profile view for Avestruz: who someone is and what they say of
//! themselves, following them, and under that their notes, newest first.
//!
//! All of it is in one scroller, so that reading on into the notes leaves
//! the rest behind, as wide as the popup is.

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::widget::{self, button, container, text};

use crate::fl;
use crate::images::{ImageShape, ImageState, ImageStore, LARGEST_AVATAR, STRIP_ASPECT};
use crate::messages::Message;
use crate::nostr::models::{display_name, short_npub};
use crate::nostr::nip05;
use crate::state::{AppModel, FollowProblem, Nip05State, ProfileState};
use crate::views::components::{MediaPlaceholder, POPUP_MAX_WIDTH, PlaceInScroller, avatar, link_line, list_scroller, zap_icon};

/// How wide the picture on a profile is: the largest an avatar is shown,
/// which is what avatars are kept for.
const PROFILE_AVATAR_SIZE: f32 = LARGEST_AVATAR;

/// How much the corners of the picture across the top of a profile are
/// rounded, and how tall it is at most: it comes cut to a strip, and is
/// shown as wide as the profile, which is no wider than the popup.
const PROFILE_BANNER_RADIUS: f32 = 6.0;
const PROFILE_BANNER_MAX_HEIGHT: f32 = POPUP_MAX_WIDTH / STRIP_ASPECT as f32;

/// Text sizes of a profile: the name, what the person says of themselves,
/// and the small print.
const PROFILE_NAME_SIZE: u16 = 18;
const PROFILE_TEXT_SIZE: u16 = 13;
const PROFILE_SMALL_SIZE: u16 = 11;

/// The picture across the top of a profile: once it is here, a strip as
/// wide as the profile; until then, the room it will take; nothing, if it
/// can't be had.
fn banner<'a>(images: &'a ImageStore, url: &str) -> Option<Element<'a, Message>> {
    match images.get(url, ImageShape::Strip) {
        ImageState::Ready(loaded) => Some(
            cosmic::widget::image(loaded.handle.clone())
                .width(Length::Fill)
                .height(Length::Shrink)
                .content_fit(ContentFit::Contain)
                .border_radius([PROFILE_BANNER_RADIUS; 4])
                .into(),
        ),
        // As tall as a strip as wide as this will be, whatever its size.
        ImageState::Loading => Some(
            MediaPlaceholder::new(Some((STRIP_ASPECT * 1_000, 1_000)), PROFILE_BANNER_MAX_HEIGHT)
                .radius(PROFILE_BANNER_RADIUS)
                .into(),
        ),
        ImageState::Failed => None,
    }
}

/// A line of small print, wrapped.
fn small_print<'a>(message: String) -> Element<'a, Message> {
    text(message).size(PROFILE_SMALL_SIZE).wrapping(Wrapping::WordOrGlyph).width(Length::Fill).into()
}

impl AppModel {
    /// The line above a profile: the way back, and what this is.
    fn view_profile_header(&self) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("profile-back"))
            .on_press(Message::ClosePage);
        widget::Row::new()
            .push(back)
            .push(text(fl!("profile-title")).size(16).font(cosmic::font::bold()))
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// The key of the profile `state`, short, with a button that copies it
    /// whole.
    fn view_profile_key(&self, state: &ProfileState) -> Element<'_, Message> {
        let copy = button::icon(widget::icon::from_name("edit-copy-symbolic").size(12))
            .tooltip(if state.npub_copied { fl!("profile-npub-copied") } else { fl!("profile-copy-npub") })
            .on_press(Message::CopyNpub(state.public_key))
            .padding(2);
        widget::Row::new()
            .push(text(short_npub(&state.public_key)).size(PROFILE_SMALL_SIZE).wrapping(Wrapping::None))
            .push(copy)
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
    }

    /// The name at a domain the person of `state` gives as theirs, if the
    /// domain said it is: a name nobody vouched for is not shown.
    fn view_profile_nip05(&self, state: &ProfileState) -> Option<Element<'_, Message>> {
        let check = self.nip05.get(&state.public_key).filter(|check| check.state == Nip05State::Verified)?;
        let verified = widget::Row::new()
            .push(widget::icon::from_name("emblem-ok-symbolic").size(PROFILE_SMALL_SIZE + 1))
            .push(
                text(nip05::shown(&check.name))
                    .size(PROFILE_SMALL_SIZE)
                    .class(cosmic::theme::Text::Accent)
                    .wrapping(Wrapping::None),
            )
            .spacing(4)
            .align_y(Alignment::Center);
        Some(widget::tooltip(verified, text(fl!("profile-verified")), widget::tooltip::Position::Bottom).into())
    }

    /// Following the person of `state`, or no longer: for an account that
    /// can publish, on a profile that is not its own, once who it follows
    /// is known. One change at a time.
    fn view_follow_button(&self, state: &ProfileState) -> Option<Element<'_, Message>> {
        self.account.as_ref().filter(|account| account.can_sign && account.public_key != state.public_key)?;
        let following = self.follows.as_ref()?.contains(&state.public_key);
        // Pressed again after being told there is no list to add to, it
        // writes the first.
        let first_list = state.follow_problem == Some(FollowProblem::NoList);
        let follow = Message::Follow { person: state.public_key, follow: !following, first_list };
        let button = match (following, first_list) {
            (true, _) => button::standard(fl!("profile-unfollow")),
            (false, true) => button::suggested(fl!("profile-follow-anyway")),
            (false, false) => button::suggested(fl!("profile-follow")),
        };
        Some(button.on_press_maybe(self.follow_pending.is_none().then_some(follow)).into())
    }

    /// Who the profile `state` is of: the picture across its top, their
    /// picture, their name and key, following them, and, as far as they
    /// said, something about themselves, their website and where they can
    /// be zapped.
    fn view_profile_person<'a>(&'a self, state: &'a ProfileState) -> Element<'a, Message> {
        let profile = self.profiles.get(&state.public_key);
        let name = display_name(&self.profiles, &state.public_key);
        let picture = profile.and_then(|profile| profile.picture.as_deref());
        let picture = avatar(&self.images, picture, &state.public_key, &name, PROFILE_AVATAR_SIZE);

        // The name is cut rather than wrapped when it doesn't fit.
        let mut who = widget::Column::new()
            .push(
                container(text(name.clone()).size(PROFILE_NAME_SIZE).font(cosmic::font::bold()).wrapping(Wrapping::None))
                    .clip(true),
            )
            .push(self.view_profile_key(state))
            .spacing(2)
            .width(Length::Fill);
        if let Some(verified) = self.view_profile_nip05(state) {
            who = who.push(verified);
        }
        let mut identity = widget::Row::new().push(picture).push(who).spacing(12).align_y(Alignment::Center);
        if let Some(follow) = self.view_follow_button(state) {
            identity = identity.push(follow);
        }

        let mut person = widget::Column::new().spacing(8).padding([12, 0]).width(Length::Fill);
        if let Some(banner) = profile.and_then(|profile| profile.banner.as_deref()).and_then(|url| banner(&self.images, url)) {
            person = person.push(banner);
        }
        person = person.push(identity);
        match &state.follow_problem {
            Some(FollowProblem::NoList) => person = person.push(small_print(fl!("profile-follow-no-list", name = name.as_ref()))),
            Some(FollowProblem::Failed(error)) => {
                person = person.push(
                    text(error.as_str())
                        .size(PROFILE_SMALL_SIZE + 1)
                        .class(cosmic::theme::Text::Accent)
                        .wrapping(Wrapping::WordOrGlyph),
                );
            }
            None => {}
        }

        if let Some(about) = profile.and_then(|profile| profile.about.as_deref()) {
            person = person.push(text(about).size(PROFILE_TEXT_SIZE).wrapping(Wrapping::WordOrGlyph).width(Length::Fill));
        }
        if let Some(website) = profile.and_then(|profile| profile.website.as_ref()) {
            let accent = cosmic::theme::active().cosmic().accent_text_color().into();
            person = person.push(link_line(&website.url, &website.label, accent, PROFILE_TEXT_SIZE));
        }
        if let Some(lightning) = profile.and_then(|profile| profile.lightning.as_deref()) {
            let address = widget::Row::new()
                .push(widget::icon::icon(zap_icon()).size(PROFILE_SMALL_SIZE + 1))
                .push(text(lightning).size(PROFILE_SMALL_SIZE).wrapping(Wrapping::WordOrGlyph))
                .spacing(4)
                .align_y(Alignment::Center);
            person = person.push(address);
        }
        person.into()
    }

    /// Render the profile view, of the profile `state`.
    pub fn view_profile<'a>(&'a self, state: &'a ProfileState) -> Element<'a, Message> {
        let nothing = if state.notes.refreshing { fl!("profile-loading") } else { fl!("profile-no-notes") };
        // Which of their notes show is chosen above them, as above the
        // feed: it is one choice, for every list.
        let modes = container(self.view_feed_modes()).width(Length::Fill).align_x(Alignment::Center).padding([8, 0]);
        let content = widget::Column::new()
            .push(self.view_profile_person(state))
            .push(widget::divider::horizontal::default())
            .push(modes)
            .push(widget::divider::horizontal::default())
            // Where the notes start, for stepping through them (see
            // `handle_step_note`).
            .push(PlaceInScroller::new(&state.notes.position, &state.notes_top))
            .push(self.view_notes(&state.notes, nothing));

        widget::Column::new()
            .push(self.view_profile_header())
            .push(widget::divider::horizontal::default())
            .push(self.with_new_notes(list_scroller(&state.notes.position, content), &state.notes))
            .into()
    }
}

// SPDX-License-Identifier: MIT

//! Someone's picture, round, or what stands in for it.

use cosmic::Element;
use cosmic::iced::{Alignment, Background, Border, Color, Length};
use cosmic::widget::{container, text};
use nostr_sdk::prelude::PublicKey;

use crate::images::{ImageShape, ImageState, ImageStore};

/// A colour of someone's own, from their public key: the same person is the
/// same colour everywhere, before their picture loads and if they have none.
/// Kept to the middle of the range so the white initial reads on it.
fn key_color(public_key: &PublicKey) -> Color {
    let bytes = public_key.to_bytes();
    let channel = |index: usize| 0.25 + 0.45 * f32::from(bytes.get(index).copied().unwrap_or(0)) / 255.0;
    Color::from_rgb(channel(0), channel(1), channel(2))
}

/// The picture at `picture`, round and `size` across. Until it has loaded,
/// or if there is none, a disc in the person's own colour with the initial
/// of `name`.
pub fn avatar<'a, Message: 'a>(
    images: &'a ImageStore,
    picture: Option<&str>,
    public_key: &PublicKey,
    name: &str,
    size: f32,
) -> Element<'a, Message> {
    // Asked for round: the picture comes as its middle square with the
    // corners already transparent, so it only has to be drawn at `size`.
    if let Some(ImageState::Ready(loaded)) = picture.map(|url| images.get(url, ImageShape::Circle)) {
        return cosmic::widget::image(loaded.handle.clone()).width(Length::Fixed(size)).height(Length::Fixed(size)).into();
    }

    let initial: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let fill = key_color(public_key);
    container(text(initial).size(size * 0.45).font(cosmic::font::bold()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(cosmic::theme::Container::custom(move |_theme| container::Style {
            background: Some(Background::Color(fill)),
            text_color: Some(Color::WHITE),
            border: Border { radius: (size / 2.0).into(), ..Border::default() },
            ..Default::default()
        }))
        .into()
}

#[cfg(test)]
mod tests {
    use super::key_color;
    use nostr_sdk::prelude::Keys;

    #[test]
    fn a_key_always_gives_the_same_colour_and_one_white_reads_on() {
        for _ in 0..50 {
            let key = Keys::generate().public_key();
            let color = key_color(&key);
            assert_eq!(color, key_color(&key));
            for channel in [color.r, color.g, color.b] {
                assert!((0.25..=0.7).contains(&channel), "{channel}");
            }
        }
    }
}

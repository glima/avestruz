// SPDX-License-Identifier: MIT

//! The space a picture will take, held while it loads.
//!
//! A note's image arrives after the note does. If its row grew when it did,
//! everything below would jump. [`MediaPlaceholder`] takes the room the
//! picture will take once it is there, when the note says how large the
//! picture is (NIP-92 `imeta`), so the row is the same height before and
//! after.
//!
//! A picture is shown as wide as the room there is for it, from one edge
//! to the other, and as tall as that makes it; but never larger than it
//! is: one narrower than the room keeps its own size. [`shown_size`] is
//! that rule, for the placeholder, the picture and the estimate of a row's
//! height alike.

use cosmic::iced::core::layout;
use cosmic::iced::core::mouse;
use cosmic::iced::core::renderer;
use cosmic::iced::core::widget::Tree;
use cosmic::iced::core::{Border, Color, Element, Layout, Length, Rectangle, Size, Widget};

/// How large a picture is shown, given its size in pixels and the width
/// there is for it: scaled down to that width, as tall as that makes it,
/// and never scaled up.
///
/// This is what the image widget does with `ContentFit::Contain`, a `Fill`
/// width that stops at the picture's own and a `Shrink` height, written out
/// so that the placeholder and the estimate of a row's height agree with it.
pub fn shown_size(size: (u32, u32), width: f32) -> Size {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let across = width.min(w);
    Size::new(across, h * (across / w))
}

/// A quiet rounded box where a picture is about to be.
pub struct MediaPlaceholder {
    /// The picture's size in pixels, if the note said.
    size: Option<(u32, u32)>,
    /// How tall to be when the size isn't known.
    fallback_height: f32,
    radius: f32,
}

impl MediaPlaceholder {
    /// A placeholder for a picture of `size` pixels, if known: as large as
    /// the picture will be shown. As wide as the room there is and
    /// `fallback_height` tall otherwise.
    pub fn new(size: Option<(u32, u32)>, fallback_height: f32) -> Self {
        Self { size, fallback_height, radius: 0.0 }
    }

    /// Round the corners, as the picture's will be.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for MediaPlaceholder
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let width = limits.max().width;
        layout::Node::new(match self.size {
            Some(size) => shown_size(size, width),
            None => Size::new(width, self.fallback_height),
        })
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        // The text colour, faint: visible on a light theme and a dark one.
        let fill = Color { a: 0.08, ..style.text_color };
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: Border { radius: self.radius.into(), ..Border::default() },
                ..renderer::Quad::default()
            },
            fill,
        );
    }
}

impl<'a, Message, Theme, Renderer> From<MediaPlaceholder> for Element<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn from(placeholder: MediaPlaceholder) -> Self {
        Element::new(placeholder)
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaPlaceholder, shown_size};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Element, Size, layout};

    fn laid_out(placeholder: MediaPlaceholder, width: f32) -> Size {
        let mut element: Element<'_, (), (), ()> = placeholder.into();
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        element.as_widget_mut().layout(&mut tree, &(), &limits).size()
    }

    #[test]
    fn a_picture_is_as_wide_as_the_room_there_is_and_never_larger_than_it_is() {
        // Wider than the room: scaled to the width.
        assert_eq!(shown_size((1200, 800), 300.0), Size::new(300.0, 200.0));
        // Tall: as wide as the room all the same, however tall that is.
        assert_eq!(shown_size((600, 1800), 300.0), Size::new(300.0, 900.0));
        // Just as wide as the room: as it is.
        assert_eq!(shown_size((300, 150), 300.0), Size::new(300.0, 150.0));
        // Small: never scaled up.
        assert_eq!(shown_size((100, 50), 300.0), Size::new(100.0, 50.0));
        // Nonsense sizes don't divide by zero.
        assert_eq!(shown_size((0, 0), 300.0), Size::new(1.0, 1.0));
    }

    #[test]
    fn the_placeholder_is_as_large_as_the_picture_will_be() {
        assert_eq!(laid_out(MediaPlaceholder::new(Some((1200, 800)), 180.0), 300.0), Size::new(300.0, 200.0));
        assert_eq!(laid_out(MediaPlaceholder::new(Some((600, 1800)), 180.0), 300.0), Size::new(300.0, 900.0));
        assert_eq!(laid_out(MediaPlaceholder::new(Some((100, 50)), 180.0), 300.0), Size::new(100.0, 50.0));
        assert_eq!(laid_out(MediaPlaceholder::new(None, 180.0), 300.0), Size::new(300.0, 180.0));
    }
}

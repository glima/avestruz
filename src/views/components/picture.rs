// SPDX-License-Identifier: MIT

//! A picture in a note: as wide as its row and no wider than it is, as
//! tall as its proportions make it, and drawn pixel for pixel where it
//! can be.
//!
//! What draws a frame is a CPU renderer, and of all it does, scaling a
//! picture costs the most. A picture is therefore kept at the size it is
//! shown at (see [`crate::images`]), and this widget draws it as it is
//! kept: where the pixels kept are the pixels its room takes, each is
//! taken as it is, and where they aren't (a row narrower than the widest,
//! a screen that turned out another scale than the picture was made for)
//! it is scaled as any picture is, smoothly, and slower.
//!
//! It is laid out by the size of the picture itself and not by what is
//! kept of it, which is rounded to whole pixels: the room it takes is the
//! room that was kept for it when its note said how large it is.
//!
//! iced's own picture widget asks the renderer how large the picture is
//! when it is laid out, for which the renderer makes its own copy of it
//! there and then; it draws it smoothly whatever its size; and it takes
//! its proportions from the pixels. This one is told its size.
//!
//! A click on a picture is a click on its row, and the picture takes none
//! of them but one: with the right button, if it was told what that does.
//! The toolkit's own way of hearing a click, an area around a widget,
//! takes every press of the left button for itself, whether it was told
//! what one does or not, and the row would hear of none.

use cosmic::iced::core::image::{self, FilterMethod};
use cosmic::iced::core::layout;
use cosmic::iced::core::mouse;
use cosmic::iced::core::renderer;
use cosmic::iced::core::widget::Tree;
use cosmic::iced::core::{Clipboard, Element, Event, Layout, Length, Radians, Rectangle, Shell, Size, Widget, border};
use cosmic::widget::image::Handle;

use crate::images::LoadedImage;

/// How far, in pixels, what is kept of a picture may be from the room it
/// is drawn in and still be drawn pixel for pixel: less than one. A row is
/// seldom a whole number of pixels wide, and the picture kept is.
const SAME_SIZE: f32 = 1.0;

/// A picture in a note. See the [module docs](self).
pub struct Picture<Message> {
    handle: Handle,
    /// How large the picture itself is: what it is laid out by.
    size: Size,
    radius: f32,
    /// What a click on it with the right button does, if anything.
    on_right_press: Option<Message>,
}

impl<Message> Picture<Message> {
    /// `loaded`, with square corners.
    pub fn new(loaded: &LoadedImage) -> Self {
        Self {
            handle: loaded.handle.clone(),
            size: Size::new(loaded.width.max(1) as f32, loaded.height.max(1) as f32),
            radius: 0.0,
            on_right_press: None,
        }
    }

    /// Its corners rounded by `radius`.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// A click on it with the right button says `message`.
    pub fn on_right_press(mut self, message: Message) -> Self {
        self.on_right_press = Some(message);
        self
    }
}

/// How large a picture `size` large is shown in a row `width` wide: as
/// wide as the row and no wider than it is, in its own proportions.
fn shown(size: Size, width: f32) -> Size {
    let across = width.min(size.width);
    Size::new(across, across * size.height / size.width)
}

/// How a picture kept at `kept` pixels is drawn in room of `room` pixels:
/// each pixel as it is, if they are the same size, and smoothly if not.
fn filter(kept: Size<u32>, room: Size) -> FilterMethod {
    let same = (kept.width as f32 - room.width).abs() < SAME_SIZE && (kept.height as f32 - room.height).abs() < SAME_SIZE;
    if same { FilterMethod::Nearest } else { FilterMethod::Linear }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Picture<Message>
where
    Message: Clone,
    Renderer: image::Renderer<Handle = Handle>,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(shown(self.size, limits.max().width))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let right_press = matches!(event, Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)));
        if let Some(message) = self.on_right_press.as_ref().filter(|_| right_press && cursor.is_over(layout.bounds())) {
            shell.publish(message.clone());
            shell.capture_event();
        }
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
        let bounds = layout.bounds();
        let scale = style.scale_factor as f32;
        let filter_method = match &self.handle {
            Handle::Rgba { width, height, .. } => {
                filter(Size::new(*width, *height), Size::new(bounds.width * scale, bounds.height * scale))
            }
            _ => FilterMethod::Linear,
        };
        renderer.draw_image(
            image::Image {
                handle: self.handle.clone(),
                filter_method,
                rotation: Radians(0.0),
                border_radius: border::Radius::from(self.radius),
                opacity: 1.0,
                snap: true,
            },
            bounds,
            bounds,
        );
    }
}

impl<'a, Message, Theme, Renderer> From<Picture<Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Renderer: image::Renderer<Handle = Handle> + 'a,
{
    fn from(picture: Picture<Message>) -> Self {
        Element::new(picture)
    }
}

#[cfg(test)]
mod tests {
    use super::{Picture, filter, shown};
    use crate::images::LoadedImage;
    use cosmic::iced::core::image::FilterMethod;
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Event, Layout, Point, Rectangle, Shell, Widget, clipboard, layout, mouse};
    use cosmic::iced::{Length, Size};
    use cosmic::widget::image::Handle;

    #[test]
    fn a_picture_takes_a_click_with_the_right_button_and_no_other() {
        let loaded = LoadedImage { handle: Handle::from_rgba(2, 1, vec![0; 8]), width: 200, height: 100 };
        let mut picture: Picture<&str> = Picture::new(&loaded).on_right_press("copy");
        let renderer = crate::views::components::testing::renderer();
        let widget: &mut dyn Widget<&str, cosmic::Theme, cosmic::Renderer> = &mut picture;
        assert_eq!(widget.size().width, Length::Fill);
        let mut tree = Tree::empty();
        let node = widget.layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, Size::new(600.0, 900.0)));
        assert_eq!(node.size(), Size::new(200.0, 100.0));

        // What a press of `button` at `at` says, and whether the picture
        // took it for itself.
        let mut pressed = |button: mouse::Button, at: Point| {
            let mut said = Vec::new();
            let mut shell = Shell::new(&mut said);
            widget.update(
                &mut tree,
                &Event::Mouse(mouse::Event::ButtonPressed(button)),
                Layout::new(&node),
                mouse::Cursor::Available(at),
                &renderer,
                &mut clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, Size::new(600.0, 900.0)),
            );
            let taken = shell.is_event_captured();
            (said, taken)
        };
        let (on_it, beside_it) = (Point::new(100.0, 50.0), Point::new(300.0, 50.0));
        assert_eq!(pressed(mouse::Button::Right, on_it), (vec!["copy"], true));
        assert_eq!(pressed(mouse::Button::Right, beside_it), (vec![], false), "beside it, in the room it leaves");
        // The left button is the row's: the picture neither answers nor takes it.
        assert_eq!(pressed(mouse::Button::Left, on_it), (vec![], false));
    }

    #[test]
    fn a_picture_is_as_wide_as_its_row_no_wider_than_itself_and_in_its_own_proportions() {
        // Wider than its row: the row's width.
        assert_eq!(shown(Size::new(4_000.0, 3_000.0), 600.0), Size::new(600.0, 450.0));
        // Narrower: its own size, a point to the pixel.
        assert_eq!(shown(Size::new(300.0, 100.0), 600.0), Size::new(300.0, 100.0));
        // Taller than it is wide: as tall as that makes it.
        assert_eq!(shown(Size::new(1_000.0, 2_000.0), 600.0), Size::new(600.0, 1_200.0));
    }

    #[test]
    fn a_picture_kept_at_the_size_of_its_room_is_drawn_pixel_for_pixel() {
        // A row 614 points wide at 170% is 1043.8 pixels: the picture was
        // kept at 1044, and what is kept is what is drawn.
        assert_eq!(filter(Size::new(1_044, 783), Size::new(1_043.8, 782.85)), FilterMethod::Nearest);
        assert_eq!(filter(Size::new(614, 460), Size::new(614.0, 460.5)), FilterMethod::Nearest);
        // In a narrower row, or on a screen of another scale than it was
        // made for, it is scaled, and smoothly.
        assert_eq!(filter(Size::new(1_044, 783), Size::new(900.0, 675.0)), FilterMethod::Linear);
        assert_eq!(filter(Size::new(614, 460), Size::new(1_043.8, 782.9)), FilterMethod::Linear);
        assert_eq!(filter(Size::new(1_044, 783), Size::new(1_043.8, 700.0)), FilterMethod::Linear);
    }
}

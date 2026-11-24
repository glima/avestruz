// SPDX-License-Identifier: MIT

//! How far a scroller is scrolled, kept where it outlives the widget tree,
//! and the scroller the views' lists are shown in.
//!
//! Another view shown in a list's place throws the scroller's own state
//! away, and so does closing the popup. A [`ScrollPosition`], the first
//! thing in a scroller, keeps how far it is scrolled in a cell the model
//! owns; coming back, the view scrolls to where the cell says it was.
//!
//! A scroller that is new, as every one is in a popup that just opened,
//! does that by itself: [`Restoring`], around it, scrolls it to where the
//! cell says before it is first drawn. Nobody sees its top go by, and no
//! message has to find a window that may not be there yet.
//!
//! It takes no room and draws nothing. What it keeps is how far its own
//! place is above the top of what is in view, which, for the first thing in
//! a scroller, is how far the scroller is scrolled.
//!
//! Over the row of a long note that shows in full, the same thing keeps
//! how far the top of that row is scrolled out of view: what the list is
//! scrolled back by when the note is cut again (see
//! [`Expanded`](crate::state::Expanded)).
//!
//! ```mermaid
//! sequenceDiagram
//!     participant model as The model's cell
//!     participant position as ScrollPosition
//!     participant scroller as The scroller
//!     Note over scroller: the reader scrolls
//!     scroller->>position: drawn, so far above the top of the view
//!     position->>model: that far
//!     Note over scroller: the popup closes: the scroller and its state are gone
//!     Note over scroller: the popup opens: a new scroller, at its top
//!     model->>scroller: scrolled to where the cell says, before the first frame (Restoring)
//!     Note over scroller: the reader sees the list where it was
//! ```
//!
//! Why the place has to be kept at all, and what else is kept the same
//! way, is in [`crate::guide::architecture`].

use std::cell::Cell;

use cosmic::iced::core::layout;
use cosmic::iced::core::mouse;
use cosmic::iced::core::overlay;
use cosmic::iced::core::renderer;
use cosmic::iced::core::widget::operation::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::iced::core::widget::tree::{self, Tree};
use cosmic::iced::core::widget::{Id, Operation};
use cosmic::iced::core::{Clipboard, Element, Event, Layout, Length, Rectangle, Shell, Size, Vector, Widget};
use cosmic::iced::widget::scrollable;
use cosmic::widget::{self, container};

use crate::messages::Message;
use crate::views::components::{LIST_PADDING_LEFT, LIST_PADDING_RIGHT, LIST_SCROLL_ID};

/// `content`, a view's list and whatever it has above and under it, in the
/// scroller the views share, which keeps `position` at how far it is
/// scrolled, and starts out there.
///
/// The scroller is as tall as what is in it, up to the room there is for
/// it: what the popup may be tall (see `popup_max_height`), less whatever
/// the view puts above it.
pub fn list_scroller<'a>(
    position: &'a Cell<f32>,
    content: impl Into<cosmic::Element<'a, Message>>,
) -> cosmic::Element<'a, Message> {
    let content = widget::Column::new().push(ScrollPosition::new(position)).push(content);
    // Room on the right for the scrollbar, which is drawn over the content.
    // No `on_scroll`: a list itself says when its end is near, and why the
    // scrollable isn't asked is in the list's module docs.
    let scroller = scrollable(container(content).width(Length::Fill).padding([0, LIST_PADDING_RIGHT, 0, LIST_PADDING_LEFT]))
        .id(LIST_SCROLL_ID.clone())
        .height(Length::Shrink);

    Restoring::new(position, LIST_SCROLL_ID.clone(), scroller).into()
}

/// Whether a [`Restoring`] has put its scroller where it was. Widget state:
/// it starts over with the widget tree, as the scroller's own place does.
#[derive(Default)]
struct Restored(bool);

/// `content`, which holds the scroller `scroller`, scrolled to `position`
/// before it is first drawn. After that the scroller is its user's.
pub struct Restoring<'a, Message, Theme, Renderer> {
    position: &'a Cell<f32>,
    scroller: Id,
    content: Element<'a, Message, Theme, Renderer>,
}

impl<'a, Message, Theme, Renderer> Restoring<'a, Message, Theme, Renderer> {
    pub fn new(position: &'a Cell<f32>, scroller: Id, content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self { position, scroller, content: content.into() }
    }
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for Restoring<'_, Message, Theme, Renderer> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Restored>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Restored::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        match tree.children.first_mut() {
            Some(child) => self.content.as_widget_mut().layout(child, renderer, limits),
            None => layout::Node::new(Size::ZERO),
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let first_time = !std::mem::replace(&mut tree.state.downcast_mut::<Restored>().0, true);
        let Some(child) = tree.children.first_mut() else {
            return;
        };
        // Before the content hears of anything: what keeps the place would
        // otherwise put down where a new scroller starts, which is its top.
        let place = self.position.get();
        if first_time && place > 0.0 {
            let mut to_place = scroll_to::<()>(self.scroller.clone(), AbsoluteOffset { x: None, y: Some(place) });
            self.content.as_widget_mut().operate(child, layout, renderer, &mut to_place);
        }
        self.content.as_widget_mut().update(child, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(child) = tree.children.first() {
            self.content.as_widget().draw(child, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        match tree.children.first() {
            Some(child) => self.content.as_widget().mouse_interaction(child, layout, cursor, viewport, renderer),
            None => mouse::Interaction::None,
        }
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        if let Some(child) = tree.children.first_mut() {
            self.content.as_widget_mut().operate(child, layout, renderer, operation);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let child = tree.children.first_mut()?;
        self.content.as_widget_mut().overlay(child, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<Restoring<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(restoring: Restoring<'a, Message, Theme, Renderer>) -> Self {
        Element::new(restoring)
    }
}

/// Keeps `position` at how far its own place is above the top of what is
/// in view: for the first thing in a scroller, how far that is scrolled.
pub struct ScrollPosition<'a> {
    position: &'a Cell<f32>,
}

impl<'a> ScrollPosition<'a> {
    pub fn new(position: &'a Cell<f32>) -> Self {
        Self { position }
    }
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for ScrollPosition<'_> {
    /// As tall as its content, which is nothing. Not a fixed height of
    /// zero: a column leaves out a child that says it takes no room.
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(limits.max().width, 0.0))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.position.set((viewport.y - layout.position().y).max(0.0));
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, Message, Theme, Renderer: renderer::Renderer> From<ScrollPosition<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(position: ScrollPosition<'a>) -> Self {
        Element::new(position)
    }
}

/// Keeps `place` at how far down what is in a scroller its own place is:
/// put before a list that is not the first thing in its scroller, how far
/// the scroller is scrolled when the list's first row is at the top of
/// what is in view.
///
/// It goes by `scrolled`, which the scroller's [`ScrollPosition`] keeps:
/// what is in view starts that far down what is in the scroller, so the
/// top of that is known, and how far under it this is. It takes no room
/// and draws nothing, as a [`ScrollPosition`] does.
pub struct PlaceInScroller<'a> {
    scrolled: &'a Cell<f32>,
    place: &'a Cell<f32>,
}

impl<'a> PlaceInScroller<'a> {
    pub fn new(scrolled: &'a Cell<f32>, place: &'a Cell<f32>) -> Self {
        Self { scrolled, place }
    }
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for PlaceInScroller<'_> {
    /// As a [`ScrollPosition`]: as tall as its content, which is nothing.
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(limits.max().width, 0.0))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let top = viewport.y - self.scrolled.get();
        self.place.set((layout.position().y - top).max(0.0));
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, Message, Theme, Renderer: renderer::Renderer> From<PlaceInScroller<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(place: PlaceInScroller<'a>) -> Self {
        Element::new(place)
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollPosition;
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Element, Event, Layout, Point, Rectangle, Shell, Size, clipboard, layout, mouse, window};
    use std::cell::Cell;

    #[test]
    fn it_takes_no_room_and_keeps_how_far_the_scroller_is_scrolled() {
        let position = Cell::new(0.0);
        let mut element: Element<'_, (), (), ()> = ScrollPosition::new(&position).into();
        assert!(!element.as_widget().size_hint().is_void(), "a column would leave it out");
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(300.0, f32::INFINITY));
        let node = element.as_widget_mut().layout(&mut tree, &(), &limits);
        assert_eq!(node.size(), Size::new(300.0, 0.0));

        // The scroller's content starts 40 down the window, and is scrolled
        // 250: what is in view starts at 290.
        let node = node.move_to(Point::new(0.0, 40.0));
        let mut scrolled_to = |y: f32| {
            let mut messages = Vec::new();
            element.as_widget_mut().update(
                &mut tree,
                &Event::Window(window::Event::RedrawRequested(std::time::Instant::now())),
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &(),
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &Rectangle::new(Point::new(0.0, 40.0 + y), Size::new(300.0, 500.0)),
            );
            position.get()
        };
        assert_eq!(scrolled_to(250.0), 250.0);
        assert_eq!(scrolled_to(0.0), 0.0);
    }

    #[test]
    fn what_is_under_something_in_a_scroller_knows_how_far_down_it_is_however_far_that_is_scrolled() {
        use super::PlaceInScroller;

        let (scrolled, place) = (Cell::new(0.0), Cell::new(0.0));
        let mut element: Element<'_, (), (), ()> = PlaceInScroller::new(&scrolled, &place).into();
        assert!(!element.as_widget().size_hint().is_void(), "a column would leave it out");
        let mut tree = Tree::new(element.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(300.0, f32::INFINITY));
        let node = element.as_widget_mut().layout(&mut tree, &(), &limits);
        assert_eq!(node.size(), Size::new(300.0, 0.0));

        // The scroller's content starts 40 down the window, and this is
        // 180 down that, under whatever is above the list.
        let node = node.move_to(Point::new(0.0, 40.0 + 180.0));
        let mut scrolled_to = |y: f32| {
            // As the scroller's `ScrollPosition`, before it, has kept it.
            scrolled.set(y);
            let mut messages = Vec::new();
            element.as_widget_mut().update(
                &mut tree,
                &Event::Window(window::Event::RedrawRequested(std::time::Instant::now())),
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &(),
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &Rectangle::new(Point::new(0.0, 40.0 + y), Size::new(300.0, 500.0)),
            );
            place.get()
        };
        assert_eq!(scrolled_to(0.0), 180.0);
        assert_eq!(scrolled_to(120.0), 180.0, "scrolled short of it");
        assert_eq!(scrolled_to(900.0), 180.0, "scrolled past it");
    }

    /// One frame of `element` in a window 300 by 500: laid out, and told
    /// it is about to be drawn, as the runtime does before every frame.
    fn frame(element: &mut cosmic::Element<'_, crate::messages::Message>, tree: &mut Tree) {
        let renderer = crate::views::components::testing::renderer();
        let limits = layout::Limits::new(Size::ZERO, Size::new(300.0, 500.0));
        let node = element.as_widget_mut().layout(tree, &renderer, &limits);
        let mut messages = Vec::new();
        element.as_widget_mut().update(
            tree,
            &Event::Window(window::Event::RedrawRequested(std::time::Instant::now())),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &renderer,
            &mut clipboard::Null,
            &mut Shell::new(&mut messages),
            &Rectangle::new(Point::ORIGIN, node.size()),
        );
    }

    /// Something far taller than the popup, to scroll.
    fn tall<'a>() -> cosmic::Element<'a, crate::messages::Message> {
        cosmic::widget::Space::new().width(cosmic::iced::Length::Fill).height(5_000).into()
    }

    #[test]
    fn a_new_scroller_starts_where_the_last_one_was_left() {
        // The popup closed with its list scrolled to 420, and opens again:
        // a new widget tree, and a scroller that knows of no place.
        let position = Cell::new(420.0);
        let mut element = super::list_scroller(&position, tall());
        let mut tree = Tree::new(element.as_widget());

        frame(&mut element, &mut tree);
        assert_eq!(position.get(), 420.0, "scrolled there before its first frame, not left at its top");

        // After that it is its user's: what the cell says is what the
        // scroller does, not the other way about.
        position.set(100.0);
        let mut element = super::list_scroller(&position, tall());
        tree.diff(element.as_widget_mut());
        frame(&mut element, &mut tree);
        assert_eq!(position.get(), 420.0);
    }

    /// How tall a view lays out in a popup that may be `popup` tall: a
    /// header 40 tall, and under it the scroller, with `content` `content`
    /// tall in it.
    fn view_height(content: f32, popup: f32) -> f32 {
        let position = Cell::new(0.0);
        let space = |height: f32| cosmic::widget::Space::new().width(cosmic::iced::Length::Fill).height(height);
        let mut element: cosmic::Element<'_, crate::messages::Message> =
            cosmic::widget::Column::new().push(space(40.0)).push(super::list_scroller(&position, space(content))).into();
        let mut tree = Tree::new(element.as_widget());
        let renderer = crate::views::components::testing::renderer();
        let limits = layout::Limits::new(Size::ZERO, Size::new(300.0, popup));
        element.as_widget_mut().layout(&mut tree, &renderer, &limits).size().height
    }

    #[test]
    fn a_long_list_makes_the_popup_as_tall_as_it_may_be_and_a_short_one_as_tall_as_itself() {
        // However tall the popup may be: no height of its own holds the
        // scroller back from the room there is.
        for popup in [450.0, 600.0, 952.0, 1_620.0] {
            assert_eq!(view_height(5_000.0, popup), popup, "a long list fills a popup that may be {popup} tall");
        }
        // With little in it, the popup is as tall as its header and that.
        assert_eq!(view_height(120.0, 952.0), 160.0);
        assert_eq!(view_height(912.0, 952.0), 952.0, "what just fits is not scrolled");
    }

    #[test]
    fn a_new_scroller_that_was_never_scrolled_starts_at_its_top() {
        let position = Cell::new(0.0);
        let mut element = super::list_scroller(&position, tall());
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree);
        assert_eq!(position.get(), 0.0);
    }
}

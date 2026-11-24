// SPDX-License-Identifier: MIT

//! A virtual list whose rows are as tall as their content.
//!
//! A feed is long, grows at both ends, and no two notes are the same height.
//! iced's virtual [`List`](cosmic::iced::widget::list::List) only renders the
//! rows in view, but to know where they sit it builds and lays out **every**
//! item whenever its content is new, and draws nothing until it has.
//!
//! [`VirtualList`] never measures a row before it is in view. A row that has
//! been shown has its real height; one that hasn't has the height the caller
//! estimates for its item. It only ever builds the rows in the viewport, so
//! showing a feed costs the same whatever its length.
//!
//! ```mermaid
//! flowchart TB
//!     subgraph list["A list of 500 notes, scrolled some way down"]
//!         above["Rows above: measured when they went by. Not built"]
//!         subgraph viewport["The viewport"]
//!             r1["A row: built, laid out, measured"]
//!             r2["A row: built, laid out, measured"]
//!             r3["A row: built, laid out, measured"]
//!         end
//!         below["Rows below: never shown. Each as tall as its estimate. Not built"]
//!         above --- r1 --- r2 --- r3 --- below
//!     end
//!     heights[("RowHeights: by note id, in the model")]
//!     r2 -- "what each row measured" --> heights
//!     heights -- "where each row starts" --> above
//! ```
//!
//! Each frame, the list works out where every row starts from the heights
//! it knows and the estimates for the rest (a sum, with no widget built),
//! finds the first row that reaches into the viewport, and builds rows
//! from there until one starts below it. Each row built is measured, and
//! what it measured is kept. If a row turned out another height than it
//! was laid out with, the list is laid out once more.
//!
//! Scrolled from the top, everything above the viewport has been measured, so
//! what is on screen never moves: only the scrollbar's idea of the total
//! changes as estimates below turn into measurements. After a jump over rows
//! never shown (dragging the scrollbar), the rows above are still estimates;
//! scrolling back up into them moves what is on screen by however far each
//! estimate was off, so the closer the estimate, the steadier the list.
//!
//! The estimate has to depend on the item and on how wide the list is, and
//! on nothing else. Assuming the average of the
//! rows measured so far looks better and is worse: every newly measured row
//! would move the average, and with it every unmeasured row above the
//! viewport, so after a jump the list would land on different rows frame
//! after frame.
//!
//! The list can say when it has been scrolled near its end, which is when a
//! feed fetches its next page. It says so in answer to any event but a redraw
//! (the pointer moving, a wheel turning), never while the window is being
//! redrawn: a message published during a redraw makes the
//! runtime take the window's size afresh in the middle of the frame, and if
//! the popup is being resized just then (as it is while it opens), the frame
//! is presented into a buffer of the old size and `iced_tiny_skia` panics.
//! That rules out the scrollable's own `on_scroll`, which reports during
//! redraws, and again each time a measured row changes the list's height.
//!
//! A row that is another height at its reader's asking (a long note
//! shown in full, or cut again) is the one row measured before it is in
//! view: its list is told ([`RowHeights::measure_anew`]), and measures it
//! in the layout that follows, so that the list is the right length at
//! once.
//!
//! Heights are remembered by row *key*, not index, in a [`RowHeights`] the
//! caller owns. Rows added at the front leave the rest measured, and the
//! measurements outlive the widget tree: a popup that is closed and opened
//! again lays its list out exactly as it was.
//!
//! Where the list is scrolled to outlives the widget tree the same way, but
//! is not the list's to keep: its scroller may hold more than the list. See
//! [`ScrollPosition`](super::ScrollPosition).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use cosmic::iced::core::layout;
use cosmic::iced::core::mouse;
use cosmic::iced::core::overlay;
use cosmic::iced::core::renderer;
use cosmic::iced::core::widget::Operation;
use cosmic::iced::core::widget::tree::{self, Tree};
use cosmic::iced::core::window;
use cosmic::iced::core::{Clipboard, Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Vector, Widget};

/// How far two heights may differ and still be the same height.
const SAME_HEIGHT: f32 = 0.01;

/// The measured heights of a list's rows, by row key, at one list width.
///
/// Owned by whoever owns the list's items, and lent to the [`VirtualList`]
/// built for each frame, which fills it in as rows come into view.
pub struct RowHeights<K> {
    measured: RefCell<Measured<K>>,
}

struct Measured<K> {
    /// The list width the heights were measured at. Text wraps differently
    /// at another width, so the heights are only good for this one.
    width: f32,
    heights: HashMap<K, f32>,
    /// The rows to measure before the list is next laid out, in view or
    /// not: see [`RowHeights::measure_anew`].
    anew: HashSet<K>,
}

impl<K> Default for RowHeights<K> {
    fn default() -> Self {
        Self { measured: RefCell::new(Measured { width: 0.0, heights: HashMap::new(), anew: HashSet::new() }) }
    }
}

impl<K: Eq + Hash + Clone> RowHeights<K> {
    /// How many rows have been measured.
    pub fn len(&self) -> usize {
        self.measured.borrow().heights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The measured height of the row `key`, if it has been in view.
    pub fn get(&self, key: &K) -> Option<f32> {
        self.measured.borrow().heights.get(key).copied()
    }

    /// How tall the rows of `items` are together: each its measured height,
    /// or what `estimate` says of the item until it has one, at the width
    /// the list was last laid out at (none, before it ever was). With the
    /// items before a row, that is where the row starts.
    pub fn total<T>(&self, items: &[T], key: impl Fn(&T) -> K, estimate: impl Fn(&T, f32) -> f32) -> f32 {
        let width = self.measured.borrow().width;
        items.iter().map(|item| self.get(&key(item)).unwrap_or_else(|| estimate(item, width).max(0.0))).sum()
    }

    /// Where each row of `items` starts, from the top of the list: each
    /// after the heights of those before it, measured or estimated as in
    /// [`Self::total`]. What a list is scrolled to to have a row at the
    /// top of what is in view.
    pub fn starts<T>(&self, items: &[T], key: impl Fn(&T) -> K, estimate: impl Fn(&T, f32) -> f32) -> Vec<f32> {
        let width = self.measured.borrow().width;
        let mut start = 0.0;
        items
            .iter()
            .map(|item| {
                let here = start;
                start += self.get(&key(item)).unwrap_or_else(|| estimate(item, width).max(0.0));
                here
            })
            .collect()
    }

    /// Forget how tall the row `key` measured, and have it measured before
    /// the list is next laid out, whether it is in view or not: the row is
    /// about to be another height, at its reader's asking.
    ///
    /// A row that only comes to be another height is found out when it
    /// is next in view, and the list is laid out again then. That is one
    /// layout too late for what follows such a change at once, as
    /// scrolling the list to a place counted from its new length.
    pub fn measure_anew(&self, key: K) {
        let mut measured = self.measured.borrow_mut();
        measured.heights.remove(&key);
        measured.anew.insert(key);
    }

    /// The rows to measure before the list is laid out, which are then no
    /// longer waiting to be.
    fn take_anew(&self) -> HashSet<K> {
        std::mem::take(&mut self.measured.borrow_mut().anew)
    }

    /// Lay out at `width` from now on, forgetting heights measured at another.
    fn set_width(&self, width: f32) {
        let mut measured = self.measured.borrow_mut();
        if measured.width != width {
            measured.width = width;
            measured.heights.clear();
        }
    }

    /// Remember that the row `key` is `height` tall. Returns whether that is
    /// news.
    fn record(&self, key: &K, height: f32) -> bool {
        let mut measured = self.measured.borrow_mut();
        match measured.heights.get_mut(key) {
            Some(known) if (*known - height).abs() <= SAME_HEIGHT => false,
            Some(known) => {
                *known = height;
                true
            }
            None => {
                measured.heights.insert(key.clone(), height);
                true
            }
        }
    }
}

/// How tall the row of an item is assumed to be until it has been in view,
/// in a list as wide as the second argument.
type Estimate<'a, T> = Box<dyn Fn(&T, f32) -> f32 + 'a>;

/// A virtual list of rows of any height. See the [module docs](self).
pub struct VirtualList<'a, T, K, Message, Theme, Renderer> {
    items: &'a [T],
    heights: &'a RowHeights<K>,
    key: Box<dyn Fn(&T) -> K + 'a>,
    view_item: Box<dyn Fn(usize, &'a T) -> Element<'a, Message, Theme, Renderer> + 'a>,
    estimate: Estimate<'a, T>,
    /// The rows in view, in the same order as [`State::rows`]. Rebuilt from
    /// `view_item` the first time this widget instance handles an event,
    /// because the app builds a new instance on every `view()`.
    visible: Vec<Element<'a, Message, Theme, Renderer>>,
    /// The viewport, and the list's own top, that `visible` was built for.
    synced: Option<(Rectangle, f32)>,
    /// How close to the list's end the viewport has to come, and what to
    /// publish when it does.
    near_end: Option<(f32, Message)>,
}

impl<'a, T, K, Message, Theme, Renderer> VirtualList<'a, T, K, Message, Theme, Renderer> {
    /// A list of `items`, each built by `view_item(index, item)` and known,
    /// across frames and reorderings, by `key(item)`. Until a row has been in
    /// view it is assumed to be `estimate(item, width)` tall, in a list
    /// `width` wide.
    pub fn new(
        items: &'a [T],
        heights: &'a RowHeights<K>,
        key: impl Fn(&T) -> K + 'a,
        estimate: impl Fn(&T, f32) -> f32 + 'a,
        view_item: impl Fn(usize, &'a T) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self {
        Self {
            items,
            heights,
            key: Box::new(key),
            estimate: Box::new(estimate),
            view_item: Box::new(view_item),
            visible: Vec::new(),
            synced: None,
            near_end: None,
        }
    }

    /// Publish `message` when the viewport comes within `distance` of the
    /// list's end: once for each place the list is scrolled to, so scrolling
    /// on asks again, and staying put doesn't.
    pub fn on_near_end(mut self, distance: f32, message: Message) -> Self {
        self.near_end = Some((distance, message));
        self
    }
}

/// One row in view: which item it shows, where it sits, and its widget state.
struct Row<K> {
    key: K,
    node: layout::Node,
    tree: Tree,
}

struct State<K> {
    /// The limits rows are laid out with: the list's width, and any height.
    limits: layout::Limits,
    /// Where each row starts, measured or assumed, and after the last one,
    /// where the list ends. Rebuilt by every layout.
    offsets: Vec<f32>,
    /// The rows in view, top to bottom.
    rows: Vec<Row<K>>,
    /// The list length and viewport position at which nearing the end was
    /// last published.
    near_end_told: Option<(usize, f32)>,
}

impl<'a, T, K, Message, Theme, Renderer> VirtualList<'a, T, K, Message, Theme, Renderer>
where
    K: Eq + Hash + Clone,
    Renderer: renderer::Renderer,
{
    /// Make `self.visible` and `state.rows` hold exactly the rows in
    /// `viewport`, each built for this frame, laid out, and placed under the
    /// one before it. Returns whether any of them turned out to be a different
    /// height than the list was laid out with.
    ///
    /// A row that was already in view keeps its widget state (hover, press),
    /// reconciled against the freshly built row.
    fn sync_visible(&mut self, state: &mut State<K>, layout: Layout<'_>, renderer: &Renderer, viewport: &Rectangle) -> bool {
        let top = layout.position().y;
        if self.synced == Some((*viewport, top)) {
            return false;
        }
        self.synced = Some((*viewport, top));

        let view_top = viewport.y - top;
        let view_bottom = view_top + viewport.height;

        // The last row that starts at or above the top of the viewport.
        let starts = state.offsets.get(..self.items.len()).unwrap_or_default();
        let first = starts.partition_point(|&start| start <= view_top).saturating_sub(1);
        let mut y = starts.get(first).copied().unwrap_or(0.0);

        let mut previous = std::mem::take(&mut state.rows);
        self.visible.clear();
        let mut changed = false;
        for (index, item) in self.items.iter().enumerate().skip(first) {
            if y >= view_bottom {
                break;
            }
            let key = (self.key)(item);
            let mut element = (self.view_item)(index, item);
            let mut tree = match previous.iter().position(|row| row.key == key) {
                Some(at) => {
                    let mut tree = previous.swap_remove(at).tree;
                    tree.diff(element.as_widget_mut());
                    tree
                }
                None => Tree::new(element.as_widget()),
            };
            let node = element.as_widget_mut().layout(&mut tree, renderer, &state.limits).move_to((0.0, y));
            let height = node.size().height;
            changed |= self.heights.record(&key, height);
            y += height;
            state.rows.push(Row { key, node, tree });
            self.visible.push(element);
        }
        changed
    }
}

impl<'a, T, K, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for VirtualList<'a, T, K, Message, Theme, Renderer>
where
    K: Eq + Hash + Clone + 'static,
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<K>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<K> { limits: layout::Limits::NONE, offsets: Vec::new(), rows: Vec::new(), near_end_told: None })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<K>>();

        // Rows are as wide as the list and as tall as they like.
        let width = limits.max().width;
        let row_limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        if state.limits != row_limits {
            state.limits = row_limits;
            state.rows.clear();
            self.visible.clear();
        }
        self.heights.set_width(width);

        // The rows whose height was asked to be measured anew: each is
        // built and laid out here, once, so that the list is as long as
        // it will be from this layout on.
        let anew = self.heights.take_anew();
        if !anew.is_empty() {
            for (index, item) in self.items.iter().enumerate() {
                let key = (self.key)(item);
                if anew.contains(&key) {
                    let mut element = (self.view_item)(index, item);
                    let mut row = Tree::new(element.as_widget());
                    let height = element.as_widget_mut().layout(&mut row, renderer, &row_limits).size().height;
                    self.heights.record(&key, height);
                }
            }
        }

        // The rows in view stay as they are until the next event places them
        // again: a layout asked for mid-frame is followed by a draw.
        self.synced = None;

        state.offsets.clear();
        state.offsets.reserve(self.items.len() + 1);
        let mut y = 0.0;
        for item in self.items {
            state.offsets.push(y);
            y += self.heights.get(&(self.key)(item)).unwrap_or_else(|| (self.estimate)(item, width).max(0.0));
        }
        state.offsets.push(y);

        layout::Node::new(limits.resolve(Length::Fill, Length::Shrink, Size::new(width, y)))
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
        let state = tree.state.downcast_mut::<State<K>>();
        if self.sync_visible(state, layout, renderer, viewport) {
            // The list is not the height it was laid out with. The rows in
            // view are already where they belong; the total, and the rows
            // below, follow in the next layout.
            shell.invalidate_layout();
            shell.request_redraw();
        }

        let offset = layout.position() - Point::ORIGIN;
        for (element, row) in self.visible.iter_mut().zip(&mut state.rows) {
            element.as_widget_mut().update(
                &mut row.tree,
                event,
                Layout::with_offset(offset, &row.node),
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }

        // Never in answer to a redraw: see the module docs for why.
        if let Some((distance, message)) = &self.near_end
            && !matches!(event, Event::Window(window::Event::RedrawRequested(_)))
            && !self.items.is_empty()
        {
            let end = state.offsets.last().copied().unwrap_or(0.0);
            let view_bottom = viewport.y + viewport.height - layout.position().y;
            let here = (self.items.len(), viewport.y);
            if end - view_bottom <= *distance && state.near_end_told != Some(here) {
                state.near_end_told = Some(here);
                shell.publish(message.clone());
            }
        }
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
        let state = tree.state.downcast_ref::<State<K>>();
        let offset = layout.position() - Point::ORIGIN;
        for (element, row) in self.visible.iter().zip(&state.rows) {
            element.as_widget().draw(&row.tree, renderer, theme, style, Layout::with_offset(offset, &row.node), cursor, viewport);
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
        let state = tree.state.downcast_ref::<State<K>>();
        let offset = layout.position() - Point::ORIGIN;
        self.visible
            .iter()
            .zip(&state.rows)
            .map(|(element, row)| {
                element.as_widget().mouse_interaction(
                    &row.tree,
                    Layout::with_offset(offset, &row.node),
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .max()
            .unwrap_or_default()
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        let state = tree.state.downcast_mut::<State<K>>();
        let offset = layout.position() - Point::ORIGIN;
        for (element, row) in self.visible.iter_mut().zip(&mut state.rows) {
            element.as_widget_mut().operate(&mut row.tree, Layout::with_offset(offset, &row.node), renderer, operation);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State<K>>();
        let offset = layout.position() - Point::ORIGIN;
        let children: Vec<_> = self
            .visible
            .iter_mut()
            .zip(&mut state.rows)
            .filter_map(|(element, row)| {
                element.as_widget_mut().overlay(
                    &mut row.tree,
                    Layout::with_offset(offset, &row.node),
                    renderer,
                    viewport,
                    translation,
                )
            })
            .collect();
        (!children.is_empty()).then(|| overlay::Group::with_children(children).overlay())
    }
}

impl<'a, T, K, Message, Theme, Renderer> From<VirtualList<'a, T, K, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    T: 'a,
    K: Eq + Hash + Clone + 'static,
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(list: VirtualList<'a, T, K, Message, Theme, Renderer>) -> Self {
        Element::new(list)
    }
}

#[cfg(test)]
mod tests {
    use super::{RowHeights, VirtualList};
    use cosmic::iced::core::widget::{Tree, tree};
    use cosmic::iced::core::{
        Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Widget, clipboard, layout, mouse, renderer, window,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    const WIDTH: f32 = 300.0;
    const VIEW: f32 = 300.0;
    /// What every row is assumed to be until it has been in view.
    const ESTIMATE: f32 = 50.0;

    /// A list item: its key, and how tall its row is.
    type Item = (u32, f32);

    /// A row of known height whose tree state is its `marker`, recording
    /// where it was drawn. The null renderer runs it without a display.
    struct Probe {
        height: f32,
        marker: u32,
        draws: Rc<RefCell<Vec<(u32, Rectangle)>>>,
    }

    impl<Msg, Theme, Renderer: renderer::Renderer> Widget<Msg, Theme, Renderer> for Probe {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fixed(self.height))
        }

        fn tag(&self) -> tree::Tag {
            tree::Tag::of::<u32>()
        }

        fn state(&self) -> tree::State {
            tree::State::new(self.marker)
        }

        fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
            layout::Node::new(Size::new(limits.max().width, self.height))
        }

        fn draw(
            &self,
            tree: &Tree,
            _renderer: &mut Renderer,
            _theme: &Theme,
            _style: &renderer::Style,
            layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
            self.draws.borrow_mut().push((*tree.state.downcast_ref::<u32>(), layout.bounds()));
        }
    }

    type Headless<'a> = Element<'a, (), (), ()>;

    /// Records each row built (by index) and each row drawn (by the marker
    /// its *state* holds, and where).
    #[derive(Default)]
    struct Log {
        built: Rc<RefCell<Vec<usize>>>,
        draws: Rc<RefCell<Vec<(u32, Rectangle)>>>,
    }

    /// A list whose rows carry `marker_base + key` as their state.
    fn list<'a>(items: &'a [Item], heights: &'a RowHeights<u32>, marker_base: u32, log: &Log) -> Headless<'a> {
        let (built, draws) = (log.built.clone(), log.draws.clone());
        VirtualList::new(
            items,
            heights,
            |item: &Item| item.0,
            |_, _| ESTIMATE,
            move |index, (key, height)| {
                built.borrow_mut().push(index);
                Headless::new(Probe { height: *height, marker: marker_base + key, draws: draws.clone() })
            },
        )
        .into()
    }

    fn lay_out(element: &mut Headless<'_>, tree: &mut Tree, width: f32) -> layout::Node {
        let limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        element.as_widget_mut().layout(tree, &(), &limits)
    }

    /// One frame scrolled to `scroll_y`, as the runtime runs it: layout, a
    /// redraw event, layout again for as long as the list asks, then draw.
    /// Returns the list's laid-out height.
    fn frame(element: &mut Headless<'_>, tree: &mut Tree, scroll_y: f32, log: &Log) -> f32 {
        let viewport = Rectangle::new(Point::new(0.0, scroll_y), Size::new(WIDTH, VIEW));
        let mut node = lay_out(element, tree, WIDTH);
        for _ in 0..16 {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            element.as_widget_mut().update(
                tree,
                &Event::Window(window::Event::RedrawRequested(std::time::Instant::now())),
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &(),
                &mut clipboard::Null,
                &mut shell,
                &viewport,
            );
            if !shell.is_layout_invalid() {
                break;
            }
            node = lay_out(element, tree, WIDTH);
        }
        log.draws.borrow_mut().clear();
        element.as_widget().draw(
            tree,
            &mut (),
            &(),
            &renderer::Style::default(),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &viewport,
        );
        node.size().height
    }

    /// Where each drawn row starts, top to bottom.
    fn drawn_at(log: &Log) -> Vec<f32> {
        log.draws.borrow().iter().map(|(_, bounds)| bounds.y).collect()
    }

    /// The markers of the drawn rows, top to bottom.
    fn drawn_markers(log: &Log) -> Vec<u32> {
        log.draws.borrow().iter().map(|(marker, _)| *marker).collect()
    }

    /// `count` items keyed 0.., with heights cycling through `heights`.
    fn items(count: u32, heights: &[f32]) -> Vec<Item> {
        (0..count).map(|key| (key, heights[key as usize % heights.len()])).collect()
    }

    #[test]
    fn rows_sit_under_one_another_at_their_own_heights() {
        let items = items(100, &[40.0, 100.0, 60.0, 30.0, 200.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());

        frame(&mut element, &mut tree, 0.0, &log);

        // 40 + 100 + 60 + 30 = 230: the fifth row starts inside the 300px
        // viewport and the sixth would start at 430, outside it.
        assert_eq!(drawn_at(&log), [0.0, 40.0, 140.0, 200.0, 230.0]);
        assert_eq!(heights.len(), 5, "only the rows in view were measured");
        assert!(log.built.borrow().iter().all(|index| *index < 5), "only the rows in view were built");
    }

    #[test]
    fn a_row_asked_to_be_measured_anew_is_measured_by_the_next_layout_in_view_or_not() {
        let mut items = items(100, &[40.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        {
            let mut element = list(&items, &heights, 0, &log);
            let mut tree = Tree::new(element.as_widget());
            // Eight rows are in view and measured: 320 in all, and 92 at
            // their estimate.
            assert_eq!(frame(&mut element, &mut tree, 0.0, &log), 8.0 * 40.0 + 92.0 * ESTIMATE);
        }

        // One in view grows and one far out of view shrinks, at their
        // reader's asking: the list is told of both.
        items[2].1 = 400.0;
        items[90].1 = 10.0;
        heights.measure_anew(2);
        heights.measure_anew(90);
        log.built.borrow_mut().clear();
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());

        // The first layout has them both, with no event in between, and
        // built no other row for it.
        let height = lay_out(&mut element, &mut tree, WIDTH).size().height;
        assert_eq!(height, 7.0 * 40.0 + 400.0 + 10.0 + 91.0 * ESTIMATE);
        assert_eq!(*log.built.borrow(), [2, 90]);
        assert_eq!((heights.get(&2), heights.get(&90)), (Some(400.0), Some(10.0)));

        // Once: the next layout builds nothing.
        log.built.borrow_mut().clear();
        assert_eq!(lay_out(&mut element, &mut tree, WIDTH).size().height, height);
        assert!(log.built.borrow().is_empty());

        // A row that is no longer in the list is not waited for.
        heights.measure_anew(1_000);
        assert_eq!(lay_out(&mut element, &mut tree, WIDTH).size().height, height);
        assert_eq!(heights.get(&1_000), None);
    }

    #[test]
    fn rows_not_yet_seen_are_as_tall_as_their_estimate() {
        let items = items(1_000, &[40.0, 100.0, 60.0, 30.0, 200.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());

        // Before anything is measured, every row is its estimate.
        assert_eq!(lay_out(&mut element, &mut tree, WIDTH).size().height, 1_000.0 * ESTIMATE);

        // Five rows measured, 430px in all; the other 995 are still estimates.
        let height = frame(&mut element, &mut tree, 0.0, &log);
        assert_eq!(height, 430.0 + 995.0 * ESTIMATE);
    }

    #[test]
    fn scrolling_down_never_moves_what_is_on_screen() {
        let cycle = [40.0, 100.0, 60.0, 30.0, 200.0];
        let items = items(200, &cycle);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());

        // Scroll down a little at a time, as a wheel does, past 60 rows.
        let mut scroll = 0.0;
        while scroll < 5_000.0 {
            frame(&mut element, &mut tree, scroll, &log);
            // Every drawn row is exactly where the rows above it put it.
            for (marker, bounds) in log.draws.borrow().iter() {
                let above: f32 = items.iter().take(*marker as usize).map(|(_, height)| height).sum();
                assert_eq!(bounds.y, above, "row {marker} at scroll {scroll}");
            }
            scroll += 120.0;
        }
    }

    #[test]
    fn jumping_far_builds_only_the_rows_that_land_in_view() {
        let items = items(5_000, &[40.0, 100.0, 60.0, 30.0, 200.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree, 0.0, &log);
        log.built.borrow_mut().clear();

        // Past 4000 rows that were never shown, so are 50px each: row 4000
        // starts at 200,180 (the five measured rows are 430px, not 250px).
        frame(&mut element, &mut tree, 200_200.0, &log);

        let built = log.built.borrow();
        let (first, last) = (built.iter().min().unwrap(), built.iter().max().unwrap());
        assert_eq!(*first, 4_000, "the row the estimates put at the top of the viewport");
        assert!(last - first < 8, "a viewport's worth of rows, not a list's: {first}..={last}");
        // The rows drawn cover the viewport without a gap.
        let draws = log.draws.borrow();
        assert_eq!(draws.first().unwrap().1.y, 200_180.0);
        for pair in draws.windows(2) {
            assert_eq!(pair[0].1.y + pair[0].1.height, pair[1].1.y);
        }
        let last_drawn = draws.last().unwrap().1;
        assert!(last_drawn.y + last_drawn.height >= 200_200.0 + VIEW);
    }

    #[test]
    fn a_jump_lands_on_the_same_rows_however_many_frames_follow() {
        let items = items(5_000, &[40.0, 100.0, 60.0, 30.0, 200.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree, 0.0, &log);

        frame(&mut element, &mut tree, 123_456.0, &log);
        let landed = (drawn_markers(&log), drawn_at(&log));

        // Measuring the rows in view must not move the rows above them.
        for _ in 0..5 {
            frame(&mut element, &mut tree, 123_456.0, &log);
            assert_eq!((drawn_markers(&log), drawn_at(&log)), landed);
        }
    }

    #[test]
    fn rows_added_at_the_front_leave_the_others_measured_and_in_their_state() {
        let older = items(50, &[40.0, 100.0, 60.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&older, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree, 0.0, &log);
        assert_eq!(drawn_markers(&log), [0, 1, 2, 3, 4]);

        // A newer note arrives: key 900, 70px, in front of the rest. The app
        // builds a new widget for the new list.
        let mut newer = vec![(900, 70.0)];
        newer.extend(older.iter().copied());
        let mut rebuilt = list(&newer, &heights, 1_000, &log);
        tree.diff(rebuilt.as_widget_mut());
        frame(&mut rebuilt, &mut tree, 0.0, &log);

        // The new row is new (marker 1900); rows 0..=3 are the same rows as
        // before (their state kept its old marker), each 70px further down.
        assert_eq!(drawn_markers(&log), [1_900, 0, 1, 2, 3]);
        assert_eq!(drawn_at(&log), [0.0, 70.0, 110.0, 210.0, 270.0]);
        assert_eq!(heights.get(&4), Some(100.0), "a row that left the view is still measured");
    }

    #[test]
    fn measurements_outlive_the_widget_tree() {
        let items = items(500, &[40.0, 100.0, 60.0, 30.0, 200.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());
        let mut scroll = 0.0;
        while scroll <= 2_000.0 {
            frame(&mut element, &mut tree, scroll, &log);
            scroll += 150.0;
        }
        let height = frame(&mut element, &mut tree, 1_800.0, &log);
        let (rows, at) = (drawn_markers(&log), drawn_at(&log));

        // The popup closes and opens again: a new widget and a new tree, the
        // same heights. The list is the same height before anything is drawn,
        // and scrolled back to 1800 it shows the same rows in the same places.
        let mut reopened = list(&items, &heights, 0, &log);
        let mut fresh = Tree::new(reopened.as_widget());
        assert_eq!(lay_out(&mut reopened, &mut fresh, WIDTH).size().height, height);
        frame(&mut reopened, &mut fresh, 1_800.0, &log);
        assert_eq!((drawn_markers(&log), drawn_at(&log)), (rows, at));
    }

    /// A list of `items` that publishes `()` within 200px of its end.
    fn telling_list<'a>(items: &'a [Item], heights: &'a RowHeights<u32>) -> Headless<'a> {
        VirtualList::new(
            items,
            heights,
            |item: &Item| item.0,
            |_, _| ESTIMATE,
            |_, (_, height)| Headless::new(Probe { height: *height, marker: 0, draws: Rc::default() }),
        )
        .on_near_end(200.0, ())
        .into()
    }

    /// Lay the list out and hand it `event` scrolled to `scroll_y`. Returns
    /// how many messages it published.
    fn published(element: &mut Headless<'_>, tree: &mut Tree, scroll_y: f32, event: Event) -> usize {
        let node = lay_out(element, tree, WIDTH);
        let viewport = Rectangle::new(Point::new(0.0, scroll_y), Size::new(WIDTH, VIEW));
        let mut messages = Vec::new();
        element.as_widget_mut().update(
            tree,
            &event,
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &(),
            &mut clipboard::Null,
            &mut Shell::new(&mut messages),
            &viewport,
        );
        messages.len()
    }

    #[test]
    fn nearing_the_end_is_told_once_for_each_place_and_never_while_redrawing() {
        // Twenty 50px rows: the list ends at 1000, so with a 300px viewport
        // "within 200px of the end" starts at a scroll of 500.
        let items = items(20, &[50.0]);
        let heights = RowHeights::default();
        let mut element = telling_list(&items, &heights);
        let mut tree = Tree::new(element.as_widget());
        let moved = || Event::Mouse(mouse::Event::CursorMoved { position: Point::ORIGIN });
        let redraw = || Event::Window(window::Event::RedrawRequested(std::time::Instant::now()));

        assert_eq!(published(&mut element, &mut tree, 0.0, moved()), 0, "far from the end");
        assert_eq!(published(&mut element, &mut tree, 499.0, moved()), 0, "201px from the end");
        assert_eq!(published(&mut element, &mut tree, 500.0, redraw()), 0, "never while redrawing");
        assert_eq!(published(&mut element, &mut tree, 500.0, moved()), 1, "200px from the end");
        assert_eq!(published(&mut element, &mut tree, 500.0, moved()), 0, "said already, and nothing moved");
        assert_eq!(published(&mut element, &mut tree, 560.0, moved()), 1, "scrolled on: ask again");

        // The next page arrives: the end is far again, then near again.
        let longer = self::items(40, &[50.0]);
        let mut rebuilt = telling_list(&longer, &heights);
        tree.diff(rebuilt.as_widget_mut());
        assert_eq!(published(&mut rebuilt, &mut tree, 560.0, moved()), 0);
        assert_eq!(published(&mut rebuilt, &mut tree, 1_500.0, moved()), 1);
    }

    #[test]
    fn rows_are_as_tall_together_as_measured_or_else_estimated() {
        // Rows of 80px, estimated at ESTIMATE until seen.
        let items = items(20, &[80.0]);
        let heights = RowHeights::default();
        let total = |upto: usize| heights.total(&items[..upto], |item: &Item| item.0, |_, _| ESTIMATE);
        assert_eq!(total(10), 10.0 * ESTIMATE);
        // Each row starts where those before it end.
        let starts = || heights.starts(&items, |item: &Item| item.0, |_, _| ESTIMATE);
        assert_eq!(starts()[..3], [0.0, ESTIMATE, 2.0 * ESTIMATE]);
        assert_eq!(starts().len(), items.len());

        // A frame at the top measures the rows in view, the first four.
        let mut element = list(&items, &heights, 0, &Log::default());
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree, 0.0, &Log::default());
        assert_eq!(total(4), 320.0);
        assert_eq!(starts()[..6], [0.0, 80.0, 160.0, 240.0, 320.0, 320.0 + ESTIMATE]);
        assert_eq!(total(10), 320.0 + 6.0 * ESTIMATE);
    }

    #[test]
    fn another_width_measures_again() {
        let items = items(100, &[40.0, 100.0]);
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());
        frame(&mut element, &mut tree, 0.0, &log);
        assert!(!heights.is_empty());

        // Text wraps differently in a narrower list: old heights are wrong.
        lay_out(&mut element, &mut tree, WIDTH - 50.0);
        assert!(heights.is_empty());
    }

    #[test]
    fn an_empty_list_has_no_height_and_builds_nothing() {
        let items: Vec<Item> = Vec::new();
        let (heights, log) = (RowHeights::default(), Log::default());
        let mut element = list(&items, &heights, 0, &log);
        let mut tree = Tree::new(element.as_widget());

        assert_eq!(frame(&mut element, &mut tree, 0.0, &log), 0.0);
        assert!(log.built.borrow().is_empty());
        assert!(log.draws.borrow().is_empty());
    }
}

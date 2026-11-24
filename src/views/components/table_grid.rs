// SPDX-License-Identifier: MIT

//! A table of a note: cells in rows and columns, each column as wide as
//! what is widest in it, and all of them together no wider than the room
//! there is.
//!
//! A table that fits is as wide as what it holds, and no wider. One that
//! doesn't is as wide as the room: the columns that would take less than
//! an even share of it keep their width, and the others share what is
//! left evenly and wrap what they hold. Every cell of a row is as tall
//! as the tallest of them, so that rows stay rows.
//!
//! Under each row but the last is a line, the one under the first row
//! the plainest: that row says what the columns are.
//!
//! The cells are elements like any other, and are told of everything a
//! cell could want to know: a link in one can be clicked. The table
//! itself takes no click, so one on it is one on what is under it.

use cosmic::iced::core::Renderer as _;
use cosmic::iced::core::layout::{Limits, Node};
use cosmic::iced::core::mouse;
use cosmic::iced::core::overlay;
use cosmic::iced::core::renderer::{self, Quad};
use cosmic::iced::core::widget::{Operation, Tree, tree};
use cosmic::iced::core::{Alignment, Clipboard, Color, Element, Event, Layout, Length, Rectangle, Shell, Size, Vector, Widget};

/// The room around what a cell holds: beside it, and above and under it.
pub const TABLE_CELL_PADDING_X: f32 = 10.0;
pub const TABLE_CELL_PADDING_Y: f32 = 5.0;

/// How thick the line under a row is.
pub const TABLE_LINE: f32 = 1.0;

/// How narrow a column may get, whatever else wants the room.
const COLUMN_MIN_WIDTH: f32 = 24.0;

/// A table of cells: see the module.
pub struct TableGrid<'a, Message> {
    /// The cells, row by row.
    cells: Vec<cosmic::Element<'a, Message>>,
    /// Where in its column each column's cells stand: as many as there
    /// are columns.
    columns: Vec<Alignment>,
    /// The colour of the line under the first row, and of the others.
    head_line: Color,
    line: Color,
}

impl<'a, Message> TableGrid<'a, Message> {
    /// A table of `cells`, row by row, with a column for each of
    /// `columns`. Cells past the last whole row are left out.
    pub fn new(columns: Vec<Alignment>, mut cells: Vec<cosmic::Element<'a, Message>>, head_line: Color, line: Color) -> Self {
        let whole = cells.len() - cells.len() % columns.len().max(1);
        cells.truncate(if columns.is_empty() { 0 } else { whole });
        Self { cells, columns, head_line, line }
    }
}

/// How tall each row came out, the room around its cells included: what
/// the lines are drawn by.
#[derive(Default)]
struct Rows(Vec<f32>);

/// How wide each column is, of columns that would be `natural` wide if
/// they had the room, in `room`: as wide as that if they fit, and
/// otherwise the narrow ones as they are and the rest sharing what is
/// left.
pub(super) fn column_widths(natural: &[f32], room: f32) -> Vec<f32> {
    if natural.iter().sum::<f32>() <= room {
        return natural.to_vec();
    }
    // The narrowest first: each takes what it wants if that is no more
    // than an even share of what is left, and the share if it is.
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by(|a, b| natural.get(*a).zip(natural.get(*b)).map_or(std::cmp::Ordering::Equal, |(a, b)| a.total_cmp(b)));
    let mut widths = vec![0.0; natural.len()];
    let mut left = room;
    for (taken, column) in order.iter().enumerate() {
        let share = left / (natural.len() - taken) as f32;
        let width = natural.get(*column).copied().unwrap_or_default().min(share).max(COLUMN_MIN_WIDTH);
        if let Some(slot) = widths.get_mut(*column) {
            *slot = width;
        }
        left = (left - width).max(0.0);
    }
    widths
}

impl<Message> Widget<Message, cosmic::Theme, cosmic::Renderer> for TableGrid<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Rows>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Rows::default())
    }

    fn children(&self) -> Vec<Tree> {
        self.cells.iter().map(|cell| Tree::new(cell.as_widget())).collect()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.cells);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &cosmic::Renderer, limits: &Limits) -> Node {
        let columns = self.columns.len();
        if columns == 0 {
            return Node::new(Size::ZERO);
        }
        let around = 2.0 * TABLE_CELL_PADDING_X;
        let unbounded = Limits::new(Size::ZERO, Size::INFINITE);

        // How wide each column would be with all the room it wants.
        let mut natural = vec![0.0_f32; columns];
        for (at, (cell, state)) in self.cells.iter_mut().zip(&mut tree.children).enumerate() {
            let width = cell.as_widget_mut().layout(state, renderer, &unbounded).size().width;
            if let Some(widest) = natural.get_mut(at % columns) {
                *widest = widest.max(width);
            }
        }
        let room = limits.max().width - around * columns as f32;
        let widths = if room.is_finite() { column_widths(&natural, room.max(0.0)) } else { natural };

        // Each cell in the width its column has, and each row as tall as
        // its tallest.
        let mut nodes = Vec::with_capacity(self.cells.len());
        let mut heights: Vec<f32> = Vec::new();
        for (at, (cell, state)) in self.cells.iter_mut().zip(&mut tree.children).enumerate() {
            let width = widths.get(at % columns).copied().unwrap_or_default();
            let node = cell.as_widget_mut().layout(state, renderer, &Limits::new(Size::ZERO, Size::new(width, f32::INFINITY)));
            if at % columns == 0 {
                heights.push(0.0);
            }
            if let Some(tallest) = heights.last_mut() {
                *tallest = tallest.max(node.size().height);
            }
            nodes.push(node);
        }

        // Each where it stands.
        let (mut x, mut y) = (0.0, 0.0);
        for (at, node) in nodes.iter_mut().enumerate() {
            let (row, column) = (at / columns, at % columns);
            if column == 0 {
                x = 0.0;
                if let Some(above) = row.checked_sub(1).and_then(|above| heights.get(above)) {
                    y += above + 2.0 * TABLE_CELL_PADDING_Y + TABLE_LINE;
                }
            }
            let width = widths.get(column).copied().unwrap_or_default();
            let height = heights.get(row).copied().unwrap_or_default();
            let align = self.columns.get(column).copied().unwrap_or(Alignment::Start);
            node.move_to_mut((x + TABLE_CELL_PADDING_X, y + TABLE_CELL_PADDING_Y));
            node.align_mut(align, Alignment::Start, Size::new(width, height));
            x += width + around;
        }
        let wide = widths.iter().sum::<f32>() + around * columns as f32;
        let tall = heights.last().map_or(0.0, |last| y + last + 2.0 * TABLE_CELL_PADDING_Y);
        tree.state.downcast_mut::<Rows>().0 = heights.iter().map(|height| height + 2.0 * TABLE_CELL_PADDING_Y).collect();
        Node::with_children(Size::new(wide, tall), nodes)
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
        for ((cell, state), layout) in self.cells.iter_mut().zip(&mut tree.children).zip(layout.children()) {
            cell.as_widget_mut().update(state, event, layout, cursor, renderer, clipboard, shell, viewport);
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
        for ((cell, state), layout) in self.cells.iter().zip(&tree.children).zip(layout.children()) {
            cell.as_widget().draw(state, renderer, theme, style, layout, cursor, viewport);
        }
        // A line under each row but the last.
        let bounds = layout.bounds();
        let rows = &tree.state.downcast_ref::<Rows>().0;
        let mut y = bounds.y;
        for (row, height) in rows.iter().enumerate().take(rows.len().saturating_sub(1)) {
            y += height;
            let line = Rectangle { x: bounds.x, y, width: bounds.width, height: TABLE_LINE };
            renderer.fill_quad(
                Quad { bounds: line, snap: true, ..Quad::default() },
                if row == 0 { self.head_line } else { self.line },
            );
            y += TABLE_LINE;
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
        self.cells
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((cell, state), layout)| cell.as_widget().mouse_interaction(state, layout, cursor, viewport, renderer))
            .max()
            .unwrap_or_default()
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &cosmic::Renderer, operation: &mut dyn Operation) {
        for ((cell, state), layout) in self.cells.iter_mut().zip(&mut tree.children).zip(layout.children()) {
            cell.as_widget_mut().operate(state, layout, renderer, operation);
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
        overlay::from_children(&mut self.cells, tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<TableGrid<'a, Message>> for Element<'a, Message, cosmic::Theme, cosmic::Renderer> {
    fn from(table: TableGrid<'a, Message>) -> Self {
        Self::new(table)
    }
}

#[cfg(test)]
mod tests {
    use super::{COLUMN_MIN_WIDTH, TABLE_CELL_PADDING_X, TableGrid, column_widths};
    use cosmic::iced::core::layout::Limits;
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Alignment, Color, Size};
    use cosmic::iced::widget::text::Wrapping;

    /// How large a table of `cells`, two to a row, lays out in room
    /// `room` wide, and where its cells are.
    fn laid_out(cells: &[&str], room: f32) -> (Size, Vec<cosmic::iced::core::Rectangle>) {
        let renderer = crate::views::components::testing::renderer();
        let cells = cells
            .iter()
            .map(|said| cosmic::widget::text(said.to_string()).size(13).wrapping(Wrapping::WordOrGlyph).into())
            .collect::<Vec<cosmic::Element<'_, ()>>>();
        let mut table: cosmic::Element<'_, ()> =
            TableGrid::new(vec![Alignment::Start, Alignment::End], cells, Color::WHITE, Color::WHITE).into();
        let mut tree = Tree::new(table.as_widget());
        let node = table.as_widget_mut().layout(&mut tree, &renderer, &Limits::new(Size::ZERO, Size::new(room, f32::INFINITY)));
        (node.size(), node.children().iter().map(|cell| cell.bounds()).collect())
    }

    #[test]
    fn a_table_that_fits_is_as_wide_as_what_it_holds() {
        let (size, cells) = laid_out(&["Key", "Since", "j", "0.1"], 600.0);
        assert!(size.width < 200.0, "{size:?}");
        // Rows are rows: the cells of one start as high as each other,
        let [key, since, j, version] = cells.as_slice() else { panic!("four cells: {cells:?}") };
        assert_eq!((key.y, j.y), (since.y, version.y));
        assert!(j.y > key.y + key.height);
        // and columns are columns: the first starts where the first does,
        // and the second, set to its far side, ends where the second does.
        assert_eq!(key.x, TABLE_CELL_PADDING_X);
        assert_eq!(j.x, key.x);
        assert!((since.x + since.width - (version.x + version.width)).abs() < 0.01, "{since:?} {version:?}");
    }

    #[test]
    fn a_table_that_does_not_fit_is_as_wide_as_the_room_and_wraps_what_it_holds() {
        let long = "words upon words upon words, ".repeat(12);
        let (narrow, _) = laid_out(&["What", "N", &long, "1"], 300.0);
        let (wide, _) = laid_out(&["What", "N", &long, "1"], 600.0);
        assert!((narrow.width - 300.0).abs() < 0.01 && (wide.width - 600.0).abs() < 0.01, "{narrow:?} {wide:?}");
        assert!(narrow.height > wide.height, "the narrower, the more lines: {narrow:?} {wide:?}");
    }

    #[test]
    fn columns_that_fit_are_as_wide_as_what_they_hold() {
        assert_eq!(column_widths(&[40.0, 120.0, 60.0], 400.0), [40.0, 120.0, 60.0]);
    }

    #[test]
    fn columns_that_do_not_fit_share_the_room_the_narrow_ones_first() {
        // The narrow one keeps its width, and the two that want more
        // than a share of what is left get as much each.
        assert_eq!(column_widths(&[900.0, 40.0, 500.0], 300.0), [130.0, 40.0, 130.0]);
        // All the room is used, and none of it twice.
        assert_eq!(column_widths(&[900.0, 40.0, 500.0], 300.0).iter().sum::<f32>(), 300.0);
        // With no room at all, a column is still something.
        assert_eq!(column_widths(&[900.0, 500.0], 10.0), [COLUMN_MIN_WIDTH, COLUMN_MIN_WIDTH]);
    }
}

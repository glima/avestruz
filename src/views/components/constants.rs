// SPDX-License-Identifier: MIT

//! Sizes shared between views, and the scroller they share.

use std::sync::LazyLock;

use cosmic::widget;

/// The scrollable holding the list of the view on show: a feed's, a
/// thread's.
///
/// One id for them all, because to the runtime they are one widget. Views
/// are laid out alike, and a scrollable that turns up where another was
/// takes over its state, its id (libcosmic's widget tree hands the old id
/// to the new widget) and how far it was scrolled. A view that wants its
/// list somewhere says so by scrolling this, every time it comes on show.
pub static LIST_SCROLL_ID: LazyLock<widget::Id> = LazyLock::new(widget::Id::unique);

/// How wide the popup is, at the least and at the most. Said twice, to the
/// compositor that places the popup and to the wrapper that sizes it to its
/// content, which would otherwise pin it to libcosmic's 360.
pub const POPUP_MIN_WIDTH: f32 = 420.0;
pub const POPUP_MAX_WIDTH: f32 = 640.0;

/// The room beside a view's list, inside its scroller: at the left, and at
/// the right, where the scrollbar is drawn over what is scrolled. And how
/// wide that leaves the list in a popup at its widest: what a picture
/// that fills its row is as wide as.
pub const LIST_PADDING_LEFT: u16 = 12;
pub const LIST_PADDING_RIGHT: u16 = 14;
pub const LIST_MAX_WIDTH: f32 = POPUP_MAX_WIDTH - (LIST_PADDING_LEFT + LIST_PADDING_RIGHT) as f32;

/// How tall the popup may be while the height of its screen is not known:
/// what fits any screen. And the least it may ever be told it may be,
/// however short the screen: the least the compositor is asked for.
pub const POPUP_MAX_HEIGHT_UNMEASURED: f32 = 600.0;
pub const POPUP_MIN_HEIGHT: f32 = 300.0;

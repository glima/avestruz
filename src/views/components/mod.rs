// SPDX-License-Identifier: MIT

//! Reusable UI components for Avestruz.
//!
//! Most of what is on screen is a list of notes, and a note's row is
//! most of these put together:
//!
//! ```mermaid
//! flowchart TB
//!     subgraph scroller["The scroller, with its ScrollPosition first"]
//!         subgraph list["VirtualList: only the rows in view are built"]
//!             subgraph row["note_row: one note"]
//!                 direction TB
//!                 head["avatar, name, when"]
//!                 text["What it says: text, links, mentions"]
//!                 media["media_placeholder: its pictures, their room held"]
//!                 video["video_box: its videos, played where they are"]
//!                 quote["quoted_note: a card, and a card in it"]
//!                 preview["link_preview: a card for the page it links to"]
//!                 actions["Reply, repost (menu), react (emoji), zap, share link, bookmark, relays seen on"]
//!                 head --- text --- media --- video --- quote --- preview --- actions
//!             end
//!         end
//!     end
//! ```
//!
//! | Module | What it is |
//! |---|---|
//! | [`avatar`](mod@avatar) | Someone's picture, round, or what stands in for it |
//! | [`card`](mod@card) | How something shown inside a note is set apart from it |
//! | [`chip`](mod@chip) | A small choice button |
//! | [`constants`](mod@constants) | Sizes shared between views, and the scroller they share |
//! | [`icons`](mod@icons) | The app icon, embedded |
//! | [`link_preview`](mod@link_preview) | What a web page a note links to says of itself, as a card |
//! | [`media_placeholder`](mod@media_placeholder) | The space a picture will take, held while it loads |
//! | [`menu`](mod@menu) | A small menu that opens under what was pressed |
//! | [`note_row`](mod@note_row) | One note as a row of a list |
//! | [`notification_row`](mod@notification_row) | One thing the account is told of, as a row of a list |
//! | [`picture`](mod@picture) | A picture in a note, drawn pixel for pixel where it can be |
//! | [`qr_code`](mod@qr_code) | A QR code, as a picture |
//! | [`reaction_menu`](mod@reaction_menu) | What a note can be reacted to with: the emoji used most, and all there are |
//! | [`quoted_note`](mod@quoted_note) | A note another note quotes, as a card under it |
//! | [`scroll_position`](mod@scroll_position) | The scroller the views' lists are in, and how far it is scrolled |
//! | [`status_line`](mod@status_line) | A line of small centred text |
//! | [`video_box`](mod@video_box) | A video a note links to, where it plays, and the bar it is played by |
//! | [`virtual_list`](mod@virtual_list) | A virtual list of rows of any height: what the feed, threads and profiles scroll in |

pub mod avatar;
pub mod badge;
pub mod card;
pub mod chip;
pub mod constants;
pub mod icons;
pub mod link_preview;
pub mod media_placeholder;
pub mod menu;
pub mod note_row;
pub mod notification_row;
pub mod picture;
pub mod qr_code;
pub mod quoted_note;
pub mod reaction_menu;
pub mod scroll_position;
pub mod status_line;
pub mod video_box;
pub mod virtual_list;

pub use avatar::avatar;
pub use badge::{count_badge, dot_badge};
pub use card::{CARD_BORDER, CARD_RADIUS, card_style};
pub use chip::chip;
pub use constants::{
    LIST_MAX_WIDTH, LIST_PADDING_LEFT, LIST_PADDING_RIGHT, LIST_SCROLL_ID, POPUP_MAX_HEIGHT_UNMEASURED, POPUP_MAX_WIDTH,
    POPUP_MIN_HEIGHT, POPUP_MIN_WIDTH,
};
pub use icons::{
    app_icon, bell_icon, bookmark_icon, fold_icon, like_icon, link_icon, relays_icon, reply_icon, repost_icon, zap_icon,
};
pub use link_preview::{link_preview, link_preview_height};
pub use media_placeholder::{MediaPlaceholder, shown_size};
pub use menu::{MENU_GAP, MENU_WIDTH, choice_menu, menu, menu_at, menu_height};
pub use note_row::{
    Bookmarking, Reacting, RowContext, RowSizes, estimated_note_height, link_line, marking, note_row, opening_thread,
};
pub use notification_row::{estimated_notification_height, notification_row};
pub use picture::Picture;
pub use qr_code::qr_code;
pub use quoted_note::{quote_link, quoted_note, quoted_note_height};
pub use reaction_menu::{emoji_search_id, reaction_menu};
pub use scroll_position::{PlaceInScroller, Restoring, ScrollPosition, list_scroller};
pub use status_line::status_line;
pub use video_box::{video_box, video_height};
pub use virtual_list::{RowHeights, VirtualList};

/// What the tests of the views are run with.
#[cfg(test)]
pub(crate) mod testing {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use cosmic::iced::core::renderer::Headless;

    /// The renderer rows are laid out by in tests: the CPU one, whichever
    /// the program is built to draw with, and with no window to draw in.
    pub(crate) fn renderer() -> cosmic::Renderer {
        let made = pin!(<cosmic::Renderer as Headless>::new(cosmic::iced::Font::default(), 14.0.into(), Some("tiny-skia")));
        // Making it waits for nothing: it is there at the first asking.
        match made.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Some(renderer)) => renderer,
            _ => panic!("the CPU renderer is always to be had"),
        }
    }
}

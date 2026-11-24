// SPDX-License-Identifier: MIT

//! Icons embedded in the binary.

use cosmic::widget::icon;

/// The app icon. Embedded rather than looked up by name in the icon theme, so
/// it is there before the applet is installed.
const APP_ICON_SVG: &[u8] = include_bytes!("../../../resources/icon.svg");

/// The icons of a note's actions. The icon theme has none for a zap, and
/// those it has for the others don't make a set with it.
const REPLY_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/reply-symbolic.svg");
const REPOST_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/repost-symbolic.svg");
const LIKE_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/like-symbolic.svg");
const LIKED_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/liked-symbolic.svg");
const BOOKMARK_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/bookmark-symbolic.svg");
const BOOKMARKED_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/bookmarked-symbolic.svg");
const ZAP_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/zap-symbolic.svg");
const RELAYS_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/relays-symbolic.svg");
const LINK_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/link-symbolic.svg");

/// A bell: what the account is told of. The icon theme's are filled or
/// outlined as the theme likes, and this one goes with the others here.
const BELL_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/bell-symbolic.svg");

/// The icons of the way to the rest of a long note, and back. The icon
/// theme has arrows that go one way, and these go two.
const SHOW_MORE_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/show-more-symbolic.svg");
const SHOW_LESS_ICON_SVG: &[u8] = include_bytes!("../../../resources/icons/show-less-symbolic.svg");

/// An embedded icon that is drawn in whatever colour it is given.
fn symbolic(svg: &'static [u8]) -> icon::Handle {
    let mut handle = icon::from_svg_bytes(svg);
    handle.symbolic = true;
    handle
}

/// A speech bubble: answering a note.
pub fn reply_icon() -> icon::Handle {
    symbolic(REPLY_ICON_SVG)
}

/// Two arrows, each after the other: passing a note on.
pub fn repost_icon() -> icon::Handle {
    symbolic(REPOST_ICON_SVG)
}

/// A heart: in outline for a note that can be liked, filled for one that is.
pub fn like_icon(liked: bool) -> icon::Handle {
    symbolic(if liked { LIKED_ICON_SVG } else { LIKE_ICON_SVG })
}

/// A bolt of lightning: what a note was zapped.
pub fn zap_icon() -> icon::Handle {
    symbolic(ZAP_ICON_SVG)
}

/// A ribbon: in outline for a note that can be bookmarked, filled for one
/// that is.
pub fn bookmark_icon(bookmarked: bool) -> icon::Handle {
    symbolic(if bookmarked { BOOKMARKED_ICON_SVG } else { BOOKMARK_ICON_SVG })
}

/// Two links of a chain: a link to a note, to give to someone.
pub fn link_icon() -> icon::Handle {
    symbolic(LINK_ICON_SVG)
}

/// Two arrowheads: pointing apart, for showing more of a long note, or at
/// each other, for showing `less` of it.
pub fn fold_icon(less: bool) -> icon::Handle {
    symbolic(if less { SHOW_LESS_ICON_SVG } else { SHOW_MORE_ICON_SVG })
}

/// A bell: the way to what the account is told of.
pub fn bell_icon() -> icon::Handle {
    symbolic(BELL_ICON_SVG)
}

/// Two machines, one on the other: the relays a note was seen on.
pub fn relays_icon() -> icon::Handle {
    symbolic(RELAYS_ICON_SVG)
}

/// The app icon. A symbolic icon wherever it is shown: drawn in the theme's
/// foreground colour, which can be told from a dark ground and from a light
/// one, as the colour in the file cannot.
pub fn app_icon() -> icon::Handle {
    symbolic(APP_ICON_SVG)
}

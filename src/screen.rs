// SPDX-License-Identifier: MIT

//! The screen the popup opens on: how much of its height there is for the
//! popup, and how many of its pixels a point is.
//!
//! The popup may be as tall as its screen has room, and a picture is kept
//! at as many pixels as it takes on it (see [`crate::images`]). Neither
//! is told to a program: both are what the compositor does with a surface
//! of its own. So the window keeps one that it asks: the surface that is
//! there from the start and shows nothing (see
//! [`keep_what_is_copied`](crate::state::AppModel::keep_what_is_copied))
//! is a point wide and reaches from the top of the screen to the bottom,
//! beside whatever keeps room for itself at an edge, as the panel does.
//! How tall the compositor made it is how much room there is, and the
//! scale it is drawn at is the screen's.
//!
//! It is asked when the window starts, and again whenever the popup
//! opens, since a screen is something that gets plugged in and out, and
//! set to another scale. Until it has answered, the popup keeps to a
//! height that fits any screen, and pictures to a size that is sharp on
//! any.
//!
//! Heights are logical: a display 2160 pixels tall at 170% is 1270 points
//! tall, and points are what a layout is in. Its scale is how many pixels
//! that makes a point: 1.7.

/// How tall a surface is, at the least, that the compositor has laid out
/// from the top of a screen to the bottom: one that is less was not laid
/// out yet, and says nothing of its screen.
const LAID_OUT: f32 = 100.0;

/// A screen, as far as the window has to know it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    /// How much of its height there is for the popup, in points: all of
    /// it, but what keeps room for itself at its top or bottom.
    pub height: f32,
    /// How many of its pixels a point is.
    pub scale: f32,
}

/// What a surface that reaches from the top of its screen to the bottom
/// says of it: one that is `height` points tall, and drawn at `scale`.
/// `None` if it was not laid out yet, or says nonsense.
pub fn measured(height: f32, scale: f32) -> Option<Screen> {
    let sound = height.is_finite() && height >= LAID_OUT && scale.is_finite() && scale > 0.0;
    sound.then_some(Screen { height, scale })
}

#[cfg(test)]
mod tests {
    use super::{Screen, measured};

    #[test]
    fn a_surface_as_tall_as_its_screen_has_room_says_how_tall_that_is_and_at_what_scale() {
        // 2160 pixels at 170% are 1270 points, less a panel of 40.
        assert_eq!(measured(1230.0, 1.7), Some(Screen { height: 1230.0, scale: 1.7 }));
        assert_eq!(measured(1080.0, 1.0), Some(Screen { height: 1080.0, scale: 1.0 }));
    }

    #[test]
    fn a_surface_not_laid_out_yet_is_no_answer_rather_than_a_wrong_one() {
        // As it is before the compositor has said how large it is.
        assert_eq!(measured(1.0, 1.0), None);
        assert_eq!(measured(0.0, 1.7), None);
        // And nonsense.
        assert_eq!(measured(1230.0, 0.0), None);
        assert_eq!(measured(1230.0, f32::NAN), None);
        assert_eq!(measured(f32::INFINITY, 1.0), None);
    }
}

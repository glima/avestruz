// SPDX-License-Identifier: MIT

//! A QR code, as a picture: for a wallet on another device to scan.
//!
//! Always dark on white, whatever the theme: that is what scanners expect,
//! and a code in the theme's colours may not have the contrast.

use cosmic::widget::image::Handle;
use qrcode::{Color, QrCode};

/// How many pixels a side each module of the code is in the picture. The
/// view scales the picture; this only has to be enough for the edges to
/// stay sharp.
const MODULE_PIXELS: usize = 8;

/// How many modules of white are left around the code. A scanner needs four
/// to tell where the code ends.
const QUIET_MODULES: usize = 4;

/// The code for `data` as square RGBA pixels: how many a side, and the
/// pixels. `None` if `data` is more than a QR code holds.
fn pixels(data: &str) -> Option<(usize, Vec<u8>)> {
    let code = QrCode::new(data).ok()?;
    let modules = code.width();
    let dark: Vec<bool> = code.into_colors().into_iter().map(|color| color == Color::Dark).collect();

    let side = (modules + 2 * QUIET_MODULES) * MODULE_PIXELS;
    let mut pixels = vec![255_u8; side * side * 4];
    for (index, _) in dark.iter().enumerate().filter(|(_, dark)| **dark) {
        let (left, top) = ((index % modules + QUIET_MODULES) * MODULE_PIXELS, (index / modules + QUIET_MODULES) * MODULE_PIXELS);
        for y in top..top + MODULE_PIXELS {
            let row = (y * side + left) * 4;
            if let Some(row) = pixels.get_mut(row..row + MODULE_PIXELS * 4) {
                // Black, and as opaque as the white around it.
                row.as_chunks_mut::<4>().0.iter_mut().for_each(|pixel| *pixel = [0, 0, 0, 255]);
            }
        }
    }
    Some((side, pixels))
}

/// The QR code for `data`, as a picture. `None` if `data` is more than a
/// code holds.
pub fn qr_code(data: &str) -> Option<Handle> {
    let (side, pixels) = pixels(data)?;
    let side = u32::try_from(side).ok()?;
    Some(Handle::from_rgba(side, side, pixels))
}

#[cfg(test)]
mod tests {
    use super::{MODULE_PIXELS, QUIET_MODULES, pixels};

    /// Whether the pixel at (`x`, `y`) of a picture `side` wide is dark.
    fn is_dark(pixels: &[u8], side: usize, x: usize, y: usize) -> bool {
        pixels[(y * side + x) * 4] == 0
    }

    #[test]
    fn a_code_is_square_dark_on_white_with_room_around_it() {
        // A short text makes the smallest code: 21 modules a side.
        let (side, pixels) = pixels("LIGHTNING:LNBC1").unwrap();
        assert_eq!(side, (21 + 2 * QUIET_MODULES) * MODULE_PIXELS);
        assert_eq!(pixels.len(), side * side * 4);
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255), "nothing shows through");

        // The margin is white all the way round.
        let margin = QUIET_MODULES * MODULE_PIXELS;
        assert!((0..side).all(|at| !is_dark(&pixels, side, at, margin - 1) && !is_dark(&pixels, side, margin - 1, at)));
        assert!((0..side).all(|at| !is_dark(&pixels, side, at, side - margin) && !is_dark(&pixels, side, side - margin, at)));

        // Every code starts with a finder pattern in its top left corner: a
        // dark ring seven modules wide, a light ring inside it, a dark middle.
        let module = |x: usize, y: usize| is_dark(&pixels, side, margin + x * MODULE_PIXELS + 3, margin + y * MODULE_PIXELS + 3);
        assert!((0..7).all(|at| module(at, 0) && module(0, at) && module(at, 6) && module(6, at)));
        assert!((1..6).all(|at| !module(at, 1) && !module(1, at)));
        assert!(module(3, 3));
    }

    #[test]
    fn a_longer_text_makes_a_larger_code_and_too_long_a_text_none() {
        let (short, _) = pixels("LNBC1").unwrap();
        let (long, _) = pixels(&"LNBC210N1PJQ9XYZ".repeat(20)).unwrap();
        assert!(long > short);
        assert_eq!(pixels(&"x".repeat(10_000)), None);
    }
}

// SPDX-License-Identifier: MIT

//! A picture of the popup, taken by the popup itself: Ctrl+Shift+S.
//!
//! The popup closes when anything else is clicked, so a screenshot tool
//! that is clicked on has nothing left to take. This asks the renderer for
//! what it last drew of the popup and writes that as a PNG to the user's
//! pictures folder, as `avestruz-<date>_<time>.png`. The popup's rounded
//! corners are transparent in it, as they were drawn.
//!
//! The picture is of whatever shows: other people's notes, and anything
//! being written. It goes to a file and nowhere else, and where it went
//! is said in the journal.

use std::path::{Path, PathBuf};

use cosmic::iced::core::window::Screenshot;
use cosmic::iced::runtime::window::screenshot;
use cosmic::prelude::*;
use image::{ImageBuffer, Rgba};

use crate::messages::Message;
use crate::state::AppModel;

/// Why a picture of the popup was not written.
#[derive(Debug)]
pub enum ScreenshotError {
    /// What the renderer handed over is not as many bytes as its size says.
    Size { width: u32, height: u32, expected: usize, got: usize },
    /// The folder could not be made.
    Folder(PathBuf, std::io::Error),
    /// The file could not be written.
    File(PathBuf, image::ImageError),
}

impl std::fmt::Display for ScreenshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Size { width, height, expected, got } => {
                write!(f, "{width}×{height} pixels are {expected} bytes, and {got} came")
            }
            Self::Folder(folder, e) => write!(f, "{} could not be made: {e}", folder.display()),
            Self::File(path, e) => write!(f, "{} could not be written: {e}", path.display()),
        }
    }
}

/// What a picture taken at `at` is called.
fn file_name(at: chrono::DateTime<chrono::Local>) -> String {
    format!("avestruz-{}.png", at.format("%Y-%m-%d_%H-%M-%S"))
}

/// Where pictures go: the user's pictures folder, or `Pictures` at home
/// where the desktop names none.
fn pictures_folder() -> PathBuf {
    dirs::picture_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("Pictures"))
}

/// Write `taken` as a PNG called `name` in `folder`, which is made if it
/// is not there, and say where it is. The pixels are written as they
/// came, transparency included.
fn save_png(taken: &Screenshot, folder: &Path, name: &str) -> Result<PathBuf, ScreenshotError> {
    let (width, height) = (taken.size.width, taken.size.height);
    let Some(picture) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, taken.rgba.to_vec()) else {
        return Err(ScreenshotError::Size {
            width,
            height,
            expected: (width as usize) * (height as usize) * 4,
            got: taken.rgba.len(),
        });
    };
    std::fs::create_dir_all(folder).map_err(|e| ScreenshotError::Folder(folder.to_path_buf(), e))?;
    let path = folder.join(name);
    picture.save(&path).map_err(|e| ScreenshotError::File(path.clone(), e))?;
    Ok(path)
}

impl AppModel {
    /// Ctrl+Shift+S: ask the renderer for what it drew of the popup.
    /// Nothing comes of it with the popup closed: there is nothing drawn.
    pub fn handle_take_screenshot(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(id) = self.popup else {
            return Task::none();
        };
        screenshot(id).map(|taken| cosmic::Action::App(Message::ScreenshotTaken(taken)))
    }

    /// The renderer handed the picture over: write it to the pictures
    /// folder. Encoding a PNG as large as the popup takes a moment, which
    /// a thread of its own spends, so that the popup goes on being drawn.
    pub fn handle_screenshot_taken(&mut self, taken: Screenshot) {
        let name = file_name(chrono::Local::now());
        let saving = std::thread::Builder::new().name("screenshot".into()).spawn(move || {
            match save_png(&taken, &pictures_folder(), &name) {
                Ok(path) => {
                    tracing::info!(
                        "a picture of the popup is at {} ({}×{})",
                        path.display(),
                        taken.size.width,
                        taken.size.height
                    );
                }
                Err(e) => tracing::error!("a picture of the popup was not kept: {e}"),
            }
        });
        if let Err(e) = saving {
            tracing::error!("a picture of the popup was not kept: no thread to write it on: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScreenshotError, file_name, save_png};
    use chrono::TimeZone;
    use cosmic::iced::Size;
    use cosmic::iced::core::window::Screenshot;

    /// A folder of this test's own, with nothing in it, to write into.
    fn folder(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("avestruz-test-{}-screenshot-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// A picture `width` by `height`, of these bytes.
    fn taken(width: u32, height: u32, rgba: Vec<u8>) -> Screenshot {
        Screenshot::new(rgba, Size::new(width, height), 1.0)
    }

    #[test]
    fn a_picture_is_called_after_when_it_was_taken() {
        let at = chrono::Local.with_ymd_and_hms(2026, 3, 9, 7, 5, 2).unwrap();
        assert_eq!(file_name(at), "avestruz-2026-03-09_07-05-02.png");
    }

    #[test]
    fn a_picture_is_written_as_it_came_transparency_included() {
        // Two pixels: one opaque, one a corner the popup did not draw on.
        let pixels = vec![200, 100, 50, 255, 0, 0, 0, 0];
        // The folder is not there yet, and is made on the way.
        let into = folder("written").join("Pictures");
        let path = save_png(&taken(2, 1, pixels.clone()), &into, "shot.png").unwrap();
        assert_eq!(path, into.join("shot.png"));
        let read = image::open(&path).unwrap().into_rgba8();
        assert_eq!((read.width(), read.height()), (2, 1));
        assert_eq!(read.into_raw(), pixels);
        let _ = std::fs::remove_dir_all(folder("written"));
    }

    #[test]
    fn a_picture_that_is_not_the_size_it_says_is_not_written() {
        let into = folder("wrong-size");
        let problem = save_png(&taken(2, 2, vec![0; 8]), &into, "shot.png").unwrap_err();
        assert!(matches!(problem, ScreenshotError::Size { expected: 16, got: 8, .. }), "{problem}");
        assert!(!into.exists(), "nothing was made for it");
    }
}

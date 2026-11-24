// SPDX-License-Identifier: MIT

//! Image message handlers for Avestruz.
//!
//! A view that needs a picture asks the [`ImageStore`](crate::images::ImageStore),
//! which sends the address here as a `LoadImage`. This is where it is decided
//! whether to fetch it, and where the fetched picture comes back to.
//!
//! A picture is copied from here as well: a click on one with the right
//! button puts it on the clipboard. What is kept of a picture is as many
//! pixels as it is shown with, which may be far fewer than it has, so
//! what is copied is the picture itself, fetched again from where it is.
//!
//! That takes a moment, and something copied is on its way somewhere
//! else: the popup may be gone before the picture is here, and only what
//! has the keyboard may put anything on the clipboard. So the clipboard
//! is given the picture at once, as something that is still to arrive
//! ([`CopiedPicture`]): what it is asked for when it is pasted, it waits
//! for then.
//!
//! ```mermaid
//! sequenceDiagram
//!     actor person as Someone
//!     participant model as AppModel
//!     participant clipboard as The clipboard
//!     participant web as Where the picture is
//!     person->>model: a right click on a picture
//!     model->>clipboard: a picture, yet to arrive, and its address as text
//!     model->>web: fetch it
//!     web-->>model: the picture: made a PNG, the way up it says it is
//!     person->>clipboard: paste, somewhere else
//!     clipboard-->>person: the picture, once it is here, or what was shown of it if it never came
//! ```

use std::borrow::Cow;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use cosmic::iced::clipboard;
use cosmic::iced::clipboard::mime::AsMimeTypes;
use cosmic::prelude::*;

use crate::images::{self, ImageRequest, ImageShape, ImageState, LoadedImage};
use crate::messages::Message;
use crate::state::AppModel;

/// What a picture is on the clipboard as: what takes a picture from one
/// anywhere.
const PICTURE: &str = "image/png";

/// What text is on the clipboard as.
const TEXTS: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// How long a paste waits for a picture that is still on its way: a
/// little longer than a fetch may take.
const PASTE_WAITS: Duration = Duration::from_secs(25);

/// What there is of a copied picture so far.
#[derive(Default)]
struct Arrived {
    /// What was shown of it, as a PNG: what is pasted if the picture
    /// itself never comes.
    shown: Option<Vec<u8>>,
    /// The picture itself, as a PNG, or that it could not be had. `None`
    /// while it is on its way.
    whole: Option<Result<Vec<u8>, ()>>,
}

/// A picture on the clipboard, which may be still on its way from where
/// it is: as a PNG, to whatever takes pictures, and as its address, to
/// whatever takes text.
///
/// It is handed to the clipboard before the picture is here, and asked
/// for the picture when something is pasted: it waits then, until the
/// picture has come or [`PASTE_WAITS`] is up.
#[derive(Clone)]
pub struct CopiedPicture {
    url: String,
    arrived: Arc<(Mutex<Arrived>, Condvar)>,
}

impl CopiedPicture {
    /// The picture at `url`, none of it here yet.
    pub fn new(url: String) -> Self {
        Self { url, arrived: Arc::default() }
    }

    /// Note what has come of the picture, and tell whoever waits.
    fn note(&self, change: impl FnOnce(&mut Arrived)) {
        let (arrived, changed) = &*self.arrived;
        change(&mut arrived.lock().unwrap_or_else(PoisonError::into_inner));
        changed.notify_all();
    }

    /// Have the picture come: what `shown` shows of it first, which is at
    /// hand, and then the picture itself, fetched from where it is.
    pub async fn arrive(self, http: reqwest::Client, shown: Option<LoadedImage>) {
        if let Some(shown) = shown {
            let png = tokio::task::spawn_blocking(move || images::shown_as_png(&shown)).await;
            if let Ok(Ok(png)) = png {
                self.note(|arrived| arrived.shown = Some(png));
            }
        }
        let whole = images::fetch_as_png(&http, &self.url).await;
        if let Err(e) = &whole {
            tracing::debug!("the picture copied could not be fetched again ({e}): {}", self.url);
        }
        self.note(|arrived| arrived.whole = Some(whole.map_err(|_| ())));
    }

    /// The picture, as a PNG: itself, once it has come, waited for as
    /// long as `wait`; and what was shown of it, if it has not come by
    /// then or could not be had.
    fn png(&self, wait: Duration) -> Option<Vec<u8>> {
        let (arrived, changed) = &*self.arrived;
        let arrived = arrived.lock().unwrap_or_else(PoisonError::into_inner);
        let (arrived, _) =
            changed.wait_timeout_while(arrived, wait, |arrived| arrived.whole.is_none()).unwrap_or_else(PoisonError::into_inner);
        match &arrived.whole {
            Some(Ok(whole)) => Some(whole.clone()),
            _ => arrived.shown.clone(),
        }
    }
}

impl AsMimeTypes for CopiedPicture {
    fn available(&self) -> Cow<'static, [String]> {
        std::iter::once(PICTURE).chain(TEXTS).map(str::to_string).collect()
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        if mime_type == PICTURE {
            self.png(PASTE_WAITS).map(Cow::Owned)
        } else if TEXTS.contains(&mime_type) {
            Some(Cow::Owned(self.url.clone().into_bytes()))
        } else {
            None
        }
    }
}

impl AppModel {
    /// A view asked for a picture. Fetch it, unless it is here, failed, or
    /// on its way already, or too many others are.
    pub fn handle_load_image(&mut self, request: ImageRequest) -> Task<cosmic::Action<Message>> {
        if !self.images.begin(&request) {
            return Task::none();
        }
        let http = self.http.clone();
        let scale = self.screen_scale();
        Task::perform(
            async move {
                let result = images::fetch(http, request.clone(), scale).await;
                (request, result)
            },
            |(request, result)| cosmic::Action::App(Message::ImageLoaded(request, result)),
        )
    }

    /// A picture arrived, or didn't. Either way the views that wanted it
    /// are built again, and the ones still waiting ask again.
    pub fn handle_image_loaded(&mut self, request: ImageRequest, result: Result<LoadedImage, String>) {
        if let Err(e) = &result {
            // Dead links are ordinary on Nostr: not worth a banner.
            tracing::debug!("picture not shown ({e}): {}", request.url);
        }
        self.images.finish(request, result);
    }

    /// A picture was clicked with the right button: put it on the
    /// clipboard (see the [module](self)), and have it say so.
    pub fn handle_copy_picture(&mut self, url: String) -> Task<cosmic::Action<Message>> {
        let shown = match self.images.get(&url, ImageShape::Original) {
            ImageState::Ready(loaded) => Some(loaded.clone()),
            ImageState::Loading | ImageState::Failed => None,
        };
        let copied = CopiedPicture::new(url.clone());
        let arriving = copied.clone().arrive(self.http.clone(), shown);
        self.picture_copied = Some(url);
        Task::batch([clipboard::write_data(copied), Task::future(arriving).discard()])
    }
}

#[cfg(test)]
mod tests {
    use super::{CopiedPicture, PICTURE};
    use cosmic::iced::clipboard::mime::AsMimeTypes;
    use std::time::Duration;

    const URL: &str = "https://cdn.example.com/picture.jpg";

    #[test]
    fn a_copied_picture_is_a_picture_to_what_takes_pictures_and_its_address_to_what_takes_text() {
        let copied = CopiedPicture::new(URL.into());
        let offered = copied.available();
        assert_eq!(offered.first().map(String::as_str), Some(PICTURE));
        for text in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
            assert!(offered.iter().any(|kind| kind == text));
            assert_eq!(copied.as_bytes(text).as_deref(), Some(URL.as_bytes()));
        }
        assert_eq!(copied.as_bytes("text/html"), None);
    }

    #[test]
    fn a_paste_gets_the_picture_itself_once_it_has_come() {
        let copied = CopiedPicture::new(URL.into());
        copied.note(|arrived| arrived.shown = Some(b"what was shown".to_vec()));
        // It comes while the paste waits for it.
        let arriving = copied.clone();
        let came = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            arriving.note(|arrived| arrived.whole = Some(Ok(b"the picture itself".to_vec())));
        });
        assert_eq!(copied.png(Duration::from_secs(10)).as_deref(), Some(&b"the picture itself"[..]));
        came.join().unwrap();
        // And is there at once from then on.
        assert_eq!(copied.png(Duration::ZERO).as_deref(), Some(&b"the picture itself"[..]));
    }

    #[test]
    fn a_paste_gets_what_was_shown_of_a_picture_that_does_not_come() {
        let copied = CopiedPicture::new(URL.into());
        assert_eq!(copied.png(Duration::ZERO), None, "nothing of it is here");
        copied.note(|arrived| arrived.shown = Some(b"what was shown".to_vec()));
        // Not here when the wait is up.
        assert_eq!(copied.png(Duration::from_millis(20)).as_deref(), Some(&b"what was shown"[..]));
        // It could not be had: that is known at once.
        copied.note(|arrived| arrived.whole = Some(Err(())));
        assert_eq!(copied.png(Duration::from_secs(10)).as_deref(), Some(&b"what was shown"[..]));
    }
}

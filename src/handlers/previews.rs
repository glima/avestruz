// SPDX-License-Identifier: MIT

//! Link preview message handlers for Avestruz.
//!
//! A row whose note links to a web page asks the
//! [`PreviewStore`](crate::previews::PreviewStore) what the page says of
//! itself, which sends the address here as a `LoadPreview`. This is where it
//! is decided whether to fetch the page, and where what it said comes back
//! to.

use cosmic::prelude::*;

use crate::messages::Message;
use crate::previews::{self, Linked};
use crate::state::AppModel;

impl AppModel {
    /// A view asked what a page says of itself. Fetch its head, unless that
    /// is known or on its way already, or too many others are.
    pub fn handle_load_preview(&mut self, url: String) -> Task<cosmic::Action<Message>> {
        if !self.previews.begin(&url) {
            return Task::none();
        }
        let http = self.http.clone();
        Task::perform(
            async move {
                let result = previews::fetch(http, url.clone()).await;
                (url, result)
            },
            |(url, result)| cosmic::Action::App(Message::PreviewLoaded(url, result)),
        )
    }

    /// A page answered, or didn't. Either way the rows that link to it are
    /// built again: with its preview, with the offer to play it if it
    /// turned out to be a video, or without the room kept for either.
    pub fn handle_preview_loaded(&mut self, url: String, result: Result<Linked, String>) {
        if let Err(e) = &result {
            // Pages that are gone, or say nothing, are ordinary: not worth
            // a banner.
            tracing::debug!("link not previewed ({e}): {url}");
        }
        self.previews.finish(url, result.ok());
    }
}

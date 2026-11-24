// SPDX-License-Identifier: MIT

//! Video message handlers for Avestruz.
//!
//! Most of what a video is told is told to the model at once (see
//! [`crate::video`]). The one thing that takes time is a YouTube video:
//! what there is to play of it has to be found first, by yt-dlp (see
//! [`crate::youtube`]).

use cosmic::prelude::*;

use crate::fl;
use crate::messages::Message;
use crate::state::AppModel;
use crate::youtube::{self, FindError};

impl AppModel {
    /// The video at `url` was pressed: play it, or hold it if it plays. A
    /// YouTube video is on its way from here, while yt-dlp is asked what
    /// there is to play of it.
    pub fn handle_video_press(&mut self, url: String) -> Task<cosmic::Action<Message>> {
        match self.videos.press(&url) {
            Some(video) => Task::perform(youtube::find_stream(video), move |result| {
                cosmic::Action::App(Message::VideoFound(url.clone(), result))
            }),
            None => Task::none(),
        }
    }

    /// yt-dlp answered for the YouTube video at `url`: with the address of
    /// a stream, which is played, or with why there is none, which is said
    /// where the video would have played.
    pub fn handle_video_found(&mut self, url: &str, result: Result<String, FindError>) {
        let stream = result.map_err(|e| {
            tracing::warn!("no stream of a YouTube video was found: {e:?}");
            match e {
                FindError::NotInstalled => fl!("video-ytdlp-missing"),
                FindError::TimedOut => fl!("video-ytdlp-slow"),
                FindError::NoRuntime => fl!("video-ytdlp-runtime"),
                FindError::Failed(why) => fl!("video-ytdlp-failed", error = why),
            }
        });
        self.videos.found(url, stream);
    }
}

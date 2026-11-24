// SPDX-License-Identifier: MIT

//! Settings message handlers for Avestruz.
//!
//! A setting changed here is written to COSMIC's config store and applied
//! at once. One changed elsewhere (another instance, a hand in the config
//! files) arrives through the config watch, and is applied the same.

use crate::config::{Config, LogLevel, NOTE_TEXT_SIZE_MAX, NOTE_TEXT_SIZE_MIN};
use cosmic::prelude::*;

use crate::logging;
use crate::messages::Message;
use crate::state::{AppModel, Page};
use crate::views::components::RowHeights;

impl AppModel {
    /// How much to log was chosen in the settings: keep it, and log that
    /// much from now on, without a restart.
    pub fn handle_set_log_level(&mut self, level: LogLevel) {
        tracing::info!("Setting console log level to {}", level.as_filter_str());
        self.config.log_level = level;
        self.save_config("log level");
        logging::set_console_level(level);
    }

    /// How large notes are set was chosen in the settings: keep it. The
    /// rows are other heights from now on.
    pub fn handle_set_note_text_size(&mut self, size: u16) {
        let size = size.clamp(NOTE_TEXT_SIZE_MIN, NOTE_TEXT_SIZE_MAX);
        if size == self.config.note_text_size {
            return;
        }
        self.config.note_text_size = size;
        self.save_config("note text size");
        self.forget_row_heights();
    }

    /// How long the event store keeps other people's notes was chosen in
    /// the settings: keep it, and have the store let go now of what is
    /// older than that.
    pub fn handle_set_keep_days(&mut self, days: u32) -> Task<cosmic::Action<Message>> {
        if days == self.config.keep_days {
            return Task::none();
        }
        self.config.keep_days = days;
        self.save_config("days kept");
        self.swept_at = None;
        self.sweep_store()
    }

    /// Forget how tall the rows of every list measured: they are about to
    /// be set in another size, and measure again as they come into view.
    fn forget_row_heights(&mut self) {
        self.following.heights = RowHeights::default();
        for page in &mut self.pages {
            match page {
                Page::Thread(state) => state.heights = RowHeights::default(),
                Page::Profile(state) => state.notes.heights = RowHeights::default(),
                Page::Bookmarks(page) => page.notes.heights = RowHeights::default(),
                Page::Search(page) => page.heights = RowHeights::default(),
                Page::Notifications(page) => page.heights = RowHeights::default(),
            }
        }
    }

    /// The config changed on disk. A new log level is logged at from now
    /// on, unless `RUST_LOG` says how much to log; notes are set in a new
    /// size; the lists show the notes they are now to show; and the event
    /// store lets go of what is older than it is now to keep.
    pub fn handle_update_config(&mut self, config: Config) -> Task<cosmic::Action<Message>> {
        if config.log_level != self.config.log_level && !logging::level_set_by_environment() {
            logging::set_console_level(config.log_level);
        }
        let resized = config.note_text_size() != self.config.note_text_size();
        let read_another_way = config.feed_mode != self.config.feed_mode;
        let kept_another_while = config.keep_days != self.config.keep_days;
        self.config = config;
        self.suggest_reactions();
        if resized {
            self.forget_row_heights();
        }
        let shown = if read_another_way { self.show_feed_mode() } else { Task::none() };
        let swept = if kept_another_while {
            self.swept_at = None;
            self.sweep_store()
        } else {
            Task::none()
        };
        Task::batch([shown, swept])
    }
}

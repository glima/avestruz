// SPDX-License-Identifier: MIT

//! Store message handlers for Avestruz: having the event store let go of
//! what it has kept for longer than the settings say (see
//! [`crate::nostr::sweep`]).
//!
//! Once a day, in the background, and when the settings say another number
//! of days. A day begins when the account's bookmarks are known, which is
//! when the popup first opens: what is bookmarked stays, and what is
//! bookmarked takes the account's key to read. Bookmarks that can't be
//! read keep everything where it is until they can. An account without
//! its key keeps no bookmarks here, and its day begins within the hour.
//!
//! Nothing on show changes: the lists are in the model, not in the store.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use cosmic::prelude::*;
use nostr_sdk::prelude::{EventId, Timestamp};

use crate::messages::Message;
use crate::state::{AppModel, BookmarkState};

/// How long after the store let go of what is old it does so again.
const SWEEP_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a day is, for counting the days notes are kept.
const DAY_SECS: u64 = 24 * 60 * 60;

impl AppModel {
    /// Have the event store let go of what is older than the settings say
    /// it keeps, unless it did within the day, or keeps everything, or it
    /// isn't known yet what stays: whose store it is, and what they
    /// bookmarked. Called every so often, and when either becomes known.
    pub(crate) fn sweep_store(&mut self) -> Task<cosmic::Action<Message>> {
        let days = self.config.keep_days;
        if days == 0 || self.swept_at.is_some_and(|at| at.elapsed() < SWEEP_EVERY) {
            return Task::none();
        }
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref()) else {
            return Task::none();
        };
        let bookmarked: HashSet<EventId> = match &self.bookmarks {
            BookmarkState::Known(bookmarks) => bookmarks.notes().into_iter().collect(),
            BookmarkState::Unknown if !account.can_sign => HashSet::new(),
            BookmarkState::Unknown | BookmarkState::Unreadable(_) => return Task::none(),
        };
        let account = account.public_key;
        let before = Timestamp::from_secs(Timestamp::now().as_secs().saturating_sub(u64::from(days) * DAY_SECS));
        self.swept_at = Some(Instant::now());
        Task::future(async move {
            match service.sweep(account, &bookmarked, before).await {
                Ok(swept) => tracing::info!("the event store let go of {swept} event(s) older than {days} day(s)"),
                Err(e) => tracing::warn!("the event store could not let go of what is old: {e}"),
            }
        })
        .discard()
    }
}

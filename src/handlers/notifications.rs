// SPDX-License-Identifier: MIT

//! Notification message handlers for Avestruz: finding out what others
//! did that concerns the account, showing it, and keeping track of how
//! much of it was looked at.
//!
//! What names the account is read from the store when the account is
//! known, and asked of the relays after: its own, and the ones it reads
//! (see [`NostrService::notifications`](crate::nostr::client::NostrService::notifications)).
//! From then on the relays are
//! asked again each time the feed is looked at for new notes, for what
//! was written since. Scrolling the list towards its end asks for the
//! page before the oldest.
//!
//! What is new is what was written after the notifications were last
//! looked at: one moment, kept in the settings for each account
//! ([`Config::notifications_seen`](crate::config::Config::notifications_seen)).
//! Looking is having their page on show in an open popup. The moment is
//! moved to now when that begins and again when it ends, so that what
//! arrived meanwhile is not new the next time; what was new when it
//! began stays picked out for as long as the page is looked at
//! ([`NotificationsState::seen_before`](crate::state::NotificationsState::seen_before)).
//!
//! Zaps show once it is known which key the account's wallet service
//! signs receipts with, which its Lightning address is asked once (see
//! [`crate::nostr::notifications`]).

use std::time::Duration;

use cosmic::prelude::*;
use cosmic::widget::segmented_button;
use nostr_sdk::prelude::{PublicKey, ToBech32};

use crate::handlers::navigation::scroll_list_to;
use crate::helpers::unix_now;
use crate::messages::Message;
use crate::nostr::client::{FetchError, Notified, Source};
use crate::nostr::notifications::Tab;
use crate::nostr::zap;
use crate::state::{AppModel, NotificationsPage, NotificationsState, Page, ViewState};

/// How far back from the newest notification the relays are asked again
/// when they are asked for what is new: events reach a relay some time
/// after they say they were written.
const CHECK_OVERLAP: Duration = Duration::from_secs(10 * 60);

impl AppModel {
    /// The public key of the account, if somebody is signed in.
    fn account_key(&self) -> Option<PublicKey> {
        self.account.as_ref().map(|account| account.public_key)
    }

    /// What the settings keep the account's last look at its
    /// notifications under: its `npub`.
    fn seen_key(&self) -> Option<String> {
        self.account_key().and_then(|account| account.to_bech32().ok())
    }

    /// When the account last looked at its notifications.
    pub(crate) fn notifications_seen_at(&self) -> u64 {
        self.seen_key().and_then(|key| self.config.notifications_seen.get(&key).copied()).unwrap_or(0)
    }

    /// The account has looked at its notifications, as of now.
    fn mark_notifications_seen(&mut self) {
        if let Some(key) = self.seen_key() {
            self.config.notifications_seen.insert(key, unix_now());
            self.save_config("notifications seen");
        }
    }

    /// Which notifications the page shows: as the control above it says.
    pub(crate) fn notifications_tab(&self) -> Tab {
        self.notification_tabs.active_data::<Tab>().copied().unwrap_or_default()
    }

    /// Whether the notifications are being looked at: their page is on
    /// show, in an open popup.
    fn notifications_in_view(&self) -> bool {
        self.popup.is_some() && self.view_state == ViewState::Page && matches!(self.pages.last(), Some(Page::Notifications(_)))
    }

    /// See whether the notifications came into view or went out of it,
    /// whatever did that: what was seen before is kept for the visit that
    /// begins, and either way everything there is has now been seen. To
    /// be called after every message.
    pub(crate) fn watch_notifications(&mut self) {
        let in_view = self.notifications_in_view();
        if in_view == self.notifications.in_view {
            return;
        }
        self.notifications.in_view = in_view;
        if in_view {
            self.notifications.seen_before = self.notifications_seen_at();
        }
        self.mark_notifications_seen();
    }

    /// How many notifications are new: none while they are looked at.
    pub(crate) fn unread_notifications(&self) -> usize {
        if self.notifications.in_view { 0 } else { self.notifications.list.unread(self.notifications_seen_at()) }
    }

    /// Forget what the account was told of: the account changed.
    pub(crate) fn reset_notifications(&mut self) {
        self.notifications = NotificationsState::default();
    }

    /// Find out what the account is told of: from the store now, and from
    /// the relays after. Does nothing until both the account and the
    /// store are there.
    pub(crate) fn load_notifications(&mut self) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account_key()) else {
            return Task::none();
        };
        // An account that never looked has seen everything there is: what
        // is new is what comes from now on, not all it was ever told.
        if let Some(key) = self.seen_key()
            && !self.config.notifications_seen.contains_key(&key)
        {
            self.mark_notifications_seen();
        }
        self.notifications.loading = true;
        Task::perform(async move { service.notifications(Source::Store, account, None, None).await }, move |result| {
            cosmic::Action::App(Message::NotificationsLoaded { account, source: Source::Store, older: false, result })
        })
    }

    /// Ask the relays what names the account: what was written after
    /// `since` and at or before `until`, where those are given.
    fn ask_for_notifications(&mut self, since: Option<u64>, until: Option<u64>) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account_key()) else {
            return Task::none();
        };
        let older = until.is_some();
        if older {
            self.notifications.loading_older = true;
        } else {
            self.notifications.loading = true;
        }
        Task::perform(async move { service.notifications(Source::Relays, account, since, until).await }, move |result| {
            cosmic::Action::App(Message::NotificationsLoaded { account, source: Source::Relays, older, result })
        })
    }

    /// It is time to ask the relays what is new for the account: what
    /// was written since the newest notification known, and a while
    /// before it; or the newest there is, if they have not answered yet.
    /// Not while they are being asked.
    pub(crate) fn check_notifications(&mut self) -> Task<cosmic::Action<Message>> {
        if self.notifications.loading {
            return Task::none();
        }
        if !self.notifications.settled {
            return self.ask_for_notifications(None, None);
        }
        let since = self.notifications.list.newest().map(|newest| newest.saturating_sub(CHECK_OVERLAP.as_secs()));
        self.ask_for_notifications(since, None)
    }

    /// A page of what the account is told of came from the store or from
    /// the relays (or didn't).
    pub fn handle_notifications_loaded(
        &mut self,
        account: PublicKey,
        source: Source,
        older: bool,
        result: Result<Notified, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        if self.account_key() != Some(account) {
            return Task::none();
        }
        let first = !self.notifications.settled;
        let state = &mut self.notifications;
        if older {
            state.loading_older = false;
        } else if source == Source::Relays {
            state.loading = false;
        }
        let page = match result {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!("what names the account could not be asked of {source:?}: {e:?}");
                // A page that didn't come is not the end of them: it is
                // asked for again when the list is next scrolled there.
                if source == Source::Relays && !older {
                    state.failed = first;
                }
                return if source == Source::Store { self.ask_for_notifications(None, None) } else { Task::none() };
            }
        };
        tracing::info!(
            "notifications: {} from {source:?}{}",
            page.notifications.len(),
            if older { ", further back" } else { "" }
        );
        match (source, older) {
            (Source::Store, _) => {}
            // The newest there are. The first answer says where the page
            // before it ends, and whether there is one. So does one that
            // is as long as was asked for and ends after everything known
            // (what the store had from last week, say): there may be more
            // in between, so what is older is let go of, and comes back
            // as the list is scrolled to its end. A list with a hole in
            // it would be filled in above whoever had read past the hole.
            (Source::Relays, false) => {
                (state.settled, state.failed) = (true, false);
                let apart = page.full && page.oldest.zip(state.list.newest()).is_some_and(|(oldest, known)| oldest > known);
                if first || apart {
                    (state.oldest, state.end_reached) = (page.oldest, !page.full);
                }
                if let Some(oldest) = page.oldest.filter(|_| apart) {
                    state.list.forget_before(oldest);
                }
            }
            (Source::Relays, true) => {
                state.end_reached = !page.full || page.oldest.is_none();
                state.oldest = page.oldest.or(state.oldest);
            }
        }
        let people: Vec<PublicKey> = page
            .notifications
            .iter()
            .flat_map(|told| std::iter::once(told.who).chain(told.note.iter().flat_map(|note| note.people())))
            .collect();
        let notes: Vec<_> = page.notifications.iter().filter_map(|told| told.note.as_ref().map(|note| note.id)).collect();
        let news = state.list.merge(page.notifications);
        if !news.is_empty() && source == Source::Relays {
            tracing::info!("notifications: {} not known before", news.len());
        }

        let asked = if source == Source::Store { self.ask_for_notifications(None, None) } else { Task::none() };
        Task::batch([asked, self.look_up_people(people), self.look_up_engagement(notes), self.look_up_zapper()])
    }

    /// Ask the account's Lightning address which key its wallet service
    /// signs zap receipts with, once, when the account is known to have
    /// such an address.
    pub(crate) fn look_up_zapper(&mut self) -> Task<cosmic::Action<Message>> {
        if self.notifications.zapper_asked {
            return Task::none();
        }
        let Some(account) = self.account_key() else {
            return Task::none();
        };
        let Some(address) = self.profiles.get(&account).and_then(|profile| profile.lightning.clone()) else {
            return Task::none();
        };
        self.notifications.zapper_asked = true;
        let http = self.http.clone();
        Task::perform(async move { zap::zapper_key(http, &address).await }, move |result| {
            cosmic::Action::App(Message::ZapperKnown(account, result.map_err(|e| format!("{e:?}"))))
        })
    }

    /// The account's Lightning address said which key signs its zap
    /// receipts (or could not be asked: it is asked again the next time
    /// there is reason to).
    pub fn handle_zapper_known(&mut self, account: PublicKey, result: Result<Option<PublicKey>, String>) {
        if self.account_key() != Some(account) {
            return;
        }
        match result {
            Ok(zapper) => self.notifications.list.set_zapper(zapper),
            Err(e) => {
                tracing::warn!("the account's Lightning address could not be asked who signs its zap receipts: {e}");
                self.notifications.zapper_asked = false;
            }
        }
    }

    /// Show what the account is told of, over whatever is on show now,
    /// and ask the relays whether there is more.
    pub fn handle_open_notifications(&mut self) -> Task<cosmic::Action<Message>> {
        self.menu_open = false;
        if self.account.is_none() {
            return Task::none();
        }
        let opened = self.open_page(Page::Notifications(Box::default()));
        let _: &NotificationsPage = match self.pages.last() {
            Some(Page::Notifications(page)) => page,
            _ => return opened,
        };
        Task::batch([opened, self.check_notifications(), self.look_up_zapper()])
    }

    /// The list of notifications was scrolled near its end: ask for the
    /// page before the oldest, unless that is under way, or there is none.
    pub fn handle_load_older_notifications(&mut self) -> Task<cosmic::Action<Message>> {
        let state = &self.notifications;
        if !state.settled || state.loading_older || state.end_reached {
            return Task::none();
        }
        match state.oldest {
            Some(oldest) => self.ask_for_notifications(None, Some(oldest.saturating_sub(1))),
            None => Task::none(),
        }
    }

    /// Another kind of notification was chosen to show: from its top.
    pub fn handle_select_notifications_tab(&mut self, chosen: segmented_button::Entity) -> Task<cosmic::Action<Message>> {
        if self.notification_tabs.data::<Tab>(chosen).is_none() || self.notification_tabs.active() == chosen {
            return Task::none();
        }
        self.notification_tabs.activate(chosen);
        scroll_list_to(0.0)
    }
}

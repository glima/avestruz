// SPDX-License-Identifier: MIT

//! The account's relays: where it writes, where it reads and where it
//! searches, shown as its lists have them and changed there.
//!
//! The lists are the account's relay list (NIP-65) and its other lists of
//! relays (see [`crate::nostr::relay_lists`]). They show as the event
//! store has them at once, and as the relays have them when they have
//! answered; until then nothing can be changed, since what is saved
//! replaces what a list names.
//!
//! What a list keeps to itself takes the signer to read, which may ask a
//! person about each request, so it is read when it is needed: the relays
//! never connected to and the relays things are looked up on when the
//! popup opens, since they are gone by all the time; the search relays
//! when a search page opens; and the rest when the relays page does.
//!
//! A search relay has to search, a relay direct messages are sent to has
//! to know who is asking for them, a relay other people write to, to
//! reach the account, has to take what they write, and a relay they read
//! the account's notes on has to let them (see
//! [`Section::hindrances`]). While the page shows, the relays of those
//! sets are asked what they say of themselves (NIP-11), each once, and
//! what one says that gets in the way of what its set is for is said
//! under its name. One that doesn't answer is not said anything of.
//!
//! Relays are added and taken out on the page, and nothing is written
//! until Save: the popup can close meanwhile, and what was changed is
//! still there when it opens. Save has each list that names other relays
//! than the page asked for again, changed and sent out, with everything
//! else in it as it was. A first list is only written after saying so,
//! as a first contact list is, and a list whose private part can't be read
//! is never written over.

use std::collections::HashSet;

use cosmic::prelude::*;
use nostr_sdk::prelude::{Event, PublicKey, RelayUrl};

use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::handlers::compose::publish_error_text;
use crate::handlers::feed::fetch_error_text;
use crate::messages::Message;
use crate::nostr::client::{FetchError, ListError, Source};
use crate::nostr::relay_info;
use crate::nostr::relay_lists::{Kept, ListedRelays, RelayLists, Section, typed_relay};
use crate::state::{AppModel, RelaysState, ViewState};
use crate::views::relays::section_title;

/// The lists whose private part is read when the popup opens: they are
/// gone by whatever is on show.
pub(crate) const ALWAYS_READ: [Section; 2] = [Section::Blocked, Section::Indexer];

impl AppModel {
    /// Forget the account's relays: the account changed, and they were
    /// its own, with whatever was changed and not saved.
    pub(crate) fn reset_relays(&mut self) {
        self.relays = RelaysState::default();
    }

    /// Show the account's relays, in place of the list on show, and ask
    /// for its lists again, every one read in full.
    pub fn handle_open_relays(&mut self) -> Task<cosmic::Action<Message>> {
        self.menu_open = false;
        self.view_state = ViewState::Relays;
        self.relays.open.extend(Section::LISTS);
        // One that didn't say last time is asked again.
        self.relays.said.retain(|_, info| info.is_some());
        Task::batch([self.ask_relay_lists(Source::Store), self.ask_relay_use(), self.ask_what_relays_support()])
    }

    /// Ask the relays of the sets that ask something of a relay what
    /// they say of themselves, while the relays page shows: those on the
    /// page that were not asked yet.
    fn ask_what_relays_support(&mut self) -> Task<cosmic::Action<Message>> {
        if self.view_state != ViewState::Relays {
            return Task::none();
        }
        let unasked: HashSet<RelayUrl> = Section::ALL
            .into_iter()
            .filter(|section| section.asks_of_relays())
            .flat_map(|section| self.relays.edited.of(section))
            .filter(|relay| !self.relays.said.contains_key(*relay))
            .cloned()
            .collect();
        let asked = unasked.into_iter().map(|relay| {
            self.relays.said.insert(relay.clone(), None);
            let http = self.http.clone();
            Task::perform(
                async move {
                    let info = relay_info::fetch(&http, &relay).await;
                    (relay, info)
                },
                |(relay, info)| cosmic::Action::App(Message::RelaySaid { relay, info }),
            )
        });
        Task::batch(asked.collect::<Vec<_>>())
    }

    /// Ask how the relays this is connected to are doing, for the relays
    /// page to say.
    fn ask_relay_use(&self) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        Task::perform(async move { service.relay_use().await }, |in_use| cosmic::Action::App(Message::RelayUseLoaded(in_use)))
    }

    /// Find out what the account's relay lists say, with the private part
    /// of the lists of `open` read, while the popup is open, and unless
    /// the relays said already and those were read then.
    pub(crate) fn load_relay_lists(&mut self, open: &[Section]) -> Task<cosmic::Action<Message>> {
        // Reading takes the key, as reading the bookmarks does: not
        // before there is someone to see what for.
        if self.popup.is_none() {
            return Task::none();
        }
        let unread =
            |section: &Section| self.relays.lists.as_ref().is_none_or(|lists| *lists.of(*section).kept() == Kept::Unread);
        if self.relays.from_relays && !open.iter().any(unread) {
            return Task::none();
        }
        self.relays.open.extend(open);
        self.ask_relay_lists(Source::Store)
    }

    /// Ask `source` for the account's relay lists, unless they are being
    /// changed: what the change leaves is what they are.
    fn ask_relay_lists(&mut self, source: Source) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        if self.relays.saving {
            return Task::none();
        }
        if source == Source::Relays {
            self.relays.loading = true;
        }
        let signer = self.signer.clone();
        let known = self.relays.lists.clone();
        let open: Vec<Section> = self.relays.open.iter().copied().collect();
        Task::perform(async move { service.relay_lists(source, &signer, account, known, &open).await }, move |result| {
            cosmic::Action::App(Message::RelayListsLoaded { account, source, result })
        })
    }

    /// The account's relay lists came from the store or from the relays,
    /// or didn't. After the store, the relays are asked.
    pub fn handle_relay_lists_loaded(
        &mut self,
        account: PublicKey,
        source: Source,
        result: Result<RelayLists, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        // Whoever signed out meanwhile took their relays with them.
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return Task::none();
        }
        let from_relays = source == Source::Relays;
        if from_relays {
            self.relays.loading = false;
        }
        match result {
            // A change on its way says what the lists are when it is done;
            // and the store's word is not taken over the relays'.
            Ok(_) if self.relays.saving || (!from_relays && self.relays.from_relays) => {}
            Ok(lists) => {
                tracing::debug!("the account's relay lists were read, by {source:?}");
                // Each set of relays that was not changed here shows
                // what its list says now; one that was stays as it is.
                let (before, now) = (self.relays.lists.as_ref().map(RelayLists::sets).unwrap_or_default(), lists.sets());
                for section in Section::ALL {
                    if self.relays.edited.of(section) == before.of(section) {
                        self.relays.edited.set(section, now.of(section).to_vec());
                    }
                }
                self.relays.lists = Some(lists);
                self.relays.from_relays = from_relays;
                self.go_by_relay_lists();
            }
            Err(e) => {
                tracing::warn!("the account's relay lists could not be asked of {source:?}: {e:?}");
                if from_relays && self.view_state == ViewState::Relays {
                    self.error_message = Some(fl!("relays-not-read", error = fetch_error_text(e)));
                }
            }
        }
        let next = match source {
            Source::Store => self.ask_relay_lists(Source::Relays),
            // The relays were just asked: how they did is news.
            Source::Relays if self.view_state == ViewState::Relays => self.ask_relay_use(),
            Source::Relays => Task::none(),
        };
        Task::batch([next, self.ask_what_relays_support()])
    }

    /// Have the relay client go by what the lists say now: where things
    /// are looked up, and which relays are never connected to.
    fn go_by_relay_lists(&self) {
        let (Some(service), Some(lists)) = (&self.nostr, &self.relays.lists) else {
            return;
        };
        service.use_listed(lists.of(Section::Indexer).relays(), lists.of(Section::Blocked).relays());
    }

    /// Whether the relays of `section` can be changed now: by an account
    /// with its key, once the relays have said what its lists are, while
    /// no change is on its way. Not those of a list whose private part
    /// was not read: a list written without it would lose it.
    pub(crate) fn relays_can_change(&self, section: Section) -> bool {
        let read = match section {
            Section::Outbox | Section::Inbox => true,
            _ => self.relays.lists.as_ref().is_some_and(|lists| matches!(lists.of(section).kept(), Kept::Read(_))),
        };
        self.can_publish() && self.relays.from_relays && !self.relays.saving && read
    }

    /// Add the relay typed for `section` to it, on the page: nothing is
    /// written until the relays are saved.
    pub fn handle_add_relay(&mut self, section: Section) -> Task<cosmic::Action<Message>> {
        if !self.relays_can_change(section) {
            return Task::none();
        }
        let typed = self.relays.typed(section).trim().to_string();
        if typed.is_empty() {
            return Task::none();
        }
        match typed_relay(&typed) {
            Some(relay) => {
                self.relays.edited.add(section, relay);
                self.relays.typed.remove(&section);
            }
            None => self.error_message = Some(fl!("relays-not-a-relay", typed = typed)),
        }
        self.ask_what_relays_support()
    }

    /// Take `relay` out of `section`, on the page.
    pub fn handle_remove_relay(&mut self, section: Section, relay: &RelayUrl) {
        if self.relays_can_change(section) {
            self.relays.edited.remove(section, relay);
        }
    }

    /// Move `relay` a place up `section`, or down, on the page.
    pub fn handle_shift_relay(&mut self, section: Section, relay: &RelayUrl, up: bool) {
        if self.relays_can_change(section) {
            self.relays.edited.shift(section, relay, up);
        }
    }

    /// Whether there is anything to save, and it can be saved now.
    pub(crate) fn relays_can_save(&self) -> bool {
        self.can_publish() && self.relays.from_relays && !self.relays.saving && self.relays.changed()
    }

    /// Write the relays as the page has them to the account's lists: each
    /// list that names others, one after the other.
    pub fn handle_save_relays(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.relays_can_save() {
            return Task::none();
        }
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        let Some(lists) = &self.relays.lists else {
            return Task::none();
        };
        let (listed, edited) = (lists.sets(), self.relays.edited.clone());
        let differs = |section: Section| listed.of(section) != edited.of(section);
        let advertised_changed = differs(Section::Outbox) || differs(Section::Inbox);
        let to_change: Vec<(Section, ListedRelays)> = Section::LISTS
            .into_iter()
            .filter(|section| self.relays_can_change(*section) && differs(*section))
            .map(|section| (section, lists.of(section)))
            .collect();
        let (signer, first_lists) = (self.signer.clone(), self.relays.first_lists);
        self.relays.saving = true;
        self.error_message = None;

        let saving = async move {
            let advertised = if advertised_changed {
                let (outbox, inbox) = (edited.of(Section::Outbox), edited.of(Section::Inbox));
                Some(service.set_advertised_relays(&signer, account, outbox, inbox, first_lists).await)
            } else {
                None
            };
            let mut changed = Vec::new();
            for (section, known) in to_change {
                let relays = edited.of(section);
                let result = service.set_listed_relays(&signer, account, section, relays, first_lists, Some(known)).await;
                changed.push((section, result));
            }
            (advertised, changed)
        };
        Task::perform(saving, move |(advertised, listed)| {
            cosmic::Action::App(Message::RelaysSaved { account, advertised, listed })
        })
    }

    /// The account's lists were changed, or weren't: each is as it came
    /// back, and the page shows what they say now. What was not saved
    /// stays on the page, changed, to be saved again.
    pub fn handle_relays_saved(
        &mut self,
        account: PublicKey,
        advertised: Option<Result<Option<Event>, ListError>>,
        listed: Vec<(Section, Result<ListedRelays, ListError>)>,
    ) {
        self.relays.saving = false;
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return;
        }
        let Some(lists) = self.relays.lists.as_mut() else {
            return;
        };
        // What went wrong first, and with which set of relays.
        let mut failed: Option<(Section, ListError)> = None;
        match advertised {
            Some(Ok(list)) => {
                lists.advertised = list;
                let said = lists.sets();
                for section in [Section::Outbox, Section::Inbox] {
                    self.relays.edited.set(section, said.of(section).to_vec());
                }
            }
            Some(Err(e)) => failed = Some((Section::Outbox, e)),
            None => {}
        }
        for (section, result) in listed {
            match result {
                Ok(changed) => {
                    self.relays.edited.set(section, changed.relays());
                    lists.listed.insert(section, changed);
                }
                Err(e) => failed = failed.or(Some((section, e))),
            }
        }
        self.go_by_relay_lists();
        self.error_message = match failed {
            None => {
                self.relays.first_lists = false;
                None
            }
            Some((section, ListError::NoList)) => {
                self.relays.first_lists = true;
                Some(fl!("relays-no-list", list = section_title(section)))
            }
            Some((_, ListError::Fetch(e))) => {
                tracing::error!("a relay list could not be asked for: {e:?}");
                Some(fl!("relays-error", error = fetch_error_text(e)))
            }
            Some((_, ListError::Unreadable(e))) => {
                tracing::error!("a relay list could not be read: {e:?}");
                Some(fl!("relays-unreadable", error = sign_error_text(e)))
            }
            Some((_, ListError::Publish(e))) => {
                tracing::error!("a relay list did not go out: {e:?}");
                Some(fl!("relays-error", error = publish_error_text(e)))
            }
        };
    }
}

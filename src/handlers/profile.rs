// SPDX-License-Identifier: MIT

//! Profile message handlers for Avestruz.
//!
//! A profile is a page: who someone is and what they say of themselves,
//! and under that their notes, newest first. It opens with whatever is
//! known of them already. The event store is asked for the rest and answers
//! within the frame; the relays are asked at the same time, for their notes
//! and for whether they have said something newer of themselves.
//!
//! The notes are a list like the feed's, kept and filled as the feed is
//! (see [`crate::handlers::feed`]).
//!
//! The name at a domain someone gives as theirs (NIP-05) is checked with
//! that domain when their profile is open, and only then: a feed of notes
//! would have a hundred domains asked, and told who reads what.
//!
//! Following and unfollowing change the account's contact list, which is
//! one list everywhere and replaced whole. It is asked of the relays again
//! just before it is changed, one change at a time, and the feed starts
//! over with whoever it then follows.

use cosmic::iced::clipboard;
use cosmic::prelude::*;
use nostr_sdk::prelude::{PublicKey, ToBech32};

use crate::fl;
use crate::handlers::compose::publish_error_text;
use crate::handlers::feed::fetch_profiles;
use crate::messages::Message;
use crate::nostr::client::{FetchError, FollowError};
use crate::nostr::nip05;
use crate::state::{AppModel, FollowProblem, Nip05Check, Nip05State, Page, ProfileState, ViewState};

impl AppModel {
    /// Whether the profile of `public_key` is the page on show.
    fn profile_on_show(&self, public_key: PublicKey) -> bool {
        self.view_state == ViewState::Page && self.profile_on_top().is_some_and(|state| state.public_key == public_key)
    }

    /// Whether a profile of `public_key` is open, on show or under other
    /// pages.
    fn profile_is_open(&self, public_key: PublicKey) -> bool {
        self.pages.iter().any(|page| matches!(page, Page::Profile(state) if state.public_key == public_key))
    }

    /// Show the profile of `public_key`, over whatever is on show now.
    pub fn handle_open_profile(&mut self, public_key: PublicKey) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        if self.profile_on_show(public_key) {
            return Task::none();
        }
        let notes = self.new_feed();
        let list = notes.id;
        let opened = self.open_page(Page::Profile(Box::new(ProfileState::new(public_key, notes))));

        // Asked about here and now, whatever was asked before: the store
        // for what it has, and the relays for anything newer.
        self.profiles_requested.insert(public_key);
        let stored = {
            let service = service.clone();
            Task::perform(async move { service.cached_profiles(vec![public_key]).await }, |found| {
                cosmic::Action::App(Message::ProfilesFetched(found))
            })
        };
        let fetched = fetch_profiles(service, vec![public_key]);
        // A domain that didn't answer the last time may answer now.
        if self.nip05.get(&public_key).is_some_and(|check| check.state == Nip05State::Unverified) {
            self.nip05.remove(&public_key);
        }
        Task::batch([opened, stored, fetched, self.load_feed(list), self.check_nip05(public_key)])
    }

    /// Copy the `npub` of `public_key`, whose profile is on show.
    pub fn handle_copy_npub(&mut self, public_key: PublicKey) -> Task<cosmic::Action<Message>> {
        for state in self.profiles_mut().filter(|state| state.public_key == public_key) {
            state.npub_copied = true;
        }
        let Ok(npub) = public_key.to_bech32();
        clipboard::write(npub)
    }

    // =========================================================================
    // Names at a domain (NIP-05)
    // =========================================================================

    /// Ask the domain in the name `person` gives as theirs whether it is,
    /// if their profile is open and that name was not asked about yet.
    pub(crate) fn check_nip05(&mut self, person: PublicKey) -> Task<cosmic::Action<Message>> {
        if !self.profile_is_open(person) {
            return Task::none();
        }
        let Some(name) = self.profiles.get(&person).and_then(|profile| profile.nip05.clone()) else {
            return Task::none();
        };
        if self.nip05.get(&person).is_some_and(|check| check.name == name) {
            return Task::none();
        }
        self.nip05.insert(person, Nip05Check { name: name.clone(), state: Nip05State::Checking });
        let http = self.http.clone();
        Task::perform(
            async move {
                let verified = nip05::verify(&http, person, &name).await;
                (name, verified)
            },
            move |(name, verified)| cosmic::Action::App(Message::Nip05Checked { person, name, verified }),
        )
    }

    /// The domain in `name` said whether it is the name of `person`. If
    /// they gave another name since, this answer is not about it.
    pub fn handle_nip05_checked(&mut self, person: PublicKey, name: &str, verified: bool) {
        if let Some(check) = self.nip05.get_mut(&person).filter(|check| check.name == name) {
            check.state = if verified { Nip05State::Verified } else { Nip05State::Unverified };
        }
    }

    // =========================================================================
    // Following
    // =========================================================================

    /// Follow `person` (`follow`), or stop following them. `first_list`
    /// when the account agreed to a first contact list being written.
    pub fn handle_follow(&mut self, person: PublicKey, follow: bool, first_list: bool) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        if !self.can_publish() || self.follow_pending.is_some() || person == account {
            return Task::none();
        }
        self.follow_pending = Some(person);
        for state in self.profiles_mut().filter(|state| state.public_key == person) {
            state.follow_problem = None;
        }
        let signer = self.signer.clone();
        Task::perform(async move { service.set_following(&signer, account, person, follow, first_list).await }, move |result| {
            cosmic::Action::App(Message::FollowChanged { account, person, result })
        })
    }

    /// The contact list of `account` was changed for `person`, or wasn't.
    /// Changed, the feed starts over with whoever is followed now.
    pub fn handle_follow_changed(
        &mut self,
        account: PublicKey,
        person: PublicKey,
        result: Result<Vec<PublicKey>, FollowError>,
    ) -> Task<cosmic::Action<Message>> {
        if self.follow_pending == Some(person) {
            self.follow_pending = None;
        }
        // Whoever signed out meanwhile took their follows with them.
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return Task::none();
        }
        let problem = match result {
            Ok(follows) => {
                tracing::info!("the contact list changed: following {} account(s)", follows.len());
                return self.set_follows(follows);
            }
            Err(FollowError::NoList) => FollowProblem::NoList,
            Err(FollowError::Fetch(e)) => {
                tracing::error!("the contact list could not be asked for: {e:?}");
                let why = match e {
                    FetchError::NoRelayReachable => fl!("profile-follow-unreachable"),
                    FetchError::Incomplete => fl!("error-relays-incomplete"),
                    FetchError::Failed(error) => error,
                };
                FollowProblem::Failed(fl!("profile-follow-error", error = why))
            }
            Err(FollowError::Publish(e)) => {
                tracing::error!("the contact list did not go out: {e:?}");
                FollowProblem::Failed(fl!("profile-follow-error", error = publish_error_text(e)))
            }
        };
        let mut told = false;
        for state in self.profiles_mut().filter(|state| state.public_key == person) {
            state.follow_problem = Some(problem.clone());
            told = true;
        }
        // Asked for somewhere else than on their profile: among the
        // people a search found.
        if !told {
            self.error_message = Some(match problem {
                FollowProblem::NoList => fl!("follow-no-list-elsewhere"),
                FollowProblem::Failed(why) => why,
            });
        }
        Task::none()
    }
}

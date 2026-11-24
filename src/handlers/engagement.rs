// SPDX-License-Identifier: MIT

//! Engagement message handlers for Avestruz: what was made of the notes on
//! show, and the account's own reactions and reposts.
//!
//! Whenever notes come into a feed or a thread, the event store is asked
//! how many answered, reposted, liked and zapped them, and the relays are
//! asked the same, and whether they have the notes themselves: the ones
//! that do are the relays a note was seen on. What was asked within the
//! last couple of minutes is not asked again, so a refresh that brings the
//! same notes costs nothing.
//!
//! A reaction is a like or an emoji, chosen from what is offered under a
//! note's like: the emoji reacted with most, which are kept for that (see
//! [`crate::emoji`]), or any there is.
//!
//! A reaction or a repost shows at once and goes out behind it. Until a relay
//! has taken it the note is remembered as carrying it, so that a count
//! arriving in the meantime doesn't take it off the screen; if no relay
//! takes it, it is taken off, with the reason.

use std::time::{Duration, Instant};

use cosmic::prelude::*;
use cosmic::widget::text_input;
use nostr_sdk::prelude::{EventId, PublicKey};

use crate::emoji::{self, Group};
use crate::fl;
use crate::handlers::compose::publish_error_text;
use crate::helpers::unix_now;
use crate::messages::{Message, PublishFailure};
use crate::nostr::client::{FetchError, Source};
use crate::nostr::engagement::{self, Mark, Reaction, Standing};
use crate::state::{AppModel, EmojiPicker, REACTIONS_OFFERED, ReactionMenu, RepostMenu};
use crate::views::components::emoji_search_id;

/// How long what was made of a note is taken as known, before it is asked
/// again.
const ENGAGEMENT_FRESH: Duration = Duration::from_secs(120);

impl AppModel {
    /// Find out what was made of `notes`, those of them that were not asked
    /// about just now: from the event store at once, from the relays after.
    pub(crate) fn look_up_engagement(&mut self, notes: impl IntoIterator<Item = EventId>) -> Task<cosmic::Action<Message>> {
        let Some(service) = self.nostr.clone() else {
            return Task::none();
        };
        let now = Instant::now();
        let asked = &mut self.engagement_asked;
        let notes: Vec<EventId> = notes
            .into_iter()
            .filter(|note| match asked.get(note) {
                Some(at) if now.duration_since(*at) < ENGAGEMENT_FRESH => false,
                _ => {
                    asked.insert(*note, now);
                    true
                }
            })
            .collect();
        if notes.is_empty() {
            return Task::none();
        }
        let account = self.account.as_ref().map(|account| account.public_key);
        let ask = |source: Source| {
            let (service, notes) = (service.clone(), notes.clone());
            Task::perform(async move { service.standing(source, notes, account).await }, move |result| {
                cosmic::Action::App(Message::EngagementLoaded { source, account, result })
            })
        };
        Task::batch([ask(Source::Store), ask(Source::Relays)])
    }

    /// Forget what was made of the notes seen: the account changed, and
    /// whose likes and reposts are its own with it.
    pub(crate) fn reset_engagement(&mut self) {
        self.engagement.clear();
        self.engagement_asked.clear();
        self.pending_marks.clear();
        self.repost_menu = None;
        self.reaction_menu = None;
    }

    /// What was made of some notes came from the store or from the relays
    /// (or didn't: the counts are not worth a banner).
    pub fn handle_engagement_loaded(&mut self, source: Source, account: Option<PublicKey>, result: Result<Standing, FetchError>) {
        // Counted for another account: its likes and reposts are not this
        // one's.
        if account != self.account.as_ref().map(|account| account.public_key) {
            return;
        }
        match result {
            Ok(Standing { engagement: counted, seen_on }) => {
                // A relay that had a note has it still, whoever was asked
                // last: what is known only grows.
                self.seen_on.extend(seen_on);
                // What is on its way out is in no count yet.
                let pending = &self.pending_marks;
                self.engagement.extend(counted.into_iter().map(|(note, engagement)| {
                    let on_its_way = pending.iter().filter(|(marked, _)| *marked == note);
                    (note, on_its_way.fold(engagement, |engagement, (_, mark)| engagement.with(mark)))
                }));
            }
            Err(e) => tracing::warn!("engagement could not be fetched from {source:?}: {e:?}"),
        }
    }

    /// Show what can be done to pass the note `id` on, or put it away if
    /// it is showing.
    pub fn handle_toggle_repost_menu(&mut self, id: EventId) {
        self.reaction_menu = None;
        let showing = self.repost_menu.is_some_and(|menu| menu.note == id);
        self.repost_menu = (!showing).then_some(RepostMenu::of(id));
    }

    // =========================================================================
    // Reactions
    // =========================================================================

    /// Work out again which emoji are offered first to react with: what
    /// the settings have of the ones used changed.
    pub(crate) fn suggest_reactions(&mut self) {
        self.reaction_suggestions = emoji::suggested(&self.config.reaction_emojis, REACTIONS_OFFERED);
    }

    /// Offer what to react to the note `id` with, or put the offer away
    /// if it is showing.
    pub fn handle_toggle_reaction_menu(&mut self, id: EventId) {
        self.repost_menu = None;
        let showing = self.reaction_menu.as_ref().is_some_and(|menu| menu.note == id);
        self.reaction_menu = (!showing).then_some(ReactionMenu { note: id, picker: None });
    }

    /// Show every emoji there is, in place of the few offered first, with
    /// the field to find one by ready to be typed in.
    pub fn handle_open_emoji_picker(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(menu) = &mut self.reaction_menu else {
            return Task::none();
        };
        menu.picker = Some(EmojiPicker { typed: String::new(), group: Group::SmileysAndEmotion });
        text_input::focus(emoji_search_id())
    }

    /// The emoji picker, if it is open.
    fn emoji_picker_mut(&mut self) -> Option<&mut EmojiPicker> {
        self.reaction_menu.as_mut()?.picker.as_mut()
    }

    /// What to find an emoji by was typed.
    pub fn handle_emoji_search_typed(&mut self, typed: String) {
        if let Some(picker) = self.emoji_picker_mut() {
            picker.typed = typed;
        }
    }

    /// Another group of emoji was chosen to pick from: it shows, and what
    /// was typed to find one by is dropped, which would hide it.
    pub fn handle_emoji_group(&mut self, group: Group) {
        if let Some(picker) = self.emoji_picker_mut() {
            picker.group = group;
            picker.typed.clear();
        }
    }

    /// React to the note `id` with `reaction`. An emoji reacted with is
    /// one to offer again: it is noted in the settings, whatever comes of
    /// the reaction.
    pub fn handle_react(&mut self, id: EventId, reaction: Reaction) -> Task<cosmic::Action<Message>> {
        self.reaction_menu = None;
        if let Reaction::Emoji(used) = &reaction {
            emoji::record(&mut self.config.reaction_emojis, used, unix_now());
            self.save_config("reaction emoji");
            self.suggest_reactions();
        }
        self.handle_mark(id, Mark::React(reaction))
    }

    /// Leave `mark` on the note `id`, which is one on show: react to it,
    /// or repost it. Once is enough: a note that carries a mark of that
    /// kind stays as it is.
    pub fn handle_mark(&mut self, id: EventId, mark: Mark) -> Task<cosmic::Action<Message>> {
        self.repost_menu = None;
        let (Some(service), Some(account)) =
            (self.nostr.clone(), self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key))
        else {
            return Task::none();
        };
        let Some(note) = self.note_at_hand(id) else {
            return Task::none();
        };
        let engagement = self.engagement.entry(id).or_default();
        if engagement.has(&mark) {
            return Task::none();
        }
        *engagement = std::mem::take(engagement).with(&mark);
        self.pending_marks.insert((id, mark.clone()));

        let signer = self.signer.clone();
        let left = mark.clone();
        let publish = async move {
            let unsigned = match &left {
                Mark::React(reaction) => engagement::react(id, note.kind, note.author, reaction),
                // A repost carries the note as it was signed, and says
                // where it is.
                Mark::Repost => {
                    let event = service.event(id, note.author).await.ok_or(PublishFailure::NoteGone)?;
                    engagement::repost(&event, service.relay_holding(id).await)
                }
            };
            service.sign_and_publish(&signer, account, unsigned).await.map(|_| ()).map_err(PublishFailure::Failed)
        };
        Task::perform(publish, move |result| cosmic::Action::App(Message::Marked { note: id, mark: mark.clone(), result }))
    }

    /// The reaction to the note `id`, or its repost, went out, or didn't.
    pub fn handle_marked(&mut self, id: EventId, mark: Mark, result: Result<(), PublishFailure>) {
        self.pending_marks.remove(&(id, mark.clone()));
        if let Err(failure) = result {
            tracing::error!("the {mark:?} could not be published: {failure:?}");
            if let Some(engagement) = self.engagement.get_mut(&id) {
                *engagement = std::mem::take(engagement).without(&mark);
            }
            self.error_message = Some(match failure {
                PublishFailure::NoteGone => fl!("note-error-gone"),
                PublishFailure::Failed(error) => publish_error_text(error),
            });
        }
    }
}

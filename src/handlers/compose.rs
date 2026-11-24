// SPDX-License-Identifier: MIT

//! Composer message handlers for Avestruz.
//!
//! A note, a reply or a quote is written in a view of its own. What is
//! written belongs to what it is written to: leaving the composer keeps the
//! text as that target's draft, and coming back finds it. The draft of a
//! new note is also written to a file whenever the composer or the popup is
//! left, so it is there after a restart.
//!
//! Backspace leaves the composer as well, and is the one way out that
//! may keep nothing (see [`Leaving`]). With nothing written there is
//! nothing to keep, and it leaves. With something written it asks first:
//! keep it as a draft, discard it, or go on writing.
//!
//! ```mermaid
//! flowchart TD
//!     key["Backspace, with nothing in the text for it to rub out, or with the keyboard out of the text"] --> held{"Was it held down, rubbing the text out until a moment ago?"}
//!     held -->|"yes"| nothing["Nothing: rubbing out what was written is not leaving"]
//!     held -->|"no"| written{"Is anything written, or a file on its way to be attached?"}
//!     written -->|"no"| leave["The composer is left, and there is no draft"]
//!     written -->|"yes"| ask["It asks: keep it as a draft?"]
//!     ask -->|"Enter"| keep["Left, and what was written is kept as the draft"]
//!     ask -->|"Backspace"| discard["Left, and nothing is kept: the draft is gone, and the files are called off"]
//!     ask -->|"Escape"| stay["Not left: the keyboard is back in the text"]
//! ```
//!
//! A quote is written as a note is, with the note it quotes on show under
//! the text. That note's address is put under what was written when it goes
//! out, and may be all that goes out: a quote needs nothing said.
//!
//! A file is attached by sending it to the account's media servers and
//! putting its address in the text (see [`crate::handlers::media`]). What
//! the note says of that file goes out with it, as long as the address is
//! still in the text.
//!
//! Publishing has the account's signer sign (its key from the keyring, or
//! its remote signer) and sends to the connected relays. Until a relay has
//! taken it the text stays where it is; once one has, the note or the quote
//! shows at the top of the feed, or the reply in its place in the thread.

use std::time::{Duration, Instant};

use cosmic::iced::widget::operation::focus;
use cosmic::prelude::*;
use cosmic::widget::text_editor::{Action, Content, Motion};
use nostr_sdk::prelude::{EventId, PublicKey, Tag};

use crate::drafts;
use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::messages::{Message, PublishFailure};
use crate::nostr::account::Signer;
use crate::nostr::blossom;
use crate::nostr::client::{NostrService, PublishError};
use crate::nostr::compose::{self, Target};
use crate::nostr::models::Note;
use crate::state::{AppModel, Composer, ViewState};
use crate::views::compose::COMPOSE_EDITOR_ID;

/// How long after the text was last rubbed out a Backspace is still taken
/// for the one that did it, held down: longer than the gap between the
/// repeats of a held key, which come some thirty to a second, and
/// shorter than the gap between two presses made on purpose.
const RUBBING_OUT: Duration = Duration::from_millis(150);

/// What Backspace does to the composer, pressed to leave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leaving {
    /// Leave: there is nothing to keep.
    Leave,
    /// Ask whether what was written is kept as a draft.
    Ask,
    /// Leave, and keep nothing: it was asked, and this is the answer.
    Discard,
}

impl Leaving {
    /// What Backspace does to a composer with something `written` in it
    /// or not, a file on its way to be attached to it (`attaching`) or
    /// not, and the question already `asked` or not.
    pub(crate) fn on_backspace(written: bool, attaching: bool, asked: bool) -> Self {
        match (asked, written || attaching) {
            (true, _) => Self::Discard,
            (false, true) => Self::Ask,
            (false, false) => Self::Leave,
        }
    }
}

/// Whether a Backspace pressed at `now` is one held down that has been
/// rubbing the text out, which was last changed at `rubbed`.
fn held_from_rubbing_out(rubbed: Option<Instant>, now: Instant) -> bool {
    rubbed.is_some_and(|rubbed| now.saturating_duration_since(rubbed) < RUBBING_OUT)
}

/// Have `signer` sign `text` as `author`'s note, or reply to what `target`
/// names, which `answered` wrote, and send it to the relays, with `said`
/// for what it says of the files it links to. The text of a quote names
/// what it quotes already. Returns it as a row shows it.
async fn publish(
    service: NostrService,
    signer: Signer,
    author: PublicKey,
    target: Target,
    answered: Option<PublicKey>,
    text: String,
    said: Vec<Tag>,
) -> Result<Note, PublishFailure> {
    let parent = match target {
        // What a quote quotes is in its text.
        Target::Note | Target::Quote(_) => None,
        Target::Reply(id) => {
            let answered = answered.ok_or(PublishFailure::NoteGone)?;
            Some(service.event(id, answered).await.ok_or(PublishFailure::NoteGone)?)
        }
    };
    let unsigned = compose::build(&text, parent.as_ref(), author).tags(said);
    let event = service.sign_and_publish(&signer, author, unsigned).await.map_err(PublishFailure::Failed)?;
    Ok(service.note(event).await)
}

/// Why an event did not go out, in the user's language.
pub(crate) fn publish_error_text(error: PublishError) -> String {
    match error {
        PublishError::Sign(error) => sign_error_text(error),
        PublishError::NoRelayReachable => fl!("error-no-relay"),
        PublishError::Refused(reason) => fl!("compose-error-refused", reason = reason),
        PublishError::Failed(error) => fl!("compose-error-failed", error = error),
    }
}

/// Why what was written did not go out, in the user's language.
fn publish_failure_text(failure: PublishFailure) -> String {
    match failure {
        PublishFailure::NoteGone => fl!("compose-error-parent-gone"),
        PublishFailure::Failed(error) => publish_error_text(error),
    }
}

impl AppModel {
    /// Whether the account can publish: someone is signed in with a key.
    pub(crate) fn can_publish(&self) -> bool {
        self.account.as_ref().is_some_and(|account| account.can_sign)
    }

    /// Open the composer on `target`, which is about `about` if it is about
    /// a note, with what was written to it before, the cursor after it, and
    /// the keyboard in the text.
    fn open_composer(&mut self, target: Target, about: Option<Note>) -> Task<cosmic::Action<Message>> {
        if !self.can_publish() {
            return Task::none();
        }
        let mut content = Content::with_text(self.drafts.get(target));
        content.perform(Action::Move(Motion::DocumentEnd));
        self.composer = Some(Composer { target, about, content, publishing: false, error: None, leaving: false, rubbed: None });
        self.view_state = ViewState::Compose;
        focus(COMPOSE_EDITOR_ID.clone())
    }

    /// Write a new note.
    pub fn handle_compose_note(&mut self) -> Task<cosmic::Action<Message>> {
        self.open_composer(Target::Note, None)
    }

    /// Write a reply to the note `id`, which is one on show.
    pub fn handle_compose_reply(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        match self.note_at_hand(id) {
            Some(note) => self.open_composer(Target::Reply(id), Some(note)),
            None => Task::none(),
        }
    }

    /// Write a note that quotes the note `id`, which is one on show.
    pub fn handle_compose_quote(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        self.repost_menu = None;
        match self.note_at_hand(id) {
            Some(note) => self.open_composer(Target::Quote(id), Some(note)),
            None => Task::none(),
        }
    }

    /// Something was typed, selected or moved in the composer. Nothing is,
    /// while what is there is on its way out.
    pub fn handle_compose_edited(&mut self, action: Action) {
        if let Some(composer) = self.composer.as_mut().filter(|composer| !composer.publishing) {
            if action.is_edit() {
                composer.error = None;
                composer.rubbed = Some(Instant::now());
            }
            composer.content.perform(action);
        }
    }

    /// Backspace was pressed in the composer's text with nothing left in
    /// it to rub out: leave the composer (see [`Self::handle_abandon_composer`]).
    /// Not if it is the Backspace that rubbed out the last of the text,
    /// still held down: that one was rubbing out, and goes on doing
    /// nothing until it is let go.
    pub fn handle_composer_rubbed_out(&mut self) -> Task<cosmic::Action<Message>> {
        let now = Instant::now();
        match self.composer.as_mut() {
            Some(composer) if held_from_rubbing_out(composer.rubbed, now) => {
                composer.rubbed = Some(now);
                Task::none()
            }
            _ => self.handle_abandon_composer(),
        }
    }

    /// Backspace was pressed to leave the composer: with nothing in the
    /// text, or with the keyboard out of the text. With nothing written
    /// it is left, and there is no draft. With something written, it is
    /// asked whether that is kept as a draft; and pressed again once that
    /// is asked, the answer is no (see [`Leaving`]). Nothing, while what
    /// was written is on its way out.
    pub fn handle_abandon_composer(&mut self) -> Task<cosmic::Action<Message>> {
        let Some((target, written, asked)) = self
            .composer
            .as_ref()
            .filter(|composer| !composer.publishing)
            .map(|composer| (composer.target, !composer.content.text().trim().is_empty(), composer.leaving))
        else {
            return Task::none();
        };
        let attaching = self.uploads_for(target).next().is_some();
        match Leaving::on_backspace(written, attaching, asked) {
            Leaving::Leave => self.handle_close_composer(),
            Leaving::Discard => self.handle_discard_draft(),
            Leaving::Ask => {
                if let Some(composer) = self.composer.as_mut() {
                    composer.leaving = true;
                }
                Task::none()
            }
        }
    }

    /// Leave the composer and keep nothing of what was written: its
    /// target has no draft any more, here or on the relay of this
    /// machine, and the files that were on their way to be attached to
    /// it are called off.
    pub fn handle_discard_draft(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(composer) = self.composer.as_mut() else {
            return Task::none();
        };
        composer.content = Content::new();
        let target = composer.target;
        let attaching: Vec<u64> = self.uploads_for(target).map(|upload| upload.id).collect();
        for upload in attaching {
            self.handle_cancel_upload(upload);
        }
        self.handle_close_composer()
    }

    /// Not leaving the composer after all: the question is put away, and
    /// the keyboard is back in the text.
    pub fn handle_resume_composer(&mut self) -> Task<cosmic::Action<Message>> {
        match self.composer.as_mut() {
            Some(composer) => {
                composer.leaving = false;
                focus(COMPOSE_EDITOR_ID.clone())
            }
            None => Task::none(),
        }
    }

    /// Keep what is in the composer as its target's draft, and have the
    /// relay of this machine keep it.
    pub(crate) fn keep_draft(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(composer) = &self.composer else {
            return Task::none();
        };
        let (target, text) = (composer.target, composer.content.text());
        self.drafts.set(target, text);
        self.send_draft(target)
    }

    /// Have the relay of this machine keep the draft for `target` as it
    /// is now, with what it says of the files it links to, unless that is
    /// how it has it. It takes the account's key: the draft is encrypted
    /// to the account and signed. Where that can't
    /// be asked for, the draft of a new note goes to its file.
    pub(crate) fn send_draft(&mut self, target: Target) -> Task<cosmic::Action<Message>> {
        let text = self.drafts.get(target).to_string();
        if self.drafts_kept.get(&target).map_or("", String::as_str) == text {
            return Task::none();
        }
        let account = self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key);
        let (Some(service), Some(account)) = (self.nostr.clone(), account) else {
            return self.save_note_draft(target);
        };
        let (signer, said) = (self.signer.clone(), self.media.said_in(&text));
        Task::perform(
            async move {
                let result = service.keep_draft(&signer, account, target, &text, &said).await;
                (text, result)
            },
            move |(text, result)| cosmic::Action::App(Message::DraftKept { account, target, text, result }),
        )
    }

    /// The relay of this machine took `text` as the draft of `account`
    /// for `target`, or didn't. Taken, the file a new note's draft used
    /// to be kept in is not needed; not taken, that file is where the
    /// draft of a new note goes, so that it is there after a restart.
    pub fn handle_draft_kept(
        &mut self,
        account: PublicKey,
        target: Target,
        text: String,
        result: Result<(), String>,
    ) -> Task<cosmic::Action<Message>> {
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return Task::none();
        }
        match result {
            Ok(()) => {
                if text.trim().is_empty() {
                    self.drafts_kept.remove(&target);
                } else {
                    self.drafts_kept.insert(target, text);
                }
                if target == Target::Note {
                    Task::future(drafts::save(self.draft_path.clone(), String::new())).discard()
                } else {
                    Task::none()
                }
            }
            Err(e) => {
                tracing::warn!("a draft could not be kept on the relay of this machine: {e}");
                self.save_note_draft(target)
            }
        }
    }

    /// Ask the relay of this machine for the account's drafts, once for
    /// an account, while the popup is open: reading them takes the key.
    pub(crate) fn load_drafts(&mut self) -> Task<cosmic::Action<Message>> {
        if self.popup.is_none() || self.drafts_asked {
            return Task::none();
        }
        let account = self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key);
        let (Some(service), Some(account)) = (self.nostr.clone(), account) else {
            return Task::none();
        };
        self.drafts_asked = true;
        let signer = self.signer.clone();
        Task::perform(async move { service.kept_drafts(&signer, account).await }, move |result| {
            cosmic::Action::App(Message::KeptDraftsLoaded { account, result })
        })
    }

    /// The relay of this machine said which drafts of `account` it keeps.
    /// Whatever has been written since is newer.
    pub fn handle_kept_drafts_loaded(&mut self, account: PublicKey, result: Result<Vec<(Target, String, Vec<Tag>)>, String>) {
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return;
        }
        match result {
            Ok(kept) => {
                tracing::debug!("{} draft(s) kept on the relay of this machine", kept.len());
                for (target, text, said) in kept {
                    // What it says of the files it links to is said
                    // again when it goes out.
                    for tag in said {
                        if let Some(url) = blossom::imeta_url(&tag).map(str::to_string) {
                            self.media.said.entry(url).or_insert(tag);
                        }
                    }
                    if self.drafts.get(target).is_empty() {
                        self.drafts.set(target, text.clone());
                    }
                    self.drafts_kept.insert(target, text);
                }
            }
            Err(e) => tracing::warn!("the drafts could not be asked of the relay of this machine: {e}"),
        }
    }

    /// Forget which drafts the relay of this machine has: the account
    /// changed, and they were its own.
    pub(crate) fn reset_kept_drafts(&mut self) {
        self.drafts_kept.clear();
        self.drafts_asked = false;
    }

    /// Write the draft of a new note to its file, if `target` is that: where
    /// it is kept when the relay of this machine can't be given it.
    fn save_note_draft(&self, target: Target) -> Task<cosmic::Action<Message>> {
        if target != Target::Note {
            return Task::none();
        }
        let (path, text) = (self.draft_path.clone(), self.drafts.get(Target::Note).to_string());
        Task::future(drafts::save(path, text)).discard()
    }

    /// The draft of a new note was read at startup. Whatever has been
    /// written since is newer.
    pub fn handle_draft_loaded(&mut self, text: String) {
        if self.drafts.get(Target::Note).is_empty() {
            self.drafts.set(Target::Note, text);
        }
    }

    /// Leave the composer for the view it was opened from, keeping what was
    /// written.
    pub fn handle_close_composer(&mut self) -> Task<cosmic::Action<Message>> {
        let kept = self.keep_draft();
        self.composer = None;
        Task::batch([kept, self.show_list_view()])
    }

    /// Sign what is in the composer and send it to the relays.
    pub fn handle_publish(&mut self) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(author)) =
            (self.nostr.clone(), self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key))
        else {
            return Task::none();
        };
        let Some(composer) = self.composer.as_mut().filter(|composer| !composer.publishing) else {
            return Task::none();
        };
        let text = composer.content.text().trim().to_string();
        let text = match (composer.target, &composer.about) {
            (Target::Quote(_), Some(quoted)) => compose::quote(&text, quoted),
            _ => text,
        };
        let target = composer.target;
        // Not while something attached to it is still on its way.
        if text.is_empty() || self.media.uploads.iter().any(|upload| upload.target == target) {
            return Task::none();
        }
        composer.publishing = true;
        composer.error = None;
        let answered = composer.about.as_ref().map(|note| note.author);
        let (signer, said) = (self.signer.clone(), self.media.said_in(&text));
        Task::perform(publish(service, signer, author, target, answered, text, said), move |result| {
            cosmic::Action::App(Message::Published { target, result })
        })
    }

    /// What was written to `target` went out, or didn't.
    pub fn handle_published(&mut self, target: Target, result: Result<Note, PublishFailure>) -> Task<cosmic::Action<Message>> {
        // The composer may have been left, or opened on something else,
        // while this was on its way.
        let open = self.composer.as_ref().is_some_and(|composer| composer.target == target);
        match result {
            Ok(note) => {
                // Done with, on the relay of this machine and in the file.
                self.drafts.set(target, String::new());
                let saved = Task::batch([self.send_draft(target), self.save_note_draft(target)]);
                let shown = if open {
                    self.composer = None;
                    self.show_list_view()
                } else {
                    Task::none()
                };
                Task::batch([saved, shown, self.show_published(target, note)])
            }
            Err(failure) => {
                tracing::error!("publishing failed: {failure:?}");
                let text = publish_failure_text(failure);
                match self.composer.as_mut().filter(|_| open) {
                    Some(composer) => {
                        composer.publishing = false;
                        composer.error = Some(text);
                    }
                    // The text is its target's draft still: say where
                    // something can be read.
                    None => self.error_message = Some(text),
                }
                Task::none()
            }
        }
    }

    /// Put a note that just went out where it belongs: a new note or a
    /// quote at the top of the feed, brought into view, with the notes that
    /// waited; a reply in every open thread that has the note it answers;
    /// and any of them among its author's notes, wherever their profile is
    /// open.
    fn show_published(&mut self, target: Target, note: Note) -> Task<cosmic::Action<Message>> {
        let mut people = Vec::new();
        for state in self.profiles_mut().filter(|state| state.public_key == note.author) {
            people.extend(state.notes.feed.merge([note.clone()]));
        }
        match target {
            // With whatever waited to be shown, at the top of the feed:
            // nothing that waits is older than what the feed shows.
            Target::Note | Target::Quote(_) => {
                self.following.pending.add(note);
                let shown = self.show_new_notes(self.following.id);
                Task::batch([shown, self.look_up_people(people)])
            }
            Target::Reply(parent) => {
                for state in self.threads_mut().filter(|state| state.thread.contains(parent)) {
                    people.extend(state.thread.merge([note.clone()]));
                }
                // The note answered has one more answer, wherever it shows.
                if let Some(engagement) = self.engagement.get_mut(&parent) {
                    engagement.replies += 1;
                }
                Task::batch([self.look_up_people(people), self.look_up_engagement([note.id])])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Leaving, RUBBING_OUT, held_from_rubbing_out};
    use std::time::{Duration, Instant};

    #[test]
    fn backspace_leaves_an_empty_composer_and_asks_before_leaving_one_with_something_in_it() {
        // Nothing written, nothing on its way: there is nothing to keep.
        assert_eq!(Leaving::on_backspace(false, false, false), Leaving::Leave);
        // Something written, or a file on its way to be attached: asked.
        assert_eq!(Leaving::on_backspace(true, false, false), Leaving::Ask);
        assert_eq!(Leaving::on_backspace(false, true, false), Leaving::Ask);
        assert_eq!(Leaving::on_backspace(true, true, false), Leaving::Ask);
        // Once asked, Backspace is the answer that keeps nothing.
        assert_eq!(Leaving::on_backspace(true, false, true), Leaving::Discard);
        assert_eq!(Leaving::on_backspace(false, true, true), Leaving::Discard);
    }

    #[test]
    fn a_backspace_held_down_to_rub_the_text_out_is_not_one_that_leaves() {
        let now = Instant::now() + Duration::from_secs(60);
        // Nothing was rubbed out: the composer was only just opened.
        assert!(!held_from_rubbing_out(None, now));
        // The text was changed a repeat of a held key ago: still held.
        assert!(held_from_rubbing_out(Some(now - Duration::from_millis(40)), now));
        assert!(held_from_rubbing_out(Some(now), now));
        // Changed a while ago: this press is one made on purpose.
        assert!(!held_from_rubbing_out(Some(now - RUBBING_OUT), now));
        assert!(!held_from_rubbing_out(Some(now - Duration::from_secs(5)), now));
        // A clock that reads earlier than it did costs nothing.
        assert!(held_from_rubbing_out(Some(now + Duration::from_millis(10)), now));
    }
}

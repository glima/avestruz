// SPDX-License-Identifier: MIT

//! The account's media servers, and attaching files to what is being
//! written.
//!
//! The servers are the account's server list (see
//! [`crate::nostr::blossom`]). They show as the event store has them at
//! once, and as the relays have them when they have answered; until then
//! nothing can be changed, since what is saved replaces what the list
//! names. Servers are added, taken out and moved on the page, and
//! nothing is written until Save, as on the relays page; a first list is
//! only written after saying so. There is no server until the account
//! names one: none is built in.
//!
//! A file is attached from the composer: picked in the desktop's file
//! chooser, or pasted. The chooser is a window of its own, and the popup
//! is above every window and has the keyboard, so the popup closes while
//! the chooser is open, and opens again on the composer when it is done.
//!
//! ```mermaid
//! flowchart TD
//!     attach["Attach, in the composer"] --> any{"Any media server?"}
//!     any -->|"none"| page["The media servers page, to add one"]
//!     any -->|"yes"| chooser["The popup closes; the file chooser opens"]
//!     chooser --> picked["Files picked: the popup opens again"]
//!     paste["Ctrl+V, in the composer"] --> what{"What is on the clipboard?"}
//!     what -->|"text"| typed["Pasted into the text"]
//!     what -->|"files, a picture"| each
//!     picked --> each["For each file"]
//!     each --> ready["Read; a picture made bare; hashed"]
//!     ready --> signed["The account signs an authorization for that hash"]
//!     signed --> sent["Sent to the first server that takes it; the others fetch it"]
//!     sent --> link["Its address goes at the end of the text, and what the note says of it is kept"]
//! ```
//!
//! Each file is on its way by itself, shown under the text with how far
//! it has got, and can be called off. They belong to what they were
//! attached to: one that arrives after the composer was left goes at the
//! end of that target's draft. Nothing is published while a file for it
//! is on its way.
//!
//! What a note says of a file ([`crate::nostr::blossom::imeta`]) is kept by
//! the file's address, and said when a note that links to that address
//! goes out: taking the address out of the text takes the file out of the
//! note. It is kept with the draft, too, on the relay of this machine.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use cosmic::dialog::file_chooser::{self, FileFilter};
use cosmic::iced::clipboard;
use cosmic::iced::clipboard::mime::AllowedMimeTypes;
use cosmic::iced::widget::operation::focus;
use cosmic::prelude::*;
use cosmic::widget::text_editor::{Action, Edit, Motion};
use nostr_sdk::prelude::{Event, PublicKey, Timestamp, Url};

use crate::fl;
use crate::handlers::auth::sign_error_text;
use crate::handlers::compose::publish_error_text;
use crate::handlers::feed::fetch_error_text;
use crate::helpers::readable_size;
use crate::media::{self, MAX_BYTES, MediaError};
use crate::messages::{AttachFailure, Message};
use crate::nostr::account::Signer;
use crate::nostr::blossom::{self, Attachment, Sent, UploadError, server_name, typed_server};
use crate::nostr::client::{FetchError, ListError, Source};
use crate::nostr::compose::Target;
use crate::state::{AppModel, MediaState, Upload, ViewState};
use crate::views::compose::COMPOSE_EDITOR_ID;

/// What a picture pasted from the clipboard is called.
const PASTED_NAME: &str = "pasted";

/// The kinds of thing taken from the clipboard, the most wanted first: a
/// list of files, then text, and a picture only where there is neither.
/// Something copied in a spreadsheet is offered as text and as a picture
/// of it, and it is the text that is meant.
const FILE_LIST: &str = "text/uri-list";
const TEXTS: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
const PICTURES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// What was on the clipboard when Ctrl+V was pressed in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pasted {
    /// Text: it goes into what is written.
    Text(String),
    /// Files copied in a file manager: each is attached.
    Files(Vec<PathBuf>),
    /// A picture, copied as one: it is attached. What it is made of, and
    /// what it is called.
    Picture(Vec<u8>, String),
}

impl AllowedMimeTypes for Pasted {
    fn allowed() -> Cow<'static, [String]> {
        std::iter::once(FILE_LIST).chain(TEXTS).chain(PICTURES).map(str::to_string).collect()
    }
}

impl TryFrom<(Vec<u8>, String)> for Pasted {
    type Error = ();

    fn try_from((bytes, kind): (Vec<u8>, String)) -> Result<Self, Self::Error> {
        pasted(bytes, &kind).ok_or(())
    }
}

/// What `bytes` from the clipboard are, which it said are a `kind`. A
/// list of addresses none of which is a file is the text it is.
fn pasted(bytes: Vec<u8>, kind: &str) -> Option<Pasted> {
    if let Some(picture) = PICTURES.iter().find(|picture| **picture == kind) {
        let extension = picture.rsplit('/').next().unwrap_or_default();
        return (!bytes.is_empty()).then(|| Pasted::Picture(bytes, format!("{PASTED_NAME}.{extension}")));
    }
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    if kind == FILE_LIST {
        let listed = text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#'));
        let files: Vec<PathBuf> =
            listed.filter_map(|line| Url::parse(line).ok().filter(|url| url.scheme() == "file")?.to_file_path().ok()).collect();
        if !files.is_empty() {
            return Some(Pasted::Files(files));
        }
    }
    (!text.is_empty()).then_some(Pasted::Text(text))
}

/// A file to attach: where it is, or what it is called and made of.
#[derive(Debug, Clone)]
enum Picked {
    File(PathBuf),
    Bytes(String, Vec<u8>),
}

impl Picked {
    /// What the file is called.
    fn name(&self) -> String {
        match self {
            Self::File(path) => path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
            Self::Bytes(name, _) => name.clone(),
        }
    }
}

/// Send `picked` to the first of `servers` that takes it, as `account`,
/// which `signer` signs for: read, a picture made bare (see
/// [`crate::media`]), hashed, authorized, and sent (see
/// [`blossom::send`]), with `size` set to how large it is as it goes and
/// `sent` kept at how much of it has gone.
async fn attach(
    http: reqwest::Client,
    signer: Signer,
    account: PublicKey,
    servers: Vec<Url>,
    picked: Picked,
    size: Sent,
    sent: Sent,
) -> Result<Attachment, AttachFailure> {
    let name = picked.name();
    let bytes = match picked {
        Picked::File(path) => {
            // Not read at all, if it is too large to be sent.
            let length = tokio::fs::metadata(&path).await.map_err(|e| AttachFailure::Read(e.to_string()))?.len();
            let length = usize::try_from(length).unwrap_or(usize::MAX);
            if length > MAX_BYTES {
                return Err(AttachFailure::Media(MediaError::TooLarge(length)));
            }
            tokio::fs::read(&path).await.map_err(|e| AttachFailure::Read(e.to_string()))?
        }
        Picked::Bytes(_, bytes) => bytes,
    };
    let ready = tokio::task::spawn_blocking(move || {
        let media = media::prepare(name, bytes)?;
        let hash = blossom::hash_of(&media.bytes);
        Ok((media, hash))
    });
    let (media, hash) = ready.await.map_err(|e| AttachFailure::Read(e.to_string()))?.map_err(AttachFailure::Media)?;
    size.store(media.bytes.len(), Ordering::Relaxed);

    let unsigned = blossom::upload_auth(&media.name, &hash, Timestamp::now());
    let auth = signer.sign(account, unsigned).await.map_err(AttachFailure::Sign)?;
    blossom::send(&http, &servers, &auth, media, &hash, &sent).await.map_err(AttachFailure::Refused)
}

/// Why a server did not take a file, in the user's language.
fn upload_error_text(error: &UploadError) -> String {
    match error {
        UploadError::Unreachable(error) => fl!("attach-unreachable", error = error.as_str()),
        UploadError::Refused { status, reason } => fl!("attach-refused", status = status.to_string(), reason = reason.as_str()),
        UploadError::Unreadable(error) => fl!("attach-unreadable-answer", error = error.as_str()),
    }
}

/// Why the file called `name` was not attached, in the user's language.
fn attach_failure_text(name: &str, failure: AttachFailure) -> String {
    match failure {
        AttachFailure::Read(error) => fl!("attach-error-read", name = name, error = error),
        AttachFailure::Media(MediaError::Empty) => fl!("attach-error-empty", name = name),
        AttachFailure::Media(MediaError::TooLarge(size)) => {
            fl!("attach-error-too-large", name = name, size = readable_size(size), most = readable_size(MAX_BYTES))
        }
        AttachFailure::Media(MediaError::Unreadable) => fl!("attach-error-unreadable", name = name),
        AttachFailure::Media(MediaError::Damaged) => fl!("attach-error-damaged", name = name),
        AttachFailure::Sign(error) => fl!("attach-error-sign", name = name, error = sign_error_text(error)),
        AttachFailure::Refused(refused) => {
            let each: Vec<String> =
                refused.iter().map(|(server, error)| format!("{}: {}", server_name(server), upload_error_text(error))).collect();
            fl!("attach-error-refused", name = name, servers = each.join(" "))
        }
    }
}

impl AppModel {
    /// Forget the account's media servers, and call off what is on its
    /// way to them: the account changed, and they were its own.
    pub(crate) fn reset_media(&mut self) {
        for upload in &self.media.uploads {
            upload.handle.abort();
        }
        self.media = MediaState::default();
    }

    /// Find out what the account's media servers are, while the popup is
    /// open, unless the relays said already.
    pub(crate) fn load_media_servers(&mut self) -> Task<cosmic::Action<Message>> {
        if self.popup.is_none() || self.media.from_relays || self.media.loading {
            return Task::none();
        }
        self.ask_media_servers(Source::Store)
    }

    /// Ask `source` for the account's server list, unless it is being
    /// changed: what the change leaves is what it is.
    fn ask_media_servers(&mut self, source: Source) -> Task<cosmic::Action<Message>> {
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        if self.media.saving {
            return Task::none();
        }
        if source == Source::Relays {
            self.media.loading = true;
        }
        Task::perform(async move { service.media_server_list(source, account).await }, move |result| {
            cosmic::Action::App(Message::MediaServersLoaded { account, source, result })
        })
    }

    /// The servers are what `list` says from now on. Those on the page
    /// that were not changed there show what it says; changed, they stay
    /// as they are.
    fn take_media_servers(&mut self, list: Option<Event>) {
        let (before, now) = (self.media.servers(), blossom::servers(list.as_ref()));
        if !self.media.read || self.media.edited == before {
            self.media.edited = now;
        }
        self.media.list = list;
        self.media.read = true;
    }

    /// The account's server list came from the store or from the relays,
    /// or didn't. After the store, the relays are asked.
    pub fn handle_media_servers_loaded(
        &mut self,
        account: PublicKey,
        source: Source,
        result: Result<Option<Event>, FetchError>,
    ) -> Task<cosmic::Action<Message>> {
        // Whoever signed out meanwhile took their servers with them.
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return Task::none();
        }
        let from_relays = source == Source::Relays;
        if from_relays {
            self.media.loading = false;
        }
        match result {
            // A change on its way says what the list is when it is done;
            // and the store's word is not taken over the relays'.
            Ok(_) if self.media.saving || (!from_relays && self.media.from_relays) => {}
            Ok(list) => {
                self.take_media_servers(list);
                self.media.from_relays = from_relays;
            }
            Err(e) => {
                tracing::warn!("the account's media server list could not be asked of {source:?}: {e:?}");
                if from_relays && self.view_state == ViewState::Media {
                    self.error_message = Some(fl!("media-not-read", error = fetch_error_text(e)));
                }
            }
        }
        match source {
            Source::Store => self.ask_media_servers(Source::Relays),
            Source::Relays => Task::none(),
        }
    }

    /// Show the account's media servers, in place of what is on show, and
    /// ask for its list again.
    pub fn handle_open_media(&mut self) -> Task<cosmic::Action<Message>> {
        self.menu_open = false;
        self.view_state = ViewState::Media;
        self.ask_media_servers(Source::Store)
    }

    /// Leave the media servers: for the composer, if something is being
    /// written, and for the list that was on show otherwise.
    pub fn handle_close_media(&mut self) -> Task<cosmic::Action<Message>> {
        if self.composer.is_some() {
            self.view_state = ViewState::Compose;
            focus(COMPOSE_EDITOR_ID.clone())
        } else {
            self.show_list_view()
        }
    }

    /// Whether the servers can be changed now: by an account with its
    /// key, once the relays have said what its list is, while no change
    /// is on its way.
    pub(crate) fn media_can_change(&self) -> bool {
        self.can_publish() && self.media.from_relays && !self.media.saving
    }

    /// Add the server typed, on the page: nothing is written until the
    /// servers are saved.
    pub fn handle_add_media_server(&mut self) {
        if !self.media_can_change() {
            return;
        }
        let typed = self.media.typed.trim().to_string();
        if typed.is_empty() {
            return;
        }
        match typed_server(&typed) {
            Some(server) => {
                if !self.media.edited.contains(&server) {
                    self.media.edited.push(server);
                }
                self.media.typed.clear();
                self.error_message = None;
            }
            None => self.error_message = Some(fl!("media-not-a-server", typed = typed)),
        }
    }

    /// Take `server` out, on the page.
    pub fn handle_remove_media_server(&mut self, server: &Url) {
        if self.media_can_change() {
            self.media.edited.retain(|other| other != server);
        }
    }

    /// Move `server` a place up, or down, on the page.
    pub fn handle_shift_media_server(&mut self, server: &Url, up: bool) {
        if !self.media_can_change() {
            return;
        }
        let Some(at) = self.media.edited.iter().position(|other| other == server) else {
            return;
        };
        let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|to| *to < self.media.edited.len()) };
        if let Some(to) = to {
            self.media.edited.swap(at, to);
        }
    }

    /// Whether there is anything to save, and it can be saved now.
    pub(crate) fn media_can_save(&self) -> bool {
        self.media_can_change() && self.media.changed()
    }

    /// Write the servers as the page has them to the account's list.
    pub fn handle_save_media_servers(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.media_can_save() {
            return Task::none();
        }
        let (Some(service), Some(account)) = (self.nostr.clone(), self.account.as_ref().map(|account| account.public_key)) else {
            return Task::none();
        };
        let (signer, servers, first_list) = (self.signer.clone(), self.media.edited.clone(), self.media.first_list);
        self.media.saving = true;
        self.error_message = None;
        Task::perform(async move { service.set_media_servers(&signer, account, &servers, first_list).await }, move |result| {
            cosmic::Action::App(Message::MediaServersSaved { account, result })
        })
    }

    /// The account's server list was changed, or wasn't. What was not
    /// saved stays on the page, changed, to be saved again.
    pub fn handle_media_servers_saved(&mut self, account: PublicKey, result: Result<Option<Event>, ListError>) {
        self.media.saving = false;
        if self.account.as_ref().map(|account| account.public_key) != Some(account) {
            return;
        }
        self.error_message = match result {
            Ok(list) => {
                self.media.edited = blossom::servers(list.as_ref());
                self.media.list = list;
                self.media.first_list = false;
                None
            }
            Err(ListError::NoList) => {
                self.media.first_list = true;
                Some(fl!("media-no-list"))
            }
            Err(ListError::Fetch(e)) => {
                tracing::error!("the media server list could not be asked for: {e:?}");
                Some(fl!("media-error", error = fetch_error_text(e)))
            }
            Err(ListError::Unreadable(e)) => Some(fl!("media-error", error = sign_error_text(e))),
            Err(ListError::Publish(e)) => {
                tracing::error!("the media server list did not go out: {e:?}");
                Some(fl!("media-error", error = publish_error_text(e)))
            }
        };
    }

    /// Whether something can be attached to what is being written now:
    /// it is being written, by an account with its key, and is not on its
    /// way out.
    fn can_attach(&self) -> bool {
        self.can_publish() && self.nostr.is_some() && self.composer.as_ref().is_some_and(|composer| !composer.publishing)
    }

    /// The files on their way that are attached to `target`.
    pub(crate) fn uploads_for(&self, target: Target) -> impl Iterator<Item = &Upload> {
        self.media.uploads.iter().filter(move |upload| upload.target == target)
    }

    /// Whether there is a media server to send a file to. Where there is
    /// none, the page they are added on is shown, and says why.
    fn has_media_server(&mut self) -> Result<(), Task<cosmic::Action<Message>>> {
        if !self.media.servers().is_empty() {
            return Ok(());
        }
        let opened = self.handle_open_media();
        self.error_message = Some(fl!("media-needed"));
        Err(opened)
    }

    /// Pick what is to be attached to what is being written, in the
    /// desktop's file chooser. The popup closes meanwhile: it is above
    /// every window, the chooser among them, and has the keyboard.
    pub fn handle_attach_media(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.can_attach() || self.media.picking {
            return Task::none();
        }
        if let Err(opened) = self.has_media_server() {
            return opened;
        }
        self.media.picking = true;
        let closed = if self.popup.is_some() { self.handle_toggle_popup() } else { Task::none() };
        let (title, kinds) = (fl!("compose-attach-title"), fl!("compose-attach-kinds"));
        let picking = async move {
            let kinds = FileFilter::new(&kinds).mimetype("image/*").mimetype("video/*").mimetype("audio/*");
            let dialog = file_chooser::open::Dialog::new().title(title).filter(kinds);
            match dialog.open_files().await {
                Ok(picked) => Ok(picked.urls().iter().filter_map(|url| url.to_file_path().ok()).collect()),
                Err(file_chooser::Error::Cancelled) => Ok(Vec::new()),
                Err(e) => Err(format!("{e}: {}", std::error::Error::source(&e).map(ToString::to_string).unwrap_or_default())),
            }
        };
        Task::batch([closed, Task::perform(picking, |picked| cosmic::Action::App(Message::MediaPicked(picked)))])
    }

    /// The file chooser is done: with these files, with none, or it could
    /// not be opened. The popup opens again, on the composer.
    pub fn handle_media_picked(&mut self, picked: Result<Vec<PathBuf>, String>) -> Task<cosmic::Action<Message>> {
        self.media.picking = false;
        let opened = if self.popup.is_none() {
            // No click closed it, that the press of a button could be.
            self.popup_unfocused_at = None;
            self.handle_toggle_popup()
        } else {
            Task::none()
        };
        let attached = match picked {
            Ok(files) => self.attach_all(files.into_iter().map(Picked::File).collect()),
            Err(e) => {
                tracing::error!("the file chooser could not be opened: {e}");
                self.say_of_attaching(None, fl!("attach-error-chooser", error = e));
                Task::none()
            }
        };
        Task::batch([opened, attached, focus(COMPOSE_EDITOR_ID.clone())])
    }

    /// Ctrl+V was pressed in the composer: see what is on the clipboard.
    pub fn handle_compose_paste(&self) -> Task<cosmic::Action<Message>> {
        if !self.can_attach() {
            return Task::none();
        }
        clipboard::read_data::<Pasted>().map(|pasted| cosmic::Action::App(Message::ComposePasted(pasted)))
    }

    /// What was on the clipboard: text goes into what is written, and
    /// files or a picture are attached to it.
    pub fn handle_compose_pasted(&mut self, pasted: Option<Pasted>) -> Task<cosmic::Action<Message>> {
        match pasted {
            Some(Pasted::Text(text)) => {
                self.handle_compose_edited(Action::Edit(Edit::Paste(Arc::new(text))));
                Task::none()
            }
            Some(Pasted::Files(files)) => self.attach_pasted(files.into_iter().map(Picked::File).collect()),
            Some(Pasted::Picture(bytes, name)) => self.attach_pasted(vec![Picked::Bytes(name, bytes)]),
            None => Task::none(),
        }
    }

    /// Attach `picked`, which were pasted, if there is a server to send
    /// them to.
    fn attach_pasted(&mut self, picked: Vec<Picked>) -> Task<cosmic::Action<Message>> {
        if !self.can_attach() {
            return Task::none();
        }
        match self.has_media_server() {
            Ok(()) => self.attach_all(picked),
            Err(opened) => opened,
        }
    }

    /// Send each of `picked` on its way, attached to what is being
    /// written.
    fn attach_all(&mut self, picked: Vec<Picked>) -> Task<cosmic::Action<Message>> {
        let (Some(account), Some(target)) = (
            self.account.as_ref().filter(|account| account.can_sign).map(|account| account.public_key),
            self.composer.as_ref().map(|composer| composer.target),
        ) else {
            return Task::none();
        };
        let servers = self.media.servers();
        if servers.is_empty() {
            return Task::none();
        }
        if let Some(composer) = self.composer.as_mut().filter(|_| !picked.is_empty()) {
            composer.error = None;
        }
        let sending = picked.into_iter().map(|picked| {
            self.media.attached += 1;
            let id = self.media.attached;
            let (name, size, sent) = (picked.name(), Sent::default(), Sent::default());
            let attaching =
                attach(self.http.clone(), self.signer.clone(), account, servers.clone(), picked, size.clone(), sent.clone());
            let (task, handle) =
                Task::perform(attaching, move |result| cosmic::Action::App(Message::MediaUploaded { id, result })).abortable();
            self.media.uploads.push(Upload { id, target, name, size, sent, handle });
            task
        });
        Task::batch(sending.collect::<Vec<_>>())
    }

    /// Say `said` of attaching something to `target`: in the composer, if
    /// it is open on that, or on what is open if no target is named; and
    /// over whatever is on show otherwise.
    fn say_of_attaching(&mut self, target: Option<Target>, said: String) {
        match self.composer.as_mut().filter(|composer| target.is_none_or(|target| composer.target == target)) {
            Some(composer) => composer.error = Some(said),
            None => self.error_message = Some(said),
        }
    }

    /// The file numbered `id` is on a media server, or isn't. Its address
    /// goes at the end of what it was attached to: in the composer, if
    /// that is open on it, and in its draft otherwise.
    pub fn handle_media_uploaded(&mut self, id: u64, result: Result<Attachment, AttachFailure>) -> Task<cosmic::Action<Message>> {
        let Some(at) = self.media.uploads.iter().position(|upload| upload.id == id) else {
            // Called off.
            return Task::none();
        };
        let upload = self.media.uploads.remove(at);
        let attachment = match result {
            Ok(attachment) => attachment,
            Err(failure) => {
                tracing::error!("{} was not attached: {failure:?}", upload.name);
                self.say_of_attaching(Some(upload.target), attach_failure_text(&upload.name, failure));
                return Task::none();
            }
        };
        tracing::info!(
            "{} is at {}, and was fetched from there by {} other server(s)",
            upload.name,
            attachment.url,
            attachment.mirrored.len()
        );
        self.media.said.insert(attachment.url.clone(), attachment.imeta);
        match self.composer.as_mut().filter(|composer| composer.target == upload.target) {
            Some(composer) => {
                composer.content.perform(Action::Move(Motion::DocumentEnd));
                let text = composer.content.text();
                let lead = if text.trim().is_empty() || text.ends_with('\n') { "" } else { "\n" };
                composer.content.perform(Action::Edit(Edit::Paste(Arc::new(format!("{lead}{}\n", attachment.url)))));
                self.preview_composed_anew()
            }
            None => {
                let mut text = self.drafts.get(upload.target).to_string();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&attachment.url);
                text.push('\n');
                self.drafts.set(upload.target, text);
                self.send_draft(upload.target)
            }
        }
    }

    /// Call off the file numbered `id`, which is on its way.
    pub fn handle_cancel_upload(&mut self, id: u64) {
        if let Some(at) = self.media.uploads.iter().position(|upload| upload.id == id) {
            self.media.uploads.remove(at).handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FILE_LIST, Pasted, attach_failure_text, pasted};
    use crate::media::MediaError;
    use crate::messages::AttachFailure;
    use crate::nostr::blossom::UploadError;
    use cosmic::iced::clipboard::mime::AllowedMimeTypes;
    use nostr_sdk::prelude::Url;
    use std::path::PathBuf;

    #[test]
    fn what_is_pasted_is_files_before_text_and_text_before_a_picture() {
        let wanted = Pasted::allowed();
        let place = |kind: &str| wanted.iter().position(|allowed| allowed == kind).unwrap();
        assert!(place(FILE_LIST) < place("text/plain;charset=utf-8"));
        assert!(place("text/plain") < place("image/png"));
    }

    #[test]
    fn what_was_on_the_clipboard_is_text_files_or_a_picture() {
        let text = |said: &str| Some(Pasted::Text(said.to_string()));
        assert_eq!(pasted(b"gm\r\nnostr".to_vec(), "text/plain;charset=utf-8"), text("gm\nnostr"));
        assert_eq!(pasted(b"gm".to_vec(), "UTF8_STRING"), text("gm"));
        assert_eq!(pasted(Vec::new(), "text/plain"), None);

        let listed = b"# copied\r\nfile:///home/me/a%20picture.jpg\r\nfile:///home/me/clip.mp4\r\n".to_vec();
        let files = vec![PathBuf::from("/home/me/a picture.jpg"), PathBuf::from("/home/me/clip.mp4")];
        assert_eq!(pasted(listed, FILE_LIST), Some(Pasted::Files(files)));
        // Addresses that are no files are what they say.
        assert_eq!(pasted(b"https://example.com/a\r\n".to_vec(), FILE_LIST), text("https://example.com/a\n"));

        assert_eq!(pasted(vec![1, 2, 3], "image/png"), Some(Pasted::Picture(vec![1, 2, 3], "pasted.png".into())));
        assert_eq!(pasted(vec![1], "image/jpeg"), Some(Pasted::Picture(vec![1], "pasted.jpeg".into())));
        assert_eq!(pasted(Vec::new(), "image/png"), None);
    }

    #[test]
    fn why_a_file_was_not_attached_names_the_file_and_each_server() {
        let server = |name: &str| Url::parse(name).unwrap();
        let refused = AttachFailure::Refused(vec![
            (server("https://one.example.com"), UploadError::Refused { status: 413, reason: "too large".into() }),
            (server("https://two.example.com"), UploadError::Unreachable("no route".into())),
        ]);
        let said = attach_failure_text("holiday.jpg", refused);
        for part in ["holiday.jpg", "one.example.com", "413", "too large", "two.example.com", "no route"] {
            assert!(said.contains(part), "{part} is in: {said}");
        }
        let said = attach_failure_text("film.mp4", AttachFailure::Media(MediaError::TooLarge(300_000_000)));
        assert!(said.contains("film.mp4") && said.contains("300") && said.contains("100"), "{said}");
    }
}

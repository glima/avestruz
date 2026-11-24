// SPDX-License-Identifier: MIT

//! COSMIC application implementation for Avestruz.
//!
//! This module wires up the [`cosmic::Application`] trait — defining the
//! init, update, view, and subscription lifecycle — and re-exports the
//! core types ([`AppModel`], [`Message`], [`ViewState`]) that the rest
//! of the crate depends on.
//!
//! ```mermaid
//! flowchart TD
//!     init["init: the config is read, and four things start at once"] --> running
//!     subgraph running["For as long as the session"]
//!         direction LR
//!         subscription["subscription: two timers, the config watch, pictures and previews asked for"]
//!         update["update: one match over every Message, each arm a handler"]
//!         button["the button in the panel: another process"]
//!         window["view_window: the popup, when it is open"]
//!         subscription -->|"messages"| update
//!         window -->|"messages"| update
//!         button -->|"TogglePopup"| subscription
//!         update -->|"the model changed"| window
//!     end
//! ```
//!
//! What starts at `init`, and in which order the answers matter, is in
//! [`crate::guide::architecture`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::Subscription;
use cosmic::iced::event::Status;
use cosmic::iced::keyboard::key::Named;
use cosmic::iced::keyboard::{self, Key};
use cosmic::iced::runtime::core::event::wayland::LayerEvent;
use cosmic::iced::runtime::core::event::{PlatformSpecific, wayland};
use cosmic::iced::window::Id;
use cosmic::prelude::*;
use cosmic::widget::segmented_button;
use tokio::sync::Mutex;

use crate::button;
use crate::config::Config;
use crate::drafts::{self, Drafts};
use crate::emoji;
use crate::fl;
use crate::fonts;
use crate::http;
use crate::images::{self, ImageStore};
use crate::logging;
use crate::nostr::account::{self, Signer};
use crate::nostr::client::{self, NostrService};
use crate::nostr::engagement::Mark;
use crate::nostr::feed::FeedMode;
use crate::previews::PreviewStore;
use crate::requests;
use crate::video::Videos;

// Re-export types from state and messages modules
pub use crate::messages::Message;
use crate::nostr::notifications::Tab;
pub use crate::state::{AppModel, BookmarkState, FeedState, LoginProgress, MediaState, RelaysState, ViewState};
use crate::state::{NotificationsState, REACTIONS_OFFERED, Step};

/// The name the panel, the settings and the keyring know Avestruz by.
pub const APP_ID: &str = "io.github.cosmic-applet-avestruz";

/// Run the program as the window: the whole client, with no window of its
/// own until the button in the panel asks for the popup (see
/// [`crate::button`]). It stays when the popup closes, and leaves when
/// the button does.
pub fn run() -> cosmic::iced::Result {
    let settings =
        cosmic::app::Settings::default().no_main_window(true).exit_on_close(false).default_text_size(14.0).transparent(true);
    cosmic::app::run::<AppModel>(settings, ())
}

/// How often the feed is looked at for new notes: while the popup is open,
/// and while it is closed.
const NEW_NOTES_EVERY_OPEN: Duration = Duration::from_secs(60);

/// How often the composer says anew how far the files on their way to a
/// media server have got.
const UPLOADS_SHOWN_EVERY: Duration = Duration::from_millis(250);
const NEW_NOTES_EVERY_CLOSED: Duration = Duration::from_secs(5 * 60);

/// How often it is seen whether the event store is to let go of what is
/// old: it does so once a day, and the day may begin at any time.
const SWEEP_LOOK_EVERY: Duration = Duration::from_secs(60 * 60);

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;

    type Flags = ();

    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(core: cosmic::Core, _flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        // Log as much as the settings say, unless `RUST_LOG` does, which
        // keeps precedence as it had in `main`.
        if !logging::level_set_by_environment() {
            logging::set_console_level(config.log_level);
        }

        // Before anything is drawn: emoji would come out blank, or from
        // a text font, otherwise.
        fonts::prepare();

        // Open the event store and start connecting to relays, off the main
        // thread.
        let open_task = Task::perform(NostrService::open(client::default_store_path(), config.relays.clone()), |result| {
            cosmic::Action::App(Message::NostrReady(result))
        });

        // Someone signed in before: straight to their feed, and ask the
        // keyring whether their secret key is there.
        let (view_state, restore_task) = match &config.account {
            Some(npub) => (
                ViewState::Feed,
                Task::perform(account::restore(npub.clone()), |result| cosmic::Action::App(Message::AccountRestored(result))),
            ),
            None => (ViewState::Login, Task::none()),
        };

        let draft_path = drafts::default_draft_path();

        // The two ways a list of notes is read, for the control that
        // chooses between them, with the one kept from last time chosen.
        let feed_modes = FeedMode::ALL
            .into_iter()
            .fold(segmented_button::Model::builder(), |modes, mode| {
                let label = match mode {
                    FeedMode::Notes => fl!("feed-mode-notes"),
                    FeedMode::Replies => fl!("feed-mode-replies"),
                };
                modes.insert(|choice| {
                    let choice = choice.text(label).data(mode);
                    if mode == config.feed_mode { choice.activate() } else { choice }
                })
            })
            .build();

        // Which notifications show, for the control that chooses.
        let notification_tabs = Tab::ALL
            .into_iter()
            .fold(segmented_button::Model::builder(), |tabs, tab| {
                let label = match tab {
                    Tab::All => fl!("notifications-tab-all"),
                    Tab::Mentions => fl!("notifications-tab-mentions"),
                    Tab::Reactions => fl!("notifications-tab-reactions"),
                    Tab::Zaps => fl!("notifications-tab-zaps"),
                };
                tabs.insert(|choice| {
                    let choice = choice.text(label).data(tab);
                    if tab == Tab::All { choice.activate() } else { choice }
                })
            })
            .build();

        let reaction_suggestions = emoji::suggested(&config.reaction_emojis, REACTIONS_OFFERED);

        // Views ask the store for pictures; the store sends the addresses
        // it doesn't have down a channel, which a subscription drains.
        let (image_store, image_requests) = ImageStore::new(images::DEFAULT_BUDGET_BYTES);
        // And for what the pages notes link to say of themselves, likewise.
        let (preview_store, preview_requests) = PreviewStore::new();

        let app = AppModel {
            core,
            config,
            popup: None,
            keeper: Id::unique(),
            popup_unfocused_at: None,
            unread_told: false,
            view_state,
            screen: None,
            nostr: None,
            account: None,
            login_input: String::new(),
            login_input_hidden: true,
            login_error: None,
            login_refusal: None,
            login_progress: LoginProgress::Idle,
            signer: Signer::default(),
            following: FeedState::new(0),
            follows: None,
            feeds_made: 0,
            feed_checked_at: None,
            feed_modes,
            notifications: NotificationsState::default(),
            notification_tabs,
            pages: Vec::new(),
            composer: None,
            drafts: Drafts::default(),
            draft_path: draft_path.clone(),
            drafts_kept: HashMap::new(),
            drafts_asked: false,
            zap: None,
            engagement: HashMap::new(),
            engagement_asked: HashMap::new(),
            seen_on: HashMap::new(),
            pending_marks: HashSet::new(),
            repost_menu: None,
            reaction_menu: None,
            who_menu: None,
            reaction_suggestions,
            share_copied: None,
            picture_copied: None,
            bookmarks: BookmarkState::Unknown,
            bookmarks_asked_at: None,
            bookmark_pending: None,
            bookmark_first_list: false,
            swept_at: None,
            menu_open: false,
            relays: RelaysState::default(),
            media: MediaState::default(),
            profiles: HashMap::new(),
            profiles_requested: HashSet::new(),
            nip05: HashMap::new(),
            follow_pending: None,
            images: image_store,
            image_requests: Arc::new(Mutex::new(image_requests)),
            previews: preview_store,
            videos: Videos::default(),
            preview_requests: Arc::new(Mutex::new(preview_requests)),
            http: http::client(),
            error_message: None,
        };

        let draft_task = Task::perform(drafts::load(draft_path), |text| cosmic::Action::App(Message::DraftLoaded(text)));

        // Before any popup: see `keep_what_is_copied`.
        let keeper = app.keep_what_is_copied();
        let measured = app.measure_screen_soon();
        (app, Task::batch([keeper, open_task, restore_task, draft_task, measured]))
    }

    /// Escape, pressed where nothing in the popup had a use for it of its
    /// own (an open dropdown has: it closes). libcosmic listens for the key
    /// and asks here what it means, which by default is nothing.
    fn on_escape(&mut self) -> Task<cosmic::Action<Self::Message>> {
        self.handle_escape()
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// Nothing: there is no window but the popup. The button that opens it
    /// is in the panel, and another process (see [`crate::button`]).
    fn view(&self) -> Element<'_, Self::Message> {
        cosmic::widget::Space::new().into()
    }

    /// The popup is drawn using this view method. The surface that keeps
    /// what is copied shows nothing.
    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        if id == self.keeper {
            return cosmic::widget::Space::new().into();
        }
        self.view_popup(id)
    }

    /// Register subscriptions for this application.
    fn subscription(&self) -> Subscription<Self::Message> {
        // The feed is looked at for new notes every so often: often while
        // the popup is open, now and then while it isn't. They wait to be
        // asked for either way.
        let look_every = if self.popup.is_some() { NEW_NOTES_EVERY_OPEN } else { NEW_NOTES_EVERY_CLOSED };

        // A video that plays is looked at for its next picture, and one
        // that is held now and then, while there is one.
        let video = self.videos.tick_every(self.popup.is_some());

        // How far the files on their way have got is shown as it changes.
        let uploading = self.popup.is_some() && !self.media.uploads.is_empty();

        Subscription::batch([
            if uploading {
                cosmic::iced::time::every(UPLOADS_SHOWN_EVERY).map(|_| Message::UploadTick)
            } else {
                Subscription::none()
            },
            video.map_or_else(Subscription::none, |every| cosmic::iced::time::every(every).map(|_| Message::VideoTick)),
            cosmic::iced::time::every(look_every).map(|_| Message::CheckForNewNotes),
            cosmic::iced::time::every(SWEEP_LOOK_EVERY).map(|_| Message::SweepStore),
            // Watch for application configuration changes.
            self.core().watch_config::<Config>(Self::APP_ID).map(|update| Message::UpdateConfig(update.config)),
            // Pictures the views asked for and didn't have: one `LoadImage`
            // each. `handle_load_image` decides which are actually fetched.
            requests::subscription(&self.image_requests, Message::LoadImage),
            // The pages notes link to, likewise: one `LoadPreview` each.
            requests::subscription(&self.preview_requests, Message::LoadPreview),
            // What the button in the panel says: that it was pressed, and
            // in the end that it is gone.
            Subscription::run(from_button),
            // And what is said from outside: that a key was pressed that
            // the desktop was told opens it.
            Subscription::run(from_outside),
            // The popup lost the keyboard: something else was clicked. And
            // the keys of a list, where no widget had a use for the key: a
            // text being typed takes them for itself. What they do is in
            // `handlers::keys`.
            cosmic::iced::event::listen_with(|event, status, _window| match event {
                cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, repeat, .. })
                    if status == Status::Ignored && !modifiers.command() && !modifiers.alt() && !modifiers.logo() =>
                {
                    match key.as_ref() {
                        Key::Character("j") => Some(Message::StepNote(Step::Next)),
                        Key::Character("k") => Some(Message::StepNote(Step::Previous)),
                        Key::Named(Named::Home) => Some(Message::StepNote(Step::Top)),
                        Key::Character("l") | Key::Named(Named::Enter) => Some(Message::OpenMarked),
                        Key::Character("h") => Some(Message::GoBack),
                        Key::Named(Named::Backspace) => Some(Message::Backspace { held: repeat }),
                        Key::Character(".") => Some(Message::ShowWaiting),
                        Key::Character("y") => Some(Message::ShareMarked),
                        Key::Character(" ") => Some(Message::PlayMarked),
                        Key::Character("r") => Some(Message::ReplyMarked),
                        Key::Character("R") => Some(Message::RepostMarked),
                        Key::Named(Named::ArrowDown) => Some(Message::StepChoice(Step::Next)),
                        Key::Named(Named::ArrowUp) => Some(Message::StepChoice(Step::Previous)),
                        Key::Character("z") => Some(Message::ZapMarked),
                        Key::Character("b") => Some(Message::BookmarkMarked),
                        Key::Character("B") => Some(Message::GoToBookmarks),
                        Key::Character("?") => Some(Message::ShowKeys),
                        _ => None,
                    }
                }
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(wayland::Event::Layer(
                    LayerEvent::Unfocused,
                    _,
                    id,
                ))) => Some(Message::PopupUnfocused(id)),
                _ => None,
            }),
        ])
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// This function dispatches messages to the appropriate handler modules:
    /// - `handlers::navigation` - The popup, which view it shows, and the pages opened over the feed
    /// - `handlers::auth` - Signing in and out
    /// - `handlers::feed` - Loading the feed, other lists of notes, and authors' profiles
    /// - `handlers::thread` - Opening a note's thread
    /// - `handlers::profile` - Opening someone's profile
    /// - `handlers::compose` - Writing and publishing notes, replies and quotes
    /// - `handlers::media` - The account's media servers, and attaching files to what is being written
    /// - `handlers::keys` - The keyboard, on a list: stepping through it, opening a row, going back
    /// - `handlers::engagement` - What was made of notes, and liking and reposting them
    /// - `handlers::bookmarks` - What the account has bookmarked, and bookmarking a note
    /// - `handlers::zap` - Zapping a note: the invoice, and its receipt
    /// - `handlers::images` - Fetching the pictures views ask for
    /// - `handlers::previews` - Asking the pages notes link to what they say of themselves
    /// - `handlers::video` - Playing a video, and finding what there is to play of a YouTube one
    /// - `handlers::share` - Copying a link to a note that anyone can open
    /// - `handlers::settings` - The settings: how large notes are set, how long they are kept, how much is logged
    /// - `handlers::store` - Having the event store let go of what is old
    /// - `handlers::notifications` - What others did that concerns the account
    ///
    /// Messages are not logged wholesale: several carry a page of notes, and
    /// one carries what is typed in the sign-in field. Handlers log what
    /// happened instead.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        let task = self.handle(message);
        // Whatever the message did, the notifications may have come into
        // view by it, or gone out of it.
        self.watch_notifications();
        // The button in the panel shows whether there are any not looked
        // at: it is told when that changes.
        let unread = self.unread_notifications() > 0;
        if unread != self.unread_told {
            self.unread_told = unread;
            button::tell_unread(unread);
        }
        task
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    /// Do what `message` asks: the one place every message goes through.
    fn handle(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            // Navigation handlers
            Message::TogglePopup => self.handle_toggle_popup(),
            Message::StepNote(step) => self.handle_step_note(step),
            Message::OpenMarked => self.handle_open_marked(),
            Message::ShareMarked => self.handle_share_marked(),
            Message::PlayMarked => self.handle_play_marked(),
            Message::ReplyMarked => self.handle_reply_marked(),
            Message::RepostMarked => self.handle_repost_marked(),
            Message::StepChoice(step) => self.handle_step_choice(step),
            Message::ZapMarked => self.handle_zap_marked(),
            Message::BookmarkMarked => self.handle_bookmark_marked(),
            Message::GoToBookmarks => self.handle_go_to_bookmarks(),
            Message::ShowKeys => self.handle_show_keys(),
            Message::ShowWaiting => self.handle_show_waiting(),
            Message::GoBack => self.handle_go_back(),
            Message::Backspace { held } => self.handle_backspace(held),
            Message::PopupClosed(id) => self.handle_popup_closed(id),
            Message::PopupUnfocused(id) => self.handle_popup_unfocused(id),
            Message::ButtonGone => cosmic::iced::exit(),
            Message::ScreenMeasured(screen) => {
                self.handle_screen_measured(screen);
                Task::none()
            }
            Message::ClosePage => self.handle_close_page(),
            Message::ToggleMenu => {
                self.menu_open = !self.menu_open;
                Task::none()
            }
            Message::CloseMenu => {
                self.menu_open = false;
                Task::none()
            }
            Message::OpenSettings => {
                self.handle_open_settings();
                Task::none()
            }
            Message::CloseSettings | Message::CloseKeys => self.show_list_view(),
            Message::OpenKeys => {
                self.handle_open_keys();
                Task::none()
            }

            // Search handlers
            Message::OpenSearch => self.handle_open_search(),
            Message::SearchTyped(page, typed) => {
                self.handle_search_typed(page, typed);
                Task::none()
            }
            Message::Search(page) => self.handle_search(page),
            Message::SearchShow(page, what) => self.handle_search_show(page, what),
            Message::SearchFound { page, serial, what, result } => self.handle_search_found(page, serial, what, result),

            Message::SearchNamed { page, name, person } => self.handle_search_named(page, &name, person),
            Message::RelayInfoLoaded { page, serial, info } => {
                self.handle_relay_info_loaded(page, serial, info);
                Task::none()
            }

            // Relays handlers
            Message::OpenRelays => self.handle_open_relays(),
            Message::CloseRelays => self.show_list_view(),
            Message::RelayListsLoaded { account, source, result } => self.handle_relay_lists_loaded(account, source, result),
            Message::RelayUseLoaded(in_use) => {
                self.relays.in_use = in_use;
                Task::none()
            }
            Message::RelaySaid { relay, info } => {
                self.relays.said.insert(relay, info);
                Task::none()
            }
            Message::RelayTyped(section, typed) => {
                self.relays.typed.insert(section, typed);
                Task::none()
            }
            Message::AddRelay(section) => self.handle_add_relay(section),
            Message::RemoveRelay(section, relay) => {
                self.handle_remove_relay(section, &relay);
                Task::none()
            }
            Message::ShiftRelay(section, relay, up) => {
                self.handle_shift_relay(section, &relay, up);
                Task::none()
            }
            Message::SaveRelays => self.handle_save_relays(),
            Message::RelaysSaved { account, advertised, listed } => {
                self.handle_relays_saved(account, advertised, listed);
                Task::none()
            }
            // Media handlers
            Message::OpenMedia => self.handle_open_media(),
            Message::CloseMedia => self.handle_close_media(),
            Message::MediaServersLoaded { account, source, result } => self.handle_media_servers_loaded(account, source, result),
            Message::MediaServerTyped(typed) => {
                self.media.typed = typed;
                Task::none()
            }
            Message::AddMediaServer => {
                self.handle_add_media_server();
                Task::none()
            }
            Message::RemoveMediaServer(server) => {
                self.handle_remove_media_server(&server);
                Task::none()
            }
            Message::ShiftMediaServer(server, up) => {
                self.handle_shift_media_server(&server, up);
                Task::none()
            }
            Message::SaveMediaServers => self.handle_save_media_servers(),
            Message::MediaServersSaved { account, result } => {
                self.handle_media_servers_saved(account, result);
                Task::none()
            }
            Message::AttachMedia => self.handle_attach_media(),
            Message::MediaPicked(picked) => self.handle_media_picked(picked),
            Message::ComposePaste => self.handle_compose_paste(),
            Message::ComposePasted(pasted) => self.handle_compose_pasted(pasted),
            Message::MediaUploaded { id, result } => self.handle_media_uploaded(id, result),
            Message::CancelUpload(id) => {
                self.handle_cancel_upload(id);
                Task::none()
            }
            // Nothing changed but how far they have got, which is read
            // when the view is drawn.
            Message::UploadTick => Task::none(),
            Message::ClearError => {
                self.error_message = None;
                Task::none()
            }
            // Image handlers
            Message::CopyPicture(url) => self.handle_copy_picture(url),
            Message::LoadImage(request) => self.handle_load_image(request),
            Message::ImageLoaded(request, result) => {
                self.handle_image_loaded(request, result);
                Task::none()
            }
            // Link preview handlers
            Message::LoadPreview(url) => self.handle_load_preview(url),
            Message::PreviewLoaded(url, result) => {
                self.handle_preview_loaded(url, result);
                Task::none()
            }

            // Videos: the one that plays is the model's, and all there is
            // to do is tell it.
            Message::VideoPress(url) => self.handle_video_press(url),
            Message::VideoFound(url, result) => {
                self.handle_video_found(&url, result);
                Task::none()
            }
            Message::VideoMute => {
                self.videos.toggle_mute();
                Task::none()
            }
            Message::VideoVolume(shown) => {
                self.videos.set_volume(shown);
                Task::none()
            }
            Message::VideoVolumeSet => {
                self.videos.volume_set();
                Task::none()
            }
            Message::VideoSound(over) => {
                self.videos.sound_over(over);
                Task::none()
            }
            Message::VideoSpeedMenu => {
                self.videos.toggle_speed_menu();
                Task::none()
            }
            Message::CloseVideoSpeedMenu => {
                self.videos.close_speed_menu();
                Task::none()
            }
            Message::VideoSpeed(speed) => {
                self.videos.set_speed(speed);
                Task::none()
            }
            Message::VideoSeeking(seconds) => {
                self.videos.seeking(seconds);
                Task::none()
            }
            Message::VideoSeek => {
                self.videos.seek();
                Task::none()
            }
            Message::VideoTick => {
                self.videos.tick();
                Task::none()
            }

            Message::CopyShareLink(id) => self.handle_copy_share_link(id),

            Message::OpenNotifications => self.handle_open_notifications(),
            Message::NotificationsLoaded { account, source, older, result } => {
                self.handle_notifications_loaded(account, source, older, result)
            }
            Message::LoadOlderNotifications => self.handle_load_older_notifications(),
            Message::SelectNotificationsTab(chosen) => self.handle_select_notifications_tab(chosen),
            Message::ZapperKnown(account, result) => {
                self.handle_zapper_known(account, result);
                Task::none()
            }

            Message::ExpandNote(id) => {
                self.handle_expand_note(id);
                Task::none()
            }
            Message::CollapseNote(id) => self.handle_collapse_note(id),

            Message::OpenUrl(url) => {
                self.handle_open_url(&url);
                Task::none()
            }
            Message::UpdateConfig(config) => self.handle_update_config(config),
            Message::SetLogLevel(level) => {
                self.handle_set_log_level(level);
                Task::none()
            }
            Message::SetNoteTextSize(size) => {
                self.handle_set_note_text_size(size);
                Task::none()
            }
            Message::SetKeepDays(days) => self.handle_set_keep_days(days),
            Message::SweepStore => self.sweep_store(),

            // Auth handlers
            Message::LoginInputChanged(input) => {
                self.handle_login_input_changed(input);
                Task::none()
            }
            Message::ToggleLoginInputVisibility => {
                self.login_input_hidden = !self.login_input_hidden;
                Task::none()
            }
            Message::SubmitLogin => self.handle_submit_login(),
            Message::CancelLogin => {
                self.signer.cancel_sign_in();
                Task::none()
            }
            Message::CopyClientKey => self.handle_copy_client_key(),
            Message::LoginComplete(result) => self.handle_login_complete(result),
            Message::AccountRestored(result) => self.handle_account_restored(result),
            Message::SignOut => self.handle_sign_out(),
            Message::SignedOut(result) => {
                self.handle_signed_out(result);
                Task::none()
            }

            // Feed handlers
            Message::NostrReady(result) => self.handle_nostr_ready(result),
            Message::FeedCached(id, result) => self.handle_feed_cached(id, result),
            Message::RefreshFeed => self.handle_refresh_feed(),
            Message::FeedFetched { list, page, until, result } => self.handle_feed_fetched(list, page, until, result),
            Message::CheckForNewNotes => self.handle_check_for_new_notes(),
            Message::ShowNewNotes(id) => self.show_new_notes(id),
            Message::LoadOlderNotes(id) => self.handle_load_older_notes(id),
            Message::SelectFeedMode(chosen) => self.handle_select_feed_mode(chosen),
            Message::ContactsLoaded { feed, source, result } => self.handle_contacts_loaded(feed, source, result),
            Message::ProfilesCached { requested, found } => self.handle_profiles_cached(requested, found),
            Message::ProfilesFetched(found) => self.handle_profiles_fetched(found),

            // Thread handlers
            Message::OpenThread(id, whereabouts) => self.handle_open_thread(id, whereabouts),
            Message::ThreadLoaded { focused, source, result } => self.handle_thread_loaded(focused, source, result),

            // Profile handlers
            Message::OpenProfile(public_key) => self.handle_open_profile(public_key),
            Message::CopyNpub(public_key) => self.handle_copy_npub(public_key),
            Message::Nip05Checked { person, name, verified } => {
                self.handle_nip05_checked(person, &name, verified);
                Task::none()
            }
            Message::Follow { person, follow, first_list } => self.handle_follow(person, follow, first_list),
            Message::FollowChanged { account, person, result } => self.handle_follow_changed(account, person, result),

            // Engagement handlers
            Message::EngagementLoaded { source, account, result } => {
                self.handle_engagement_loaded(source, account, result);
                Task::none()
            }
            Message::ToggleReactionMenu(id) => {
                self.handle_toggle_reaction_menu(id);
                Task::none()
            }
            Message::CloseReactionMenu => {
                self.reaction_menu = None;
                Task::none()
            }
            Message::OpenEmojiPicker => self.handle_open_emoji_picker(),
            Message::EmojiSearchTyped(typed) => {
                self.handle_emoji_search_typed(typed);
                Task::none()
            }
            Message::EmojiGroup(group) => {
                self.handle_emoji_group(group);
                Task::none()
            }
            Message::React(id, reaction) => self.handle_react(id, reaction),
            Message::ToggleRepostMenu(id) => {
                self.handle_toggle_repost_menu(id);
                Task::none()
            }
            Message::CloseRepostMenu => {
                self.repost_menu = None;
                Task::none()
            }
            Message::Repost(id) => self.handle_mark(id, Mark::Repost),
            Message::ShowWho(note, what) => self.handle_show_who(note, what),
            Message::WhoLoaded { note, what, result } => self.handle_who_loaded(note, what, result),
            Message::CloseWho => {
                self.who_menu = None;
                Task::none()
            }
            Message::Marked { note, mark, result } => {
                self.handle_marked(note, mark, result);
                Task::none()
            }

            // Bookmark handlers
            Message::BookmarksLoaded { account, source, result } => self.handle_bookmarks_loaded(account, source, result),
            Message::Bookmark(id) => self.handle_bookmark(id),
            Message::Bookmarked { account, note, result } => {
                self.handle_bookmarked(account, note, result);
                Task::none()
            }
            Message::OpenBookmarks => self.handle_open_bookmarks(),
            Message::LoadBookmarkedNotes(page) => self.handle_load_bookmarked_notes(page),
            Message::BookmarkedNotesLoaded { page, taken, from, to, notes } => {
                self.handle_bookmarked_notes_loaded(page, taken, from, to, notes)
            }

            // Zap handlers
            Message::ZapNote(id) => self.handle_zap_note(id),
            Message::ZapAmountChanged(amount) => {
                self.handle_zap_amount_changed(&amount);
                Task::none()
            }
            Message::ZapCommentChanged(comment) => {
                self.handle_zap_comment_changed(&comment);
                Task::none()
            }
            Message::RequestZapInvoice => self.handle_request_zap_invoice(),
            Message::ZapInvoiceReady { note, result } => self.handle_zap_invoice_ready(note, result),
            Message::CopyZapInvoice => self.handle_copy_zap_invoice(),
            Message::ZapReceiptAwaited { note, paid } => self.handle_zap_receipt_awaited(note, paid),
            Message::CloseZap => self.handle_close_zap(),

            // Composer handlers
            Message::DraftKept { account, target, text, result } => self.handle_draft_kept(account, target, text, result),
            Message::KeptDraftsLoaded { account, result } => {
                self.handle_kept_drafts_loaded(account, result);
                Task::none()
            }
            Message::DraftLoaded(text) => {
                self.handle_draft_loaded(text);
                Task::none()
            }
            Message::ComposeNote => self.handle_compose_note(),
            Message::ComposeReply(id) => self.handle_compose_reply(id),
            Message::ComposeQuote(id) => self.handle_compose_quote(id),
            Message::ComposeEdited(action) => {
                self.handle_compose_edited(action);
                Task::none()
            }
            Message::CloseComposer => self.handle_close_composer(),
            Message::ComposerRubbedOut => self.handle_composer_rubbed_out(),
            Message::DiscardDraft => self.handle_discard_draft(),
            Message::ResumeComposer => self.handle_resume_composer(),
            Message::Publish => self.handle_publish(),
            Message::Published { target, result } => self.handle_published(target, result),
        }
    }
}

/// What is said to the window from outside, as messages: a keyboard
/// shortcut of the desktop's toggles the popup as the button does.
fn from_outside() -> impl cosmic::iced::futures::Stream<Item = Message> {
    button::from_outside(button::socket_path(), Message::TogglePopup)
}

/// What the button in the panel says, as messages.
fn from_button() -> impl cosmic::iced::futures::Stream<Item = Message> {
    button::from_button(Message::TogglePopup, Message::ButtonGone)
}

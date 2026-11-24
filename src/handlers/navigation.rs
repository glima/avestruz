// SPDX-License-Identifier: MIT

//! Navigation message handlers for Avestruz.
//!
//! This module handles the popup's lifecycle, how tall it may be, and
//! which view it shows.
//!
//! The popup may be as tall as the screen it opens on has room: all of
//! it, but what the panel keeps for itself and a gap at either end. A
//! view with a long list in it is that tall, and one with little to show
//! is as tall as what it shows. How much room there is is asked when the
//! window starts and again each time the popup opens (see
//! [`crate::screen`]): the popup opens at once with the last answer, and
//! takes the new one when it comes, if it differs.
//!
//! Under everything is the feed. Threads and profiles open over it as
//! pages, one from another, and stack up; Back closes the last. Each keeps
//! what it shows and its place, so going back is to where one was. The
//! composer, the zap view and the settings show in place of whatever list
//! was on show, and leave it as it was.

use cosmic::iced::advanced::widget::operation::Scrollable;
use cosmic::iced::advanced::widget::{self, Operation};
use std::time::{Duration, Instant};

use cosmic::applet::cosmic_panel_config::{CosmicPanelConfig, PanelAnchor};
use cosmic::cosmic_config::ConfigGet;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{IcedMargin, SctkLayerSurfaceSettings};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface,
};
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::iced::window::{self, Id};
use cosmic::iced::{Limits, Rectangle, Vector};
use cosmic::prelude::*;
use nostr_sdk::prelude::EventId;

use crate::fl;
use crate::handlers::relays::ALWAYS_READ;
use crate::images::SCALE_UNKNOWN;
use crate::links::{self, OpenError};
use crate::messages::Message;
use crate::screen::{self, Screen};
use crate::state::{AppModel, BookmarksPage, Expanded, Page, ProfileState, SearchPage, ThreadState, ViewState};
use crate::views::components::{
    LIST_SCROLL_ID, POPUP_MAX_HEIGHT_UNMEASURED, POPUP_MAX_WIDTH, POPUP_MIN_HEIGHT, POPUP_MIN_WIDTH, RowHeights,
};

/// How many pages can be open one over another. Opening one more closes
/// the first: each holds a list of notes, and the applet runs for as long
/// as the session does.
pub(crate) const MAX_OPEN_PAGES: usize = 16;

/// The name the compositor knows the popup's surface by.
const POPUP_NAMESPACE: &str = "avestruz";

/// What the compositor knows the surface that keeps what is copied by.
const KEEPER_NAMESPACE: &str = "avestruz-clipboard";

/// The room between the popup and the edges of the screen, and the panel.
const POPUP_GAP: i32 = 8;

/// How long after the popup closed for a click outside it a press of the
/// panel button is taken to be that click: the button is outside it.
const SAME_PRESS: Duration = Duration::from_millis(400);

/// Which part of the panel an applet sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wing {
    Start,
    Center,
    End,
}

/// Which part of its panel the button is in, as the panel's settings have
/// it: the end, where no panel says (run by hand, there is none).
fn panel_wing() -> Wing {
    let found = || {
        let panel = std::env::var("COSMIC_PANEL_NAME").ok()?;
        let config = CosmicPanelConfig::cosmic_config(&panel).ok()?;
        let wings: Option<(Vec<String>, Vec<String>)> = config.get("plugins_wings").ok()?;
        let center: Option<Vec<String>> = config.get("plugins_center").ok()?;
        wing_of(crate::app::APP_ID, wings.as_ref(), center.as_deref())
    };
    found().unwrap_or(Wing::End)
}

/// Which part of a panel the applet `applet` is in, by the lists of what
/// is at its two ends and in its middle. `None` if it is in none of them.
fn wing_of(applet: &str, wings: Option<&(Vec<String>, Vec<String>)>, center: Option<&[String]>) -> Option<Wing> {
    let among = |applets: &[String]| applets.iter().any(|named| named == applet);
    match wings {
        Some((start, _)) if among(start) => Some(Wing::Start),
        Some((_, end)) if among(end) => Some(Wing::End),
        _ if center.is_some_and(among) => Some(Wing::Center),
        _ => None,
    }
}

/// Where on its screen the popup is put: against the edge the panel is on,
/// and along that edge at the end the button is in, or in the middle if
/// the button is. Under the button, as near as can be told: the window
/// is told which panel and which part of it, not where in it.
fn popup_anchor(panel: PanelAnchor, wing: Wing) -> Anchor {
    let (edge, start, end) = match panel {
        PanelAnchor::Top => (Anchor::TOP, Anchor::LEFT, Anchor::RIGHT),
        PanelAnchor::Bottom => (Anchor::BOTTOM, Anchor::LEFT, Anchor::RIGHT),
        PanelAnchor::Left => (Anchor::LEFT, Anchor::TOP, Anchor::BOTTOM),
        PanelAnchor::Right => (Anchor::RIGHT, Anchor::TOP, Anchor::BOTTOM),
    };
    match wing {
        Wing::Start => edge | start,
        Wing::Center => edge,
        Wing::End => edge | end,
    }
}

/// How tall the popup may be on a screen that has `room` for it, if that
/// is known: all of it but the gap at either end, in whole points, and
/// what fits any screen if it isn't.
fn max_height_on(room: Option<f32>) -> f32 {
    match room {
        Some(height) => (height - 2.0 * POPUP_GAP as f32).floor().max(POPUP_MIN_HEIGHT),
        None => POPUP_MAX_HEIGHT_UNMEASURED,
    }
}

/// How long the surface that says what the screen is like is given to be
/// laid out, when the window starts, before it is asked.
const LAID_OUT_WITHIN: Duration = Duration::from_millis(500);

/// Ask the surface `keeper`, which reaches from the top of its screen to
/// the bottom, what the screen is like: how tall the compositor made it,
/// and the scale it is drawn at (see [`crate::screen`]).
fn measure(keeper: Id) -> Task<cosmic::Action<Message>> {
    window::size(keeper).then(move |size| {
        window::scale_factor(keeper)
            .map(move |scale| cosmic::Action::App(Message::ScreenMeasured(screen::measured(size.height, scale))))
    })
}

/// Scroll the list of the view on show to `y` from its top.
pub(crate) fn scroll_list_to(y: f32) -> Task<cosmic::Action<Message>> {
    scroll_to(LIST_SCROLL_ID.clone(), AbsoluteOffset { x: None, y: Some(y) })
}

/// Scroll the list of the view on show to `y` from its top, or as far as
/// it goes, if that is less.
///
/// A scroller told to go past its end shows its end and keeps the place
/// it was told, to go there when what it holds has grown that long. That
/// suits a list that is still filling. It does not suit one that just
/// got shorter under its reader, which would move by itself later; this
/// leaves the scroller where it shows.
pub(crate) fn scroll_list_within(y: f32) -> Task<cosmic::Action<Message>> {
    struct Within {
        y: f32,
    }

    impl<T> Operation<T> for Within {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&widget::Id>,
            bounds: Rectangle,
            content_bounds: Rectangle,
            _translation: Vector,
            state: &mut dyn Scrollable,
        ) {
            if id == Some(&*LIST_SCROLL_ID) {
                let furthest = (content_bounds.height - bounds.height).max(0.0);
                state.scroll_to(AbsoluteOffset { x: None, y: Some(self.y.min(furthest)) });
            }
        }
    }

    cosmic::iced::advanced::widget::operate(Within { y })
}

impl AppModel {
    /// How tall the popup may be: as tall as its screen has room.
    pub(crate) fn popup_max_height(&self) -> f32 {
        max_height_on(self.screen.map(|screen| screen.height))
    }

    /// How many pixels a point is on the screen the popup opens on: what
    /// pictures are made for. Until the screen has said, more than on
    /// most, so that what is made meanwhile is sharp on any.
    pub(crate) fn screen_scale(&self) -> f32 {
        self.screen.map_or(SCALE_UNKNOWN, |screen| screen.scale)
    }

    /// Ask what the screen is like that the popup opens on.
    pub(crate) fn measure_screen(&self) -> Task<cosmic::Action<Message>> {
        measure(self.keeper)
    }

    /// Ask what the screen is like once the surface that says so has had
    /// the time to be laid out: when the window starts.
    pub(crate) fn measure_screen_soon(&self) -> Task<cosmic::Action<Message>> {
        let keeper = self.keeper;
        Task::future(tokio::time::sleep(LAID_OUT_WITHIN)).then(move |()| measure(keeper))
    }

    /// What the screen is like was found out, or wasn't: the last answer
    /// stands until there is another.
    pub fn handle_screen_measured(&mut self, screen: Option<Screen>) {
        if let Some(screen) = screen {
            if self.screen != Some(screen) {
                tracing::debug!("the screen has {:.0} points for the popup, {} pixels to the point", screen.height, screen.scale);
            }
            self.screen = Some(screen);
        }
    }

    /// Open the surface that keeps what is copied, and says what the
    /// screen is like: a point wide, down the edge of the screen from top
    /// to bottom, under every window, that shows nothing and takes
    /// neither the keyboard nor the pointer.
    ///
    /// What is copied is the copier's to hand over when it is pasted, and
    /// the toolkit keeps it with the first surface it opened, for as long
    /// as that is there. The popup is gone at the next click anywhere
    /// else, which is where something copied is on its way to: copied
    /// from the popup alone, a link was not there to paste. So a surface
    /// is opened before any popup, and stays.
    ///
    /// Being there, it is also what is asked how much room the screen
    /// has for the popup, and at what scale it is drawn (see
    /// [`crate::screen`]): it is as tall as the compositor makes a
    /// surface that reaches from the top of the screen to the bottom.
    pub(crate) fn keep_what_is_copied(&self) -> Task<cosmic::Action<Message>> {
        get_layer_surface(SctkLayerSurfaceSettings {
            id: self.keeper,
            layer: Layer::Background,
            keyboard_interactivity: KeyboardInteractivity::None,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT,
            namespace: KEEPER_NAMESPACE.into(),
            // As tall as the compositor makes it, between the two edges.
            size: Some((Some(1), None)),
            // No part of it takes a click.
            input_zone: Some(Vec::new()),
            exclusive_zone: 0,
            ..SctkLayerSurfaceSettings::default()
        })
    }

    /// Handle popup toggle: open it by the panel, or close it.
    ///
    /// The popup is a surface of the window's own on the compositor, not
    /// the panel's (see [`crate::button`]), so it is not the panel that
    /// places it under the button: it is put against the edge the panel
    /// is on, at the end of it the button is nearer (see [`popup_anchor`]).
    /// It has the keyboard while it is open, as a popup of the panel's
    /// has.
    pub fn handle_toggle_popup(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(p) = self.popup.take() {
            return Task::batch([destroy_layer_surface(p), self.popup_left()]);
        }
        // The press that asks for it now is the click that closed it a
        // moment ago: the popup was open, and the button was pressed to
        // close it.
        if self.popup_unfocused_at.take().is_some_and(|at| at.elapsed() < SAME_PRESS) {
            return Task::none();
        }
        let new_id = Id::unique();
        self.popup.replace(new_id);
        let anchor = popup_anchor(self.core.applet.anchor, panel_wing());
        let settings = SctkLayerSurfaceSettings {
            id: new_id,
            layer: Layer::Top,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            anchor,
            namespace: POPUP_NAMESPACE.into(),
            margin: IcedMargin { top: POPUP_GAP, right: POPUP_GAP, bottom: POPUP_GAP, left: POPUP_GAP },
            // As large as what is in it, within its limits.
            size: None,
            // Beside whatever keeps room for itself at an edge, as the
            // panel does: not over it.
            exclusive_zone: 0,
            size_limits: Limits::NONE
                .min_width(POPUP_MIN_WIDTH)
                .max_width(POPUP_MAX_WIDTH)
                .min_height(POPUP_MIN_HEIGHT)
                .max_height(self.popup_max_height()),
            ..SctkLayerSurfaceSettings::default()
        };
        // It opens on what it showed when it closed, at the same place
        // (see `list_scroller`). What is new since is looked for, and
        // waits to be asked for; what was bookmarked elsewhere since is
        // looked for too. The screen may not be the one it was.
        Task::batch([
            get_layer_surface(settings),
            self.check_for_new_notes_if_stale(),
            self.load_bookmarks(),
            self.load_relay_lists(&ALWAYS_READ),
            self.load_media_servers(),
            self.load_drafts(),
            self.measure_screen(),
        ])
    }

    /// The popup lost the keyboard, which it has while it is open:
    /// something outside it was clicked. It closes, as a popup of the
    /// panel's does, with everything in it kept as it is.
    pub fn handle_popup_unfocused(&mut self, id: Id) -> Task<cosmic::Action<Message>> {
        if self.popup != Some(id) {
            return Task::none();
        }
        self.popup = None;
        self.popup_unfocused_at = Some(Instant::now());
        Task::batch([destroy_layer_surface(id), self.popup_left()])
    }

    /// Escape was pressed: put away what was opened last. A menu open
    /// over the list, if one is; the popup itself otherwise, as a click
    /// outside it would, with everything in it kept as it is.
    pub fn handle_escape(&mut self) -> Task<cosmic::Action<Message>> {
        let who = self.who_menu.take().is_some();
        if who || self.repost_menu.is_some() || self.reaction_menu.is_some() || self.menu_open || self.videos.speed_menu() {
            self.repost_menu = None;
            self.reaction_menu = None;
            self.menu_open = false;
            self.videos.close_speed_menu();
            Task::none()
        } else if self.view_state == ViewState::Compose && self.composer.as_ref().is_some_and(|composer| composer.leaving) {
            // Asked whether what was written is kept: neither, it is
            // gone on with.
            self.handle_resume_composer()
        } else if self.popup.is_some() {
            self.handle_toggle_popup()
        } else {
            Task::none()
        }
    }

    /// Handle popup closed event. What is in the composer is kept: the
    /// popup closes whenever something else is clicked, and may not be opened
    /// again before the session ends.
    pub fn handle_popup_closed(&mut self, id: Id) -> Task<cosmic::Action<Message>> {
        if self.popup.as_ref() == Some(&id) {
            self.popup = None;
        }
        self.popup_left()
    }

    /// The popup closed, either way. What is in the composer is kept; a
    /// menu left open is not there to be found when the popup opens again;
    /// and a video that played is held where it was.
    fn popup_left(&mut self) -> Task<cosmic::Action<Message>> {
        self.repost_menu = None;
        self.reaction_menu = None;
        self.who_menu = None;
        self.menu_open = false;
        self.share_copied = None;
        self.picture_copied = None;
        self.videos.pause();
        self.keep_draft()
    }

    /// The thread on show, or under what is: the page opened last, if it
    /// is one.
    pub(crate) fn thread_on_top(&self) -> Option<&ThreadState> {
        match self.pages.last() {
            Some(Page::Thread(state)) => Some(state.as_ref()),
            _ => None,
        }
    }

    /// The profile on show, or under what is: the page opened last, if it
    /// is one.
    pub(crate) fn profile_on_top(&self) -> Option<&ProfileState> {
        match self.pages.last() {
            Some(Page::Profile(state)) => Some(state.as_ref()),
            _ => None,
        }
    }

    /// The bookmarks page on show, or under what is: the page opened
    /// last, if it is one.
    pub(crate) fn bookmarks_on_top(&self) -> Option<&BookmarksPage> {
        match self.pages.last() {
            Some(Page::Bookmarks(page)) => Some(page.as_ref()),
            _ => None,
        }
    }

    /// The search on show, if a search is the page on top.
    pub(crate) fn search_on_top(&self) -> Option<&SearchPage> {
        match self.pages.last() {
            Some(Page::Search(page)) => Some(page.as_ref()),
            _ => None,
        }
    }

    /// Every open thread, on show or under other pages.
    pub(crate) fn threads_mut(&mut self) -> impl Iterator<Item = &mut ThreadState> {
        self.pages.iter_mut().filter_map(|page| match page {
            Page::Thread(state) => Some(state.as_mut()),
            Page::Profile(_) | Page::Bookmarks(_) | Page::Search(_) | Page::Notifications(_) => None,
        })
    }

    /// Every open profile, on show or under other pages.
    pub(crate) fn profiles_mut(&mut self) -> impl Iterator<Item = &mut ProfileState> {
        self.pages.iter_mut().filter_map(|page| match page {
            Page::Profile(state) => Some(state.as_mut()),
            Page::Thread(_) | Page::Bookmarks(_) | Page::Search(_) | Page::Notifications(_) => None,
        })
    }

    /// Show `page` over whatever is on show now, from its top: the scroller
    /// has the place of the list it was opened from.
    pub(crate) fn open_page(&mut self, page: Page) -> Task<cosmic::Action<Message>> {
        if self.pages.len() >= MAX_OPEN_PAGES {
            self.pages.remove(0);
        }
        self.pages.push(page);
        self.view_state = ViewState::Page;
        // One of those who left something on a note may have been pressed
        // to get here: the list of them is not there to come back to.
        self.who_menu = None;
        scroll_list_to(0.0)
    }

    /// Close the page on show: back to the page it was opened from, or to
    /// the feed, where that was left.
    pub fn handle_close_page(&mut self) -> Task<cosmic::Action<Message>> {
        self.pages.pop();
        self.show_list_view()
    }

    /// Show the settings, in place of the list on show.
    pub fn handle_open_settings(&mut self) {
        self.menu_open = false;
        self.view_state = ViewState::Settings;
    }

    /// Show what the keys do, in place of the list on show.
    pub fn handle_open_keys(&mut self) {
        self.menu_open = false;
        self.view_state = ViewState::Keys;
    }

    /// What the list of notes on show keeps of its rows: which of its
    /// long notes show in full, how tall each row measured, and how far
    /// it is scrolled. The list of the page opened last, or else the
    /// feed's.
    fn rows_on_show(&mut self) -> (&mut Expanded, &RowHeights<EventId>, f32) {
        let page = match self.view_state {
            ViewState::Page => self.pages.last_mut(),
            _ => None,
        };
        let (expanded, heights, position) = match page {
            Some(Page::Thread(state)) => (&mut state.expanded, &state.heights, &state.position),
            Some(Page::Profile(state)) => (&mut state.notes.expanded, &state.notes.heights, &state.notes.position),
            Some(Page::Bookmarks(page)) => (&mut page.notes.expanded, &page.notes.heights, &page.notes.position),
            Some(Page::Search(page)) => (&mut page.expanded, &page.heights, &page.notes_position),
            Some(Page::Notifications(page)) => (&mut page.expanded, &page.heights, &page.position),
            None => (&mut self.following.expanded, &self.following.heights, &self.following.position),
        };
        (expanded, heights, position.get())
    }

    /// Show all the note `id` says, in its row in the list on show.
    /// Nothing is scrolled: the row grows down from where it is, and
    /// what is under it makes room.
    pub fn handle_expand_note(&mut self, id: EventId) {
        let (expanded, heights, _) = self.rows_on_show();
        expanded.expand(id);
        heights.measure_anew(id);
    }

    /// Show the note `id` cut again, in the row that showed all of it.
    ///
    /// A row that shrinks leaves what is in view further down the list,
    /// and if its top was above what is in view, the note itself out of
    /// sight: the list is scrolled back to the top of the row, so that
    /// what its reader sees is the note they folded. With its top in
    /// view, nothing moves but what is under it.
    ///
    /// Either way the list is told where it is, within what is left of
    /// it. Its scroller would otherwise keep a place past its new end,
    /// and go there by itself when the list next grew.
    pub fn handle_collapse_note(&mut self, id: EventId) -> Task<cosmic::Action<Message>> {
        let (expanded, heights, position) = self.rows_on_show();
        let Some(above) = expanded.collapse(&id) else {
            return Task::none();
        };
        heights.measure_anew(id);
        scroll_list_within((position - above).max(0.0))
    }

    /// Show the list that is under whatever was opened over it: the page
    /// opened last, or else the feed, each where it was left.
    pub(crate) fn show_list_view(&mut self) -> Task<cosmic::Action<Message>> {
        match self.pages.last() {
            Some(page) => {
                self.view_state = ViewState::Page;
                scroll_list_to(page.position())
            }
            None => {
                self.view_state = ViewState::Feed;
                self.restore_feed_position()
            }
        }
    }

    /// Open a link from a note in the system's browser, if it is a web
    /// link (see [`crate::links`]).
    pub fn handle_open_url(&mut self, url: &str) {
        match links::open_in_browser(url) {
            Ok(()) => {}
            Err(OpenError::NotWeb) => tracing::warn!("refused to open a link that is not a web address"),
            Err(OpenError::Failed(e)) => {
                tracing::error!("the link could not be opened: {e}");
                self.error_message = Some(fl!("error-open-link", error = e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Wing, max_height_on, popup_anchor, wing_of};
    use cosmic::applet::cosmic_panel_config::PanelAnchor;
    use cosmic::iced::platform_specific::shell::commands::layer_surface::Anchor;

    #[test]
    fn the_popup_is_put_by_the_panels_edge_at_the_end_the_button_is_in() {
        // A panel along the top: under it, at the left, in the middle or
        // at the right, as the button is.
        assert_eq!(popup_anchor(PanelAnchor::Top, Wing::End), Anchor::TOP | Anchor::RIGHT);
        assert_eq!(popup_anchor(PanelAnchor::Top, Wing::Start), Anchor::TOP | Anchor::LEFT);
        assert_eq!(popup_anchor(PanelAnchor::Top, Wing::Center), Anchor::TOP);
        assert_eq!(popup_anchor(PanelAnchor::Bottom, Wing::Start), Anchor::BOTTOM | Anchor::LEFT);
        // A panel down a side: beside it, at its top, middle or bottom.
        assert_eq!(popup_anchor(PanelAnchor::Left, Wing::Start), Anchor::LEFT | Anchor::TOP);
        assert_eq!(popup_anchor(PanelAnchor::Right, Wing::End), Anchor::RIGHT | Anchor::BOTTOM);
        assert_eq!(popup_anchor(PanelAnchor::Right, Wing::Center), Anchor::RIGHT);
    }

    #[test]
    fn the_button_is_in_the_part_of_the_panel_that_lists_it() {
        let named = |names: &[&str]| names.iter().map(|name| (*name).to_string()).collect::<Vec<_>>();
        let wings = (named(&["workspaces", "us"]), named(&["clock", "them"]));
        let center = named(&["middle"]);
        let wing = |applet| wing_of(applet, Some(&wings), Some(&center));
        assert_eq!(wing("us"), Some(Wing::Start));
        assert_eq!(wing("them"), Some(Wing::End));
        assert_eq!(wing("middle"), Some(Wing::Center));
        // In none of them, or in a panel that lists nothing.
        assert_eq!(wing("nobody"), None);
        assert_eq!(wing_of("us", None, None), None);
    }

    #[test]
    fn the_popup_may_be_as_tall_as_its_screen_has_room_less_a_gap_at_either_end() {
        // 2160 pixels at 170% under a panel of 40 points, at 100%, and a
        // small laptop's 1080 at 125%.
        assert_eq!(max_height_on(Some(2160.0 / 1.7 - 40.0)), 1_214.0);
        assert_eq!(max_height_on(Some(2120.0)), 2_104.0);
        assert_eq!(max_height_on(Some(824.0)), 808.0);
    }

    #[test]
    fn a_screen_not_measured_yet_gets_a_popup_that_fits_any() {
        assert_eq!(max_height_on(None), 600.0);
    }

    #[test]
    fn a_very_short_screen_still_gets_the_least_a_popup_asks_for() {
        assert_eq!(max_height_on(Some(310.0)), 300.0);
        assert_eq!(max_height_on(Some(120.0)), 300.0);
    }
}

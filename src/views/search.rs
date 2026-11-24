// SPDX-License-Identifier: MIT

//! The search view for Avestruz: where the search is typed, above the
//! people it found or the notes, each a list of its own.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length, mouse};
use cosmic::widget::{self, button, container, text, text_input};
use nostr_sdk::prelude::PublicKey;

use crate::fl;
use crate::helpers::truncate_chars;
use crate::messages::Message;
use crate::nostr::models::{Note, display_name, short_npub};
use crate::nostr::relay_lists::NIP_SEARCH;
use crate::nostr::search::SearchFor;
use crate::state::{AppModel, SearchPage, SearchProgress};
use crate::views::components::note_row::relay_name;
use crate::views::components::{
    VirtualList, avatar, estimated_note_height, list_scroller, marking, note_row, opening_thread, status_line,
};

/// How wide someone's picture is among the people found: as in a note's
/// row.
const PERSON_AVATAR_SIZE: f32 = 36.0;

/// How many characters of what someone says of themselves show under
/// their name: about a line.
const PERSON_ABOUT_CHARS: usize = 120;

/// Where the search is typed, by which it is told to take the keyboard.
static SEARCH_INPUT: LazyLock<widget::Id> = LazyLock::new(widget::Id::unique);

/// The id of where the search is typed.
pub fn search_input_id() -> widget::Id {
    SEARCH_INPUT.clone()
}

impl AppModel {
    /// The line above a search: the way back, where the search is typed,
    /// and the button that searches, as Enter does.
    fn view_search_header<'a>(&'a self, page: &'a SearchPage) -> Element<'a, Message> {
        let id = page.id;
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("search-back"))
            .on_press(Message::ClosePage);
        let typed = text_input(fl!("search-placeholder"), &page.typed)
            .id(search_input_id())
            .on_input(move |typed| Message::SearchTyped(id, typed))
            .on_submit(move |_| Message::Search(id))
            .width(Length::Fill);
        let go = button::icon(widget::icon::from_name("system-search-symbolic"))
            .tooltip(fl!("search-go"))
            .on_press_maybe((!page.typed.trim().is_empty()).then_some(Message::Search(id)));
        widget::Row::new().push(back).push(typed).push(go).spacing(8).align_y(Alignment::Center).padding([4, 12]).into()
    }

    /// The choice between the people found and the notes: the one on show
    /// stands out.
    fn view_search_tabs(page: &SearchPage) -> Element<'_, Message> {
        let tab = |what: SearchFor, label: String| {
            let tab = if page.what == what { button::standard(label) } else { button::text(label) };
            tab.on_press(Message::SearchShow(page.id, what))
        };
        widget::Row::new()
            .push(tab(SearchFor::People, fl!("search-people")))
            .push(tab(SearchFor::Notes, fl!("search-notes")))
            .spacing(8)
            .padding([0, 12, 4, 12])
            .into()
    }

    /// One of the people found: their picture, their name, their key,
    /// whether the account follows them, and the start of what they say
    /// of themselves. A click opens their
    /// profile.
    fn view_person(&self, person: &PublicKey) -> Element<'_, Message> {
        let name = display_name(&self.profiles, person);
        let profile = self.profiles.get(person);
        let picture = profile.and_then(|profile| profile.picture.as_deref());
        let picture = avatar(&self.images, picture, person, &name, PERSON_AVATAR_SIZE);

        // Cut rather than wrapped: every row is as tall as every other.
        let line = |words: String, size: u16| container(text(words).size(size).wrapping(Wrapping::None)).clip(true);
        // Told apart from whoever else goes by the name: the key, and
        // whether the account follows them.
        let followed = self.follows.as_ref().is_some_and(|follows| follows.contains(person));
        let key = if followed { format!("{} · {}", short_npub(person), fl!("search-following")) } else { short_npub(person) };
        let mut who = widget::Column::new()
            .push(container(text(name.into_owned()).size(13).font(cosmic::font::bold()).wrapping(Wrapping::None)).clip(true))
            .push(line(key, 11))
            .spacing(2)
            .width(Length::Fill);
        if let Some(about) = profile.and_then(|profile| profile.about.as_deref()) {
            let first_line = about.lines().next().unwrap_or_default();
            who = who.push(line(truncate_chars(first_line, PERSON_ABOUT_CHARS).into_owned(), 11));
        }
        let mut row = widget::Row::new().push(picture).push(who).spacing(8).align_y(Alignment::Center).padding([6, 0]);
        // Followed, or not any more, from here: one at a time, and once
        // it is known who is followed.
        let own = self.account.as_ref().is_some_and(|account| account.public_key == *person);
        if self.can_publish() && !own && self.follows.is_some() {
            let change = Message::Follow { person: *person, follow: !followed, first_list: false };
            let label = if followed { fl!("profile-unfollow") } else { fl!("profile-follow") };
            let follow = if followed { button::standard(label) } else { button::suggested(label) };
            row = row.push(follow.on_press_maybe(self.follow_pending.is_none().then_some(change)));
        }
        widget::mouse_area(row).on_press(Message::OpenProfile(*person)).interaction(mouse::Interaction::Pointer).into()
    }

    /// The people a search found, the best match first.
    fn view_people_found<'a>(&'a self, page: &'a SearchPage) -> Element<'a, Message> {
        let people = page.people.iter().fold(widget::Column::new(), |people, person| people.push(self.view_person(person)));
        list_scroller(&page.people_position, people)
    }

    /// The notes a search found, the best match first.
    fn view_notes_found<'a>(&'a self, page: &'a SearchPage) -> Element<'a, Message> {
        let context = self.row_context(&page.expanded, page.marked);
        let sizes = context.sizes;
        let list: VirtualList<'_, Note, _, Message, cosmic::Theme, cosmic::Renderer> = VirtualList::new(
            &page.notes,
            &page.heights,
            |note: &Note| note.id,
            move |note, width| estimated_note_height(note, sizes, width, context.expanded.has(&note.id)),
            move |_index, note| marking(opening_thread(note_row(note, &context), note.id, &context), note.id, &context),
        );
        list_scroller(&page.notes_position, list)
    }

    /// What is under where the search is typed: what was found, or why
    /// there is nothing to show.
    fn view_search_body<'a>(&'a self, page: &'a SearchPage) -> Element<'a, Message> {
        // A relay's address was typed: its notes are what there is to show.
        if page.relay.is_some() && page.what == SearchFor::People {
            return status_line(fl!("search-relay-people"));
        }
        // What is searched for is asked of the search relays, and with
        // none there is nobody to ask. A relay's own notes are asked of it.
        match self.search_relays() {
            _ if page.relay.is_some() => {}
            None => return status_line(fl!("search-reading-relays")),
            Some(relays) if relays.is_empty() => {
                let to_relays = button::standard(fl!("search-open-relays")).on_press(Message::OpenRelays);
                return widget::Column::new()
                    .push(status_line(fl!("search-no-relays")))
                    .push(container(to_relays).width(Length::Fill).align_x(Alignment::Center).padding([0, 0, 16, 0]))
                    .into();
            }
            Some(_) => {}
        }
        let found_none = match page.what {
            SearchFor::People => page.people.is_empty(),
            SearchFor::Notes => page.notes.is_empty(),
        };
        match page.progress(page.what) {
            SearchProgress::Idle => status_line(fl!("search-hint")),
            SearchProgress::Searching => status_line(fl!("search-searching")),
            SearchProgress::Failed(why) => status_line(fl!("search-failed", error = why.as_str())),
            SearchProgress::Found if found_none => status_line(fl!("search-nothing")),
            SearchProgress::Found => match page.what {
                SearchFor::People => self.view_people_found(page),
                SearchFor::Notes => self.view_notes_found(page),
            },
        }
    }

    /// What the relay whose notes are on show says of itself, over them:
    /// its address, its name, a line about it, and whether it searches.
    fn view_relay_shown<'a>(page: &'a SearchPage) -> Option<Element<'a, Message>> {
        let relay = page.relay.as_ref()?;
        let mut said = vec![relay_name(relay).to_string()];
        if let Some(info) = &page.relay_info {
            // Many call themselves by their address.
            said.extend(info.name.clone().filter(|name| name != relay_name(relay)));
            said.extend(info.description.clone());
            let searches = info.supports(NIP_SEARCH) == Some(true);
            said.push(if searches { fl!("search-relay-searches") } else { fl!("search-relay-no-search") });
        }
        let line = text(said.join(" · ")).size(11).wrapping(Wrapping::WordOrGlyph).width(Length::Fill);
        Some(container(line).padding([6, 12]).into())
    }

    /// Render the search view.
    pub fn view_search<'a>(&'a self, page: &'a SearchPage) -> Element<'a, Message> {
        widget::Column::new()
            .push(self.view_search_header(page))
            .push(Self::view_search_tabs(page))
            .push(widget::divider::horizontal::default())
            .push_maybe(Self::view_relay_shown(page))
            .push(self.view_search_body(page))
            .into()
    }
}

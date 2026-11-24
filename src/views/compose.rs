// SPDX-License-Identifier: MIT

//! The composer view for Avestruz: a note, a reply or a quote being written.
//!
//! A reply shows what it answers above the text, in brief, and a quote what
//! it quotes under the text, set apart as other clients will show it. The
//! text is in
//! an editor whose state is in the app model, so a popup that closes takes
//! nothing with it. Ctrl+Enter publishes, as the button does.
//!
//! Above the text, a control chooses between writing it and looking at how
//! it would show: the note as a row of a list would have it, whole, in a
//! frame that stands where the text does and scrolls when it has more than
//! there is room for (see [`crate::handlers::compose`]).
//!
//! The paperclip under the text attaches a file, which is sent to the
//! account's media servers; while it is on its way it shows under the
//! text with how far it has got, and when it is there its address goes
//! into the text.

use std::collections::HashMap;
use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::keyboard::Key;
use cosmic::iced::keyboard::key::Named;
use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Border, Length};
use cosmic::widget::text_editor::{Binding, Catalog, KeyPress, Status, Style, text_editor};
use cosmic::widget::{self, button, container, text};
use nostr_sdk::prelude::PublicKey;

use crate::fl;
use crate::helpers::{relative_time, truncate_chars, unix_now};
use crate::messages::Message;
use crate::nostr::compose::Target;
use crate::nostr::models::{Note, Profile, display_name};
use crate::state::{AppModel, Composer, Preview, Upload};
use crate::views::components::{avatar, note_being_written};

/// The editor the text is written in.
pub static COMPOSE_EDITOR_ID: LazyLock<widget::Id> = LazyLock::new(widget::Id::unique);

/// How tall the editor is with little in it, and how tall it grows before
/// the text scrolls inside it.
const EDITOR_MIN_HEIGHT: f32 = 120.0;
const EDITOR_MAX_HEIGHT: f32 = 300.0;

/// How wide each of the two choices above the text is at least: writing
/// it, and looking at how it would show.
const COMPOSE_TAB_WIDTH: u16 = 76;
/// The room between the frame what is written shows in and what it shows,
/// as the text has in its own.
const PREVIEW_PADDING: f32 = 8.0;
/// The room kept on the right of what the frame shows for the bar it
/// scrolls by.
const PREVIEW_SCROLLBAR: f32 = 8.0;
/// How much of the popup's height is not the frame's: the line above the
/// composer, the note written to, the files on their way, and the line
/// with the way to publish, with the room between them.
const AROUND_THE_PREVIEW: f32 = 280.0;

/// How much of the note replied to or quoted is shown with the text, in
/// characters, and how wide its author's picture is.
const QUOTE_MAX_CHARS: usize = 140;
const QUOTE_AVATAR_SIZE: f32 = 24.0;

/// What a note says as one line of plain text: links by their label, people
/// by name, and line breaks as spaces.
fn plain_text(note: &Note, profiles: &HashMap<PublicKey, Profile>) -> String {
    let text: String = note
        .content
        .segments
        .iter()
        .map(|segment| segment.words(&|person| format!("@{}", display_name(profiles, person))))
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How the editor looks: the theme's editor, with the thin rounded border
/// its text inputs have rather than a heavy square one.
fn editor_style(theme: &cosmic::Theme, status: Status) -> Style {
    let mut style = theme.style(&<cosmic::Theme as Catalog>::default(), status);
    style.border.width = 1.0;
    style.border.radius = theme.cosmic().corner_radii.radius_s.into();
    style
}

/// How the note a quote quotes is set apart under the text: in a thin
/// rounded border, as the editor is, in the colour of the lines between
/// rows.
fn quoted_style() -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(|theme| {
        let cosmic = theme.cosmic();
        container::Style {
            border: Border { color: cosmic.bg_divider().into(), width: 1.0, radius: cosmic.corner_radii.radius_s.into() },
            ..container::Style::default()
        }
    })
}

/// What the keys do in the editor: Ctrl+Enter publishes; pasting asks
/// what is on the clipboard, which may be a file to attach and not text
/// (see [`crate::handlers::media`]); Backspace with nothing in the text
/// (`empty`) for it to rub out leaves the composer (see
/// [`crate::handlers::compose`]); and the rest is the editor's own.
fn key_binding(press: KeyPress, empty: bool) -> Option<Binding<Message>> {
    if press.key == Key::Named(Named::Enter) && press.modifiers.command() {
        return Some(Binding::Custom(Message::Publish));
    }
    // Ctrl+P is the composer's, wherever the keyboard is: nothing of the
    // text's.
    if press.modifiers.command() && matches!(press.key.as_ref(), Key::Character("p" | "P")) {
        return None;
    }
    if empty && press.key == Key::Named(Named::Backspace) && matches!(press.status, Status::Focused { .. }) {
        return Some(Binding::Custom(Message::ComposerRubbedOut));
    }
    match Binding::from_key_press(press) {
        Some(Binding::Paste) => Some(Binding::Custom(Message::ComposePaste)),
        binding => binding,
    }
}

/// `shown`, which is what is being written as it would show, in the frame
/// that stands where the text does: as tall as what it holds, no shorter
/// than the text is at its shortest and no taller than `room`, past which
/// it scrolls.
fn framed<'a>(shown: Element<'a, Message>, room: f32) -> Element<'a, Message> {
    // Beside it, something as tall as the text is at its shortest and no
    // wider than nothing: the frame is never shorter than that.
    let shown = widget::Row::new()
        .push(widget::Space::new().height(Length::Fixed(EDITOR_MIN_HEIGHT - 2.0 * PREVIEW_PADDING)))
        .push(shown);
    // Room on the right for the bar it scrolls by.
    let shown =
        container(shown).padding([PREVIEW_PADDING, PREVIEW_PADDING + PREVIEW_SCROLLBAR, PREVIEW_PADDING, PREVIEW_PADDING]);
    container(widget::scrollable(shown.width(Length::Fill)).height(Length::Shrink))
        .width(Length::Fill)
        .max_height(room)
        .class(quoted_style())
        .into()
}

/// Whether what was written is kept as a draft, asked of whoever pressed
/// Backspace to leave the composer, and the three answers there are, each
/// with the key that gives it.
fn view_leaving<'a>() -> Element<'a, Message> {
    let answers = widget::Row::new()
        .push(button::suggested(fl!("compose-leaving-keep")).on_press(Message::CloseComposer))
        .push(button::destructive(fl!("compose-leaving-discard")).on_press(Message::DiscardDraft))
        .push(button::text(fl!("compose-leaving-stay")).on_press(Message::ResumeComposer))
        .spacing(8)
        .align_y(Alignment::Center);
    widget::Column::new()
        .push(text(fl!("compose-leaving")).size(13).wrapping(Wrapping::WordOrGlyph))
        .push(answers)
        .spacing(8)
        .into()
}

/// A file on its way to a media server: what it is called, how far it has
/// got, and the way to call it off.
fn view_upload(upload: &Upload) -> Element<'_, Message> {
    let how_far = match upload.percent() {
        Some(percent) => fl!("compose-upload-sent", percent = percent),
        None => fl!("compose-upload-preparing"),
    };
    let cancel = button::icon(widget::icon::from_name("window-close-symbolic"))
        .extra_small()
        .tooltip(fl!("compose-upload-cancel"))
        .on_press(Message::CancelUpload(upload.id));
    widget::Row::new()
        .push(widget::icon::from_name("mail-attachment-symbolic").size(14))
        // A long name is cut rather than wrapped.
        .push(container(text(upload.name.as_str()).size(12).wrapping(Wrapping::None)).width(Length::Fill).clip(true))
        .push(text(how_far).size(11).wrapping(Wrapping::None))
        .push(cancel)
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

impl AppModel {
    /// The line above the composer: the way back, and what is being written.
    fn view_compose_header(&self, composer: &Composer) -> Element<'_, Message> {
        let back = button::icon(widget::icon::from_name("go-previous-symbolic"))
            .tooltip(fl!("compose-back"))
            .on_press(Message::CloseComposer);
        let title = match composer.target {
            Target::Note => fl!("compose-title-note"),
            Target::Reply(_) => fl!("compose-title-reply"),
            Target::Quote(_) => fl!("compose-title-quote"),
        };
        // Two choices, one of them always made: writing it, and looking at
        // how it would show.
        let tabs = widget::segmented_control::horizontal(&self.compose_tabs)
            .on_activate(Message::SelectComposeTab)
            .minimum_button_width(COMPOSE_TAB_WIDTH)
            .width(Length::Shrink);
        widget::Row::new()
            .push(back)
            .push(
                container(text(title).size(16).font(cosmic::font::bold()).wrapping(Wrapping::None))
                    .width(Length::Fill)
                    .clip(true),
            )
            .push(tabs)
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([4, 12])
            .into()
    }

    /// The note a reply answers or a quote quotes, in brief: who wrote it,
    /// when, and how it starts.
    fn view_note_in_brief<'a>(&'a self, note: &'a Note) -> Element<'a, Message> {
        let name = display_name(&self.profiles, &note.author);
        let picture = self.profiles.get(&note.author).and_then(|profile| profile.picture.as_deref());
        let author_picture = avatar(&self.images, picture, &note.author, &name, QUOTE_AVATAR_SIZE);
        let said = plain_text(note, &self.profiles);

        let who = widget::Row::new()
            .push(
                container(text(name).size(12).font(cosmic::font::bold()).wrapping(Wrapping::None)).width(Length::Fill).clip(true),
            )
            .push(text(relative_time(unix_now(), note.created_at)).size(11).wrapping(Wrapping::None))
            .spacing(8)
            .align_y(Alignment::Center);
        let body = widget::Column::new()
            .push(who)
            .push(text(truncate_chars(&said, QUOTE_MAX_CHARS).into_owned()).size(12).wrapping(Wrapping::WordOrGlyph))
            .spacing(2)
            .width(Length::Fill);
        widget::Row::new().push(author_picture).push(body).spacing(8).into()
    }

    /// What is being written as it would show once it is out, where the
    /// text is while it is written: who writes it and all it says, as a
    /// row of a list would have it, in a frame the size of what it holds,
    /// no smaller than the text's and no taller than the popup has room
    /// for, past which it scrolls. With nothing written, it says so.
    fn view_preview<'a>(&'a self, preview: &'a Preview) -> Element<'a, Message> {
        let context = self.row_context(&preview.expanded, None);
        let said = &preview.note.content;
        let nothing = said.segments.is_empty() && said.images.is_empty() && said.videos.is_empty() && said.quotes.is_empty();
        let shown: Element<'a, Message> = if nothing {
            text(fl!("compose-preview-empty")).size(13).into()
        } else {
            note_being_written(&preview.note, &context)
        };
        framed(shown, (self.popup_max_height() - AROUND_THE_PREVIEW).max(EDITOR_MIN_HEIGHT))
    }

    /// Render the composer. With nothing being written there is nothing to
    /// show but what is under it.
    pub fn view_compose(&self) -> Element<'_, Message> {
        let Some(composer) = &self.composer else {
            return self.view_feed();
        };
        let (placeholder, publish) = match composer.target {
            Target::Note => (fl!("compose-placeholder-note"), fl!("compose-publish")),
            Target::Reply(_) => (fl!("compose-placeholder-reply"), fl!("compose-reply")),
            Target::Quote(_) => (fl!("compose-placeholder-quote"), fl!("compose-publish")),
        };

        // Not a blank line or a space: those are something to rub out.
        let empty = composer.content.text().is_empty();
        let editor = text_editor(&composer.content)
            .id(COMPOSE_EDITOR_ID.clone())
            .placeholder(placeholder)
            .key_binding(move |press| key_binding(press, empty))
            .style(editor_style)
            .size(14)
            .padding(8)
            .wrapping(Wrapping::WordOrGlyph)
            .min_height(EDITOR_MIN_HEIGHT)
            .max_height(EDITOR_MAX_HEIGHT)
            // The menu would be a popup of its own, over a popup.
            .context_menu(false);
        // Asked whether it is kept as a draft, the text is not written
        // to: the keys are the answer's.
        let editor = if composer.leaving { editor } else { editor.on_action(Message::ComposeEdited) };
        // In its place, how it would show, while that is looked at.
        let editor: Element<'_, Message> = match &composer.preview {
            Some(preview) => self.view_preview(preview),
            None => editor.into(),
        };

        let written = composer.content.text().trim().chars().count();
        // A quote needs nothing said: the note quoted is something to
        // publish.
        let quoting = matches!(composer.target, Target::Quote(_));
        // Nor does anything go out while a file attached to it is still
        // on its way.
        let uploads: Vec<&Upload> = self.uploads_for(composer.target).collect();
        let can_publish = (written > 0 || quoting) && !composer.publishing && self.nostr.is_some() && uploads.is_empty();
        let publish_label = if composer.publishing { fl!("compose-publishing") } else { publish };
        let attach = button::icon(widget::icon::from_name("mail-attachment-symbolic"))
            .tooltip(fl!("compose-attach"))
            .on_press_maybe((!composer.publishing && !self.media.picking).then_some(Message::AttachMedia));
        let footer = widget::Row::new()
            .push(attach)
            .push(
                container(text(fl!("compose-count", count = written)).size(11).wrapping(Wrapping::None))
                    .width(Length::Fill)
                    .clip(true),
            )
            .push(text(fl!("compose-shortcut")).size(11).wrapping(Wrapping::None))
            .push(button::suggested(publish_label).on_press_maybe(can_publish.then_some(Message::Publish)))
            .spacing(8)
            .align_y(Alignment::Center);

        let mut body = widget::Column::new().spacing(8).padding([8, 12]).width(Length::Fill);
        match &composer.about {
            Some(quoted) if quoting => {
                let quoted = container(self.view_note_in_brief(quoted)).padding(8).width(Length::Fill).class(quoted_style());
                body = body.push(editor).push(quoted);
            }
            Some(answered) => body = body.push(self.view_note_in_brief(answered)).push(editor),
            None => body = body.push(editor),
        }
        for upload in uploads {
            body = body.push(view_upload(upload));
        }
        if let Some(error) = &composer.error {
            body = body.push(text(error.as_str()).size(12).class(cosmic::theme::Text::Accent).wrapping(Wrapping::WordOrGlyph));
        }
        // The question is asked where the way to publish was: nothing
        // goes out while leaving is being thought about.
        body = body.push(if composer.leaving { view_leaving() } else { footer.into() });

        widget::Column::new()
            .push(self.view_compose_header(composer))
            .push(widget::divider::horizontal::default())
            .push(body)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{EDITOR_MIN_HEIGHT, framed, plain_text};
    use crate::messages::Message;
    use crate::nostr::content::{self, FEED_NOTE_MAX_CHARS};
    use crate::nostr::models::{Note, Profile};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Length, Size, layout};
    use nostr_sdk::prelude::{EventId, Keys, Kind, ToBech32};
    use std::collections::HashMap;

    #[test]
    fn the_frame_of_what_is_written_is_as_tall_as_what_it_holds_within_its_bounds() {
        let renderer = crate::views::components::testing::renderer();
        // How tall the frame is around something `tall`, with `room` to
        // be at most.
        let framing = |tall: f32, room: f32| {
            let held: cosmic::Element<'_, Message> = cosmic::widget::Space::new().width(Length::Fill).height(tall).into();
            let mut frame = framed(held, room);
            let mut tree = Tree::new(frame.as_widget());
            let limits = layout::Limits::new(Size::ZERO, Size::new(600.0, 2_000.0));
            let laid_out = frame.as_widget_mut().layout(&mut tree, &renderer, &limits).size();
            assert_eq!(laid_out.width, 600.0, "as wide as the composer");
            laid_out.height
        };

        // Never shorter than the text is with little in it: the way to
        // publish does not come up under a short note.
        assert_eq!(framing(10.0, 500.0), EDITOR_MIN_HEIGHT);
        // As tall as what it holds, with the room around that.
        assert_eq!(framing(200.0, 500.0), 216.0);
        // And no taller than there is room for: the rest is scrolled to.
        assert_eq!(framing(900.0, 500.0), 500.0);
    }

    #[test]
    fn a_note_in_brief_is_one_line_with_names_and_labels() {
        let friend = Keys::generate().public_key();
        let said = format!("gm nostr:{}\n\nsee https://www.example.com/a/page?x=1  #nostr", friend.to_bech32().unwrap());
        let note = Note {
            id: EventId::from_byte_array([7; 32]),
            kind: Kind::TextNote,
            author: Keys::generate().public_key(),
            created_at: 0,
            content: content::parse(&said, FEED_NOTE_MAX_CHARS),
            reply_to: None,
            quoted: Vec::new(),
        };
        let profiles = HashMap::from([(friend, Profile { name: Some("fiatjaf".into()), ..Profile::default() })]);

        assert_eq!(plain_text(&note, &profiles), "gm @fiatjaf see example.com/a/page #nostr");
    }
}

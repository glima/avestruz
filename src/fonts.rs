// SPDX-License-Identifier: MIT

//! Fonts: which ones emoji are drawn with.
//!
//! Text is shaped and drawn by `cosmic-text`, which rasterizes glyphs with
//! `swash`. Left to itself it gets emoji wrong in two ways, and both are
//! put right here, once, before anything is drawn (see [`prepare`]).
//!
//! **A font whose glyphs can't be drawn.** Of the ways a font can carry
//! colour glyphs, `swash` draws bitmaps (`CBDT`, `sbix`) and the first
//! version of layered outlines (`COLR` version 0). It does not draw the
//! second (`COLR` version 1, with gradients), and a font that has only
//! those gets its glyphs chosen all the same, since it says it has them:
//! they come out as nothing at all. Some distributions ship their emoji
//! font that way. Such fonts are taken out of the list this process
//! chooses from, and what they would have drawn falls to the next font
//! that has it.
//!
//! **A text font that comes first.** What the font of the interface has
//! no glyph for is looked for in a list of other fonts, in order, and an
//! emoji font is the last of that list: after text fonts that have a face
//! or a heart of their own, in one colour. So a note would show some emoji
//! in colour and some as line drawings. The fonts for emoji are put at the
//! head of that list instead (see [`emoji_families`]): the ones in colour
//! first, then any other that is one by its name. A font for emoji is
//! one that has a glyph for a face ([`AN_EMOJI`]) and either colour or
//! the name: a text font with a few emoji among its letters is not one,
//! and neither is a font with colour glyphs that are no emoji.
//!
//! **A font of the applet's own.** Fonts in the `fonts` directory beside
//! the event store (see [`own_fonts_dir`]) are read as well as the
//! system's, and come before them. That is where to put an emoji font that
//! can be drawn where the system's can't, without changing what any other
//! program draws: `just emoji-font-setup` fetches one.
//!
//! Nothing is changed outside this process: no file, and no setting.

use std::path::{Path, PathBuf};

use cosmic::iced::advanced::graphics::text::cosmic_text::fontdb::{Database, ID, Source};
use cosmic::iced::advanced::graphics::text::cosmic_text::skrifa::raw::TableProvider;
use cosmic::iced::advanced::graphics::text::cosmic_text::skrifa::{FontRef, MetadataProvider, Tag};
use cosmic::iced::advanced::graphics::text::cosmic_text::{Fallback, FontSystem, PlatformFallback};
use cosmic::iced::advanced::graphics::text::font_system;
use unicode_script::Script;

/// Where the applet's own fonts are: the user's data directory, beside
/// the event store.
pub fn own_fonts_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("cosmic-applet-avestruz").join("fonts")
}

/// An emoji every font for emoji has a glyph for, and few others: by
/// which one is told.
const AN_EMOJI: char = '😀';

/// What a face has of glyphs in colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    /// None: an ordinary font.
    None,
    /// Some that can be drawn: bitmaps, or layered outlines of the first
    /// kind.
    Drawn,
    /// Only ones that can't: layered outlines of the second kind, with
    /// nothing to draw instead.
    Undrawn,
}

/// What `font` has of glyphs in colour. Its `COLR` table lists the glyphs
/// of the first kind, and says of which version it is: of version 1 or
/// later with none listed that way, and no bitmaps beside it, there is
/// nothing to draw.
fn colour(font: &FontRef) -> Colour {
    if [b"CBDT", b"sbix"].into_iter().any(|bitmaps| font.table_data(Tag::new(bitmaps)).is_some()) {
        return Colour::Drawn;
    }
    match font.colr() {
        Ok(colr) if colr.num_base_glyph_records() > 0 => Colour::Drawn,
        Ok(colr) if colr.version() >= 1 => Colour::Undrawn,
        _ => Colour::None,
    }
}

/// What the face numbered `index` of the font file `data` has of glyphs
/// in colour, and whether it has emoji: `None` of a file that is no font.
fn read(data: &[u8], index: u32) -> Option<(Colour, bool)> {
    let font = FontRef::from_index(data, index).ok()?;
    Some((colour(&font), font.charmap().map(AN_EMOJI).is_some()))
}

/// A face, as far as choosing the fonts for emoji goes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Face {
    id: ID,
    /// What its family is called.
    family: String,
    colour: Colour,
    /// It has emoji: a glyph for [`AN_EMOJI`].
    has_emoji: bool,
    /// It says of itself that it is for emoji: its name does.
    for_emoji: bool,
    /// It is one of the applet's own (see [`own_fonts_dir`]).
    own: bool,
}

/// Every face of `fonts`, with those under `own` marked as the applet's.
fn faces(fonts: &Database, own: &Path) -> Vec<Face> {
    let known: Vec<Face> = fonts
        .faces()
        .map(|face| Face {
            id: face.id,
            family: face.families.first().map_or_else(|| face.post_script_name.clone(), |(family, _)| family.clone()),
            colour: Colour::None,
            has_emoji: false,
            for_emoji: face.post_script_name.contains("Emoji"),
            own: match &face.source {
                Source::File(path) | Source::SharedFile(path, _) => path.starts_with(own),
                Source::Binary(_) => false,
            },
        })
        .collect();
    known
        .into_iter()
        .map(|face| {
            let (colour, has_emoji) = fonts.with_face_data(face.id, read).flatten().unwrap_or((Colour::None, false));
            Face { colour, has_emoji, ..face }
        })
        .collect()
}

/// The families emoji are to be looked for in before any other, in that
/// order, of `faces`: those that have emoji in colour that can be drawn,
/// the applet's own before the system's, then those that have them in
/// one colour and are for emoji by their name. Each once. A font with
/// colour glyphs and no emoji is not one, nor is a text font that has a
/// few emoji among its letters; and none whose glyphs can't be drawn.
fn emoji_families(faces: &[Face]) -> Vec<String> {
    // The lower, the sooner.
    let rank = |face: &Face| match (face.has_emoji, face.colour, face.own, face.for_emoji) {
        (true, Colour::Drawn, true, _) => Some(0),
        (true, Colour::Drawn, false, _) => Some(1),
        (true, Colour::None, _, true) => Some(2),
        _ => None,
    };
    let mut ranked: Vec<(u8, &str)> = faces.iter().filter_map(|face| Some((rank(face)?, face.family.as_str()))).collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    let mut families: Vec<String> = Vec::new();
    for (_, family) in ranked {
        if !families.iter().any(|known| known == family) {
            families.push(family.to_string());
        }
    }
    families
}

/// The fonts a glyph is looked for in when the font asked for has none:
/// as the platform has them, with the fonts for emoji before the rest.
struct EmojiFirst {
    platform: PlatformFallback,
    /// The fonts for emoji, then the platform's own list.
    common: Vec<&'static str>,
}

impl EmojiFirst {
    /// With `emoji`, the families of the fonts for emoji, first. Their
    /// names are kept for as long as the process runs, which is how long
    /// the list is used: this is done once.
    fn new(emoji: Vec<String>) -> Self {
        let platform = PlatformFallback;
        let mut common: Vec<&'static str> = emoji.into_iter().map(|family| &*Box::leak(family.into_boxed_str())).collect();
        for family in platform.common_fallback() {
            if !common.contains(family) {
                common.push(family);
            }
        }
        Self { platform, common }
    }
}

impl Fallback for EmojiFirst {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.platform.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        self.platform.script_fallback(script, locale)
    }
}

/// See that emoji are drawn, and with the fonts for emoji: read the
/// applet's own fonts, take out the fonts whose colour glyphs can't be
/// drawn, and have the fonts for emoji looked in before the others. To be
/// called once, before any text is laid out. See the [module docs](self).
pub fn prepare() {
    let Ok(mut system) = font_system().write() else {
        return;
    };
    let system = system.raw();
    let own = own_fonts_dir();
    let fonts = system.db_mut();
    fonts.load_fonts_dir(&own);

    let faces = faces(fonts, &own);
    for face in faces.iter().filter(|face| face.colour == Colour::Undrawn) {
        tracing::info!("{} is left out: its colour glyphs are of a kind that can't be drawn here (COLR version 1)", face.family);
        fonts.remove_face(face.id);
    }

    let emoji = emoji_families(&faces);
    if emoji.is_empty() {
        return;
    }
    tracing::info!("emoji are looked for in {emoji:?} before any other font");
    // The list of fonts to fall back to is given to the text system when
    // it is made, and not after: so it is made again, of the same fonts.
    let emptied = FontSystem::new_with_locale_and_db(String::new(), Database::new());
    let (locale, fonts) = std::mem::replace(system, emptied).into_locale_and_db();
    *system = FontSystem::new_with_locale_and_db_and_fallback(locale, fonts, EmojiFirst::new(emoji));
}

#[cfg(test)]
mod tests {
    use super::{AN_EMOJI, Colour, EmojiFirst, Face, emoji_families, read};
    use cosmic::iced::advanced::graphics::text::cosmic_text::fontdb::ID;
    use cosmic::iced::advanced::graphics::text::cosmic_text::{Fallback, PlatformFallback};

    /// A font file with the `tables` named, each with what is in it.
    fn font(tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        // The tables are listed in the order of their names, as a font
        // file has them.
        let mut tables = tables.to_vec();
        tables.sort_by_key(|(tag, _)| **tag);
        let mut file = vec![0, 1, 0, 0, 0, tables.len() as u8, 0, 0, 0, 0, 0, 0];
        let mut at = file.len() + tables.len() * 16;
        for (tag, content) in &tables {
            file.extend_from_slice(*tag);
            file.extend_from_slice(&[0; 4]);
            file.extend_from_slice(&(at as u32).to_be_bytes());
            file.extend_from_slice(&(content.len() as u32).to_be_bytes());
            at += content.len();
        }
        for (_, content) in &tables {
            file.extend_from_slice(content);
        }
        file
    }

    /// A `COLR` table of `version`, listing `as_version_0` glyphs the way
    /// version 0 does: its header, which is all that is read of it, as
    /// long as that version has it.
    fn colr(version: u16, as_version_0: u16) -> Vec<u8> {
        let mut table = [version.to_be_bytes(), as_version_0.to_be_bytes()].concat();
        table.resize(if version == 0 { 14 } else { 34 }, 0);
        table
    }

    /// A `cmap` table with a glyph for `character` and for no other.
    fn cmap(character: char) -> Vec<u8> {
        let code = (character as u32).to_be_bytes();
        let mut table = vec![0, 0, 0, 1];
        // One list of characters, for Unicode in full, right after this.
        table.extend_from_slice(&[0, 3, 0, 10, 0, 0, 0, 12]);
        // Of the kind that lists runs of characters: one run, of one.
        table.extend_from_slice(&[0, 12, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 1]);
        table.extend_from_slice(&[code, code, 1u32.to_be_bytes()].concat());
        table
    }

    #[test]
    fn a_font_with_only_gradient_colour_glyphs_cannot_be_drawn() {
        let colour = |tables: &[(&[u8; 4], &[u8])]| read(&font(tables), 0).map(|(colour, _)| colour);
        assert_eq!(colour(&[(b"cmap", &cmap('a')), (b"COLR", &colr(1, 0))]), Some(Colour::Undrawn));

        // No colour glyphs at all: an ordinary font.
        assert_eq!(colour(&[(b"cmap", &cmap('a')), (b"glyf", &[0; 8])]), Some(Colour::None));
        assert_eq!(colour(&[(b"COLR", &colr(0, 0))]), Some(Colour::None), "a list of none");
        // Layered outlines of the first kind, alone or beside the second.
        assert_eq!(colour(&[(b"COLR", &colr(0, 120))]), Some(Colour::Drawn));
        assert_eq!(colour(&[(b"COLR", &colr(1, 120))]), Some(Colour::Drawn));
        // Bitmaps, alone or to draw instead.
        assert_eq!(colour(&[(b"cmap", &cmap('a')), (b"CBDT", &[0; 4])]), Some(Colour::Drawn));
        assert_eq!(colour(&[(b"COLR", &colr(1, 0)), (b"CBDT", &[0; 4])]), Some(Colour::Drawn));
        assert_eq!(colour(&[(b"sbix", &[0; 4]), (b"COLR", &colr(1, 0))]), Some(Colour::Drawn));
        // A table cut short says nothing.
        assert_eq!(colour(&[(b"COLR", &[0, 1])]), Some(Colour::None));
    }

    #[test]
    fn a_font_has_emoji_if_it_has_a_glyph_for_one() {
        let has_emoji = |tables: &[(&[u8; 4], &[u8])]| read(&font(tables), 0).map(|(_, has_emoji)| has_emoji);
        assert_eq!(has_emoji(&[(b"cmap", &cmap(AN_EMOJI)), (b"CBDT", &[0; 4])]), Some(true));
        assert_eq!(has_emoji(&[(b"cmap", &cmap('a')), (b"CBDT", &[0; 4])]), Some(false));
        assert_eq!(has_emoji(&[(b"CBDT", &[0; 4])]), Some(false), "no list of characters at all");
    }

    #[test]
    fn a_file_that_is_no_font_says_nothing() {
        for file in [&b""[..], b"not a font", &[0, 1, 0, 0, 0, 9]] {
            assert_eq!(read(file, 0), None, "{file:?}");
        }
        // A font file has one face, and no second.
        let one = font(&[(b"cmap", &cmap(AN_EMOJI)), (b"COLR", &colr(1, 0))]);
        assert_eq!(read(&one, 0), Some((Colour::Undrawn, true)));
        assert_eq!(read(&one, 1), None);
    }

    /// Faces called `families`, each with what it has of colour, whether
    /// it has emoji, whether it is for emoji by its name, and whether it
    /// is the applet's own.
    fn faces(families: &[(&str, Colour, bool, bool, bool)]) -> Vec<Face> {
        families
            .iter()
            .map(|(family, colour, has_emoji, for_emoji, own)| Face {
                id: ID::dummy(),
                family: (*family).to_string(),
                colour: *colour,
                has_emoji: *has_emoji,
                for_emoji: *for_emoji,
                own: *own,
            })
            .collect()
    }

    #[test]
    fn emoji_are_looked_for_in_colour_first_and_in_the_applets_own_font_before_the_systems() {
        let installed = faces(&[
            // A text font with a few emoji among its letters.
            ("DejaVu Sans", Colour::None, true, false, false),
            ("Noto Emoji", Colour::None, true, true, false),
            ("Noto Color Emoji", Colour::Undrawn, true, true, false),
            // A font with colour glyphs that are no emoji.
            ("Mezenets Unicode", Colour::Drawn, false, false, false),
            ("Twemoji", Colour::Drawn, true, false, false),
            ("Twemoji", Colour::Drawn, true, false, false),
            ("Bitmap Emoji", Colour::Drawn, true, true, true),
        ]);
        // The applet's own, the system's in colour, then the one in one
        // colour; no text font, and not the one that can't be drawn.
        assert_eq!(emoji_families(&installed), ["Bitmap Emoji", "Twemoji", "Noto Emoji"]);

        // A family with a face that can be drawn and one that can't, as
        // when the applet's own font is called what the system's is: it
        // is the one that can be drawn that is left to find by that name.
        let both = faces(&[
            ("Noto Color Emoji", Colour::Undrawn, true, true, false),
            ("Noto Color Emoji", Colour::Drawn, true, true, true),
        ]);
        assert_eq!(emoji_families(&both), ["Noto Color Emoji"]);

        assert!(emoji_families(&faces(&[("DejaVu Sans", Colour::None, true, false, false)])).is_empty());
        assert!(emoji_families(&faces(&[("Noto Color Emoji", Colour::Undrawn, true, true, false)])).is_empty());
    }

    #[test]
    fn the_fonts_for_emoji_come_before_the_platforms_and_the_rest_is_the_platforms() {
        let platform = PlatformFallback;
        let first = EmojiFirst::new(vec!["Bitmap Emoji".into(), "Noto Color Emoji".into()]);

        let common = first.common_fallback();
        assert_eq!(common[..2], ["Bitmap Emoji", "Noto Color Emoji"]);
        // Every one of the platform's is still there, in its order, and
        // none twice.
        let rest: Vec<&str> = platform.common_fallback().iter().copied().filter(|family| *family != "Noto Color Emoji").collect();
        assert_eq!(common[2..], rest[..]);
        assert!(rest.contains(&"DejaVu Sans"));

        assert_eq!(first.forbidden_fallback(), platform.forbidden_fallback());
        let script = unicode_script::Script::Arabic;
        assert_eq!(first.script_fallback(script, "en"), platform.script_fallback(script, "en"));
    }
}

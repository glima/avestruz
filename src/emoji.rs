// SPDX-License-Identifier: MIT

//! Emoji to react with: the ones this reader uses most, kept for next
//! time, and all there are, to pick another from.
//!
//! The ones offered first are the ones used most, and of those used as
//! often the ones used last (see [`suggested`]); a few common ones fill
//! the room until there is that much history. Which were used, how often
//! and when is kept in the settings of this machine
//! ([`Config::reaction_emojis`](crate::config::Config::reaction_emojis)),
//! and nowhere else: it is nobody's business what one reacts with most.
//!
//! What an emoji is is what the `emojis` crate knows as one, which is
//! Unicode's list: that is what tells a reaction that is an emoji from one
//! that is some other text ([`known`]), and what there is to pick from
//! ([`found`]).

pub use emojis::Group;
use serde::{Deserialize, Serialize};

/// How many emoji are kept track of: the most used. Beyond that many the
/// least used are forgotten.
const KEPT_MAX: usize = 30;

/// How many emoji a search for one shows at the most.
const FOUND_MAX: usize = 96;

/// What is offered until there is history enough to offer: a few that
/// are common as reactions.
const USUAL: [&str; 7] = ["👍", "❤️", "😂", "🥲", "👀", "🫡", "🫂"];

/// An emoji that was reacted with, how many times, and when last, in
/// seconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Used {
    pub emoji: String,
    pub count: u32,
    pub last_used: u64,
}

/// `text` as the emoji it is, written the one way each is written here,
/// if it is one: an emoji may be written with or without the mark that
/// says to show it as a picture.
pub fn known(text: &str) -> Option<&'static str> {
    emojis::get(text).map(emojis::Emoji::as_str)
}

/// The more used of two, the one used last if as often: which comes
/// first.
fn by_use(a: &Used, b: &Used) -> std::cmp::Ordering {
    b.count.cmp(&a.count).then(b.last_used.cmp(&a.last_used))
}

/// Note in `used` that `emoji` was reacted with at `now`. What is no
/// emoji is not noted. No more than [`KEPT_MAX`] are kept: the least
/// used go, the one just used never.
pub fn record(used: &mut Vec<Used>, emoji: &str, now: u64) {
    let Some(emoji) = known(emoji) else {
        return;
    };
    match used.iter_mut().find(|entry| known(&entry.emoji) == Some(emoji)) {
        Some(entry) => {
            entry.count = entry.count.saturating_add(1);
            entry.last_used = now;
        }
        None => {
            // Room is made first, so that the newcomer, used once, is not
            // the one to go.
            used.sort_by(by_use);
            used.truncate(KEPT_MAX - 1);
            used.push(Used { emoji: emoji.to_string(), count: 1, last_used: now });
        }
    }
}

/// The emoji to offer first, `max` at the most: those of `used`, the
/// most used first, then the usual ones, each once.
pub fn suggested(used: &[Used], max: usize) -> Vec<String> {
    let mut used: Vec<&Used> = used.iter().collect();
    used.sort_by(|a, b| by_use(a, b));
    let mut offered: Vec<&'static str> = Vec::new();
    for emoji in used.iter().map(|entry| entry.emoji.as_str()).chain(USUAL).filter_map(known) {
        if !offered.contains(&emoji) {
            offered.push(emoji);
        }
    }
    offered.into_iter().take(max).map(String::from).collect()
}

/// The emoji that stands for `group` where the groups are chosen from:
/// its first.
pub fn standing_for(group: Group) -> &'static str {
    group.emojis().next().map_or("", emojis::Emoji::as_str)
}

/// The emoji to pick from: those whose name or short code has every word
/// of `typed` in it, [`FOUND_MAX`] at the most, if anything is typed, and
/// all of `group` if not.
pub fn found(typed: &str, group: Group) -> Vec<&'static str> {
    let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return group.emojis().map(emojis::Emoji::as_str).collect();
    }
    let named =
        |emoji: &emojis::Emoji, word: &str| emoji.name().contains(word) || emoji.shortcodes().any(|code| code.contains(word));
    emojis::iter()
        .filter(|emoji| words.iter().all(|word| named(emoji, word)))
        .map(emojis::Emoji::as_str)
        .take(FOUND_MAX)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{FOUND_MAX, Group, KEPT_MAX, USUAL, Used, found, known, record, standing_for, suggested};

    #[test]
    fn an_emoji_is_known_however_it_is_written_and_nothing_else_is() {
        // With and without the mark that says to show it as a picture.
        assert_eq!(known("❤️"), Some("❤️"));
        assert_eq!(known("❤"), Some("❤️"));
        assert_eq!(known("🤌"), Some("🤌"));
        // With a skin tone, it is itself.
        assert_eq!(known("👍🏽"), Some("👍🏽"));
        for other in ["", "+", "-", ":soapbox:", "lol", "😂😂", "😂 ", "nice 👍"] {
            assert_eq!(known(other), None, "{other:?}");
        }
        // Every one of the usual ones is one.
        assert!(USUAL.iter().all(|emoji| known(emoji).is_some()));
    }

    #[test]
    fn the_most_used_are_offered_first_and_the_usual_ones_fill_the_room() {
        // With no history: the usual ones, as many as are asked for.
        assert_eq!(suggested(&[], 8), USUAL);
        assert_eq!(suggested(&[], 3), USUAL[..3]);

        let mut used = Vec::new();
        record(&mut used, "🤌", 10);
        record(&mut used, "🧨", 20);
        record(&mut used, "🤌", 30);
        record(&mut used, "🤯", 40);
        // Used twice; then of those used once, the one used last.
        assert_eq!(suggested(&used, 4), ["🤌", "🤯", "🧨", "👍"]);
        assert_eq!(used.iter().find(|entry| entry.emoji == "🤌"), Some(&Used { emoji: "🤌".into(), count: 2, last_used: 30 }));

        // One of the usual ones, once used, is offered where its use puts
        // it, and once: however it was written.
        record(&mut used, "❤", 50);
        record(&mut used, "❤️", 60);
        record(&mut used, "❤", 70);
        let offered = suggested(&used, 8);
        assert_eq!(offered[..5], ["❤️", "🤌", "🤯", "🧨", "👍"]);
        assert_eq!(offered.iter().filter(|emoji| *emoji == "❤️").count(), 1);
        assert_eq!(offered.len(), 8);
    }

    #[test]
    fn what_is_no_emoji_is_not_kept_and_only_so_many_are() {
        let mut used = Vec::new();
        for other in ["+", "", ":shortcode:", "text"] {
            record(&mut used, other, 1);
        }
        assert!(used.is_empty());

        // More than are kept: each used once, one after the other, but
        // for the first, which was used twice.
        let all: Vec<&str> = Group::AnimalsAndNature.emojis().map(|emoji| emoji.as_str()).take(KEPT_MAX + 10).collect();
        assert_eq!(all.len(), KEPT_MAX + 10);
        record(&mut used, all[0], 0);
        for (at, emoji) in all.iter().enumerate() {
            record(&mut used, emoji, at as u64 + 1);
        }
        assert_eq!(used.len(), KEPT_MAX);
        // The one used most is kept, and the one used last; the ones used
        // once and longest ago are gone.
        let kept = |emoji: &str| used.iter().any(|entry| entry.emoji == emoji);
        assert!(kept(all[0]) && kept(all[KEPT_MAX + 9]));
        assert!(!kept(all[1]) && !kept(all[2]));

        // What was written by hand into the settings and is no emoji is
        // never offered.
        let edited = [Used { emoji: "not one".into(), count: 99, last_used: 0 }];
        assert_eq!(suggested(&edited, 8), USUAL);
    }

    #[test]
    fn an_emoji_is_found_by_what_it_is_called_or_in_its_group() {
        assert!(found("clown", Group::Flags).contains(&"🤡"), "whatever group is on show");
        assert!(found("  Thumbs  UP ", Group::Flags).contains(&"👍"));
        // By its short code, where its name says it otherwise.
        assert!(found("+1", Group::Flags).contains(&"👍"));
        assert!(found("no such thing as this", Group::Flags).is_empty());
        assert!(found("e", Group::Flags).len() <= FOUND_MAX);

        // With nothing typed: the group, all of it, and no other.
        let flags = found("  ", Group::Flags);
        assert!(flags.len() > 200 && flags.contains(&"🇧🇷") && !flags.contains(&"🤡"));
        // Each group has an emoji to stand for it, and no two the same.
        let standing: Vec<&str> = Group::iter().map(standing_for).collect();
        assert!(standing.iter().all(|emoji| !emoji.is_empty()));
        assert_eq!(standing.iter().collect::<std::collections::HashSet<_>>().len(), standing.len());
    }
}

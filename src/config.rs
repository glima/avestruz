// SPDX-License-Identifier: MIT

//! Persisted configuration schema for Avestruz.
//!
//! Settings are stored via COSMIC's config system and survive restarts.
//! Secrets are not settings: an account's secret key is in the system keyring
//! (see [`crate::nostr::account`]), and only its public half is named here.

use std::collections::BTreeMap;

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

use crate::emoji::Used;
use crate::nostr::feed::FeedMode;

/// The relays a new installation reads from: three large public relays.
pub const DEFAULT_RELAYS: [&str; 3] = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net"];

/// How much is logged, to the journal or the terminal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    /// Only errors.
    Error,
    /// Warnings and errors.
    Warn,
    /// Informational messages and above (the default).
    #[default]
    Info,
    /// Debug messages and above.
    Debug,
    /// Everything.
    Trace,
}

impl LogLevel {
    /// Every level, from the least said to the most.
    pub const ALL: [Self; 5] = [Self::Error, Self::Warn, Self::Info, Self::Debug, Self::Trace];

    /// The `EnvFilter` base directive for this level.
    pub fn as_filter_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// The label for the settings dropdown. The names of the levels are
    /// technical, and the same in every language.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warning",
            Self::Info => "Info (default)",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
    }
}

impl AsRef<str> for LogLevel {
    fn as_ref(&self) -> &str {
        self.display_name()
    }
}

/// How large the text of notes is set: unless the settings say otherwise,
/// and at the least and the most they can say.
pub const NOTE_TEXT_SIZE_DEFAULT: u16 = 13;
pub const NOTE_TEXT_SIZE_MIN: u16 = 10;
pub const NOTE_TEXT_SIZE_MAX: u16 = 24;

/// For how many days the event store keeps what it can ask the relays for
/// again, unless the settings say otherwise; and the numbers of days the
/// settings offer, of which `0` is for good.
pub const KEEP_DAYS_DEFAULT: u32 = 30;
pub const KEEP_DAYS_CHOICES: [u32; 5] = [7, 30, 90, 365, 0];

/// Configuration for Avestruz
#[derive(Debug, Clone, CosmicConfigEntry, PartialEq)]
#[version = 1]
pub struct Config {
    /// The signed-in account, as an `npub`. `None` when signed out.
    pub account: Option<String>,
    /// The relays to read from, as `wss://` URLs.
    pub relays: Vec<String>,
    /// How much is logged.
    pub log_level: LogLevel,
    /// How large the text of notes is set. Read through
    /// [`Config::note_text_size`]: the file is anyone's to edit.
    pub note_text_size: u16,
    /// Which notes the feed and the profiles show: with replies, or not.
    pub feed_mode: FeedMode,
    /// For how many days the event store keeps other people's notes and
    /// what was made of them (see [`crate::nostr::sweep`]). `0` keeps them
    /// for good.
    pub keep_days: u32,
    /// The emoji reacted with, how often and when last: what is offered
    /// first to react with (see [`crate::emoji`]). Kept on this machine
    /// only.
    pub reaction_emojis: Vec<Used>,
    /// When each account last looked at its notifications, in seconds
    /// since the Unix epoch, by `npub`: what was written after is new to
    /// it. Kept on this machine only.
    pub notifications_seen: BTreeMap<String, u64>,
}

impl Config {
    /// How large the text of notes is set, within what can be read.
    pub fn note_text_size(&self) -> u16 {
        self.note_text_size.clamp(NOTE_TEXT_SIZE_MIN, NOTE_TEXT_SIZE_MAX)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            account: None,
            relays: DEFAULT_RELAYS.map(String::from).to_vec(),
            log_level: LogLevel::default(),
            note_text_size: NOTE_TEXT_SIZE_DEFAULT,
            feed_mode: FeedMode::default(),
            keep_days: KEEP_DAYS_DEFAULT,
            reaction_emojis: Vec::new(),
            notifications_seen: BTreeMap::new(),
        }
    }
}

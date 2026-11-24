#![no_main]

use libfuzzer_sys::fuzz_target;

use cosmic_applet_avestruz::nostr::content::{self, FEED_NOTE_MAX_CHARS};

fuzz_target!(|data: &str| {
    // A note's text is whatever its author sent, and anyone can be an
    // author. Its first line here stands for an `imeta` tag, fields apart
    // by tabs: what the note says of what it links to, as untrusted as
    // the text. Neither may make the parser panic.
    let (tag, text) = data.split_once('\n').unwrap_or(("", data));
    let tag: Vec<String> = tag.split('\t').map(str::to_string).collect();
    let attached: Vec<_> = content::imeta(&tag).into_iter().collect();
    let _ = content::parse_attached(text, FEED_NOTE_MAX_CHARS, &attached);
});

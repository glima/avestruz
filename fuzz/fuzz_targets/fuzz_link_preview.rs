#![no_main]

use libfuzzer_sys::fuzz_target;

use cosmic_applet_avestruz::previews;

fuzz_target!(|data: &str| {
    // The head of a page a note links to: fetched from wherever the note
    // says, so it is whatever that server chose to send. Reading it for a
    // title, a description and a picture must never panic.
    let Ok(url) = url::Url::parse("https://example.com/a/page") else {
        return;
    };
    let _ = previews::parse(data, &url);
});

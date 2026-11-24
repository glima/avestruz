#![no_main]

use libfuzzer_sys::fuzz_target;

use cosmic_applet_avestruz::nostr::relay_info;

fuzz_target!(|data: &str| {
    // The document a relay answers with when asked what it is (NIP-11):
    // JSON from a server the account may never have heard of. Reading it
    // must never panic.
    let _ = relay_info::parse(data);
});

#![no_main]

use libfuzzer_sys::fuzz_target;

use cosmic_applet_avestruz::media;

fuzz_target!(|data: &[u8]| {
    // A file picked or pasted to attach to a note. A JPEG, a PNG or a
    // WebP is taken apart byte by byte to leave out what it says beside
    // the picture, and what it is is told by how it starts, not by its
    // name: so any bytes at all may get there, and none may make it
    // panic.
    let _ = media::prepare("attached".to_string(), data.to_vec());
});

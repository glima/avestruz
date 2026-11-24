#![no_main]

use libfuzzer_sys::fuzz_target;

use cosmic_applet_avestruz::youtube;

fuzz_target!(|data: &[u8]| {
    // A link in a note that may be to a YouTube video, and what YouTube
    // answers when asked what a video is called. A video's id goes on
    // from here to yt-dlp's command line, so what is taken for one has to
    // be one; and neither reading may panic.
    let _ = youtube::title(data);
    if let Ok(link) = std::str::from_utf8(data)
        && let Some(video) = youtube::video_at(link)
    {
        let page = video.page();
        let id = page.strip_prefix("https://www.youtube.com/watch?v=").unwrap_or_default();
        assert!(id.len() == 11 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'), "{page}");
    }
});

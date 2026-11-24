// SPDX-License-Identifier: MIT

//! YouTube videos: telling a link to one from any other, and finding what
//! there is to play of it.
//!
//! A link to YouTube is to a page, not to a file. In a browser the page
//! plays the video with a player of YouTube's own, which is a program that
//! runs in a web engine, and this client has none. What it has is
//! GStreamer (see [`crate::video`]), which plays a stream given its
//! address. That address is not on the page for the reading: it is worked
//! out by running what YouTube sends, and YouTube changes how every few
//! weeks. Keeping up with that is the whole of what
//! [yt-dlp](https://github.com/yt-dlp/yt-dlp) does, so it is asked: run as
//! a program of its own, if it is installed, when a video is played and
//! not before.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant row as The video's row
//!     participant videos as Videos, in the model
//!     participant ytdlp as yt-dlp, a program of its own
//!     participant youtube as YouTube
//!     participant gst as GStreamer: playbin
//!     row->>videos: pressed: a YouTube link
//!     videos-->>row: on its way (the row says so)
//!     videos->>ytdlp: which stream of the video with this id?
//!     ytdlp->>youtube: the page, and what it takes to read it
//!     youtube-->>ytdlp: the streams there are
//!     ytdlp-->>videos: the address of one with pictures and sound
//!     videos->>gst: play this address
//!     gst->>youtube: the stream
//! ```
//!
//! yt-dlp is run as its user has set it up: its own configuration file
//! holds here as at a terminal, which is where whatever it needs to get
//! past YouTube on this machine is said (a JavaScript runtime, cookies, a
//! proxy). If `yt-dlp --get-url` finds a video at a terminal, it is found
//! here; and if not, what yt-dlp says is shown where the video would have
//! played.
//!
//! Nothing a note says reaches yt-dlp's command line but the eleven
//! characters of a video's id, and those only once they are known to be
//! letters, figures, `-` and `_` (see [`VideoId`]).
//!
//! A stream's address is not good the moment it is given: YouTube turns
//! away whoever asks for it before a time it names, a few seconds on,
//! with a 403. yt-dlp is told that time and waits it out before it
//! downloads anything; so it is asked for it here, along with the
//! address, and waited for before the stream is handed to be played.
//!
//! One address is asked for, since that is what a `playbin` plays: one
//! file with both the pictures and the sound in it. YouTube keeps such a
//! file only in a small size (360 lines), and its larger sizes as
//! pictures and sound apart; for a row of a popup that is enough, and the
//! link opens the video in the browser for more.
//!
//! Of a video that is live there is no such file. It comes as HLS: its
//! pictures in one run of short files and its sound in another, in
//! several sizes, all listed in one playlist. That playlist is one
//! address too, and GStreamer plays what it lists, pictures with sound;
//! so where there is no file, it is the playlist that is asked for (see
//! [`FORMAT`] and [`found`]).
//!
//! ```mermaid
//! flowchart TD
//!     asked["yt-dlp is asked for a stream"] --> file{"Is there one file of pictures and sound?"}
//!     file -->|"yes: a video that was put up"| address["Its address is played"]
//!     file -->|"no: a video that is live"| hls{"Are there pictures that come as HLS?"}
//!     hls -->|"yes"| playlist["The playlist that lists them, and their sound, is played"]
//!     hls -->|"no"| said["What yt-dlp says is shown where the video would have played"]
//! ```

use std::process::Stdio;
use std::time::Duration;

use reqwest::Url;

use crate::helpers::unix_now;

/// How long yt-dlp gets to find a stream. It answers in a few seconds when
/// it answers.
const FIND_TIMEOUT: Duration = Duration::from_secs(40);

/// How long a stream is waited for to be good to fetch, at the most. It
/// is a few seconds; a clock that is wrong must not make it an hour.
const AVAILABLE_WAIT_MAX: Duration = Duration::from_secs(15);

/// What yt-dlp is asked to write before the stream's address: when the
/// stream is good to fetch from, in seconds since the Unix epoch, or `NA`
/// if YouTube named no time.
const AVAILABLE_AT: &str = "available_at=%(available_at)s";

/// What yt-dlp is asked to write of the stream it chose: what its sound
/// is in, which is `none` if it has none.
const SOUND: &str = "sound=%(acodec)s";

/// And the playlist the stream is listed in, with the others there are of
/// the video, if it is listed in one; `NA` if not.
const LISTED_IN: &str = "listed_in=%(manifest_url)s";

/// The stream asked for: the best that is one file of pictures and sound,
/// no taller than a popup has any use for. Failing any such file, as of a
/// video that is live, pictures that come as HLS: not to be played
/// themselves, having no sound, but for the playlist they are listed in.
const FORMAT: &str = "b[height<=720]/b/bv[protocol^=m3u8]";

/// How many characters a video's id has.
const ID_CHARS: usize = 11;

/// The proportions of a YouTube short, which is held upright, in pixels as
/// they come.
const SHORT_SIZE: (u32, u32) = (1080, 1920);

/// What YouTube knows a video by: eleven letters, figures, `-` or `_`.
/// There is no making one of anything else, so one can be handed to a
/// command line, or written into an address, as it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VideoId(String);

impl VideoId {
    /// `id` as a video's id, if it is shaped like one.
    fn parse(id: &str) -> Option<Self> {
        let shaped = id.len() == ID_CHARS && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        // The one word of eleven letters that stands where an id does, in
        // the address of an embedded playlist.
        (shaped && id != "videoseries").then(|| Self(id.to_string()))
    }
}

/// A YouTube video, as a link names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    pub id: VideoId,
    /// Its size in pixels, as far as the link gives it away: a short is
    /// upright, and anything else is left to be what most videos are.
    pub size: Option<(u32, u32)>,
}

impl Video {
    /// The address of its page, with nothing in it but which video it is:
    /// not what a link carried besides, which says who shared it.
    pub fn page(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id.0)
    }

    /// The pictures YouTube keeps to stand for it, the better first: one
    /// as large as a popup can show, which only videos of some size have,
    /// and a small one, which every video has. Both in the proportions
    /// videos are.
    pub fn posters(&self) -> Vec<String> {
        ["maxresdefault", "mqdefault"].map(|picture| format!("https://i.ytimg.com/vi/{}/{picture}.jpg", self.id.0)).into()
    }

    /// Where YouTube says what it is called, in a few hundred bytes of
    /// JSON (oEmbed): its page takes most of a megabyte to get to that.
    pub fn about(&self) -> String {
        // The address of its page, as one address says another: an id
        // has nothing in it that needs saying differently.
        format!("https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{}", self.id.0)
    }
}

/// The YouTube video `url` links to, if it links to one: its page, a short,
/// a live one, an embedded one, or the short address of any of them. Only
/// an `https` link is one, as with everything fetched. A channel, a
/// playlist or a search is not a video.
pub fn video(url: &Url) -> Option<Video> {
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    let host = ["www.", "m.", "music."].iter().find_map(|prefix| host.strip_prefix(prefix)).unwrap_or(host);
    let mut path = url.path_segments()?;
    let (first, second) = (path.next().unwrap_or_default(), path.next());
    let (id, short) = match (host, first, second) {
        ("youtu.be", id, None) => (id.to_string(), false),
        ("youtube.com", "watch", None) => (url.query_pairs().find(|(name, _)| name == "v")?.1.into_owned(), false),
        ("youtube.com", "shorts", Some(id)) => (id.to_string(), true),
        ("youtube.com", "live" | "embed" | "v", Some(id)) | ("youtube-nocookie.com", "embed", Some(id)) => {
            (id.to_string(), false)
        }
        _ => return None,
    };
    Some(Video { id: VideoId::parse(&id)?, size: short.then_some(SHORT_SIZE) })
}

/// The YouTube video the address `url` links to, if it is an address and
/// links to one (see [`video`]).
pub fn video_at(url: &str) -> Option<Video> {
    video(&Url::parse(url).ok()?)
}

/// What a video is called, in what YouTube answers when asked
/// [about](Video::about) it.
pub fn title(answer: &[u8]) -> Option<String> {
    let answer: serde_json::Value = serde_json::from_slice(answer).ok()?;
    answer.get("title")?.as_str().map(str::to_string)
}

/// Why no stream of a video was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindError {
    /// yt-dlp is not installed, or not where programs are looked for.
    NotInstalled,
    /// yt-dlp did not answer in [`FIND_TIMEOUT`].
    TimedOut,
    /// yt-dlp has no JavaScript runtime it may use, and YouTube gives up
    /// its streams to nothing that can't run what it sends. What yt-dlp
    /// says of that is a warning; the error after it is about something
    /// else, and no help.
    NoRuntime,
    /// yt-dlp could not: what it said, or what kept it from being run.
    Failed(String),
}

/// What went wrong, in what yt-dlp wrote to its error output. That it
/// has no JavaScript runtime, or may not use the one it has to solve what
/// YouTube sets it, if it warned of either: whatever error follows comes
/// of that. Otherwise the last of the lines it marks as errors, without
/// the mark, or else the last line it wrote at all.
fn said_wrong(errors: &str) -> FindError {
    let lines = || errors.lines().map(str::trim).filter(|line| !line.is_empty());
    let unprepared = |line: &str| line.starts_with("WARNING:") && (line.contains("JavaScript runtime") || line.contains("[jsc]"));
    if lines().any(unprepared) {
        return FindError::NoRuntime;
    }
    let marked = lines().rev().find_map(|line| line.strip_prefix("ERROR:"));
    FindError::Failed(marked.or_else(|| lines().next_back()).unwrap_or("it said nothing").trim().to_string())
}

/// What to play of what yt-dlp found, in what it wrote: the first line
/// that is an `https` address, which is the stream's. Its configuration
/// may have it write more. If the stream has no sound (see [`SOUND`]),
/// the playlist it is listed in instead (see [`LISTED_IN`]), which lists
/// the sound as well; and nothing if it is listed in none, rather than
/// pictures in silence.
fn found(output: &str) -> Option<String> {
    let lines = || output.lines().map(str::trim);
    let https = |line: &&str| Url::parse(line).is_ok_and(|url| url.scheme() == "https");
    if lines().any(|line| line == "sound=none") {
        return lines().find_map(|line| line.strip_prefix("listed_in=")).filter(https).map(str::to_string);
    }
    lines().find(https).map(str::to_string)
}

/// How long to wait before the stream yt-dlp found is asked for, in what
/// it wrote (see [`AVAILABLE_AT`]), it being `now` seconds since the Unix
/// epoch: until the time YouTube named, if it named one that is still to
/// come, and no longer than [`AVAILABLE_WAIT_MAX`].
fn wait_until_available(output: &str, now: u64) -> Duration {
    let named = output.lines().find_map(|line| line.trim().strip_prefix("available_at=")?.parse::<u64>().ok());
    Duration::from_secs(named.map_or(0, |at| at.saturating_sub(now))).min(AVAILABLE_WAIT_MAX)
}

/// Ask yt-dlp for an address at which `video` is to be had with both its
/// pictures and its sound, and answer with it once it is good to fetch
/// from. See the [module docs](self).
pub async fn find_stream(video: Video) -> Result<String, FindError> {
    let mut ytdlp = tokio::process::Command::new("yt-dlp");
    ytdlp
        .args(["--no-playlist", "--format", FORMAT])
        .args(["--print", AVAILABLE_AT, "--print", SOUND, "--print", LISTED_IN, "--print", "urls", "--"])
        .arg(video.page())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Not left running if it is given up on.
        .kill_on_drop(true);
    let answer = match tokio::time::timeout(FIND_TIMEOUT, ytdlp.output()).await {
        Err(_) => return Err(FindError::TimedOut),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(FindError::NotInstalled),
        Ok(Err(e)) => return Err(FindError::Failed(e.to_string())),
        Ok(Ok(answer)) => answer,
    };
    if !answer.status.success() {
        let errors = String::from_utf8_lossy(&answer.stderr);
        tracing::debug!("yt-dlp failed, saying: {errors}");
        return Err(said_wrong(&errors));
    }
    let output = String::from_utf8_lossy(&answer.stdout);
    let stream = found(&output).ok_or_else(|| FindError::Failed("it named no stream".to_string()))?;
    tokio::time::sleep(wait_until_available(&output, unix_now())).await;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::{AVAILABLE_WAIT_MAX, FindError, Video, VideoId, found, said_wrong, title, video_at, wait_until_available};
    use std::time::Duration;

    fn id_at(url: &str) -> Option<String> {
        video_at(url).map(|video| video.id.0)
    }

    #[test]
    fn a_youtube_video_is_known_by_its_id_however_its_link_is_written() {
        for link in [
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "https://youtube.com/watch?v=aqz-KE-bpKQ&t=42s",
            "https://m.youtube.com/watch?feature=share&v=aqz-KE-bpKQ",
            "https://music.youtube.com/watch?v=aqz-KE-bpKQ&list=PL123",
            "https://youtu.be/aqz-KE-bpKQ",
            "https://youtu.be/aqz-KE-bpKQ?si=Wh0Sh4redIt",
            "https://www.youtube.com/shorts/aqz-KE-bpKQ",
            "https://www.youtube.com/live/aqz-KE-bpKQ?feature=shared",
            "https://www.youtube.com/embed/aqz-KE-bpKQ",
            "https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ",
            "https://www.youtube.com/v/aqz-KE-bpKQ",
        ] {
            assert_eq!(id_at(link).as_deref(), Some("aqz-KE-bpKQ"), "{link}");
        }

        // A short is upright; nothing else gives its shape away.
        assert_eq!(video_at("https://www.youtube.com/shorts/aqz-KE-bpKQ").unwrap().size, Some((1080, 1920)));
        assert_eq!(video_at("https://youtu.be/aqz-KE-bpKQ").unwrap().size, None);
    }

    #[test]
    fn what_is_not_a_video_is_not_one() {
        for link in [
            "https://www.youtube.com/",
            "https://www.youtube.com/@BlenderOfficial",
            "https://www.youtube.com/playlist?list=PL6B3937A5D230E335",
            "https://www.youtube.com/results?search_query=bunny",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/watch?list=PL6B3937A5D230E335",
            "https://www.youtube.com/embed/videoseries?list=PL6B3937A5D230E335",
            "https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg",
            "https://youtu.be/",
            "https://youtu.be/aqz-KE-bpKQ/more",
            // Not YouTube, whatever it looks like.
            "https://example.com/watch?v=aqz-KE-bpKQ",
            "https://youtube.com.example.com/watch?v=aqz-KE-bpKQ",
            "https://notyoutu.be/aqz-KE-bpKQ",
            // Not fetched, so not played.
            "http://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "not an address",
        ] {
            assert_eq!(id_at(link), None, "{link}");
        }
    }

    #[test]
    fn only_what_is_shaped_like_an_id_is_one() {
        assert!(VideoId::parse("aqz-KE-bpKQ").is_some());
        assert!(VideoId::parse("_-_-_-_-_-_").is_some());
        for not in [
            "",
            "short",
            "twelve-chars",
            "aqz-KE-bpK",
            "aqz KE bpKQ",
            "aqz-KE-bpK;",
            "--exec=boom",
            "aqz-KE-bpK\n",
            "ééééééééééé",
        ] {
            assert!(VideoId::parse(not).is_none(), "{not:?}");
        }
        // What a link says beside the id reaches nothing that is asked.
        let shared = video_at("https://youtu.be/aqz-KE-bpKQ?si=Wh0Sh4redIt&t=1%20--exec%20boom").unwrap();
        assert_eq!(shared.page(), "https://www.youtube.com/watch?v=aqz-KE-bpKQ");
        assert_eq!(
            shared.about(),
            "https://www.youtube.com/oembed?format=json&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Daqz-KE-bpKQ"
        );
    }

    #[test]
    fn a_video_has_two_pictures_to_stand_for_it_the_better_first() {
        let video = Video { id: VideoId::parse("aqz-KE-bpKQ").unwrap(), size: None };
        assert_eq!(
            video.posters(),
            ["https://i.ytimg.com/vi/aqz-KE-bpKQ/maxresdefault.jpg", "https://i.ytimg.com/vi/aqz-KE-bpKQ/mqdefault.jpg"]
        );
    }

    #[test]
    fn youtube_says_what_a_video_is_called() {
        let answer = br#"{"title":"Big Buck Bunny 60fps 4K","author_name":"Blender","type":"video","thumbnail_url":"https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg"}"#;
        assert_eq!(title(answer).as_deref(), Some("Big Buck Bunny 60fps 4K"));
        assert_eq!(title(b"Bad Request"), None);
        assert_eq!(title(br#"{"title":7}"#), None);
        assert_eq!(title(b"{}"), None);
    }

    #[test]
    fn a_stream_is_waited_for_until_youtube_says_it_is_good() {
        let wrote = |available: &str| format!("available_at={available}\nhttps://rr2.googlevideo.com/videoplayback?expire=1\n");
        let now = 1_795_041_533;
        assert_eq!(wait_until_available(&wrote("1795041538"), now), Duration::from_secs(5));
        // Good already, or no time named: not waited for.
        assert_eq!(wait_until_available(&wrote("1795041533"), now), Duration::ZERO);
        assert_eq!(wait_until_available(&wrote("1795041000"), now), Duration::ZERO);
        assert_eq!(wait_until_available(&wrote("NA"), now), Duration::ZERO);
        assert_eq!(wait_until_available("https://rr2.googlevideo.com/videoplayback\n", now), Duration::ZERO);
        // A clock that is wrong, here or there, costs a moment at most.
        assert_eq!(wait_until_available(&wrote("1795045133"), now), AVAILABLE_WAIT_MAX);
        // The address is found beside it as it was alone.
        assert_eq!(found(&wrote("1795041538")).as_deref(), Some("https://rr2.googlevideo.com/videoplayback?expire=1"));
    }

    #[test]
    fn what_yt_dlp_writes_is_read_for_a_stream_or_for_what_went_wrong() {
        let stream = "https://rr2---sn-nx57ynsk.googlevideo.com/videoplayback?expire=1795058366&id=o-ACD";
        assert_eq!(found(&format!("{stream}\n")).as_deref(), Some(stream));
        // Its configuration may have it write more than was asked.
        assert_eq!(found(&format!("Big Buck Bunny\n  {stream}  \nhttps://second.example.com/\n")).as_deref(), Some(stream));
        assert_eq!(found(""), None);
        assert_eq!(found("NA\nhttp://plain.example.com/video\n"), None, "only https is played");

        // A video that is live: its pictures have no sound, and the
        // playlist they are listed in is what is played.
        let pictures = "https://manifest.googlevideo.com/api/manifest/hls_playlist/itag/311/playlist/index.m3u8";
        let playlist = "https://manifest.googlevideo.com/api/manifest/hls_variant/id/TNHAtXvAbEo.1/file/index.m3u8";
        let live = |sound: &str, listed_in: &str| format!("available_at=NA\nsound={sound}\nlisted_in={listed_in}\n{pictures}\n");
        assert_eq!(found(&live("none", playlist)).as_deref(), Some(playlist));
        // With sound, it is played itself, whatever it is listed in.
        assert_eq!(found(&live("mp4a.40.2", playlist)).as_deref(), Some(pictures));
        assert_eq!(found(&live("NA", "NA")).as_deref(), Some(pictures));
        // Pictures in silence are not played.
        assert_eq!(found(&live("none", "NA")), None);
        assert_eq!(found(&live("none", "http://plain.example.com/index.m3u8")), None);

        let failed = |why: &str| FindError::Failed(why.to_string());
        let errors = "WARNING: Unknown codec iamf.001.001.Opus\n\
                      ERROR: [youtube] aqz-KE-bpKQ: Sign in to confirm you're not a bot.\n";
        assert_eq!(said_wrong(errors), failed("[youtube] aqz-KE-bpKQ: Sign in to confirm you're not a bot."));
        assert_eq!(said_wrong("Traceback (most recent call last):\n  somewhere\nKeyError: 'n'\n"), failed("KeyError: 'n'"));
        assert_eq!(said_wrong("\n"), failed("it said nothing"));

        // With nothing to run what YouTube sends, the error it ends on is
        // about what came of that: the warning is what went wrong. As it
        // says it with no runtime, and with one it may not use.
        let none = "WARNING: [youtube] No supported JavaScript runtime could be found. Only deno is enabled by default\n\
                    ERROR: [youtube] aqz-KE-bpKQ: Requested format is not available. Use --list-formats\n";
        assert_eq!(said_wrong(none), FindError::NoRuntime);
        let unused = "WARNING: [youtube] [jsc] Remote component challenge solver script (node) was skipped.\n\
                      WARNING: [youtube] aqz-KE-bpKQ: n challenge solving failed: Some formats may be missing.\n\
                      ERROR: [youtube] aqz-KE-bpKQ: Requested format is not available.\n";
        assert_eq!(said_wrong(unused), FindError::NoRuntime);
    }
}

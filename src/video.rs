// SPDX-License-Identifier: MIT

//! Videos notes link to, played where the link was: in the note's row.
//!
//! Nothing of a video is fetched until its reader presses play: the server
//! it is on sees that request, a video is large, and it has sound. Then
//! GStreamer does the work. A `playbin` fetches the address and decodes
//! what it finds, the sound goes to the system's output, and the pictures
//! come here, each scaled to what a row can show and converted to RGBA,
//! through an `appsink`. The app looks for a new one thirty times a second
//! while a video plays (see [`TICK_PLAYING`]) and shows the latest.
//!
//! ```mermaid
//! flowchart LR
//!     url["An https address"] --> playbin["playbin: fetches, demuxes, decodes"]
//!     playbin -->|"sound"| audio["The system's audio output"]
//!     playbin -->|"pictures"| convert["videoconvert, videoscale: RGBA, no side over the limit"]
//!     convert --> sink["appsink: keeps the latest frame, drops the rest"]
//!     sink -->|"taken at each tick"| playback["Playback: the frame as an image, where it is, how long it is"]
//!     playback --> row["The row's video box"]
//! ```
//!
//! One video plays at a time, and it plays while it is on screen. The
//! rows of a list are built anew each time the popup is, and only those in
//! view: so [`Videos`] counts the times the popup is built, a row that
//! shows the video says so as it is built, and a video no row has shown
//! through a few builds of the popup, and for a moment, is held. That is
//! one rule for a row scrolled out of view and for another view shown in
//! the list's place. A closed popup is not built at all, and holds its
//! video as it closes.
//!
//! Whether a video's sound is off, and how loud it is, is asked of what
//! plays it, not kept here (see [`SOUND_EVERY`]): where the system
//! remembers how an app's sound was left, as PipeWire and PulseAudio do, a
//! video starts as the last one ended, muted if that was and as loud as
//! that was, and the mute button and its slider say so.
//!
//! A video plays at the speed its reader chose (see [`Speed`]), which
//! holds for the next one too until the applet is started again. A speed
//! is a seek: to where the video is, to play on from there at another
//! rate. Its sound keeps its pitch, where GStreamer has the element for
//! that (`scaletempo`).
//!
//! The room a video takes in its row is settled before anything of it is
//! known ([`box_size`]): as wide as the row, in the proportions the note
//! says it has, or those of most videos if it doesn't say. What plays is
//! fitted inside, so a row is the same height before, while and after.
//!
//! A link to YouTube is to a page, and what there is to play of it has to
//! be found first (see [`crate::youtube`]). Its [`Playback`] is there from
//! the press, saying it is on its way, and gets its player when the
//! stream is found.
//!
//! The address comes from whoever wrote the note. Only `https` is played,
//! and it is GStreamer that fetches it: its decoders read what a stranger
//! sent, which is one more reason nothing plays unasked.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use cosmic::iced::Size;
use cosmic::widget::image::Handle;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

use crate::youtube;

/// How often a video that plays is looked at for a new frame, and how
/// often one that doesn't, while the popup is open: it may still have a
/// frame to show after a seek, or something to say.
pub const TICK_PLAYING: Duration = Duration::from_millis(33);
pub const TICK_PAUSED: Duration = Duration::from_millis(250);

/// How long a video plays on with no row showing it, and through how
/// many builds of the popup, before it is held. Both, because neither
/// says it alone: a popup that took its time to draw has let a moment go
/// by without being built again, and one that is built several times
/// before it is drawn once has not been long about it.
const UNSEEN: Duration = Duration::from_millis(700);
const UNSEEN_VIEWS: u64 = 3;

/// How often a video is asked whether its sound is off, and how loud it
/// is. That is not the mute button's alone to say: the system may start a
/// video's sound as it left this app's last, off included, and a mixer
/// can turn it off, on, up or down at any time. The button and its slider
/// show what is so, a moment later at most.
const SOUND_EVERY: Duration = Duration::from_millis(250);

/// How loud a video is, as its reader is shown it and sets it: from `0.0`
/// to `1.0`, on the scale a system's mixer shows, where half way sounds
/// about half as loud. What plays it counts in amplitude, which is that
/// cubed: half way there is an eighth.
fn shown_volume(amplitude: f64) -> f64 {
    amplitude.clamp(0.0, 1.0).cbrt()
}

/// The amplitude that is shown as `shown` (see [`shown_volume`]).
fn amplitude(shown: f64) -> f64 {
    shown.clamp(0.0, 1.0).powi(3)
}

/// How fast a video plays: `1` is as it was made. One of [`Speed::ALL`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed(f64);

impl Default for Speed {
    fn default() -> Self {
        Self(1.0)
    }
}

impl Speed {
    /// The speeds there are to choose from, the slowest first.
    pub const ALL: [Self; 6] = [Self(0.5), Self(0.75), Self(1.0), Self(1.25), Self(1.5), Self(2.0)];

    /// As a player says it: `1×`, `1.25×`.
    pub fn label(self) -> String {
        format!("{}\u{d7}", self.0)
    }

    /// Whether this is the speed a video was made to play at.
    pub fn is_usual(self) -> bool {
        self == Self::default()
    }
}

/// How loud a video is, as its button says it at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loudness {
    /// Its sound is off, or turned all the way down.
    Off,
    Low,
    Medium,
    High,
}

/// The most pixels a frame has on either side when it is handed over:
/// more than the popup shows at any scale, and far fewer than a video may
/// have. Frames are copied thirty times a second.
const FRAME_MAX_SIDE: i32 = 960;

/// The most bits a second taken of a video that comes in several sizes
/// to choose from, as the playlist of a live one lists them: about what
/// one 720 lines tall takes. Its pictures are shown no larger than
/// [`FRAME_MAX_SIDE`], so the sizes over that would be fetched and
/// decoded to be scaled down.
const STREAM_BITRATE_MAX: u32 = 3_500_000;

/// The proportions a video is given room for when its note doesn't say
/// its own: what most videos have.
const USUAL_ASPECT: (u32, u32) = (16, 9);

/// The room a video takes in a row `width` wide, when its note says it is
/// `size` pixels, or doesn't say: the whole width, as tall as its
/// proportions make that, and never taller than it is wide. A video taller
/// than that plays between bars, with its controls still in view.
pub fn box_size(size: Option<(u32, u32)>, width: f32) -> Size {
    let (across, down) = size.filter(|(across, down)| *across > 0 && *down > 0).unwrap_or(USUAL_ASPECT);
    let height = width * down as f32 / across as f32;
    Size::new(width, height.min(width))
}

/// `seconds` as a player says it: `0:07`, `12:34`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let seconds = if seconds.is_finite() && seconds > 0.0 { seconds as u64 } else { 0 };
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// One picture of a video: RGBA, row after row, with nothing between the
/// rows.
struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The picture `sample` carries, copied out row by row: a row may be
/// longer in the buffer than the pixels it holds.
fn frame_of(sample: &gst::Sample) -> Option<Frame> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(sample.buffer()?, &info).ok()?;
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let stride = usize::try_from(*frame.plane_stride().first()?).ok()?;
    let row = width * 4;
    if row == 0 || stride < row {
        return None;
    }
    let mut rgba = Vec::with_capacity(row * height);
    for line in frame.plane_data(0).ok()?.chunks(stride).take(height) {
        rgba.extend_from_slice(line.get(..row)?);
    }
    (rgba.len() == row * height).then_some(Frame { width: frame.width(), height: frame.height(), rgba })
}

/// GStreamer, started once. What it said if it could not be: there is no
/// second try in one run.
fn ready() -> Result<(), String> {
    static READY: OnceLock<Result<(), String>> = OnceLock::new();
    READY.get_or_init(|| gst::init().map_err(|e| e.to_string())).clone()
}

/// Where a video's pictures go: converted to RGBA with square pixels,
/// scaled down until neither side is over [`FRAME_MAX_SIDE`], and the
/// latest kept in `latest` for whoever takes it. A frame nobody took is
/// dropped for the next: what is shown is what is current.
fn picture_sink(latest: Arc<Mutex<Option<Frame>>>) -> Result<gst::Element, String> {
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Rgba)
        .pixel_aspect_ratio(gst::Fraction::new(1, 1))
        .width_range(1..=FRAME_MAX_SIDE)
        .height_range(1..=FRAME_MAX_SIDE)
        .build();
    let sink = gst_app::AppSink::builder().caps(&caps).max_buffers(1).drop(true).build();
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(frame) = frame_of(&sample) {
                    *latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let said = |e: &dyn std::fmt::Display| format!("the video's pictures have nowhere to go: {e}");
    let make = |name: &str| gst::ElementFactory::make(name).build().map_err(|e| said(&e));
    let (convert, scale) = (make("videoconvert")?, make("videoscale")?);
    let bin = gst::Bin::new();
    let parts = [&convert, &scale, sink.upcast_ref()];
    bin.add_many(parts).map_err(|e| said(&e))?;
    gst::Element::link_many(parts).map_err(|e| said(&e))?;
    let entry = convert.static_pad("sink").ok_or_else(|| said(&"no way in"))?;
    let entry = gst::GhostPad::with_target(&entry).map_err(|e| said(&e))?;
    bin.add_pad(&entry).map_err(|e| said(&e))?;
    Ok(bin.upcast())
}

/// What a player has to say for itself, when it is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Happened {
    /// It played to its end.
    Ended,
    /// It can't go on: why, as GStreamer says it.
    Failed(String),
}

/// One video, being played by GStreamer. Dropping it stops it.
struct Player {
    playbin: gst::Element,
    latest: Arc<Mutex<Option<Frame>>>,
}

impl Player {
    /// Start playing the video at `url`, which has to be an `https`
    /// address: nothing else is fetched, and no file of this machine is
    /// played because a note names it.
    fn open(url: &str) -> Result<Self, String> {
        let https = reqwest::Url::parse(url).is_ok_and(|url| url.scheme() == "https");
        if !https {
            return Err("not an https address".to_string());
        }
        Self::start(url)
    }

    /// Start playing what is at `uri`, whatever it is.
    fn start(uri: &str) -> Result<Self, String> {
        ready()?;
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin").build())
            .map_err(|e| e.to_string())?;
        let latest: Arc<Mutex<Option<Frame>>> = Arc::default();
        playbin.set_property("uri", uri);
        playbin.set_property("video-sink", picture_sink(latest.clone())?);
        // Played faster or slower, its sound keeps its pitch, if there is
        // what does that; and changes it, as a record does, if not.
        if let Ok(tempo) = gst::ElementFactory::make("scaletempo").build() {
            playbin.set_property("audio-filter", &tempo);
        }
        // Its `flags` are left as they come. Asked for pictures and sound
        // and nothing else, it never gets as far as having either.
        // Whatever fetches it says who is asking as everything else
        // fetched here does.
        playbin.connect("source-setup", false, |values| {
            if let Some(source) = values.get(1).and_then(|value| value.get::<gst::Element>().ok())
                && source.has_property("user-agent")
            {
                source.set_property("user-agent", concat!("avestruz/", env!("CARGO_PKG_VERSION")));
            }
            None
        });
        // What comes in several sizes, as a live video does, is taken in
        // one that a row can show: whatever chooses among them is told
        // the most to choose.
        playbin.connect("element-setup", false, |values| {
            if let Some(element) = values.get(1).and_then(|value| value.get::<gst::Element>().ok())
                && element.find_property("max-bitrate").is_some_and(|most| most.value_type() == u32::static_type())
            {
                element.set_property("max-bitrate", STREAM_BITRATE_MAX);
            }
            None
        });
        let player = Self { playbin, latest };
        player.playbin.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
        Ok(player)
    }

    fn play(&self) {
        let _ = self.playbin.set_state(gst::State::Playing);
    }

    fn pause(&self) {
        let _ = self.playbin.set_state(gst::State::Paused);
    }

    fn set_muted(&self, muted: bool) {
        self.playbin.set_property("mute", muted);
    }

    /// Whether its sound is off, as whatever plays the sound has it.
    fn muted(&self) -> bool {
        self.playbin.property("mute")
    }

    /// Make it as loud as `amplitude`, of `1.0` at the most.
    fn set_volume(&self, amplitude: f64) {
        self.playbin.set_property("volume", amplitude);
    }

    /// How loud it is, in amplitude, as whatever plays the sound has it.
    fn volume(&self) -> f64 {
        self.playbin.property("volume")
    }

    /// Go to `seconds` from the start, and there exactly, and play on from
    /// there at `speed`. Going to the picture nearest before it that
    /// stands on its own would be quicker, and in a short video that may
    /// be the first one: wherever it was dragged, it would start over.
    /// Returns whether it could.
    fn seek(&self, seconds: f64, speed: Speed) -> bool {
        let to = gst::ClockTime::from_mseconds((seconds.max(0.0) * 1000.0) as u64);
        let flags = gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE;
        self.playbin.seek(speed.0, flags, gst::SeekType::Set, to, gst::SeekType::None, gst::ClockTime::NONE).is_ok()
    }

    /// How far in it is, in seconds, if it knows.
    fn position(&self) -> Option<f64> {
        self.playbin.query_position::<gst::ClockTime>().map(|at| at.mseconds() as f64 / 1000.0)
    }

    /// How long it is, in seconds, if it knows yet.
    fn duration(&self) -> Option<f64> {
        self.playbin.query_duration::<gst::ClockTime>().map(|all| all.mseconds() as f64 / 1000.0)
    }

    /// The newest picture, if there is one since the last was taken.
    fn take_frame(&self) -> Option<Frame> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Hear what it has said since it was last asked: that it ended, or
    /// that it failed. While it is short of what to play it is held, and
    /// let go again, if it is `playing`, when it has enough.
    fn poll(&self, playing: bool) -> Option<Happened> {
        let bus = self.playbin.bus()?;
        let mut happened = None;
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Eos(_) => happened = happened.or(Some(Happened::Ended)),
                gst::MessageView::Error(error) => return Some(Happened::Failed(error.error().to_string())),
                gst::MessageView::Buffering(buffering) if buffering.percent() < 100 => self.pause(),
                gst::MessageView::Buffering(_) if playing => self.play(),
                _ => {}
            }
        }
        happened
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Stopping may wait on the network to let go: not on the thread
        // the window is drawn on.
        let playbin = self.playbin.clone();
        std::thread::spawn(move || {
            let _ = playbin.set_state(gst::State::Null);
        });
    }
}

/// The video that plays, or did last, as the model keeps it and its row
/// shows it.
pub struct Playback {
    /// Which video it is: its address.
    url: String,
    /// What plays it. `None` once it failed, if it never started, or
    /// while what there is to play of it is looked for.
    player: Option<Player>,
    /// What there is to play of it is being looked for: it is a page
    /// that shows a video, not a file.
    awaited: bool,
    /// Its latest picture, if it has had one.
    pub frame: Option<Handle>,
    /// It is meant to be playing, whether or not it has anything to play
    /// yet.
    pub playing: bool,
    /// It played to its end.
    pub ended: bool,
    /// Its sound is off, as it was when last asked (see [`SOUND_EVERY`]).
    pub muted: bool,
    /// How loud it is, as shown (see [`shown_volume`]), when last asked.
    pub volume: f64,
    /// The pointer is over its mute button, or over the slider beside
    /// it that sets how loud it is.
    sound_over: bool,
    /// That slider is being dragged, wherever the pointer has got to.
    sound_dragged: bool,
    /// When that was.
    sound_asked: Instant,
    /// How fast it is to play, and how fast what plays it was last told
    /// to: it can only be told once it has something to play.
    speed: Speed,
    speed_told: Speed,
    /// How far in it is, in seconds.
    pub position: f64,
    /// How long it is, in seconds, once it is known.
    pub duration: Option<f64>,
    /// Where its reader is dragging it to, while they are.
    pub seeking: Option<f64>,
    /// Why it can't be played, as GStreamer says it.
    pub error: Option<String>,
    /// When a row last showed it, and which build of the popup that was.
    seen: Cell<(Instant, u64)>,
}

impl Playback {
    /// Start playing the video at `url`. One that can't be started says
    /// why, where it would have played.
    pub fn start(url: &str) -> Self {
        match Player::open(url) {
            Ok(player) => Self::new(url, Some(player), None),
            Err(e) => {
                tracing::warn!("a video could not be played: {e}");
                Self::new(url, None, Some(e))
            }
        }
    }

    /// The video at `url`, to play once what there is to play of it has
    /// been found (see [`Playback::found`]): on its way until then.
    fn awaiting(url: &str) -> Self {
        Self { awaited: true, playing: true, ..Self::new(url, None, None) }
    }

    /// The video at `url`, played by `player`, or not played for `error`.
    fn new(url: &str, player: Option<Player>, error: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            playing: player.is_some(),
            player,
            awaited: false,
            frame: None,
            ended: false,
            muted: false,
            volume: 1.0,
            sound_over: false,
            sound_dragged: false,
            sound_asked: Instant::now(),
            speed: Speed::default(),
            speed_told: Speed::default(),
            position: 0.0,
            duration: None,
            seeking: None,
            error,
            seen: Cell::new((Instant::now(), 0)),
        }
    }

    /// A playback of the video at `url` with nothing playing it: what the
    /// model keeps and a row shows, without GStreamer.
    #[cfg(test)]
    pub(crate) fn held(url: &str) -> Self {
        Self { url: url.to_string(), error: None, ..Self::start("not an address") }
    }

    /// A row shows it, in build `view` of the popup.
    fn seen(&self, view: u64) {
        self.seen.set((Instant::now(), view));
    }

    /// Whether no row has shown it for a while, the popup having been
    /// built `views` times so far.
    fn unseen(&self, views: u64) -> bool {
        let (when, view) = self.seen.get();
        when.elapsed() > UNSEEN && views.saturating_sub(view) >= UNSEEN_VIEWS
    }

    /// Whether it is waiting for its first picture.
    pub fn loading(&self) -> bool {
        self.playing && self.frame.is_none() && self.error.is_none()
    }

    /// How often it is to be looked at, while the popup is `open`: often
    /// while it plays or is on its way to, now and then while it is on
    /// show and doesn't, and not at all once it has failed or is paused
    /// out of sight.
    pub fn tick_every(&self, open: bool) -> Option<Duration> {
        match (self.playing, open && self.player.is_some()) {
            (true, _) => Some(TICK_PLAYING),
            (false, true) => Some(TICK_PAUSED),
            (false, false) => None,
        }
    }

    /// Hold it where it is.
    pub fn pause(&mut self) {
        self.playing = false;
        if let Some(player) = &self.player {
            player.pause();
        }
    }

    /// What there is to play of it was found, at the address `stream`, or
    /// was not, and why: play it, unless it was held meanwhile, or say why
    /// it can't be.
    fn found(&mut self, stream: Result<String, String>) {
        self.awaited = false;
        match stream.and_then(|stream| Player::open(&stream)) {
            Ok(player) => {
                if !self.playing {
                    player.pause();
                }
                self.player = Some(player);
            }
            Err(e) => {
                tracing::warn!("a video could not be played: {e}");
                self.playing = false;
                self.error = Some(e);
            }
        }
    }

    /// Play it if it is held, from the start if it had ended; hold it if
    /// it plays. `view` is the build of the popup it was pressed in.
    fn toggle(&mut self, view: u64) {
        // Still looked for: what is changed is what it does when found.
        if self.awaited {
            self.playing = !self.playing;
            self.seen(view);
            return;
        }
        let Some(player) = &self.player else {
            return;
        };
        if self.playing {
            self.pause();
            return;
        }
        if std::mem::take(&mut self.ended) {
            player.seek(0.0, self.speed);
            self.position = 0.0;
        }
        self.playing = true;
        self.seen(view);
        player.play();
    }

    /// Turn its sound off if it is on, and on if it is off: as it is,
    /// not as it was last seen to be.
    pub fn toggle_mute(&mut self) {
        match &self.player {
            Some(player) => {
                self.muted = !player.muted();
                player.set_muted(self.muted);
                self.sound_asked = Instant::now();
            }
            None => self.muted = !self.muted,
        }
    }

    /// Make it as loud as `shown` (see [`shown_volume`]). Turned up from
    /// silence it is no longer muted: whoever moves the slider means to
    /// hear it.
    pub fn set_volume(&mut self, shown: f64) {
        self.volume = shown.clamp(0.0, 1.0);
        let heard = self.volume > 0.0 && self.muted;
        if heard {
            self.muted = false;
        }
        if let Some(player) = &self.player {
            player.set_volume(amplitude(self.volume));
            if heard {
                player.set_muted(false);
            }
            // Not asked again while it is being dragged: what was just
            // set is what is so.
            self.sound_asked = Instant::now();
        }
    }

    /// Whether the slider that sets how loud it is shows: while the
    /// pointer is over it or over the mute button, and until it is let go
    /// of if it is being dragged, which seldom stays on so small a thing.
    pub fn sound_shows(&self) -> bool {
        self.sound_over || self.sound_dragged
    }

    /// How loud it is, at a glance: for the picture on its mute button.
    pub fn loudness(&self) -> Loudness {
        match self.volume {
            _ if self.muted => Loudness::Off,
            volume if volume <= 0.0 => Loudness::Off,
            volume if volume < 0.34 => Loudness::Low,
            volume if volume < 0.67 => Loudness::Medium,
            _ => Loudness::High,
        }
    }

    /// Its reader let go of it at `seconds`: go there.
    pub fn seek(&mut self, seconds: f64) {
        self.seeking = None;
        self.ended = false;
        self.position = seconds;
        if let Some(player) = &self.player
            && player.seek(seconds, self.speed)
        {
            self.speed_told = self.speed;
        }
    }

    /// How fast it plays, or is to once it has something to play.
    pub fn speed(&self) -> Speed {
        self.speed
    }

    /// Look at it: take its newest picture, hear whether it ended or
    /// failed, note how far in it is, and hold it if no row has shown it
    /// for a while, the popup having been built `views` times so far.
    fn tick(&mut self, views: u64) {
        if self.playing && self.unseen(views) {
            self.pause();
        }
        let Some(player) = &self.player else {
            return;
        };
        if let Some(frame) = player.take_frame() {
            self.frame = Some(Handle::from_rgba(frame.width, frame.height, frame.rgba));
        }
        if self.duration.is_none() {
            self.duration = player.duration().filter(|all| *all > 0.0);
        }
        let at = player.position();
        if let Some(at) = at {
            self.position = at;
        }
        // Told how fast to play once it has a picture, which is when it
        // can be told anything; and where it is, so that it plays on from
        // there. One that can't change its speed stays at the one it has.
        // One with no length is live, and is not told: played faster it
        // would run out of what there is of it, and stall there.
        if self.speed != self.speed_told
            && self.frame.is_some()
            && self.duration.is_some()
            && let Some(at) = at
        {
            if player.seek(at, self.speed) {
                self.speed_told = self.speed;
            } else {
                tracing::warn!("a video would not play at {}", self.speed.label());
                self.speed = self.speed_told;
            }
        }
        if self.sound_asked.elapsed() >= SOUND_EVERY {
            self.muted = player.muted();
            self.volume = shown_volume(player.volume());
            self.sound_asked = Instant::now();
        }
        match player.poll(self.playing) {
            Some(Happened::Ended) => {
                self.playing = false;
                self.ended = true;
            }
            Some(Happened::Failed(why)) => {
                tracing::warn!("a video stopped playing ({}): {why}", self.url);
                self.playing = false;
                self.error = Some(why);
                self.player = None;
            }
            None => {}
        }
    }
}

/// The videos of the notes on show, as the model keeps them.
#[derive(Default)]
pub struct Videos {
    /// The one that plays, or did last.
    pub playing: Option<Playback>,
    /// The links that turned out to be videos when they were asked what
    /// they are, and were then played: each has the room of a video from
    /// then on, in place of the card it had.
    pub opened: HashSet<String>,
    /// How many times the popup was built to be drawn.
    views: Cell<u64>,
    /// How fast videos play: the one that plays, and those after it.
    speed: Speed,
    /// The menu of speeds is open, on the one that plays.
    speed_menu: bool,
}

impl Videos {
    /// The popup is being built, to be drawn.
    pub fn viewing(&self) {
        self.views.set(self.views.get() + 1);
    }

    /// The playback of the video at `url`, if that is the one that plays,
    /// or did last, for a row that shows it: which keeps it playing.
    pub fn shown(&self, url: &str) -> Option<&Playback> {
        let playback = self.playing.as_ref().filter(|playback| playback.url == url)?;
        playback.seen(self.views.get());
        Some(playback)
    }

    /// The play button of the video at `url` was pressed: hold it or let
    /// it go if it is the one that plays, and start it in place of that
    /// one if not. One that failed is started again.
    ///
    /// A YouTube video can't be started from its link: it is returned,
    /// for what there is to play of it to be found and told to
    /// [`Videos::found`], and is on its way until then.
    pub fn press(&mut self, url: &str) -> Option<youtube::Video> {
        match &mut self.playing {
            Some(playback) if playback.url == url && playback.error.is_none() => {
                playback.toggle(self.views.get());
                None
            }
            _ => {
                self.opened.insert(url.to_string());
                let page = youtube::video_at(url);
                let mut playback = if page.is_some() { Playback::awaiting(url) } else { Playback::start(url) };
                playback.speed = self.speed;
                playback.seen(self.views.get());
                self.playing = Some(playback);
                page
            }
        }
    }

    /// What there is to play of the video at `url` was found, or was not,
    /// and why. Nothing comes of it if that video is no longer the one to
    /// play, or is played already.
    pub fn found(&mut self, url: &str, stream: Result<String, String>) {
        if let Some(playback) = self.playing.as_mut().filter(|playback| playback.url == url && playback.awaited) {
            playback.found(stream);
        }
    }

    /// How fast videos play.
    pub fn speed(&self) -> Speed {
        self.speed
    }

    /// Play at `speed` from now on: the one that plays, and those after
    /// it. The menu it was chosen from is put away.
    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        self.speed_menu = false;
        if let Some(playback) = &mut self.playing {
            playback.speed = speed;
        }
    }

    /// Whether the menu of speeds is open.
    pub fn speed_menu(&self) -> bool {
        self.speed_menu
    }

    /// Open the menu of speeds if it is closed, and close it if it is
    /// open.
    pub fn toggle_speed_menu(&mut self) {
        self.speed_menu = !self.speed_menu;
    }

    /// Put the menu of speeds away.
    pub fn close_speed_menu(&mut self) {
        self.speed_menu = false;
    }

    /// Hold the one that plays, if one does: the popup closed. The
    /// pointer is over nothing of it any more, and no menu is open.
    pub fn pause(&mut self) {
        self.speed_menu = false;
        if let Some(playback) = &mut self.playing {
            (playback.sound_over, playback.sound_dragged) = (false, false);
            if playback.playing {
                playback.pause();
            }
        }
    }

    /// Stop the one that plays and forget it: whoever was watching left.
    pub fn stop(&mut self) {
        self.speed_menu = false;
        self.playing = None;
    }

    /// How often the one that plays is to be looked at, while the popup is
    /// `open`, if at all.
    pub fn tick_every(&self, open: bool) -> Option<Duration> {
        self.playing.as_ref()?.tick_every(open)
    }

    /// Look at the one that plays: for its newest picture, how far in it
    /// is, whether it ended or failed, and whether it is still on screen.
    pub fn tick(&mut self) {
        if let Some(playback) = &mut self.playing {
            playback.tick(self.views.get());
        }
    }

    /// Turn the sound of the one that plays off, or back on.
    pub fn toggle_mute(&mut self) {
        if let Some(playback) = &mut self.playing {
            playback.toggle_mute();
        }
    }

    /// The slider of the one that plays is being dragged to `shown`,
    /// from `0.0` to `1.0`: make it that loud.
    pub fn set_volume(&mut self, shown: f64) {
        if let Some(playback) = &mut self.playing {
            playback.sound_dragged = true;
            playback.set_volume(shown);
        }
    }

    /// That slider was let go.
    pub fn volume_set(&mut self) {
        if let Some(playback) = &mut self.playing {
            playback.sound_dragged = false;
        }
    }

    /// The pointer came over the sound controls of the one that plays, or
    /// left them (see [`Playback::sound_shows`]).
    pub fn sound_over(&mut self, over: bool) {
        if let Some(playback) = &mut self.playing {
            playback.sound_over = over;
        }
    }

    /// The one that plays is being dragged to `seconds` in: show that,
    /// and go nowhere until it is let go.
    pub fn seeking(&mut self, seconds: f64) {
        if let Some(playback) = &mut self.playing {
            playback.seeking = Some(seconds);
        }
    }

    /// It was let go: go where it was dragged to.
    pub fn seek(&mut self) {
        if let Some(playback) = &mut self.playing
            && let Some(seconds) = playback.seeking
        {
            playback.seek(seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FRAME_MAX_SIDE, Frame, Loudness, Playback, Player, Speed, UNSEEN, UNSEEN_VIEWS, Videos, amplitude, box_size, clock,
        picture_sink, ready, shown_volume,
    };
    use cosmic::iced::Size;
    use gstreamer as gst;
    use gstreamer::prelude::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn a_video_has_the_whole_width_and_is_never_taller_than_wide() {
        assert_eq!(box_size(Some((1920, 1080)), 480.0), Size::new(480.0, 270.0));
        assert_eq!(box_size(None, 480.0), Size::new(480.0, 270.0), "what most videos are, when the note doesn't say");
        assert_eq!(box_size(Some((0, 1080)), 480.0), Size::new(480.0, 270.0));
        // A small one is given the same room: it is scaled up to play.
        assert_eq!(box_size(Some((320, 240)), 480.0), Size::new(480.0, 360.0));
        // One held upright plays between bars.
        assert_eq!(box_size(Some((1080, 1920)), 480.0), Size::new(480.0, 480.0));
    }

    #[test]
    fn time_reads_as_a_player_says_it() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-3.0), "0:00");
        assert_eq!(clock(f64::NAN), "0:00");
    }

    #[test]
    fn how_loud_is_shown_as_a_mixer_shows_it() {
        // Half way on the slider is an eighth in amplitude, and back.
        assert!((amplitude(0.5) - 0.125).abs() < 1e-9);
        assert!((shown_volume(0.125) - 0.5).abs() < 1e-9);
        for shown in [0.0, 0.01, 0.33, 0.5, 0.8, 1.0] {
            assert!((shown_volume(amplitude(shown)) - shown).abs() < 1e-9, "{shown}");
        }
        // Louder than whole, as a mixer can make it, shows as whole; and
        // nothing is asked for that is not between nothing and whole.
        assert_eq!(shown_volume(1.7), 1.0);
        assert_eq!((amplitude(-1.0), amplitude(3.0)), (0.0, 1.0));
    }

    #[test]
    fn the_mute_button_says_how_loud_and_the_slider_unmutes() {
        let mut playback = Playback::held("https://example.com/a.mp4");
        assert_eq!((playback.volume, playback.loudness()), (1.0, Loudness::High));
        for (shown, loudness) in [(0.0, Loudness::Off), (0.2, Loudness::Low), (0.5, Loudness::Medium), (0.67, Loudness::High)] {
            playback.set_volume(shown);
            assert_eq!((playback.volume, playback.loudness()), (shown, loudness));
        }
        playback.set_volume(7.0);
        assert_eq!(playback.volume, 1.0);

        // Muted, it is off however loud it would be.
        playback.toggle_mute();
        assert!(playback.muted && playback.loudness() == Loudness::Off && playback.volume == 1.0);
        // Turned all the way down it stays muted; turned up, it is heard.
        playback.set_volume(0.0);
        assert!(playback.muted);
        playback.set_volume(0.4);
        assert!(!playback.muted && playback.loudness() == Loudness::Medium);
    }

    #[test]
    fn the_slider_of_the_sound_shows_under_the_pointer_and_while_it_is_dragged() {
        let mut videos = Videos::default();
        // With no video played there is nothing to show it for.
        videos.sound_over(true);
        videos.set_volume(0.5);
        assert!(videos.playing.is_none());

        videos.playing = Some(Playback::held("https://example.com/a.mp4"));
        let shows = |videos: &Videos| videos.playing.as_ref().is_some_and(Playback::sound_shows);
        assert!(!shows(&videos));
        videos.sound_over(true);
        assert!(shows(&videos));
        videos.sound_over(false);
        assert!(!shows(&videos));

        // Dragged, it stays though the pointer wanders off it, until it
        // is let go.
        videos.sound_over(true);
        videos.set_volume(0.5);
        videos.sound_over(false);
        assert!(shows(&videos));
        videos.set_volume(0.3);
        assert!(videos.playing.as_ref().is_some_and(|playback| playback.volume == 0.3));
        videos.volume_set();
        assert!(!shows(&videos));
        // Let go with the pointer still on it, it stays.
        videos.sound_over(true);
        videos.set_volume(0.6);
        videos.volume_set();
        assert!(shows(&videos));

        // The popup closing puts it away, whatever the pointer was doing.
        videos.set_volume(0.7);
        videos.pause();
        assert!(!shows(&videos));
    }

    #[test]
    fn a_youtube_video_is_on_its_way_until_what_there_is_to_play_of_it_is_found() {
        let link = "https://youtu.be/aqz-KE-bpKQ?si=Wh0Sh4redIt";
        let awaited = |videos: &Videos| videos.playing.as_ref().is_some_and(|playback| playback.awaited);
        let mut videos = Videos::default();

        // Pressed, it is the one to play, and what to look for is said.
        let asked = videos.press(link).unwrap();
        assert_eq!(asked.page(), "https://www.youtube.com/watch?v=aqz-KE-bpKQ");
        let playback = videos.shown(link).unwrap();
        assert!(playback.loading() && playback.error.is_none() && awaited(&videos));
        assert_eq!(videos.tick_every(true), Some(super::TICK_PLAYING), "looked at, for whether it is still on show");

        // Pressed again meanwhile, it is not looked for twice: it is held,
        // and let go again.
        assert_eq!(videos.press(link), None);
        assert!(videos.shown(link).is_some_and(|playback| !playback.playing && !playback.loading()));
        assert_eq!(videos.press(link), None);
        assert!(videos.shown(link).is_some_and(Playback::loading));

        // Not found: it says why, where it would have played.
        videos.found(link, Err("yt-dlp says no".to_string()));
        let failed = videos.shown(link).unwrap();
        assert!(!failed.playing && !awaited(&videos));
        assert_eq!(failed.error.as_deref(), Some("yt-dlp says no"));
        // Found late, after that, nothing comes of it.
        videos.found(link, Ok("https://example.com/stream".to_string()));
        assert_eq!(videos.shown(link).unwrap().error.as_deref(), Some("yt-dlp says no"));

        // Pressed again, it is looked for again. What is found that is not
        // an https address is not played.
        assert!(videos.press(link).is_some());
        videos.found(link, Ok("file:///etc/passwd".to_string()));
        assert_eq!(videos.shown(link).unwrap().error.as_deref(), Some("not an https address"));

        // Another video pressed meanwhile is the one to play: the answer
        // for the first is nobody's.
        assert!(videos.press(link).is_some());
        assert_eq!(videos.press("http://example.com/b.mp4"), None);
        videos.found(link, Err("too late".to_string()));
        assert!(videos.shown(link).is_none());
        assert_eq!(videos.shown("http://example.com/b.mp4").unwrap().error.as_deref(), Some("not an https address"));
    }

    #[test]
    fn a_speed_is_chosen_from_a_few_and_holds_for_the_next_video() {
        let labels: Vec<String> = Speed::ALL.iter().map(|speed| speed.label()).collect();
        assert_eq!(labels, ["0.5×", "0.75×", "1×", "1.25×", "1.5×", "2×"]);
        assert_eq!(Speed::ALL.iter().filter(|speed| speed.is_usual()).count(), 1);
        assert_eq!(Speed::default().label(), "1×");
        let [.., faster, _] = Speed::ALL;

        let mut videos = Videos::default();
        assert!(videos.speed().is_usual() && !videos.speed_menu());
        videos.playing = Some(Playback::held("https://example.com/a.mp4"));
        videos.toggle_speed_menu();
        assert!(videos.speed_menu());
        // Chosen, it is the speed of the one that plays, and the menu is
        // put away.
        videos.set_speed(faster);
        assert!(!videos.speed_menu());
        assert_eq!((videos.speed(), videos.playing.as_ref().unwrap().speed()), (faster, faster));

        // The next video plays at it too, a YouTube one included.
        assert_eq!(videos.press("http://example.com/b.mp4"), None);
        assert_eq!(videos.playing.as_ref().unwrap().speed(), faster);
        assert!(videos.press("https://youtu.be/aqz-KE-bpKQ").is_some());
        assert_eq!(videos.playing.as_ref().unwrap().speed(), faster);

        // The menu is put away by itself as the popup closes, and when
        // whoever was watching leaves.
        videos.toggle_speed_menu();
        videos.pause();
        assert!(!videos.speed_menu());
        videos.toggle_speed_menu();
        videos.toggle_speed_menu();
        assert!(!videos.speed_menu());
        videos.toggle_speed_menu();
        videos.stop();
        assert!(!videos.speed_menu() && videos.speed() == faster);
    }

    #[test]
    fn only_an_https_address_is_played() {
        for refused in ["http://example.com/a.mp4", "file:///etc/passwd", "rtsp://example.com/live", "a.mp4", ""] {
            assert_eq!(Player::open(refused).err().as_deref(), Some("not an https address"), "{refused}");
        }
        let refused = Playback::start("file:///etc/passwd");
        assert!(!refused.playing && refused.error.is_some() && refused.tick_every(true).is_none());
    }

    /// `videos`, with the one that plays last shown `ago` ago.
    fn shown_ago(videos: &Videos, ago: Duration) {
        let playback = videos.playing.as_ref().unwrap();
        let (_, view) = playback.seen.get();
        playback.seen.set((Instant::now().checked_sub(ago).unwrap(), view));
    }

    #[test]
    fn a_video_no_row_shows_is_held_and_one_that_is_shown_plays_on() {
        let url = "https://example.com/a.mp4";
        let long = UNSEEN + Duration::from_millis(50);
        let mut videos = Videos::default();
        videos.playing = Some(Playback::held(url));
        videos.playing.as_mut().unwrap().playing = true;
        let playing = |videos: &Videos| videos.playing.as_ref().unwrap().playing;

        // Shown in each build of the popup: it plays on, however long each
        // took to draw.
        for _ in 0..10 {
            videos.viewing();
            assert!(videos.shown(url).is_some());
            shown_ago(&videos, long);
            videos.tick();
            assert!(playing(&videos));
        }

        // The popup built again and again in no time, as it is when much
        // happens at once, and the row not yet: it plays on.
        assert!(videos.shown(url).is_some());
        for _ in 0..10 {
            videos.viewing();
        }
        videos.tick();
        assert!(playing(&videos), "not for long");

        // Not shown for a while, through a few builds: it is held. Another
        // video's row is not its own.
        assert!(videos.shown(url).is_some());
        shown_ago(&videos, long);
        for _ in 1..UNSEEN_VIEWS {
            videos.viewing();
            assert!(videos.shown("https://example.com/b.mp4").is_none());
            videos.tick();
            assert!(playing(&videos), "one build is not a while");
        }
        videos.viewing();
        videos.tick();
        assert!(!playing(&videos), "out of sight");

        // And stays so until its reader says otherwise.
        videos.viewing();
        assert!(videos.shown(url).is_some());
        videos.tick();
        assert!(!playing(&videos));
    }

    #[test]
    fn pressing_play_holds_the_video_that_plays_and_starts_any_other() {
        let mut videos = Videos::default();
        videos.playing = Some(Playback::held("https://example.com/a.mp4"));
        assert!(videos.shown("https://example.com/a.mp4").is_some());
        assert!(videos.shown("https://example.com/b.mp4").is_none());

        // Another one takes its place, and has the room of a video from
        // then on. (This one can't start: there is nothing at it.)
        videos.press("http://example.com/b.mp4");
        assert!(videos.shown("https://example.com/a.mp4").is_none());
        assert!(videos.shown("http://example.com/b.mp4").is_some_and(|playback| playback.error.is_some()));
        assert!(videos.opened.contains("http://example.com/b.mp4"));

        // A file is started at once, and nothing more is asked for.
        assert_eq!(videos.press("https://example.com/a.mp4"), None);

        // The popup closing holds the one that plays, and signing out
        // forgets it.
        videos.playing = Some(Playback::held("https://example.com/a.mp4"));
        videos.playing.as_mut().unwrap().playing = true;
        videos.pause();
        assert!(videos.playing.as_ref().is_some_and(|playback| !playback.playing));
        videos.stop();
        assert!(videos.playing.is_none());
    }

    /// Whether GStreamer is there, with the elements `needed`: without
    /// them a test of it has nothing to stand on, and passes for nothing.
    fn gstreamer_has(needed: &[&str]) -> bool {
        let there = ready().is_ok() && needed.iter().all(|name| gst::ElementFactory::find(name).is_some());
        if !there {
            eprintln!("GStreamer, or one of {needed:?}, is not installed: nothing was tested");
        }
        there
    }

    /// The first frame the picture sink hands over of a test picture
    /// `width` by `height`.
    fn first_frame_of(width: i32, height: i32) -> Frame {
        let latest: Arc<Mutex<Option<Frame>>> = Arc::default();
        let pipeline = gst::Pipeline::new();
        let source = gst::ElementFactory::make("videotestsrc").property("num-buffers", 3).build().unwrap();
        let caps = gst::Caps::builder("video/x-raw").field("width", width).field("height", height).build();
        let filter = gst::ElementFactory::make("capsfilter").property("caps", &caps).build().unwrap();
        let sink = picture_sink(latest.clone()).unwrap();
        pipeline.add_many([&source, &filter, &sink]).unwrap();
        gst::Element::link_many([&source, &filter, &sink]).unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();
        let bus = pipeline.bus().unwrap();
        let ended = bus.timed_pop_filtered(gst::ClockTime::from_seconds(10), &[gst::MessageType::Eos, gst::MessageType::Error]);
        pipeline.set_state(gst::State::Null).unwrap();
        assert!(matches!(ended.unwrap().view(), gst::MessageView::Eos(_)));
        latest.lock().unwrap().take().unwrap()
    }

    #[test]
    fn frames_come_as_rgba_scaled_down_to_fit_and_in_their_own_proportions() {
        if !gstreamer_has(&["videotestsrc", "videoconvert", "videoscale", "capsfilter"]) {
            return;
        }
        let side = FRAME_MAX_SIDE as u32;
        for ((width, height), expected) in [
            ((320, 240), (320, 240)),
            ((1920, 1080), (side, side * 9 / 16)),
            ((1080, 1920), (side * 9 / 16, side)),
            ((1001, 333), (side, 319)),
        ] {
            let frame = first_frame_of(width, height);
            assert_eq!((frame.width, frame.height), expected, "{width}x{height}");
            assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
            // The test picture is opaque, and not all one colour.
            assert!(frame.rgba.chunks(4).all(|pixel| pixel[3] == 255));
            assert!(frame.rgba.chunks(4).any(|pixel| pixel[..3] != frame.rgba[..3]));
        }
    }

    /// What it takes to make a test video and play it.
    const TO_PLAY: [&str; 7] = ["videotestsrc", "jpegenc", "avimux", "filesink", "playbin", "avidemux", "jpegdec"];

    /// `seconds` of a test picture, as a file called after `name`.
    fn test_video(name: &str, seconds: u32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("avestruz-test-{}-{name}.avi", std::process::id()));
        let making = format!(
            "videotestsrc num-buffers={} ! video/x-raw,width=320,height=240,framerate=30/1 ! jpegenc ! avimux ! filesink location={}",
            seconds * 30,
            path.display()
        );
        let made = gst::parse::launch(&making).unwrap();
        made.set_state(gst::State::Playing).unwrap();
        let bus = made.bus().unwrap();
        let done = bus.timed_pop_filtered(gst::ClockTime::from_seconds(20), &[gst::MessageType::Eos, gst::MessageType::Error]);
        made.set_state(gst::State::Null).unwrap();
        assert!(matches!(done.unwrap().view(), gst::MessageView::Eos(_)));
        path
    }

    /// Look at `playback` as the app looks at it, shown each time, until
    /// `until` says so; for ten seconds at the most.
    fn watch(playback: &mut Playback, until: &dyn Fn(&Playback) -> bool) {
        let started = Instant::now();
        while !until(playback) && started.elapsed() < Duration::from_secs(10) {
            playback.seen(0);
            playback.tick(0);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(until(playback), "within ten seconds");
    }

    #[test]
    fn a_video_plays_faster_or_slower_as_it_is_told_and_seeks_at_that_speed() {
        if !gstreamer_has(&TO_PLAY) {
            return;
        }
        let path = test_video("speed", 8);
        let [slower, .., faster] = Speed::ALL;
        let mut playback = Playback::held(&format!("file://{}", path.display()));
        playback.player = Some(Player::start(&playback.url).unwrap());
        playback.playing = true;
        playback.toggle_mute();

        // How far it gets in half a second of watching, for each second
        // that takes.
        let pace = |playback: &mut Playback| {
            let (started, from) = (Instant::now(), playback.position);
            watch(playback, &|_| started.elapsed() >= Duration::from_millis(500));
            (playback.position - from) / started.elapsed().as_secs_f64()
        };

        // Told before it has anything to play, it is told again once it
        // has, and plays on from where it was.
        playback.speed = faster;
        watch(&mut playback, &|playback| playback.speed_told == faster);
        assert!(playback.frame.is_some() && playback.error.is_none());
        let fast = pace(&mut playback);
        assert!((1.6..=2.4).contains(&fast), "twice as fast: {fast}");

        // Gone elsewhere in, it plays on as fast from there.
        playback.seek(1.0);
        watch(&mut playback, &|playback| playback.position >= 1.0 && playback.position < 3.0);
        let fast = pace(&mut playback);
        assert!((1.6..=2.4).contains(&fast), "still twice as fast: {fast}");

        playback.speed = slower;
        let before = playback.position;
        watch(&mut playback, &|playback| playback.speed_told == slower);
        let slow = pace(&mut playback);
        assert!((0.3..=0.7).contains(&slow), "half as fast: {slow}");
        assert!(playback.position >= before && playback.position < before + 1.5, "from where it was");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_video_plays_to_its_end_and_can_be_played_again() {
        if !gstreamer_has(&TO_PLAY) {
            return;
        }
        // A second of a test picture, as a file.
        let path = test_video("video", 1);

        let mut playback = Playback::held(&format!("file://{}", path.display()));
        playback.player = Some(Player::start(&playback.url).unwrap());
        playback.playing = true;
        playback.toggle_mute();
        assert!(playback.loading() && playback.tick_every(false).is_some());

        watch(&mut playback, &|playback| playback.ended);
        assert!(playback.frame.is_some() && !playback.playing && playback.error.is_none());
        // Its sound is as it was left: asked of what plays it since, more
        // than once, and not only remembered.
        assert!(playback.muted && playback.sound_asked.elapsed() < Duration::from_secs(1));
        playback.toggle_mute();
        assert!(!playback.muted);
        playback.toggle_mute();
        // How loud it is, likewise: set, and then heard back from what
        // plays it.
        playback.set_volume(0.5);
        watch(&mut playback, &|playback| playback.sound_asked.elapsed() < Duration::from_millis(50));
        assert!((playback.volume - 0.5).abs() < 0.01 && !playback.muted, "{}", playback.volume);
        playback.toggle_mute();
        assert!(playback.duration.is_some_and(|all| (0.9..=1.1).contains(&all)), "{:?}", playback.duration);
        assert_eq!(playback.tick_every(false), None, "nothing to look at while it is held out of sight");

        // Played again, it starts over.
        playback.toggle(0);
        assert!(playback.playing && !playback.ended && playback.position == 0.0);
        watch(&mut playback, &|playback| playback.ended);

        // What is not a video says so, where it would have played.
        std::fs::write(&path, "not a video").unwrap();
        let mut broken = Playback::held(&playback.url);
        broken.player = Some(Player::start(&broken.url).unwrap());
        broken.playing = true;
        watch(&mut broken, &|playback| playback.error.is_some());
        assert!(!broken.playing && broken.player.is_none());
        let _ = std::fs::remove_file(&path);
    }
}

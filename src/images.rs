// SPDX-License-Identifier: MIT

//! Pictures from the web: avatars, the images in notes, and the pictures
//! across profiles.
//!
//! A view asks the [`ImageStore`] for a picture by its address. If it is
//! there, the view gets it; if not, the view gets told it is loading, and the
//! store asks for it (see [`Requests`]): the app hears of it as a
//! [`Message::LoadImage`](crate::messages::Message::LoadImage). Only rows on
//! screen are ever built, so only pictures on screen are ever asked for.
//!
//! An avatar is asked for as a [`ImageShape::Circle`]: cut to its middle
//! square and made round in its own pixels when it is decoded. The renderer
//! could round and crop it at draw time, but iced's CPU renderer does neither
//! reliably for a picture that isn't square. The picture across a profile is
//! asked for as a [`ImageShape::Strip`] and the one beside a link's preview
//! as a [`ImageShape::Thumbnail`], and they are cut the same way for the same
//! reason.
//!
//! A picture is kept at the size it is shown at, in the screen's own
//! pixels, and not as it came (see [`sized`]). What draws a frame is a CPU
//! renderer, and scaling a picture is the dearest thing it does: a
//! photograph of twelve million pixels, scaled into a row on every frame of
//! a scrolling list, costs more than everything else in the frame put
//! together. Scaled once, when it is decoded, it is copied into the frame
//! pixel for pixel (see [`Picture`](crate::views::components::Picture)).
//! Nothing shows a picture larger than its row, so nothing is lost by it.
//!
//! What a picture takes is bounded three ways: by that size, by a limit on
//! how large one may decode before it is made smaller, and by a budget on
//! the bytes kept, with the pictures least recently shown dropped first. A
//! dropped picture is fetched again if it comes back into view; there is no
//! disk cache yet.
//!
//! Addresses come from whoever wrote a profile or a note. They are fetched
//! as everything is (see [`crate::http`]), and both the download and the
//! decoded picture are capped.
//!
//! ```mermaid
//! flowchart TD
//!     url["An address, and the shape wanted"] --> fetch["Fetched: https only, so many bytes at the most"]
//!     fetch --> decode["Decoded off the main thread, unless it would decode too large"]
//!     decode --> shape{"The shape wanted"}
//!     shape -->|"Original"| size
//!     shape -->|"Circle"| circle["Cut to its middle square, and made round"]
//!     shape -->|"Strip"| strip["Cut to its middle, three times as wide as tall"]
//!     shape -->|"Thumbnail"| thumb["Cut to its middle, in a thumbnail's proportions"]
//!     circle --> size
//!     strip --> size
//!     thumb --> size["Made the size it is shown at, in the screen's pixels"]
//!     size --> keep["Kept, by address and shape"]
//!     keep --> budget{"Over the budget of decoded bytes?"}
//!     budget -->|"yes"| drop["The ones least recently shown are dropped, until it is met"]
//!     budget -->|"no"| done["Shown"]
//!     drop --> done
//! ```
//!
//! | Limit | Value | Constant |
//! |---|---|---|
//! | One download | 16 MiB | [`MAX_DOWNLOAD_BYTES`] |
//! | One picture, decoded | 64 MiB | [`MAX_DECODED_BYTES`] |
//! | All pictures, as kept | 256 MiB | [`DEFAULT_BUDGET_BYTES`] |
//! | Fetches at once | 8 | [`MAX_IN_FLIGHT`] |
//!
//! A picture that failed is remembered as failed for the run, and is not
//! asked for again.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Cursor;

use cosmic::widget::image::Handle;
use tokio::sync::mpsc;

use crate::http;
use crate::requests::Requests;
use crate::views::components::{LIST_MAX_WIDTH, POPUP_MAX_WIDTH};

/// How large a picture may be to download.
const MAX_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;

/// How large a picture may be once decoded (four bytes a pixel): 4096×4096.
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

/// How many pictures may be on their way at once.
const MAX_IN_FLIGHT: usize = 8;

/// How many requests may wait between a view asking and the app answering.
/// A request that doesn't fit is dropped: a picture still on screen is asked
/// for again the next time the view is built.
const REQUEST_QUEUE: usize = 64;

/// How many decoded bytes the store keeps before dropping pictures.
pub const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// The form a picture is wanted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageShape {
    /// As it came.
    Original,
    /// Its middle square, round: an avatar.
    Circle,
    /// Its middle, [`STRIP_ASPECT`] times as wide as it is tall: the
    /// picture across a profile.
    Strip,
    /// Its middle, in the proportions of [`THUMBNAIL_ASPECT`]: the picture
    /// beside the preview of a link.
    Thumbnail,
}

/// How many times as wide as it is tall a [`ImageShape::Strip`] is: what
/// such pictures are made for.
pub const STRIP_ASPECT: u32 = 3;

/// How wide a [`ImageShape::Thumbnail`] is for how tall: near what pages
/// make the pictures they are shared with.
pub const THUMBNAIL_ASPECT: (u32, u32) = (16, 9);

/// How wide the largest avatar is shown, in points: the one on a profile.
pub const LARGEST_AVATAR: f32 = 64.0;

/// How wide the picture beside a link's preview is shown at the most, in
/// points. It is as wide as its card is tall allows, which goes by the
/// size text is set in: this is past what any size makes it.
const WIDEST_THUMBNAIL: f32 = 320.0;

/// How many of the screen's pixels a point is taken to be while the
/// screen has not said: more than most are, so that a picture made for it
/// is sharp on any. It is then scaled as it is drawn, which costs what
/// sizing it is there to save, until the screen has answered.
pub const SCALE_UNKNOWN: f32 = 2.0;

impl ImageShape {
    /// How wide a picture of this shape is shown at the most, in points:
    /// no view has more room for one.
    fn widest(self) -> f32 {
        match self {
            Self::Original => LIST_MAX_WIDTH,
            Self::Circle => LARGEST_AVATAR,
            Self::Strip => POPUP_MAX_WIDTH,
            Self::Thumbnail => WIDEST_THUMBNAIL,
        }
    }
}

/// A picture a view wants: where it is, and in what form. The same address
/// in two forms is two pictures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRequest {
    pub url: String,
    pub shape: ImageShape,
}

/// A picture ready to draw.
#[derive(Clone)]
pub struct LoadedImage {
    /// Its pixels, as many as it takes where it is shown: see [`sized`].
    pub handle: Handle,
    /// How large the picture itself is, in the shape it was asked for in:
    /// what it is laid out by. A picture is shown no wider than this many
    /// points, and as tall as its proportions make it.
    pub width: u32,
    pub height: u32,
}

impl LoadedImage {
    /// The bytes the pixels kept take.
    fn bytes(&self) -> usize {
        match &self.handle {
            Handle::Rgba { pixels, .. } => pixels.len(),
            _ => self.width as usize * self.height as usize * 4,
        }
    }
}

impl fmt::Debug for LoadedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LoadedImage({}x{})", self.width, self.height)
    }
}

/// The middle square of `pixels`, with everything outside the circle that
/// fits it made transparent, and the circle's edge softened over one pixel.
fn circle(pixels: &image::RgbaImage) -> image::RgbaImage {
    let (width, height) = pixels.dimensions();
    let side = width.min(height);
    let mut square = image::imageops::crop_imm(pixels, (width - side) / 2, (height - side) / 2, side, side).to_image();

    let radius = side as f32 / 2.0;
    for (x, y, pixel) in square.enumerate_pixels_mut() {
        // How far the pixel's centre is inside the circle, in pixels: all
        // the way in is opaque, one pixel out is gone.
        let distance = ((x as f32 + 0.5 - radius).powi(2) + (y as f32 + 0.5 - radius).powi(2)).sqrt();
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        pixel.0[3] = (f32::from(pixel.0[3]) * coverage).round() as u8;
    }
    square
}

/// The middle of `pixels`, `across` wide for every `down` tall: the sides
/// cut off a picture that is wider than that, the top and bottom off one
/// that is taller.
fn middle(pixels: &image::RgbaImage, (across, down): (u32, u32)) -> image::RgbaImage {
    let (width, height) = pixels.dimensions();
    let (wide, tall) = if u64::from(width) * u64::from(down) >= u64::from(height) * u64::from(across) {
        ((height / down).max(1) * across, height)
    } else {
        (width, (width / across).max(1) * down)
    };
    let (wide, tall) = (wide.min(width), tall.min(height));
    image::imageops::crop_imm(pixels, (width - wide) / 2, (height - tall) / 2, wide, tall).to_image()
}

/// How many pixels a picture of `width` by `height` pixels, of the shape
/// `shape`, is kept at on a screen of `scale` pixels to the point: as many
/// as it takes where it is shown, in its own proportions.
///
/// A picture in a note is shown as wide as its row, and no wider than it
/// is itself, counting a pixel of it as a point: it is kept at that many
/// points' worth of pixels, which makes a narrow picture larger on a
/// screen of more than a pixel to the point, as drawing it there always
/// did. The other shapes are shown at sizes of their views' choosing, and
/// are only ever made smaller: no larger than the largest of those takes,
/// and in exactly the proportions they are cut to, which is what the room
/// kept for them while they load is in.
fn kept_size((width, height): (u32, u32), shape: ImageShape, scale: f32) -> (u32, u32) {
    let in_proportion =
        |across: u32| ((u64::from(height) * u64::from(across) + u64::from(width) / 2) / u64::from(width).max(1)).max(1) as u32;
    let widest = (shape.widest() * scale).round().max(1.0) as u32;
    let cut = match shape {
        ImageShape::Original => {
            let across = ((width as f32).min(shape.widest()) * scale).round().max(1.0) as u32;
            return (across, in_proportion(across));
        }
        ImageShape::Circle => (1, 1),
        ImageShape::Strip => (STRIP_ASPECT, 1),
        ImageShape::Thumbnail => THUMBNAIL_ASPECT,
    };
    if width <= widest {
        return (width, height);
    }
    // Whole times what it is cut to, where there is room for that.
    match widest / cut.0 {
        0 => (widest, in_proportion(widest)),
        times => (times * cut.0, times * cut.1),
    }
}

/// `pixels`, a picture of the shape `shape`, made the size it is shown at
/// on a screen of `scale` pixels to the point (see [`kept_size`]).
///
/// Colours are weighed by how opaque they are while the picture is made
/// another size: what is behind a see-through pixel is no part of the
/// picture, and would otherwise darken the edge of what is in front of it.
fn sized(mut pixels: image::RgbaImage, shape: ImageShape, scale: f32) -> image::RgbaImage {
    let (width, height) = pixels.dimensions();
    if width == 0 || height == 0 {
        return pixels;
    }
    let (across, down) = kept_size((width, height), shape, scale);
    if (across, down) == (width, height) {
        return pixels;
    }

    let see_through = pixels.pixels().any(|pixel| pixel.0[3] != u8::MAX);
    if see_through {
        for pixel in pixels.pixels_mut() {
            let alpha = u16::from(pixel.0[3]);
            for channel in &mut pixel.0[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
    let mut pixels = image::imageops::resize(&pixels, across, down, image::imageops::FilterType::CatmullRom);
    if see_through {
        for pixel in pixels.pixels_mut() {
            let alpha = u16::from(pixel.0[3]);
            for channel in &mut pixel.0[..3] {
                // Of a pixel that is wholly see-through, no colour is left to tell.
                *channel = (u16::from(*channel) * 255 + alpha / 2).checked_div(alpha).unwrap_or(0).min(255) as u8;
            }
        }
    }
    pixels
}

/// Decode `bytes` into pixels in the form `shape`, at the size they are
/// shown at on a screen of `scale` pixels to the point, refusing a picture
/// that would decode larger than `max_decoded_bytes`. The format is told
/// from the bytes themselves, not from the address or what the server
/// called it.
fn decode(bytes: &[u8], shape: ImageShape, max_decoded_bytes: u64, scale: f32) -> Result<LoadedImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_decoded_bytes);
    reader.limits(limits);
    let pixels = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    // Cut to its shape first, and made round last: the edge of an avatar
    // is softened over one pixel of the picture kept, not of the one that
    // came.
    let pixels = match shape {
        ImageShape::Original => pixels,
        ImageShape::Circle => middle(&pixels, (1, 1)),
        ImageShape::Strip => middle(&pixels, (STRIP_ASPECT, 1)),
        ImageShape::Thumbnail => middle(&pixels, THUMBNAIL_ASPECT),
    };
    let (width, height) = pixels.dimensions();
    let pixels = sized(pixels, shape, scale);
    let pixels = if shape == ImageShape::Circle { circle(&pixels) } else { pixels };
    let (across, down) = pixels.dimensions();
    Ok(LoadedImage { handle: Handle::from_rgba(across, down, pixels.into_raw()), width, height })
}

/// Fetch and decode the picture `request` asks for, for a screen of `scale`
/// pixels to the point.
pub async fn fetch(http: reqwest::Client, request: ImageRequest, scale: f32) -> Result<LoadedImage, String> {
    let ImageRequest { url, shape } = request;
    let bytes = http::get(&http, &url, MAX_DOWNLOAD_BYTES).await?;
    tokio::task::spawn_blocking(move || decode(&bytes, shape, MAX_DECODED_BYTES, scale)).await.map_err(|e| e.to_string())?
}

/// `picture` as a PNG, written quickly rather than small: it is on its
/// way to a clipboard, not to a disk.
fn encoded_png(picture: &image::DynamicImage) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let mut png = Vec::new();
    let encoder = PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive);
    picture.write_with_encoder(encoder).map_err(|e| e.to_string())?;
    Ok(png)
}

/// The picture `bytes` are, as a PNG, which is what a picture on a
/// clipboard is taken as everywhere: itself, if it is one already;
/// otherwise decoded, turned the way up it says it is, and encoded. An
/// animated picture is its first frame.
pub fn as_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageDecoder;
    use image::metadata::Orientation;

    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    if format == Some(image::ImageFormat::Png) && orientation == Orientation::NoTransforms {
        return Ok(bytes.to_vec());
    }
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    picture.apply_orientation(orientation);
    encoded_png(&picture)
}

/// The picture `loaded` shows, as a PNG: the pixels kept of it, which are
/// as many as it takes where it is shown, and may be fewer than it has.
pub fn shown_as_png(loaded: &LoadedImage) -> Result<Vec<u8>, String> {
    let Handle::Rgba { width, height, pixels, .. } = &loaded.handle else {
        return Err("its pixels are not kept".to_string());
    };
    let pixels = image::RgbaImage::from_raw(*width, *height, pixels.to_vec()).ok_or("its pixels are not all there")?;
    encoded_png(&image::DynamicImage::ImageRgba8(pixels))
}

/// What is at `url`, a picture, as a PNG (see [`as_png`]).
pub async fn fetch_as_png(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let bytes = http::get(http, url, MAX_DOWNLOAD_BYTES).await?;
    tokio::task::spawn_blocking(move || as_png(&bytes)).await.map_err(|e| e.to_string())?
}

/// What the store has to say about a picture.
pub enum ImageState<'a> {
    Ready(&'a LoadedImage),
    /// Asked for, or about to be.
    Loading,
    /// It could not be fetched or decoded, and won't be tried again this run.
    Failed,
}

struct Entry {
    image: LoadedImage,
    /// When it was last shown, on the store's clock.
    shown: Cell<u64>,
}

/// The pictures loaded so far, by address and form. See the [module docs](self).
pub struct ImageStore {
    loaded: HashMap<ImageRequest, Entry>,
    failed: HashSet<ImageRequest>,
    /// The asking for those that are neither.
    requests: Requests<ImageRequest>,
    /// Ticks each time a picture is shown.
    clock: Cell<u64>,
    /// The decoded bytes held, and how many may be.
    bytes: usize,
    budget: usize,
}

impl ImageStore {
    /// An empty store keeping at most `budget` decoded bytes, and the
    /// receiving end of its requests.
    pub fn new(budget: usize) -> (Self, mpsc::Receiver<ImageRequest>) {
        let (requests, receiver) = Requests::new(REQUEST_QUEUE, MAX_IN_FLIGHT);
        let store = Self { loaded: HashMap::new(), failed: HashSet::new(), requests, clock: Cell::new(0), bytes: 0, budget };
        (store, receiver)
    }

    /// The picture at `url`, in the form `shape`, if it is loaded; otherwise
    /// ask for it.
    pub fn get(&self, url: &str, shape: ImageShape) -> ImageState<'_> {
        let request = ImageRequest { url: url.to_string(), shape };
        if let Some(entry) = self.loaded.get(&request) {
            let now = self.clock.get() + 1;
            self.clock.set(now);
            entry.shown.set(now);
            return ImageState::Ready(&entry.image);
        }
        if self.failed.contains(&request) {
            return ImageState::Failed;
        }
        self.requests.ask(&request);
        ImageState::Loading
    }

    /// A request reached the app. Returns whether to start loading the
    /// picture: not if it is loaded, failed or on its way already, and not
    /// while [`MAX_IN_FLIGHT`] others are. A request turned away for that
    /// last reason is simply asked again by the next view that needs it.
    pub fn begin(&mut self, request: &ImageRequest) -> bool {
        let wanted = !self.loaded.contains_key(request) && !self.failed.contains(request);
        self.requests.begin(request, wanted)
    }

    /// The load `request` started ended. A picture is kept, dropping the
    /// ones least recently shown if the budget is exceeded; a failure is
    /// remembered.
    pub fn finish(&mut self, request: ImageRequest, result: Result<LoadedImage, String>) {
        self.requests.end(&request);
        match result {
            Ok(image) => {
                self.bytes += image.bytes();
                let now = self.clock.get() + 1;
                self.clock.set(now);
                if let Some(replaced) = self.loaded.insert(request.clone(), Entry { image, shown: Cell::new(now) }) {
                    self.bytes -= replaced.image.bytes();
                }
                self.evict(&request);
            }
            Err(_) => {
                self.failed.insert(request);
            }
        }
    }

    /// Drop the pictures least recently shown until the budget is met,
    /// sparing `keep`, which has just arrived.
    fn evict(&mut self, keep: &ImageRequest) {
        while self.bytes > self.budget {
            let oldest = self
                .loaded
                .iter()
                .filter(|(request, _)| *request != keep)
                .min_by_key(|(_, entry)| entry.shown.get())
                .map(|(request, _)| request.clone());
            let Some(request) = oldest else {
                break;
            };
            if let Some(entry) = self.loaded.remove(&request) {
                self.bytes -= entry.image.bytes();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ImageRequest, ImageShape, ImageState, ImageStore, LARGEST_AVATAR, LoadedImage, MAX_IN_FLIGHT, STRIP_ASPECT,
        THUMBNAIL_ASPECT, as_png, circle, decode, middle, shown_as_png, sized,
    };
    use crate::views::components::LIST_MAX_WIDTH;
    use cosmic::widget::image::Handle;

    /// A picture of `side`×`side` pixels: `side * side * 4` bytes.
    fn picture(side: u32) -> LoadedImage {
        LoadedImage { handle: Handle::from_rgba(side, side, vec![0; (side * side * 4) as usize]), width: side, height: side }
    }

    fn original(url: &str) -> ImageRequest {
        ImageRequest { url: url.to_string(), shape: ImageShape::Original }
    }

    fn is_ready(store: &ImageStore, url: &str) -> bool {
        matches!(store.get(url, ImageShape::Original), ImageState::Ready(_))
    }

    fn is_loading(store: &ImageStore, url: &str) -> bool {
        matches!(store.get(url, ImageShape::Original), ImageState::Loading)
    }

    /// Load `url` into `store`: begun, and finished with a `side`×`side` picture.
    fn load(store: &mut ImageStore, url: &str, side: u32) {
        store.begin(&original(url));
        store.finish(original(url), Ok(picture(side)));
    }

    /// A PNG of `width`×`height` pixels of one colour.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn a_picture_not_there_is_asked_for_once() {
        let (mut store, mut requests) = ImageStore::new(1_000);

        // A view is built several times before the app gets to the request.
        for _ in 0..3 {
            assert!(is_loading(&store, "https://a"));
        }
        assert_eq!(requests.try_recv().ok(), Some(original("https://a")));
        assert!(requests.try_recv().is_err(), "asked once");

        // On its way: still loading, and not asked for again.
        assert!(store.begin(&original("https://a")));
        assert!(is_loading(&store, "https://a"));
        assert!(requests.try_recv().is_err());
        assert!(!store.begin(&original("https://a")), "a second request for it starts nothing");

        store.finish(original("https://a"), Ok(picture(4)));
        assert!(is_ready(&store, "https://a"));
    }

    #[test]
    fn the_same_address_in_another_form_is_another_picture() {
        let (mut store, mut requests) = ImageStore::new(1_000);
        load(&mut store, "https://a", 4);

        // Loaded as it came; wanted round, it is asked for again.
        assert!(matches!(store.get("https://a", ImageShape::Circle), ImageState::Loading));
        assert_eq!(requests.try_recv().ok(), Some(ImageRequest { url: "https://a".into(), shape: ImageShape::Circle }));
        assert!(is_ready(&store, "https://a"));
    }

    #[test]
    fn a_picture_that_failed_is_not_tried_again() {
        let (mut store, mut requests) = ImageStore::new(1_000);
        assert!(store.begin(&original("https://broken")));
        store.finish(original("https://broken"), Err("404".into()));

        assert!(matches!(store.get("https://broken", ImageShape::Original), ImageState::Failed));
        assert!(requests.try_recv().is_err());
        assert!(!store.begin(&original("https://broken")));
    }

    #[test]
    fn only_so_many_load_at_once_and_the_rest_ask_again() {
        let (mut store, mut requests) = ImageStore::new(1_000);
        for n in 0..MAX_IN_FLIGHT {
            assert!(store.begin(&original(&format!("https://{n}"))));
        }

        // One more is asked for and turned away...
        assert!(is_loading(&store, "https://late"));
        assert_eq!(requests.try_recv().ok(), Some(original("https://late")));
        assert!(!store.begin(&original("https://late")));

        // ...so the next view that needs it asks again, and once there is
        // room it starts.
        assert!(is_loading(&store, "https://late"));
        assert_eq!(requests.try_recv().ok(), Some(original("https://late")));
        store.finish(original("https://0"), Ok(picture(1)));
        assert!(store.begin(&original("https://late")));
    }

    #[test]
    fn over_budget_the_pictures_least_recently_shown_go_first() {
        // Room for two 4×4 pictures (64 bytes each), not three.
        let (mut store, _requests) = ImageStore::new(150);
        load(&mut store, "https://a", 4);
        load(&mut store, "https://b", 4);
        // `a` is on screen again; `b` hasn't been shown since it arrived.
        assert!(is_ready(&store, "https://a"));

        load(&mut store, "https://c", 4);

        assert!(is_ready(&store, "https://a"));
        assert!(is_ready(&store, "https://c"));
        assert!(is_loading(&store, "https://b"), "dropped, and asked for again when shown");
    }

    #[test]
    fn a_picture_larger_than_the_budget_is_still_shown() {
        let (mut store, _requests) = ImageStore::new(10);
        load(&mut store, "https://big", 4);
        assert!(is_ready(&store, "https://big"));
    }

    #[test]
    fn a_picture_is_decoded_from_its_bytes() {
        let decoded = decode(&png(3, 2), ImageShape::Original, 1_000, 1.0).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
    }

    /// How many pixels a decoded picture is kept at.
    fn kept(loaded: &LoadedImage) -> (u32, u32) {
        match &loaded.handle {
            Handle::Rgba { width, height, .. } => (*width, *height),
            _ => panic!("pixels are what is kept"),
        }
    }

    #[test]
    fn a_picture_is_kept_at_the_size_it_is_shown_at_and_laid_out_by_its_own() {
        let row = LIST_MAX_WIDTH as u32;
        let decoded = |width, height, scale| decode(&png(width, height), ImageShape::Original, 1 << 26, scale).unwrap();

        // Wider than its row: as many pixels as the row is across on the
        // screen, and as many down as its proportions make that.
        let wide = decoded(row * 4, row * 2, 1.0);
        assert_eq!(kept(&wide), (row, row / 2));
        assert_eq!(
            kept(&decoded(row * 4, row * 2, 1.7)),
            ((row as f32 * 1.7).round() as u32, (row as f32 * 1.7 / 2.0).round() as u32)
        );
        // What it is laid out by is its own size, whatever is kept of it:
        // its row is as tall as it was when its note said how large it is.
        assert_eq!((wide.width, wide.height), (row * 4, row * 2));

        // Narrower: shown a point to the pixel, so kept as it came where a
        // point is a pixel, and as large as it is drawn where it is more.
        assert_eq!(kept(&decoded(200, 100, 1.0)), (200, 100));
        let narrow = decoded(200, 100, 1.5);
        assert_eq!(kept(&narrow), (300, 150));
        assert_eq!((narrow.width, narrow.height), (200, 100));
        // Exactly its row's width, a pixel to the point: nothing to do.
        assert_eq!(kept(&decoded(row, 10, 1.0)), (row, 10));
        // However thin, something is kept of it.
        assert_eq!(kept(&decoded(row * 100, 1, 1.0)), (row, 1));
    }

    #[test]
    fn the_other_shapes_are_made_no_larger_than_they_are_ever_shown() {
        let largest = LARGEST_AVATAR as u32;
        let avatar = |side, scale| kept(&decode(&png(side, side), ImageShape::Circle, 1 << 26, scale).unwrap());
        assert_eq!(avatar(1_000, 1.0), (largest, largest));
        assert_eq!(avatar(1_000, 2.0), (largest * 2, largest * 2));
        // Smaller than that already: kept as it came, on any screen.
        assert_eq!(avatar(40, 1.0), (40, 40));
        assert_eq!(avatar(40, 2.0), (40, 40));

        // A strip and a thumbnail stay exactly the shape they are cut to,
        // which the room kept for them while they load is in, whatever
        // the screen: nothing moves when they arrive.
        for scale in [1.0, 1.25, 1.7, 2.0] {
            let (across, down) = kept(&decode(&png(3_000, 3_000), ImageShape::Strip, 1 << 26, scale).unwrap());
            assert_eq!(across, down * STRIP_ASPECT, "at {scale}");
            assert!(
                across as f32 <= 640.0 * scale && across as f32 > 640.0 * scale - STRIP_ASPECT as f32 - 1.0,
                "at {scale}: {across}"
            );
            let (across, down) = kept(&decode(&png(3_000, 3_000), ImageShape::Thumbnail, 1 << 26, scale).unwrap());
            assert_eq!(across * THUMBNAIL_ASPECT.1, down * THUMBNAIL_ASPECT.0, "at {scale}");
        }

        // Made round after it is made small: the corners are see-through,
        // and the edge is one pixel of what is kept.
        let round = decode(&png(1_000, 1_000), ImageShape::Circle, 1 << 26, 1.0).unwrap();
        let Handle::Rgba { pixels, .. } = &round.handle else { panic!("pixels are what is kept") };
        let alpha = |x: u32, y: u32| pixels[((y * largest + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(largest / 2, largest / 2), 255);
        assert_eq!(alpha(1, largest / 2), 255, "one pixel in from the edge is inside the circle");
    }

    #[test]
    fn what_is_behind_a_see_through_pixel_does_not_darken_what_is_made_smaller() {
        // White, opaque on the left half; black, and wholly see-through, on
        // the right: what a logo on no ground looks like in a file.
        let logo = image::RgbaImage::from_fn(LIST_MAX_WIDTH as u32 * 4, 8, |x, _| {
            if x < LIST_MAX_WIDTH as u32 * 2 { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 0]) }
        });
        let small = sized(logo, ImageShape::Original, 1.0);
        assert_eq!(small.width(), LIST_MAX_WIDTH as u32);
        // Wherever anything of the picture is left, it is as white as it was.
        for pixel in small.pixels().filter(|pixel| pixel.0[3] > 8) {
            assert!(pixel.0[..3].iter().all(|&channel| channel >= 250), "{:?}", pixel.0);
        }
        assert!(small.pixels().any(|pixel| pixel.0[3] == 255) && small.pixels().any(|pixel| pixel.0[3] == 0));
    }

    #[test]
    fn the_budget_counts_the_pixels_kept_not_the_pictures_own() {
        // Sixteen times the row's pixels as it came; one row's worth kept.
        let row = LIST_MAX_WIDTH as u32;
        let big = decode(&png(row * 4, row * 4), ImageShape::Original, 1 << 28, 1.0).unwrap();
        assert_eq!(big.bytes(), (row * row * 4) as usize);
    }

    #[test]
    fn a_picture_that_would_decode_too_large_is_refused() {
        // 100×100 pixels are 40,000 bytes decoded, however small the file.
        let file = png(100, 100);
        assert!(file.len() < 1_000, "one colour compresses to almost nothing");
        assert!(decode(&file, ImageShape::Original, 39_999, 1.0).is_err());
        assert!(decode(&file, ImageShape::Original, 40_000, 1.0).is_ok());
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_refused() {
        assert!(decode(b"<html>not found</html>", ImageShape::Original, 1_000, 1.0).is_err());
        assert!(decode(&[], ImageShape::Original, 1_000, 1.0).is_err());
    }

    #[test]
    fn an_avatar_is_the_middle_square_made_round() {
        // Wide: red on the left third, green in the middle, blue on the right.
        let wide = image::RgbaImage::from_fn(300, 100, |x, _| match x / 100 {
            0 => image::Rgba([255, 0, 0, 255]),
            1 => image::Rgba([0, 255, 0, 255]),
            _ => image::Rgba([0, 0, 255, 255]),
        });

        let round = circle(&wide);

        assert_eq!(round.dimensions(), (100, 100), "the middle square");
        assert_eq!(round.get_pixel(50, 50).0, [0, 255, 0, 255], "the middle of the picture, opaque");
        assert_eq!(round.get_pixel(2, 50).0, [0, 255, 0, 255], "the circle reaches the edge of the square");
        for corner in [(0, 0), (99, 0), (0, 99), (99, 99)] {
            assert_eq!(round.get_pixel(corner.0, corner.1).0[3], 0, "corner {corner:?} is outside the circle");
        }

        // Decoding as an avatar does the same.
        let decoded = decode(&png(40, 90), ImageShape::Circle, 1_000_000, 1.0).unwrap();
        assert_eq!((decoded.width, decoded.height), (40, 40));
    }

    #[test]
    fn a_strip_is_the_middle_of_the_picture_three_times_as_wide_as_tall() {
        // Tall: red on top, green in the middle, blue at the bottom.
        let tall = image::RgbaImage::from_fn(300, 900, |_, y| match y / 300 {
            0 => image::Rgba([255, 0, 0, 255]),
            1 => image::Rgba([0, 255, 0, 255]),
            _ => image::Rgba([0, 0, 255, 255]),
        });
        let strip = |pixels: &image::RgbaImage| middle(pixels, (STRIP_ASPECT, 1));
        let cut = strip(&tall);
        assert_eq!(cut.dimensions(), (300, 100), "top and bottom cut off");
        assert_eq!(cut.get_pixel(150, 0).0, [0, 255, 0, 255], "the middle of the picture");
        assert_eq!(cut.get_pixel(150, 99).0, [0, 255, 0, 255]);

        // Wider than a strip: the sides go. Already a strip: nothing does.
        assert_eq!(strip(&image::RgbaImage::new(1_000, 100)).dimensions(), (300, 100));
        assert_eq!(strip(&image::RgbaImage::new(1_500, 500)).dimensions(), (1_500, 500));
        // Too small to cut anything from.
        assert_eq!(strip(&image::RgbaImage::new(2, 5)).dimensions(), (2, 1));

        // Decoding as a strip does the same.
        let decoded = decode(&png(90, 90), ImageShape::Strip, 1_000_000, 1.0).unwrap();
        assert_eq!((decoded.width, decoded.height), (90, 30));
    }

    #[test]
    fn a_thumbnail_is_the_middle_of_the_picture_in_its_own_proportions() {
        let thumbnail = |width, height| middle(&image::RgbaImage::new(width, height), THUMBNAIL_ASPECT).dimensions();
        // As pages make the pictures they are shared with: a little wide.
        assert_eq!(thumbnail(1_200, 630), (1_120, 630));
        // Square, and tall: the top and bottom go.
        assert_eq!(thumbnail(400, 400), (400, 225));
        assert_eq!(thumbnail(320, 1_000), (320, 180));
        // Already so: nothing goes. Too small to cut anything from: nothing can.
        assert_eq!(thumbnail(1_280, 720), (1_280, 720));
        assert_eq!(thumbnail(3, 2), (3, 2));

        let decoded = decode(&png(160, 160), ImageShape::Thumbnail, 1_000_000, 1.0).unwrap();
        assert_eq!((decoded.width, decoded.height), (160, 90));
    }

    #[test]
    fn a_picture_for_the_clipboard_is_a_png_the_way_up_it_says_it_is() {
        use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
        use std::io::Cursor;

        // Three wide and two tall, each pixel a colour of its own.
        let picture = DynamicImage::ImageRgb8(RgbImage::from_fn(3, 2, |x, y| Rgb([(x * 80) as u8, (y * 120) as u8, 200])));
        let encoded = |format: ImageFormat| {
            let mut bytes = Vec::new();
            picture.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
            bytes
        };
        let read = |png: &[u8]| image::load_from_memory_with_format(png, ImageFormat::Png).unwrap();

        // A PNG is itself.
        let png = encoded(ImageFormat::Png);
        assert_eq!(as_png(&png).unwrap(), png);
        // What is no PNG is made one, with the same pixels.
        let from_webp = read(&as_png(&encoded(ImageFormat::WebP)).unwrap());
        assert_eq!(from_webp.to_rgb8(), picture.to_rgb8());

        // A JPEG that says it lies on its side is turned upright: after
        // its JFIF header, an EXIF block that says so and nothing else.
        let jpeg = encoded(ImageFormat::Jpeg);
        let exif =
            [&[0xFF, 0xE1, 0, 34][..], b"Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0"].concat();
        let sideways = [&jpeg[..20], &exif, &jpeg[20..]].concat();
        let upright = read(&as_png(&sideways).unwrap());
        assert_eq!((upright.width(), upright.height()), (2, 3));
        let as_it_is = read(&as_png(&jpeg).unwrap());
        assert_eq!((as_it_is.width(), as_it_is.height()), (3, 2));

        assert!(as_png(b"not a picture").is_err());
    }

    #[test]
    fn what_is_shown_of_a_picture_is_a_png_of_the_pixels_kept() {
        let pixels: Vec<u8> = (0..2 * 3 * 4).map(|nth| nth as u8 * 10).collect();
        let loaded = LoadedImage { handle: Handle::from_rgba(2, 3, pixels.clone()), width: 20, height: 30 };
        let png = shown_as_png(&loaded).unwrap();
        let read = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
        assert_eq!((read.width(), read.height()), (2, 3));
        assert_eq!(read.into_rgba8().into_raw(), pixels);
    }
}

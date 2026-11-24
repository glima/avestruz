// SPDX-License-Identifier: MIT

//! A file that is to be attached to a note, before it goes anywhere: what
//! kind of file it is, how large a picture it is, and the picture without
//! what it says of where, when, with what and by whom it was made.
//!
//! A photograph says a good deal more than what it shows: where it was
//! taken, when, with which camera, what it was edited with, who it is
//! credited to, and sometimes a smaller copy of what it showed before it
//! was cropped. None of that is for a media server, or for whoever looks
//! at the note. So a JPEG, a PNG or a WebP is sent as the picture and no
//! more: of what the file is made of, only what the picture needs to look
//! as it did is kept, and everything else is left out, whatever it is.
//!
//! | Format | Kept | Left out |
//! |---|---|---|
//! | JPEG | The tables, frames and scans; of the application segments, the JFIF header, the colour profile (ICC) and Adobe's colour marker | Every other application segment (EXIF, XMP, IPTC, MPF, content credentials, makers' own), the comments, and whatever follows the end of the picture |
//! | PNG | The header, the palette, the picture data, transparency, colour (gamma, chromaticities, sRGB, ICC, HDR), pixel size, background, animation | Every other chunk (text, EXIF, the time it was changed, content credentials, makers' own), and whatever follows the end |
//! | WebP | The picture data, alpha, the colour profile, animation | EXIF, XMP, and every chunk that is none of those |
//!
//! This is done to the bytes of the file, and the picture data is copied
//! as it is: nothing is decoded and encoded again, so nothing of the
//! picture is lost.
//!
//! One thing is put back. A camera held sideways writes the picture as its
//! sensor saw it and says in the EXIF which way is up; without that the
//! picture lies on its side. Which way is up says nothing of where or with
//! what, so a picture that is not upright as written gets an EXIF block
//! that says that and nothing else.
//!
//! What comes out is then decoded, to make sure it is still a picture. If
//! it is not, while the file as it was is, the file is refused
//! ([`MediaError::Damaged`]) rather than sent as it was: what was to be
//! left out is still in it.
//!
//! Other files go as they are: a GIF, a video, a sound. **A video may say
//! where it was taken**, and that is not taken out of it here.

use std::io::Cursor;

use image::ImageFormat;

/// The largest file that is attached. It is read into memory whole, to be
/// hashed and sent.
pub const MAX_BYTES: usize = 100_000_000;

/// What a file that is not known to be anything is called.
const ANY: &str = "application/octet-stream";

/// A file ready to be sent to a media server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    /// What it was called where it was picked.
    pub name: String,
    /// What it is made of, as it is sent.
    pub bytes: Vec<u8>,
    /// What kind of file it is: its MIME type.
    pub mime: &'static str,
    /// How wide and how tall it shows, in pixels, if it is a picture that
    /// could be measured.
    pub dim: Option<(u32, u32)>,
}

/// Why a file is not attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    /// There is nothing in it.
    Empty,
    /// It is larger than [`MAX_BYTES`]: this many bytes.
    TooLarge(usize),
    /// It is a picture whose file is not put together as its format
    /// says, so what it says beside the picture could not be left out.
    Unreadable,
    /// It was a picture, and is not one any more with what it says
    /// beside the picture left out.
    Damaged,
}

/// What kind of file `bytes` are, by how they start; or, for what does not
/// say, by what the name `name` ends in.
pub fn kind_of(bytes: &[u8], name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => "audio/wav",
        [b'B', b'M', ..] if extension == "bmp" => "image/bmp",
        [0x1A, 0x45, 0xDF, 0xA3, ..] if extension == "mkv" => "video/x-matroska",
        [0x1A, 0x45, 0xDF, 0xA3, ..] => "video/webm",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        [b'I', b'D', b'3', ..] => "audio/mpeg",
        [_, _, _, _, b'f', b't', b'y', b'p', brand @ ..] => match brand.get(..4) {
            Some(b"avif" | b"avis") => "image/avif",
            Some(b"heic" | b"heix" | b"heif" | b"mif1") => "image/heic",
            Some(b"qt  ") => "video/quicktime",
            Some(b"M4A " | b"M4B ") => "audio/mp4",
            _ => "video/mp4",
        },
        _ => match extension.as_str() {
            "svg" => "image/svg+xml",
            "mp3" => "audio/mpeg",
            "mp4" | "m4v" => "video/mp4",
            "mov" => "video/quicktime",
            "webm" => "video/webm",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            _ => ANY,
        },
    }
}

/// The file called `name` that `bytes` are, ready to be sent: a JPEG, a PNG
/// or a WebP as the picture and no more (see the [module](self)), and
/// anything else as it is.
pub fn prepare(name: String, bytes: Vec<u8>) -> Result<Media, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::Empty);
    }
    if bytes.len() > MAX_BYTES {
        return Err(MediaError::TooLarge(bytes.len()));
    }
    let mime = kind_of(&bytes, &name);
    let bared = match mime {
        "image/jpeg" => Some((bare_jpeg(&bytes), ImageFormat::Jpeg)),
        "image/png" => Some((bare_png(&bytes), ImageFormat::Png)),
        "image/webp" => Some((bare_webp(&bytes), ImageFormat::WebP)),
        _ => None,
    };
    let Some((bare, format)) = bared else {
        // Not cleaned: measured by its header, if it is a picture at all.
        let measured = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().ok();
        let dim = measured.and_then(|reader| reader.into_dimensions().ok());
        return Ok(Media { name, bytes, mime, dim });
    };
    let bare = bare.ok_or(MediaError::Unreadable)?;
    let decoded =
        |bytes: &[u8]| image::load_from_memory_with_format(bytes, format).ok().map(|picture| (picture.width(), picture.height()));
    let dim = match decoded(&bare.bytes) {
        // Shown on its side, it is as wide as it is written tall.
        Some((width, height)) if bare.orientation.is_some_and(|orientation| orientation >= 5) => Some((height, width)),
        Some(size) => Some(size),
        // What can't be decoded either way is not for this to judge.
        None if decoded(&bytes).is_none() => None,
        None => return Err(MediaError::Damaged),
    };
    Ok(Media { name, bytes: bare.bytes, mime, dim })
}

/// A picture's file with nothing in it but the picture, and which way up
/// the file it was made from said the picture is: 1 to 8, as EXIF counts
/// them, where 1 is as written.
struct Bare {
    bytes: Vec<u8>,
    orientation: Option<u16>,
}

/// The EXIF tag that says which way up a picture is.
const ORIENTATION_TAG: u16 = 0x0112;

/// Which way up the EXIF block `tiff` says its picture is, if it says, and
/// says one of the eight there are. An EXIF block is a TIFF file: a
/// header that says which end its numbers start at and where its first
/// directory is, and that directory, twelve bytes to an entry.
fn orientation_in(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) })
    };
    let long = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) })
    };
    if short(2)? != 0x002A {
        return None;
    }
    let directory = usize::try_from(long(4)?).ok()?;
    let entries = usize::from(short(directory)?);
    for entry in 0..entries {
        let at = directory.checked_add(2 + entry * 12)?;
        if short(at)? == ORIENTATION_TAG {
            let orientation = short(at.checked_add(8)?)?;
            return (1..=8).contains(&orientation).then_some(orientation);
        }
    }
    None
}

/// An EXIF block that says the picture is `orientation` way up, and
/// nothing else: a directory of that one entry.
fn orientation_only(orientation: u16) -> [u8; 26] {
    let mut tiff = [0; 26];
    tiff[..4].copy_from_slice(b"MM\0\x2A");
    // The directory follows the header, which is eight bytes.
    tiff[4..8].copy_from_slice(&8u32.to_be_bytes());
    tiff[8..10].copy_from_slice(&1u16.to_be_bytes());
    tiff[10..12].copy_from_slice(&ORIENTATION_TAG.to_be_bytes());
    // One short, which is in the entry itself. No directory follows.
    tiff[12..14].copy_from_slice(&3u16.to_be_bytes());
    tiff[14..18].copy_from_slice(&1u32.to_be_bytes());
    tiff[18..20].copy_from_slice(&orientation.to_be_bytes());
    tiff
}

/// The orientation that is to be said again: one that is not as written.
fn to_say(orientation: Option<u16>) -> Option<u16> {
    orientation.filter(|orientation| *orientation != 1)
}

/// How a JPEG's EXIF segment starts, before the EXIF block.
const JPEG_EXIF: &[u8] = b"Exif\0\0";

/// Whether the segment of a JPEG marked `marker`, which holds `payload`,
/// is part of the picture. Everything is, but the application segments
/// and the comments; and of the application segments the JFIF header,
/// the colour profile, and Adobe's marker, which says how the colours
/// are written.
fn jpeg_keeps(marker: u8, payload: &[u8]) -> bool {
    match marker {
        0xE0 => payload.starts_with(b"JFIF\0"),
        0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
        0xEE => payload.starts_with(b"Adobe"),
        0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
        _ => true,
    }
}

/// Where the picture data that starts at `from` in the JPEG `bytes` ends:
/// at the next marker that is none of its own, which are the stuffed
/// zero, the restart markers and fill bytes.
fn end_of_scan(bytes: &[u8], from: usize) -> usize {
    let rest = bytes.get(from..).unwrap_or_default();
    let marker = rest.windows(2).position(|pair| matches!(pair, [0xFF, next] if !matches!(next, 0x00 | 0xD0..=0xD7 | 0xFF)));
    marker.map_or(bytes.len(), |marker| from + marker)
}

/// The JPEG `bytes` as the picture and no more. `None` if it is not put
/// together as a JPEG is.
///
/// A JPEG is a row of segments, each a marker and, for most, a length and
/// that much data; after a start of scan comes picture data, up to the
/// next marker. It ends at the end-of-image marker, and what follows that
/// is no part of the picture: a second picture, a video of the moment.
fn bare_jpeg(bytes: &[u8]) -> Option<Bare> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    // The orientation goes after the JFIF header, which has to be first
    // where there is one, and first otherwise.
    let mut exif_at = out.len();
    let mut orientation = None;
    let mut at = 2;
    // A file that ends short of its end marker is copied as far as it goes.
    while let Some(&start) = bytes.get(at) {
        if start != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // A fill byte before the marker.
            0xFF => at += 1,
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                break;
            }
            // Markers that stand alone.
            0x01 | 0xD0..=0xD8 => {
                out.extend_from_slice(&[0xFF, marker]);
                at += 2;
            }
            _ => {
                let length = usize::from(u16::from_be_bytes(bytes.get(at + 2..at + 4)?.try_into().ok()?));
                if length < 2 {
                    return None;
                }
                let end = at + 2 + length;
                let segment = bytes.get(at..end)?;
                let payload = segment.get(4..)?;
                if marker == 0xE1
                    && let Some(tiff) = payload.strip_prefix(JPEG_EXIF)
                {
                    orientation = orientation.or_else(|| orientation_in(tiff));
                }
                if jpeg_keeps(marker, payload) {
                    out.extend_from_slice(segment);
                    if marker == 0xE0 && out.len() == exif_at + segment.len() {
                        exif_at = out.len();
                    }
                }
                at = end;
                if marker == 0xDA {
                    let scan_end = end_of_scan(bytes, at);
                    out.extend_from_slice(bytes.get(at..scan_end)?);
                    at = scan_end;
                }
            }
        }
    }
    if let Some(orientation) = to_say(orientation) {
        let tiff = orientation_only(orientation);
        let length = u16::try_from(2 + JPEG_EXIF.len() + tiff.len()).ok()?;
        let segment: Vec<u8> = [&[0xFF, 0xE1][..], &length.to_be_bytes(), JPEG_EXIF, &tiff].concat();
        out.splice(exif_at..exif_at, segment);
    }
    Some(Bare { bytes: out, orientation })
}

/// How a PNG starts.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The chunks of a PNG that are part of the picture: its header, palette
/// and data; transparency; what its colours are (gamma, chromaticities,
/// sRGB, a profile, significant bits, and the three that say how bright
/// and in which colour space); how large a pixel is; its background;
/// suggested palettes; and animation.
const PNG_KEPT: [&[u8; 4]; 20] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"cICP", b"mDCV", b"cLLI", b"pHYs",
    b"bKGD", b"hIST", b"sPLT", b"acTL", b"fcTL", b"fdAT",
];

/// The checksum a PNG chunk ends in, of its name and what it holds, which
/// are `bytes`: CRC-32.
fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The PNG chunk called `name` that holds `data`.
fn png_chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let named: Vec<u8> = [&name[..], data].concat();
    let length = u32::try_from(data.len()).unwrap_or_default();
    [&length.to_be_bytes()[..], &named, &png_crc(&named).to_be_bytes()].concat()
}

/// The PNG `bytes` as the picture and no more. `None` if it is not put
/// together as a PNG is.
///
/// A PNG is a row of chunks after its signature: each a length, a name of
/// four letters, that much data, and a checksum. The header is first and
/// the end chunk last; what follows that is no part of the picture.
fn bare_png(bytes: &[u8]) -> Option<Bare> {
    let mut rest = bytes.strip_prefix(PNG_SIGNATURE)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(PNG_SIGNATURE);
    let mut header_end = None;
    let mut orientation = None;
    loop {
        let length = usize::try_from(u32::from_be_bytes(rest.get(..4)?.try_into().ok()?)).ok()?;
        let chunk = rest.get(..length.checked_add(12)?)?;
        let name: &[u8; 4] = chunk.get(4..8)?.try_into().ok()?;
        if name == b"eXIf" {
            orientation = orientation.or_else(|| orientation_in(chunk.get(8..8 + length).unwrap_or_default()));
        }
        if PNG_KEPT.contains(&name) {
            out.extend_from_slice(chunk);
        }
        if name == b"IHDR" && header_end.is_none() {
            header_end = Some(out.len());
        }
        rest = rest.get(chunk.len()..)?;
        if name == b"IEND" {
            break;
        }
    }
    // Before the picture data, where whoever shows it knows in time.
    if let (Some(orientation), Some(at)) = (to_say(orientation), header_end) {
        out.splice(at..at, png_chunk(b"eXIf", &orientation_only(orientation)));
    }
    Some(Bare { bytes: out, orientation })
}

/// The chunks of a WebP that are part of the picture: its data, lossy or
/// lossless; the header of the extended format; alpha; the colour
/// profile; and animation.
const WEBP_KEPT: [&[u8; 4]; 7] = [b"VP8 ", b"VP8L", b"VP8X", b"ALPH", b"ICCP", b"ANIM", b"ANMF"];

/// In the header of the extended format, the flags that say there is
/// EXIF, and that there is XMP.
const WEBP_HAS_EXIF: u8 = 0x08;
const WEBP_HAS_XMP: u8 = 0x04;

/// The WebP `bytes` as the picture and no more. `None` if it is not put
/// together as a WebP is.
///
/// A WebP is a RIFF file: a header that says how long it is, then chunks,
/// each a name of four letters, a length, and that much data, padded to an
/// even length. In the extended format the first chunk is a header whose
/// flags say which others there are.
fn bare_webp(bytes: &[u8]) -> Option<Bare> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let said = usize::try_from(u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?)).ok()?;
    // What follows what the header counts is no part of the file.
    let mut rest = bytes.get(12..said.checked_add(8)?.min(bytes.len()))?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(bytes.get(..12)?);
    let mut orientation = None;
    while !rest.is_empty() {
        let name: &[u8; 4] = rest.get(..4)?.try_into().ok()?;
        let length = usize::try_from(u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?)).ok()?;
        let data = rest.get(8..length.checked_add(8)?)?;
        // The last chunk may come without its padding.
        let padded = (8 + length + length % 2).min(rest.len());
        if name == b"EXIF" {
            // Some write it as a JPEG does, after a header of its own.
            let tiff = data.strip_prefix(JPEG_EXIF).unwrap_or(data);
            orientation = orientation.or_else(|| orientation_in(tiff));
        }
        if WEBP_KEPT.contains(&name) {
            out.extend_from_slice(rest.get(..8 + length)?);
            if length % 2 == 1 {
                out.push(0);
            }
        }
        rest = rest.get(padded..)?;
    }
    // Only the extended format has anything but the picture data, and
    // says in its header what.
    if out.get(12..16) == Some(b"VP8X") {
        let flags = out.get_mut(20)?;
        *flags &= !(WEBP_HAS_EXIF | WEBP_HAS_XMP);
        if let Some(orientation) = to_say(orientation) {
            *flags |= WEBP_HAS_EXIF;
            let tiff = orientation_only(orientation);
            out.extend_from_slice(b"EXIF");
            out.extend_from_slice(&u32::try_from(tiff.len()).ok()?.to_le_bytes());
            out.extend_from_slice(&tiff);
        }
    }
    let length = u32::try_from(out.len() - 8).ok()?;
    out.get_mut(4..8)?.copy_from_slice(&length.to_le_bytes());
    Some(Bare { bytes: out, orientation })
}

#[cfg(test)]
mod tests {
    use super::{
        JPEG_EXIF, MAX_BYTES, MediaError, PNG_SIGNATURE, bare_jpeg, bare_png, bare_webp, kind_of, orientation_in,
        orientation_only, png_chunk, png_crc, prepare,
    };
    use image::metadata::Orientation;
    use image::{DynamicImage, ImageDecoder, ImageFormat, Rgb, RgbImage};
    use std::io::Cursor;

    /// A picture three wide and two tall, each pixel a colour of its own.
    fn picture() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(3, 2, |x, y| Rgb([(x * 80) as u8, (y * 120) as u8, 200])))
    }

    fn encoded(format: ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        picture().write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        bytes
    }

    /// An EXIF block, numbers little end first, that says where the
    /// picture was taken and that it is `orientation` way up.
    fn exif(orientation: u16) -> Vec<u8> {
        let mut tiff = b"II\x2A\0".to_vec();
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        // Where: a pointer to the GPS directory.
        tiff.extend_from_slice(&[0x25, 0x88, 4, 0, 1, 0, 0, 0, 38, 0, 0, 0]);
        tiff.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        tiff.extend_from_slice(b"GPS: 38.7223 N, 9.1393 W");
        tiff
    }

    /// The JPEG segment marked `marker` that holds `payload`.
    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len() + 2).unwrap();
        [&[0xFF, marker][..], &length.to_be_bytes(), payload].concat()
    }

    /// A JPEG as a camera and an editor leave it: `plain` with everything
    /// said beside the picture put in after its JFIF header, the picture
    /// `orientation` way up, and a video after its end.
    fn laden_jpeg(plain: &[u8], orientation: u16) -> Vec<u8> {
        // After the start marker comes the JFIF header, 18 bytes of it.
        let (head, rest) = plain.split_at(20);
        assert_eq!(&head[2..4], [0xFF, 0xE0]);
        [
            head,
            &segment(0xE1, &[JPEG_EXIF, &exif(orientation)].concat()),
            &segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>creator: Fernando</x:xmpmeta>"),
            &segment(0xE2, b"ICC_PROFILE\0\x01\x01a colour profile"),
            &segment(0xE2, b"MPF\0a second picture"),
            &segment(0xEB, b"JP\0\0c2pa: signed by Fernando"),
            &segment(0xED, b"Photoshop 3.0\08BIM caption: at home"),
            &segment(0xFE, b"a comment: taken with my phone"),
            rest,
            b"a video of the moment",
        ]
        .concat()
    }

    fn has(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|window| window == needle)
    }

    fn pixels(bytes: &[u8], format: ImageFormat) -> Vec<u8> {
        image::load_from_memory_with_format(bytes, format).unwrap().into_rgb8().into_raw()
    }

    fn orientation_of(bytes: &[u8], format: ImageFormat) -> Orientation {
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format).into_decoder().unwrap();
        reader.orientation().unwrap()
    }

    #[test]
    fn a_file_is_known_by_how_it_starts_and_failing_that_by_its_name() {
        assert_eq!(kind_of(&encoded(ImageFormat::Jpeg), "picture"), "image/jpeg");
        assert_eq!(kind_of(&encoded(ImageFormat::Png), "picture.jpg"), "image/png");
        assert_eq!(kind_of(&encoded(ImageFormat::WebP), "picture"), "image/webp");
        assert_eq!(kind_of(&encoded(ImageFormat::Gif), "picture"), "image/gif");
        assert_eq!(kind_of(b"\0\0\0\x20ftypisom\0\0\x02\0", "clip"), "video/mp4");
        assert_eq!(kind_of(b"\0\0\0\x14ftypqt  \0\0\0\0", "clip"), "video/quicktime");
        assert_eq!(kind_of(b"\0\0\0\x1cftypavif\0\0\0\0", "picture"), "image/avif");
        assert_eq!(kind_of(b"\x1A\x45\xDF\xA3\x01", "clip"), "video/webm");
        assert_eq!(kind_of(b"ID3\x04\0", "song"), "audio/mpeg");
        assert_eq!(kind_of(b"<svg xmlns='http://www.w3.org/2000/svg'/>", "Drawing.SVG"), "image/svg+xml");
        assert_eq!(kind_of(b"anything at all", "notes.xyz"), "application/octet-stream");
    }

    #[test]
    fn which_way_up_is_read_from_exif_written_either_end_first_and_written_back_alone() {
        assert_eq!(orientation_in(&exif(6)), Some(6));
        for orientation in 1..=8 {
            assert_eq!(orientation_in(&orientation_only(orientation)), Some(orientation));
        }
        assert_eq!(orientation_in(&exif(9)), None, "there are eight ways up");
        assert_eq!(orientation_in(b"II\x2A\0\xFF\xFF\xFF\xFF"), None, "a directory that is not there");
        assert_eq!(orientation_in(b"not a TIFF header"), None);
        assert_eq!(orientation_in(b""), None);
    }

    #[test]
    fn a_jpeg_keeps_its_picture_its_colours_and_which_way_up_and_nothing_else() {
        let plain = encoded(ImageFormat::Jpeg);
        let laden = laden_jpeg(&plain, 6);
        let bare = bare_jpeg(&laden).unwrap();

        for told in [&b"GPS"[..], b"Fernando", b"second picture", b"c2pa", b"at home", b"my phone", b"video"] {
            assert!(has(&laden, told) && !has(&bare.bytes, told), "{} is left out", String::from_utf8_lossy(told));
        }
        assert!(has(&bare.bytes, b"a colour profile"), "the colour profile stays");
        assert!(has(&bare.bytes, b"JFIF\0"));
        assert_eq!(pixels(&bare.bytes, ImageFormat::Jpeg), pixels(&plain, ImageFormat::Jpeg), "the picture is as it was");
        assert_eq!(bare.orientation, Some(6));
        assert_eq!(orientation_of(&bare.bytes, ImageFormat::Jpeg), Orientation::Rotate90);
        // What says so is after the JFIF header, and all the EXIF there is.
        assert_eq!(&bare.bytes[20..24], [0xFF, 0xE1, 0, 34]);
        assert!(bare.bytes.ends_with(&[0xFF, 0xD9]), "it ends where the picture does");
    }

    #[test]
    fn a_jpeg_that_is_upright_as_written_says_nothing_beside_the_picture() {
        let plain = encoded(ImageFormat::Jpeg);
        assert_eq!(bare_jpeg(&plain).unwrap().bytes, plain, "a JPEG that is only the picture is left as it is");

        let bare = bare_jpeg(&laden_jpeg(&plain, 1)).unwrap();
        assert!(!has(&bare.bytes, b"Exif"));
        // The picture as it was encoded, and its colour profile.
        let profile = segment(0xE2, b"ICC_PROFILE\0\x01\x01a colour profile");
        assert_eq!(bare.bytes, [&plain[..20], &profile, &plain[20..]].concat());
    }

    #[test]
    fn a_jpeg_that_is_not_put_together_as_one_is_not_a_picture_to_send() {
        let plain = encoded(ImageFormat::Jpeg);
        assert!(bare_jpeg(b"not a JPEG").is_none());
        // A segment that says it is longer than the file.
        assert!(bare_jpeg(&[&plain[..2], &[0xFF, 0xE1, 0xFF, 0xFF, 1, 2, 3]].concat()).is_none());
        // One that says it is shorter than its own length.
        assert!(bare_jpeg(&[&plain[..2], &[0xFF, 0xE1, 0, 1, 1, 2, 3]].concat()).is_none());
        // Something between two segments that is no marker.
        assert!(bare_jpeg(&[&plain[..20], &[0x12, 0x34], &plain[20..]].concat()).is_none());
        // One cut short is copied as far as it goes.
        let short = &plain[..plain.len() - 2];
        assert_eq!(bare_jpeg(short).unwrap().bytes, short);
    }

    /// A PNG as an editor leaves it: `plain` with everything said beside
    /// the picture put in after its header, the picture `orientation` way
    /// up, and something after its end.
    fn laden_png(plain: &[u8], orientation: u16) -> Vec<u8> {
        // The signature, and the header chunk: 13 bytes in 12 of chunk.
        let (head, rest) = plain.split_at(8 + 25);
        [
            head,
            &png_chunk(b"gAMA", &45_455u32.to_be_bytes()),
            &png_chunk(b"tEXt", b"Author\0Fernando"),
            &png_chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta>at home</x:xmpmeta>"),
            &png_chunk(b"eXIf", &exif(orientation)),
            &png_chunk(b"tIME", &[7, 233, 1, 22, 6, 53, 25]),
            &png_chunk(b"caBX", b"c2pa: signed"),
            &png_chunk(b"prVt", b"a maker's own"),
            rest,
            b"something after the end",
        ]
        .concat()
    }

    #[test]
    fn the_checksum_of_a_png_chunk_is_the_one_a_png_ends_with() {
        // The end chunk holds nothing, and is the same in every PNG.
        assert_eq!(png_crc(b"IEND"), 0xAE42_6082);
        assert!(encoded(ImageFormat::Png).ends_with(&png_chunk(b"IEND", b"")));
    }

    #[test]
    fn a_png_keeps_its_picture_its_colours_and_which_way_up_and_nothing_else() {
        let plain = encoded(ImageFormat::Png);
        let laden = laden_png(&plain, 8);
        let bare = bare_png(&laden).unwrap();

        for told in [&b"GPS"[..], b"Fernando", b"at home", b"tIME", b"c2pa", b"maker", b"after the end"] {
            assert!(has(&laden, told) && !has(&bare.bytes, told), "{} is left out", String::from_utf8_lossy(told));
        }
        assert!(has(&bare.bytes, b"gAMA"), "what its colours are stays");
        assert_eq!(pixels(&bare.bytes, ImageFormat::Png), pixels(&plain, ImageFormat::Png), "the picture is as it was");
        assert_eq!(bare.orientation, Some(8));
        assert_eq!(orientation_of(&bare.bytes, ImageFormat::Png), Orientation::Rotate270);

        let upright = bare_png(&laden_png(&plain, 1)).unwrap();
        assert!(!has(&upright.bytes, b"eXIf"), "upright as written, it says nothing of which way up");
        assert_eq!(bare_png(&plain).unwrap().bytes, plain, "a PNG that is only the picture is left as it is");
    }

    #[test]
    fn an_animated_png_stays_animated() {
        let animated = [
            PNG_SIGNATURE,
            &png_chunk(b"IHDR", &[0; 13]),
            &png_chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]),
            &png_chunk(b"fcTL", &[0; 26]),
            &png_chunk(b"IDAT", b"the first frame"),
            &png_chunk(b"fcTL", &[1; 26]),
            &png_chunk(b"fdAT", b"the second frame"),
            &png_chunk(b"IEND", b""),
        ]
        .concat();
        assert_eq!(bare_png(&animated).unwrap().bytes, animated);
    }

    #[test]
    fn a_png_that_is_not_put_together_as_one_is_not_a_picture_to_send() {
        let plain = encoded(ImageFormat::Png);
        assert!(bare_png(b"not a PNG").is_none());
        assert!(bare_png(&plain[..plain.len() - 12]).is_none(), "it has no end");
        assert!(
            bare_png(&[PNG_SIGNATURE, &[0xFF, 0xFF, 0xFF, 0xFF], b"IHDR"].concat()).is_none(),
            "a chunk longer than the file"
        );
    }

    /// The WebP chunk called `name` that holds `data`.
    fn webp_chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let length = u32::try_from(data.len()).unwrap();
        let padding: &[u8] = if data.len() % 2 == 1 { &[0] } else { &[] };
        [&name[..], &length.to_le_bytes(), data, padding].concat()
    }

    /// The WebP that holds `chunks`.
    fn webp(chunks: &[Vec<u8>]) -> Vec<u8> {
        let chunks = chunks.concat();
        let length = u32::try_from(chunks.len() + 4).unwrap();
        [&b"RIFF"[..], &length.to_le_bytes(), b"WEBP", &chunks].concat()
    }

    #[test]
    fn a_webp_keeps_its_picture_its_colours_and_which_way_up_and_nothing_else() {
        // The header of the extended format: its flags, three bytes kept
        // for later, and how large the picture is.
        let header = |flags: u8| webp_chunk(b"VP8X", &[flags, 0, 0, 0, 2, 0, 0, 1, 0, 0]);
        let profile = webp_chunk(b"ICCP", b"a colour profile");
        let data = webp_chunk(b"VP8 ", b"the picture");
        let exif_of = |orientation| webp_chunk(b"EXIF", &exif(orientation));
        let xmp = webp_chunk(b"XMP ", b"<x:xmpmeta>creator: Fernando</x:xmpmeta>");
        let unknown = webp_chunk(b"MAKR", b"a maker's own");
        // A profile, EXIF and XMP: 0x20, 0x08 and 0x04.
        let laden = [
            webp(&[header(0x2C), profile.clone(), data.clone(), exif_of(3), xmp.clone(), unknown.clone()]),
            b"after it".to_vec(),
        ]
        .concat();

        let bare = bare_webp(&laden).unwrap();
        assert_eq!(bare.orientation, Some(3));
        let said = webp_chunk(b"EXIF", &orientation_only(3));
        assert_eq!(
            bare.bytes,
            webp(&[header(0x28), profile.clone(), data.clone(), said]),
            "the profile, the picture, and which way up"
        );

        let upright = webp(&[header(0x2C), profile.clone(), data.clone(), exif_of(1), xmp, unknown]);
        assert_eq!(
            bare_webp(&upright).unwrap().bytes,
            webp(&[header(0x20), profile, data]),
            "upright as written, it has no EXIF"
        );
    }

    #[test]
    fn a_webp_that_is_only_the_picture_is_left_as_it_is() {
        let plain = encoded(ImageFormat::WebP);
        assert_eq!(bare_webp(&plain).unwrap().bytes, plain);
        assert!(bare_webp(b"RIFF\x04\0\0\0WAVE").is_none());
        assert!(bare_webp(&webp(&[b"VP8 \xFF\xFF\0\0short".to_vec()])).is_none(), "a chunk longer than the file");
    }

    #[test]
    fn a_picture_is_made_ready_bare_and_measured_as_it_shows() {
        let plain = encoded(ImageFormat::Jpeg);
        let sideways = prepare("holiday.jpg".into(), laden_jpeg(&plain, 6)).unwrap();
        assert_eq!(sideways.mime, "image/jpeg");
        assert!(!has(&sideways.bytes, b"GPS"));
        assert_eq!(sideways.dim, Some((2, 3)), "three wide as written, it shows two wide on its side");

        let upright = prepare("holiday.png".into(), laden_png(&encoded(ImageFormat::Png), 1)).unwrap();
        assert_eq!((upright.mime, upright.dim), ("image/png", Some((3, 2))));
        assert!(!has(&upright.bytes, b"Fernando"));

        let gif = encoded(ImageFormat::Gif);
        let as_it_is = prepare("dance.gif".into(), gif.clone()).unwrap();
        assert_eq!((as_it_is.mime, as_it_is.dim, as_it_is.bytes), ("image/gif", Some((3, 2)), gif));

        let clip = b"\0\0\0\x20ftypisom and the rest of a video".to_vec();
        let video = prepare("clip.mp4".into(), clip.clone()).unwrap();
        assert_eq!((video.mime, video.dim, video.bytes), ("video/mp4", None, clip));
    }

    #[test]
    fn what_cannot_be_made_ready_is_refused_and_says_why() {
        assert_eq!(prepare("empty.jpg".into(), Vec::new()), Err(MediaError::Empty));
        assert_eq!(prepare("huge.bin".into(), vec![0; MAX_BYTES + 1]), Err(MediaError::TooLarge(MAX_BYTES + 1)));
        // It starts as a JPEG does, and is not put together as one.
        assert_eq!(prepare("broken.jpg".into(), b"\xFF\xD8\xFF\xE1\xFF\xFF".to_vec()), Err(MediaError::Unreadable));
    }
}

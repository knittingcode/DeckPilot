//! Importing, checking and scaling key icons.
//!
//! Imported files are untrusted: only PNG and JPEG are accepted (by their
//! content, not their name), at most [`MAX_IMPORT_BYTES`] long; dimensions
//! are checked by the decoder before it allocates
//! ([`MAX_SOURCE_SIZE`] per side, [`MAX_DECODE_ALLOC`] in total). The image
//! is scaled down to at most [`MAX_ICON_SIZE`] pixels per side and encoded
//! again as a PNG, which drops metadata (EXIF, text chunks, colour
//! profiles). Stored icons are decoded with the same kind of limits, so a
//! crafted profile or export cannot make the decoder allocate much either.

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use deckpilot_core::{
    Icon, IconData, IconFit, IconId, MAX_ICON_BYTES, MAX_ICON_NAME, MAX_ICON_SIZE, Profile,
};
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, Limits, RgbaImage};

/// Largest file that can be imported, in bytes.
pub const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;
/// Largest width and height of an imported file, in pixels.
pub const MAX_SOURCE_SIZE: u32 = 4096;
/// Most memory a decoder may allocate for one image.
pub const MAX_DECODE_ALLOC: u64 = 80 * 1024 * 1024;

/// Why an image cannot be used as an icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconError {
    /// The file is larger than [`MAX_IMPORT_BYTES`].
    TooLarge,
    /// Not a PNG or JPEG image.
    UnsupportedFormat,
    /// Wider or higher than allowed.
    TooManyPixels,
    /// The data could not be decoded.
    Malformed,
    /// A stored icon breaks the rules for stored icons.
    Invalid(String),
}

impl fmt::Display for IconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(
                f,
                "the image is larger than {} MiB",
                MAX_IMPORT_BYTES / 1024 / 1024
            ),
            Self::UnsupportedFormat => f.write_str("only PNG and JPEG images can be used"),
            Self::TooManyPixels => write!(
                f,
                "the image is larger than {MAX_SOURCE_SIZE} x {MAX_SOURCE_SIZE} pixels"
            ),
            Self::Malformed => f.write_str("the image is damaged or not an image"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for IconError {}

fn decode(bytes: &[u8], format: ImageFormat, max_side: u32) -> Result<RgbaImage, IconError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    match reader.decode() {
        Ok(image) => Ok(image.into_rgba8()),
        Err(image::ImageError::Limits(_)) => Err(IconError::TooManyPixels),
        Err(_) => Err(IconError::Malformed),
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, IconError> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| IconError::Malformed)?;
    Ok(png)
}

/// `image` scaled down (never up) to at most `max` pixels per side.
fn shrink(image: RgbaImage, max: u32) -> RgbaImage {
    let (width, height) = image.dimensions();
    if width <= max && height <= max {
        return image;
    }
    let scale = f64::from(max) / f64::from(width.max(height));
    let size = |side: u32| {
        // At most `max`, at least one pixel.
        let scaled = (f64::from(side) * scale).round();
        if scaled < 1.0 {
            1
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let scaled = scaled as u32;
            scaled.min(max)
        }
    };
    image::imageops::resize(&image, size(width), size(height), FilterType::Triangle)
}

/// Encodes `image` as a stored icon PNG: scaled down until it fits
/// [`MAX_ICON_SIZE`] and [`MAX_ICON_BYTES`].
fn to_stored(image: RgbaImage) -> Result<(u32, u32, Vec<u8>), IconError> {
    let mut image = shrink(image, MAX_ICON_SIZE);
    loop {
        let png = encode_png(&image)?;
        let (width, height) = image.dimensions();
        if png.len() <= MAX_ICON_BYTES {
            return Ok((width, height, png));
        }
        let side = width.max(height);
        if side <= 16 {
            return Err(IconError::TooLarge);
        }
        image = shrink(image, side * 3 / 4);
    }
}

fn clean_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ICON_NAME)
        .collect();
    if name.trim().is_empty() {
        "Icon".into()
    } else {
        name.trim().to_owned()
    }
}

/// Turns an imported PNG or JPEG file into a new icon named `name`.
pub fn import(name: &str, bytes: &[u8]) -> Result<Icon, IconError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(IconError::TooLarge);
    }
    let Ok(format @ (ImageFormat::Png | ImageFormat::Jpeg)) = image::guess_format(bytes) else {
        return Err(IconError::UnsupportedFormat);
    };
    let image = decode(bytes, format, MAX_SOURCE_SIZE)?;
    let (width, height, png) = to_stored(image)?;
    let icon = Icon {
        id: IconId::new(),
        name: clean_name(name),
        width,
        height,
        png: IconData::new(png),
    };
    icon.check().map_err(IconError::Invalid)?;
    Ok(icon)
}

/// Checks a stored icon from an untrusted source (an export) by decoding
/// it with limits, and encodes it again. Keeps its id and name.
pub fn sanitize(icon: &Icon) -> Result<Icon, IconError> {
    icon.check().map_err(IconError::Invalid)?;
    let image = decode(icon.png.bytes(), ImageFormat::Png, MAX_ICON_SIZE)?;
    let (width, height, png) = to_stored(image)?;
    Ok(Icon {
        id: icon.id,
        name: clean_name(&icon.name),
        width,
        height,
        png: IconData::new(png),
    })
}

/// Decodes a stored icon for drawing.
pub fn pixels(icon: &Icon) -> Result<RgbaImage, IconError> {
    icon.check().map_err(IconError::Invalid)?;
    decode(icon.png.bytes(), ImageFormat::Png, MAX_ICON_SIZE)
}

/// `image` scaled to exactly `width` x `height` pixels (transparent where
/// the icon does not reach).
pub fn scale(image: &RgbaImage, width: u32, height: u32, fit: IconFit) -> RgbaImage {
    let (width, height) = (width.max(1), height.max(1));
    let (source_w, source_h) = image.dimensions();
    if source_w == 0 || source_h == 0 {
        return RgbaImage::new(width, height);
    }
    let ratio_x = f64::from(width) / f64::from(source_w);
    let ratio_y = f64::from(height) / f64::from(source_h);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = |side: u32, ratio: f64| ((f64::from(side) * ratio).round().max(1.0)) as u32;
    match fit {
        IconFit::Fill => image::imageops::resize(image, width, height, FilterType::Triangle),
        IconFit::Fit => {
            let ratio = ratio_x.min(ratio_y);
            let (w, h) = (
                size(source_w, ratio).min(width),
                size(source_h, ratio).min(height),
            );
            let scaled = image::imageops::resize(image, w, h, FilterType::Triangle);
            let mut out = RgbaImage::new(width, height);
            image::imageops::overlay(
                &mut out,
                &scaled,
                i64::from((width - w) / 2),
                i64::from((height - h) / 2),
            );
            out
        }
        IconFit::Crop => {
            let ratio = ratio_x.max(ratio_y);
            let (w, h) = (
                size(source_w, ratio).max(width),
                size(source_h, ratio).max(height),
            );
            let scaled = image::imageops::resize(image, w, h, FilterType::Triangle);
            image::imageops::crop_imm(&scaled, (w - width) / 2, (h - height) / 2, width, height)
                .to_image()
        }
    }
}

/// The decoded icons of a profile, by id. Icons that cannot be decoded
/// are left out (their keys show no icon).
#[derive(Debug, Clone, Default)]
pub struct Icons(HashMap<IconId, Arc<RgbaImage>>);

impl Icons {
    pub fn from_profile(profile: &Profile) -> Self {
        Self(
            profile
                .icons
                .iter()
                .filter_map(|icon| Some((icon.id, Arc::new(pixels(icon).ok()?))))
                .collect(),
        )
    }

    pub fn get(&self, id: IconId) -> Option<&RgbaImage> {
        self.0.get(&id).map(AsRef::as_ref)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{ImageEncoder, Rgba};

    /// A `width` x `height` image: red left half, blue right half.
    pub(crate) fn halves(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        })
    }

    pub(crate) fn png(image: &RgbaImage) -> Vec<u8> {
        encode_png(image).unwrap()
    }

    fn jpeg(image: &RgbaImage) -> Vec<u8> {
        let rgb = image::DynamicImage::ImageRgba8(image.clone()).into_rgb8();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    /// A valid PNG whose header claims `width` x `height` but whose data
    /// is a 1 x 1 image: the decoder must refuse by the header.
    fn lying_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = png(&halves(2, 2));
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        // A correct checksum, so only the dimensions are wrong.
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn imports_png_and_jpeg_scaled_down_and_re_encoded() {
        let icon = import("  Logo\n", &png(&halves(512, 256))).unwrap();
        assert_eq!(icon.name, "Logo");
        assert_eq!((icon.width, icon.height), (256, 128));
        assert!(icon.check().is_ok());
        let photo = import("photo.jpg", &jpeg(&halves(100, 50))).unwrap();
        assert_eq!((photo.width, photo.height), (100, 50));
        // Stored icons are always PNG.
        assert!(photo.png.bytes().starts_with(b"\x89PNG"));
        // Small images are not scaled up.
        let small = import("", &png(&halves(8, 8))).unwrap();
        assert_eq!(
            (small.width, small.height, small.name.as_str()),
            (8, 8, "Icon")
        );
    }

    #[test]
    fn metadata_is_dropped() {
        // A text chunk with something private, inserted after IHDR.
        let mut bytes = png(&halves(4, 4));
        let text = b"tEXtComment\0secret location";
        let mut chunk = u32::try_from(text.len() - 4)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        chunk.extend(text);
        chunk.extend(crc32(text).to_be_bytes());
        bytes.splice(33..33, chunk);
        assert!(bytes.windows(6).any(|w| w == b"secret"));
        let icon = import("x", &bytes).unwrap();
        assert!(!icon.png.bytes().windows(6).any(|w| w == b"secret"));
        let clean = import("x", &png(&halves(4, 4))).unwrap();
        assert!(!clean.png.bytes().windows(4).any(|w| w == b"tEXt"));
    }

    #[test]
    fn untrusted_files_are_refused() {
        assert_eq!(
            import("x", &vec![0; MAX_IMPORT_BYTES + 1]),
            Err(IconError::TooLarge)
        );
        assert_eq!(
            import("x", b"GIF89a....."),
            Err(IconError::UnsupportedFormat)
        );
        assert_eq!(
            import("x", b"<svg></svg>"),
            Err(IconError::UnsupportedFormat)
        );
        assert_eq!(import("x", b""), Err(IconError::UnsupportedFormat));
        // Truncated data.
        let mut truncated = png(&halves(32, 32));
        truncated.truncate(60);
        assert_eq!(import("x", &truncated), Err(IconError::Malformed));
        let mut broken_jpeg = jpeg(&halves(32, 32));
        broken_jpeg.truncate(40);
        assert_eq!(import("x", &broken_jpeg), Err(IconError::Malformed));
        // Dimensions are checked before decoding, whatever the file size.
        assert_eq!(
            import("x", &lying_png(100_000, 100_000)),
            Err(IconError::TooManyPixels)
        );
        assert_eq!(
            import("x", &lying_png(MAX_SOURCE_SIZE + 1, 1)),
            Err(IconError::TooManyPixels)
        );
    }

    #[test]
    fn stored_icons_are_checked_again() {
        let icon = import("Logo", &png(&halves(64, 64))).unwrap();
        let again = sanitize(&icon).unwrap();
        assert_eq!(again.id, icon.id);
        assert_eq!(again.png, icon.png, "encoding is stable");
        assert_eq!(pixels(&icon).unwrap().dimensions(), (64, 64));

        let mut lying = icon.clone();
        lying.png = IconData::new(lying_png(64, 64));
        assert!(sanitize(&lying).is_err());
        let mut huge = icon.clone();
        huge.png = IconData::new(lying_png(4096, 4096));
        (huge.width, huge.height) = (4096, 4096);
        assert!(matches!(sanitize(&huge), Err(IconError::Invalid(_))));
        assert!(pixels(&huge).is_err());

        let mut profile = Profile::new("Work");
        profile.icons = vec![icon.clone(), lying];
        let icons = Icons::from_profile(&profile);
        assert_eq!(icons.len(), 1);
        assert!(icons.get(icon.id).is_some());
    }

    #[test]
    fn scaling_modes() {
        let image = halves(200, 100);
        let red = Rgba([255, 0, 0, 255]);
        let blue = Rgba([0, 0, 255, 255]);
        let clear = Rgba([0, 0, 0, 0]);

        // Fit: whole image, letterboxed top and bottom.
        let fit = scale(&image, 72, 72, IconFit::Fit);
        assert_eq!(fit.dimensions(), (72, 72));
        assert_eq!(*fit.get_pixel(36, 2), clear);
        assert_eq!(*fit.get_pixel(5, 36), red);
        assert_eq!(*fit.get_pixel(66, 36), blue);

        // Fill: stretched, no borders.
        let fill = scale(&image, 72, 72, IconFit::Fill);
        assert_eq!(*fill.get_pixel(5, 1), red);
        assert_eq!(*fill.get_pixel(66, 70), blue);

        // Crop: covers the key; the left and right edges are cut off, so
        // the halves meet in the middle and nothing is transparent.
        let crop = scale(&image, 72, 72, IconFit::Crop);
        assert_eq!(*crop.get_pixel(0, 0), red);
        assert_eq!(*crop.get_pixel(71, 71), blue);
        assert_eq!(*crop.get_pixel(30, 36), red);
        assert_eq!(*crop.get_pixel(42, 36), blue);

        // Any size, including odd and tiny ones.
        for (w, h) in [(1, 1), (96, 96), (120, 120), (200, 100), (3, 50)] {
            for fit in [IconFit::Fit, IconFit::Fill, IconFit::Crop] {
                assert_eq!(scale(&image, w, h, fit).dimensions(), (w, h));
            }
        }
    }
}

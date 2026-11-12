//! Wire protocol of the Elgato Stream Deck MK.2 (USB `0fd9:0080`).
//!
//! The MK.2 uses the "V2" Stream Deck protocol, as documented by
//! `python-elgato-streamdeck` (`StreamDeckOriginalV2`) and implemented by
//! the `elgato-streamdeck` crate:
//!
//! - Input report `0x01`: a 4-byte header followed by one byte per key
//!   (non-zero while pressed), keys row-major from the top left.
//! - Output report `0x02`, command `0x07`: key image upload, split into
//!   1024-byte reports with an 8-byte header
//!   `[0x02, 0x07, key, last, length LE16, page LE16]`, zero-padded.
//! - Feature report `0x03`, command `0x08`: brightness in percent, padded to
//!   32 bytes.
//! - Key images are 72 x 72 px JPEG; the device shows them rotated by 180
//!   degrees (mirrored on both axes), so they are uploaded pre-mirrored.
//!
//! Everything here is pure byte manipulation so that it can be checked
//! without hardware. DeckPilot never sends the "reset" feature report
//! (`0x03 0x02`); keys are blanked by uploading a black image instead.

use deckpilot_device::{DeviceError, Frame, ImageFormat, ImageSpec};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, RgbImage};

/// Number of keys (5 columns x 3 rows).
pub const KEY_COUNT: usize = 15;

/// Report ID of key-state input reports.
pub const INPUT_REPORT_ID: u8 = 0x01;
/// Bytes before the key states in an input report.
pub const INPUT_HEADER_LEN: usize = 4;
/// Length of an input report including the report ID.
pub const INPUT_REPORT_LEN: usize = INPUT_HEADER_LEN + KEY_COUNT;

/// Report ID of output reports.
pub const OUTPUT_REPORT_ID: u8 = 0x02;
/// Output command that uploads part of a key image.
pub const SET_KEY_IMAGE: u8 = 0x07;
/// Length of each image output report including the report ID.
pub const IMAGE_REPORT_LEN: usize = 1024;
/// Header length of an image output report.
pub const IMAGE_HEADER_LEN: usize = 8;
/// Image bytes carried by one output report.
pub const IMAGE_PAYLOAD_LEN: usize = IMAGE_REPORT_LEN - IMAGE_HEADER_LEN;

/// Report ID of feature reports.
pub const FEATURE_REPORT_ID: u8 = 0x03;
/// Length of a feature report including the report ID.
pub const FEATURE_REPORT_LEN: usize = 32;
/// Feature command that sets the display brightness.
pub const SET_BRIGHTNESS: u8 = 0x08;

/// JPEG quality used for key images (the value Elgato's protocol
/// implementations use).
pub const JPEG_QUALITY: u8 = 90;

/// Splits an encoded key image into the output reports that upload it.
/// `key` is the zero-based key index.
pub fn image_reports(key: u8, image: &[u8]) -> Vec<Vec<u8>> {
    let pages = image.chunks(IMAGE_PAYLOAD_LEN).count();
    image
        .chunks(IMAGE_PAYLOAD_LEN)
        .enumerate()
        .map(|(page, chunk)| {
            let last = page + 1 == pages;
            let length = u16::try_from(chunk.len()).unwrap_or(u16::MAX).to_le_bytes();
            let page = u16::try_from(page).unwrap_or(u16::MAX).to_le_bytes();
            let mut report = Vec::with_capacity(IMAGE_REPORT_LEN);
            report.extend_from_slice(&[
                OUTPUT_REPORT_ID,
                SET_KEY_IMAGE,
                key,
                u8::from(last),
                length[0],
                length[1],
                page[0],
                page[1],
            ]);
            report.extend_from_slice(chunk);
            report.resize(IMAGE_REPORT_LEN, 0);
            report
        })
        .collect()
}

/// The feature report that sets the brightness; values above 100 are
/// clamped.
pub fn brightness_report(percent: u8) -> [u8; FEATURE_REPORT_LEN] {
    let mut report = [0; FEATURE_REPORT_LEN];
    report[0] = FEATURE_REPORT_ID;
    report[1] = SET_BRIGHTNESS;
    report[2] = percent.min(100);
    report
}

/// Key states from an input report, or `None` when `report` is not a
/// complete key-state report.
pub fn parse_input(report: &[u8]) -> Option<[bool; KEY_COUNT]> {
    if report.len() < INPUT_REPORT_LEN || report[0] != INPUT_REPORT_ID {
        return None;
    }
    let mut states = [false; KEY_COUNT];
    for (state, byte) in states.iter_mut().zip(&report[INPUT_HEADER_LEN..]) {
        *state = *byte != 0;
    }
    Some(states)
}

/// Converts a frame into the image data the device expects for a key:
/// scaled to the key size, pre-transformed so that the device's own
/// transformation shows it upright, and JPEG-encoded.
pub fn encode_key_image(frame: &Frame, spec: &ImageSpec) -> Result<Vec<u8>, DeviceError> {
    if spec.format != ImageFormat::Jpeg {
        return Err(DeviceError::Unsupported(format!(
            "{:?} key images",
            spec.format
        )));
    }
    let image = RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec())
        .ok_or_else(|| DeviceError::InvalidFrame("frame size mismatch".into()))?;
    let mut image = DynamicImage::ImageRgb8(image);
    if (image.width(), image.height()) != (spec.width, spec.height) {
        image = image.resize_exact(spec.width, spec.height, FilterType::Triangle);
    }
    // The device rotates clockwise and then mirrors; undo it in reverse.
    let transform = spec.transform;
    if transform.flip_horizontal {
        image = image.fliph();
    }
    if transform.flip_vertical {
        image = image.flipv();
    }
    image = match transform.rotation {
        0 => image,
        90 => image.rotate270(),
        180 => image.rotate180(),
        270 => image.rotate90(),
        other => {
            return Err(DeviceError::Unsupported(format!("rotation {other}")));
        }
    };
    let rgb = image.into_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| DeviceError::InvalidFrame(e.to_string()))?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::models;

    fn spec() -> ImageSpec {
        models::stream_deck_mk2()
            .capabilities
            .keys
            .and_then(|k| k.image)
            .unwrap()
    }

    fn decode(jpeg: &[u8]) -> RgbImage {
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8()
    }

    #[test]
    fn small_images_fit_one_report() {
        let reports = image_reports(4, &[0xAA; 10]);
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.len(), 1024);
        assert_eq!(report[..8], [0x02, 0x07, 4, 1, 10, 0, 0, 0]);
        assert_eq!(report[8..18], [0xAA; 10]);
        assert!(report[18..].iter().all(|b| *b == 0), "zero padding");
    }

    #[test]
    fn large_images_are_split_into_numbered_pages() {
        let image: Vec<u8> = (0..2500u32).map(|i| (i % 251) as u8).collect();
        let reports = image_reports(14, &image);
        // 2500 = 1016 + 1016 + 468
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[0][..8], [0x02, 0x07, 14, 0, 0xF8, 0x03, 0, 0]);
        assert_eq!(reports[1][..8], [0x02, 0x07, 14, 0, 0xF8, 0x03, 1, 0]);
        assert_eq!(reports[2][..8], [0x02, 0x07, 14, 1, 0xD4, 0x01, 2, 0]);
        assert!(reports.iter().all(|r| r.len() == IMAGE_REPORT_LEN));
        let payload: Vec<u8> = reports
            .iter()
            .zip([1016, 1016, 468])
            .flat_map(|(r, len)| r[8..8 + len].to_vec())
            .collect();
        assert_eq!(payload, image, "payload is reassembled in order");
        assert!(reports[2][8 + 468..].iter().all(|b| *b == 0));
    }

    #[test]
    fn an_exact_multiple_marks_the_full_last_page() {
        let reports = image_reports(0, &[1; IMAGE_PAYLOAD_LEN * 2]);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0][3], 0);
        assert_eq!(reports[1][..8], [0x02, 0x07, 0, 1, 0xF8, 0x03, 1, 0]);
    }

    #[test]
    fn brightness_is_a_padded_feature_report() {
        let report = brightness_report(42);
        assert_eq!(report.len(), 32);
        assert_eq!(report[..3], [0x03, 0x08, 42]);
        assert!(report[3..].iter().all(|b| *b == 0));
        assert_eq!(brightness_report(0)[2], 0);
        assert_eq!(brightness_report(100)[2], 100);
        assert_eq!(brightness_report(255)[2], 100, "clamped");
    }

    #[test]
    fn input_reports_map_bytes_to_row_major_keys() {
        let mut report = [0u8; INPUT_REPORT_LEN];
        report[..4].copy_from_slice(&[0x01, 0x00, 0x0F, 0x00]);
        report[4] = 1; // top left
        report[4 + 4] = 1; // top right
        report[4 + 14] = 1; // bottom right
        let states = parse_input(&report).unwrap();
        let pressed: Vec<usize> = (0..KEY_COUNT).filter(|i| states[*i]).collect();
        assert_eq!(pressed, vec![0, 4, 14]);

        // Longer reads (padding after the key bytes) are accepted.
        let mut long = report.to_vec();
        long.extend([0xFF; 32]);
        assert_eq!(parse_input(&long), Some(states));
    }

    #[test]
    fn other_input_is_ignored() {
        assert_eq!(parse_input(&[]), None, "read timeout");
        assert_eq!(parse_input(&[0x01, 0, 0x0F, 0]), None, "truncated");
        let mut report = [0u8; INPUT_REPORT_LEN];
        report[0] = 0x02;
        report[4] = 1;
        assert_eq!(parse_input(&report), None, "other report ID");
    }

    #[test]
    fn key_images_are_72px_jpeg_rotated_by_180_degrees() {
        // Red marker in the top-left corner of an otherwise blue frame.
        let mut frame = Frame::filled(72, 72, [0, 0, 255]);
        for y in 0..12 {
            for x in 0..12 {
                frame.put(x, y, [255, 0, 0]);
            }
        }
        let jpeg = encode_key_image(&frame, &spec()).unwrap();
        assert_eq!(jpeg[..2], [0xFF, 0xD8], "JPEG start of image");
        assert_eq!(jpeg[jpeg.len() - 2..], [0xFF, 0xD9], "JPEG end of image");
        let image = decode(&jpeg);
        assert_eq!(image.dimensions(), (72, 72));
        let red = |p: &image::Rgb<u8>| p[0] > 200 && p[2] < 60;
        assert!(red(image.get_pixel(66, 66)), "marker moved to bottom right");
        assert!(!red(image.get_pixel(5, 5)));
        assert!(!red(image.get_pixel(66, 5)));
        assert!(!red(image.get_pixel(5, 66)));
    }

    #[test]
    fn other_frame_sizes_are_scaled_to_the_key() {
        let jpeg = encode_key_image(&Frame::filled(144, 144, [0, 255, 0]), &spec()).unwrap();
        let image = decode(&jpeg);
        assert_eq!(image.dimensions(), (72, 72));
        let p = image.get_pixel(36, 36);
        assert!(p[1] > 200 && p[0] < 60 && p[2] < 60);
    }

    #[test]
    fn encoding_matches_the_reference_implementation() {
        use elgato_streamdeck::images::convert_image;
        use elgato_streamdeck::info::Kind;

        let mut frame = Frame::filled(72, 72, [20, 40, 60]);
        for i in 0..72 {
            frame.put(i, i / 2, [250, 200, 10]);
        }
        let image = RgbImage::from_raw(72, 72, frame.rgb().to_vec()).unwrap();
        let reference = convert_image(Kind::Mk2, DynamicImage::ImageRgb8(image)).unwrap();
        assert_eq!(encode_key_image(&frame, &spec()).unwrap(), reference);
    }

    #[test]
    fn non_jpeg_specs_are_rejected() {
        let mut spec = spec();
        spec.format = ImageFormat::Bmp;
        assert!(encode_key_image(&Frame::filled(72, 72, [0; 3]), &spec).is_err());
    }
}

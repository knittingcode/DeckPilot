//! Drawing keys for DeckPilot: [`KeyFace`](deckpilot_core::KeyFace)s to
//! device-independent frames, and processing of imported key icons.
//!
//! Shared by the agent (device images) and the configuration app
//! (previews), so both draw exactly the same.

mod face;
pub mod icons;

pub use face::{FrameCache, render, render_with};
pub use icons::{IconError, Icons};

/// Encodes a frame as PNG, e.g. for a preview in the app.
pub fn frame_png(frame: &deckpilot_device::Frame) -> Option<Vec<u8>> {
    let image = image::RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec())?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    #[test]
    fn frames_encode_as_png() {
        let frame = deckpilot_device::Frame::filled(4, 3, [1, 2, 3]);
        let png = super::frame_png(&frame).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(
            deckpilot_core::appearance::png_dimensions(&png),
            Some((4, 3))
        );
    }
}

//! Linear gradient backgrounds, rasterised before a frame goes to the
//! device (devices get opaque RGB; nothing is animated). The geometry is
//! CSS's `linear-gradient(<angle>deg, …)`: 0 runs to the top, 90 to the
//! right; the gradient line is as long as the frame's corners need, the
//! colours spaced evenly along it, mixed in sRGB.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "keys are a few hundred pixels wide at most; channels are clamped"
)]

use deckpilot_core::Gradient;
use deckpilot_device::Frame;

/// The colour at `t` (0–1) along the gradient.
fn color_at(gradient: &Gradient, t: f32) -> [u8; 3] {
    let colors = &gradient.colors;
    match colors.len() {
        0 => return [0, 0, 0],
        1 => return colors[0].rgb(),
        _ => {}
    }
    let segments = (colors.len() - 1) as f32;
    let at = t.clamp(0.0, 1.0) * segments;
    let index = (at.floor() as usize).min(colors.len() - 2);
    let local = at - index as f32;
    let (from, to) = (colors[index].rgb(), colors[index + 1].rgb());
    std::array::from_fn(|channel| {
        let a = f32::from(from[channel]);
        let b = f32::from(to[channel]);
        (a + (b - a) * local).round().clamp(0.0, 255.0) as u8
    })
}

/// Fills `frame` with `gradient`.
pub(crate) fn paint(frame: &mut Frame, gradient: &Gradient) {
    let (width, height) = (frame.width() as f32, frame.height() as f32);
    let angle = f32::from(gradient.angle % 360).to_radians();
    let (dx, dy) = (angle.sin(), -angle.cos());
    let length = (width * dx.abs() + height * dy.abs()).max(1.0);
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let px = x as f32 + 0.5 - width / 2.0;
            let py = y as f32 + 0.5 - height / 2.0;
            let t = (px * dx + py * dy) / length + 0.5;
            frame.put(x, y, color_at(gradient, t));
        }
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::Color;

    use super::*;

    fn gradient(angle: u16) -> Gradient {
        Gradient {
            colors: vec![Color([255, 0, 0]), Color([0, 0, 255])],
            angle,
        }
    }

    #[test]
    fn gradients_run_along_their_angle() {
        let mut down = Frame::filled(72, 72, [0, 0, 0]);
        paint(&mut down, &gradient(180));
        // Top red, bottom blue, a row all one colour.
        let top = down.pixel(36, 0).unwrap();
        let bottom = down.pixel(36, 71).unwrap();
        assert!(top[0] > 245 && top[2] < 10, "{top:?}");
        assert!(bottom[2] > 245 && bottom[0] < 10, "{bottom:?}");
        assert_eq!(down.pixel(0, 10), down.pixel(71, 10));
        let middle = down.pixel(36, 36).unwrap();
        assert!((120..=135).contains(&middle[0]), "{middle:?}");

        let mut right = Frame::filled(72, 72, [0, 0, 0]);
        paint(&mut right, &gradient(90));
        assert!(right.pixel(0, 36).unwrap()[0] > 245);
        assert!(right.pixel(71, 36).unwrap()[2] > 245);
        assert_eq!(right.pixel(10, 0), right.pixel(10, 71));
    }

    #[test]
    fn three_colours_meet_in_the_middle_and_any_size_works() {
        let three = Gradient {
            colors: vec![Color([255, 0, 0]), Color([0, 255, 0]), Color([0, 0, 255])],
            angle: 90,
        };
        let mut frame = Frame::filled(101, 10, [0, 0, 0]);
        paint(&mut frame, &three);
        let middle = frame.pixel(50, 5).unwrap();
        assert!(middle[1] > 245, "{middle:?}");
        for (w, h) in [(1, 1), (200, 100), (3, 50)] {
            let mut frame = Frame::filled(w, h, [9, 9, 9]);
            paint(&mut frame, &three);
            assert_ne!(frame.pixel(0, 0), Some([9, 9, 9]));
        }
    }
}

//! The timing of a scrolling (marquee) title, as a pure function of the
//! time since it started (`docs/key-text.md`).
//!
//! A cycle is: a pause at the start (offset 0), scrolling at a constant
//! speed until the end of the text is at the end of the text area, a pause
//! there, then back to the start in one step. Offsets are whole pixels, so
//! a frame only changes when the offset does.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "distances are a few hundred pixels and times a few seconds"
)]

use std::time::Duration;

/// Most frames a second of a scrolling title, on the device and in the
/// app's preview.
pub const MARQUEE_FPS: u32 = 15;

/// The shortest time between two frames at `fps` frames a second.
pub fn frame_interval(fps: u32) -> Duration {
    Duration::from_secs(1) / fps.max(1)
}

/// When the frame after one drawn at `elapsed` is due: when the offset
/// changes next, but not sooner than one frame interval later. The
/// device's scheduler and [`marquee_timeline`] both step by this.
pub fn next_frame(marquee: &Marquee, elapsed: Duration, fps: u32) -> Duration {
    marquee
        .next_change(elapsed)
        .max(elapsed + frame_interval(fps))
}

/// One cycle of `marquee` as frames: each offset with how long it is shown,
/// stepping by [`next_frame`] (a pause is one long frame).
pub fn marquee_timeline(marquee: &Marquee, fps: u32) -> Vec<(u32, Duration)> {
    let cycle = marquee.cycle();
    let mut frames: Vec<(u32, Duration)> = Vec::new();
    let mut at = Duration::ZERO;
    while at < cycle {
        let offset = marquee.offset_at(at);
        let until = next_frame(marquee, at, fps).min(cycle);
        match frames.last_mut() {
            Some((last, shown)) if *last == offset => *shown += until.saturating_sub(at),
            _ => frames.push((offset, until.saturating_sub(at))),
        }
        at = until;
    }
    frames
}

/// How a title scrolls: how far, how fast, how long it pauses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marquee {
    /// Whole pixels the title moves (its overflow, rounded up).
    distance: u32,
    /// Pixels per second.
    speed: f32,
    /// At the start and at the end.
    pause: Duration,
}

impl Marquee {
    /// A title `distance` pixels wider than its area (rounded up to whole
    /// pixels), scrolling at `speed` pixels per second (at least 1) with
    /// `pause` at either end.
    pub fn new(distance: f32, speed: f32, pause: Duration) -> Self {
        Self {
            distance: distance.max(0.0).ceil() as u32,
            speed: speed.max(1.0),
            pause,
        }
    }

    pub fn distance(&self) -> u32 {
        self.distance
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn pause(&self) -> Duration {
        self.pause
    }

    /// The speed in thousandths of a pixel per second: offsets are worked
    /// out in whole numbers, so the time a pixel is reached is exact.
    fn milli_speed(&self) -> u128 {
        (f64::from(self.speed) * 1000.0).round().max(1.0) as u128
    }

    /// How long it takes to scroll `pixels`, rounded up to a nanosecond.
    fn time_for(&self, pixels: u32) -> Duration {
        let nanos = (u128::from(pixels) * 1_000_000_000_000).div_ceil(self.milli_speed());
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    /// How long scrolling from the start to the end takes.
    pub fn scroll_time(&self) -> Duration {
        self.time_for(self.distance)
    }

    /// One cycle: pause, scroll, pause.
    pub fn cycle(&self) -> Duration {
        self.pause * 2 + self.scroll_time()
    }

    /// The time into the current cycle.
    fn phase(&self, elapsed: Duration) -> Duration {
        let cycle = self.cycle().as_nanos().max(1);
        Duration::from_nanos((elapsed.as_nanos() % cycle) as u64)
    }

    /// The offset in whole pixels `elapsed` after the start: 0 during the
    /// first pause, then growing by `speed` per second, `distance` during
    /// the last pause.
    pub fn offset_at(&self, elapsed: Duration) -> u32 {
        let phase = self.phase(elapsed);
        let Some(scrolled) = phase.checked_sub(self.pause) else {
            return 0;
        };
        let moved = scrolled.as_nanos() * self.milli_speed() / 1_000_000_000_000;
        u32::try_from(moved).unwrap_or(u32::MAX).min(self.distance)
    }

    /// When after `elapsed` the offset changes next (at least a
    /// nanosecond later): a scheduler sleeps until then, and through
    /// the pauses.
    pub fn next_change(&self, elapsed: Duration) -> Duration {
        let phase = self.phase(elapsed);
        let cycle_start = elapsed.saturating_sub(phase);
        let scroll_end = self.pause + self.scroll_time();
        let next = if phase < scroll_end {
            // The time the next whole pixel is reached.
            let offset = self.offset_at(elapsed);
            (self.pause + self.time_for(offset + 1)).min(scroll_end)
        } else {
            // The end pause: back to the start with the next cycle.
            self.cycle()
        };
        (cycle_start + next).max(elapsed + Duration::from_nanos(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn pauses_at_both_ends_and_scrolls_at_its_speed() {
        // 60 px at 30 px/s with 1 s pauses: a 4 s cycle.
        let marquee = Marquee::new(59.2, 30.0, ms(1000));
        assert_eq!(marquee.distance(), 60);
        assert_eq!(marquee.scroll_time(), ms(2000));
        assert_eq!(marquee.cycle(), ms(4000));
        // The first pause.
        for at in [0, 500, 999, 1000] {
            assert_eq!(marquee.offset_at(ms(at)), 0, "{at}");
        }
        // Scrolling: 30 px a second.
        assert_eq!(marquee.offset_at(ms(1100)), 3);
        assert_eq!(marquee.offset_at(ms(1500)), 15);
        assert_eq!(marquee.offset_at(ms(2000)), 30);
        assert_eq!(marquee.offset_at(ms(2999)), 59);
        // The end pause, then the next cycle starts again.
        for at in [3000, 3500, 3999] {
            assert_eq!(marquee.offset_at(ms(at)), 60, "{at}");
        }
        assert_eq!(marquee.offset_at(ms(4000)), 0);
        assert_eq!(marquee.offset_at(ms(5500)), 15);
        // Offsets never go back within a cycle and never pass the end.
        let mut last = 0;
        for at in (0..4000).step_by(7) {
            let offset = marquee.offset_at(ms(at));
            assert!(offset >= last && offset <= 60, "{at}");
            last = offset;
        }
    }

    #[test]
    fn the_next_change_skips_the_pauses() {
        let marquee = Marquee::new(60.0, 30.0, ms(1000));
        // From the start: the first pixel after the pause.
        let first = marquee.next_change(Duration::ZERO);
        assert!(first > ms(1033) && first <= ms(1034), "{first:?}");
        assert_eq!(marquee.offset_at(first), 1);
        // While scrolling: the next pixel.
        let at = ms(1500);
        let next = marquee.next_change(at);
        assert_eq!(marquee.offset_at(next), marquee.offset_at(at) + 1);
        // The end pause: the next cycle.
        assert_eq!(marquee.next_change(ms(3200)), ms(4000));
        assert_eq!(marquee.offset_at(ms(4000)), 0);
        // Every change it names is one, and none is missed in between.
        let mut at = Duration::ZERO;
        let mut changes = 0;
        while at < ms(8000) {
            let next = marquee.next_change(at);
            assert!(next > at);
            assert_eq!(
                marquee.offset_at(next.saturating_sub(Duration::from_nanos(1))),
                marquee.offset_at(at)
            );
            assert_ne!(marquee.offset_at(next), marquee.offset_at(at), "{next:?}");
            at = next;
            changes += 1;
        }
        // 60 steps and one jump back per cycle.
        assert_eq!(changes, 2 * 61);
    }

    #[test]
    fn a_cycle_is_at_most_fifteen_frames_a_second_and_pauses_are_one_frame() {
        // Fast: 120 px/s would be 120 frames a second; it is 15, moving
        // 8 px a frame.
        let fast = Marquee::new(120.0, 120.0, ms(1000));
        let frames = marquee_timeline(&fast, MARQUEE_FPS);
        let total: Duration = frames.iter().map(|(_, shown)| *shown).sum();
        assert_eq!(total, fast.cycle());
        // The pause, until the first pixel is reached.
        let (offset, shown) = frames[0];
        assert!(
            offset == 0 && shown >= ms(1000) && shown < ms(1010),
            "{shown:?}"
        );
        assert_eq!(frames.last().map(|(offset, _)| *offset), Some(120));
        // The end pause: the end may be reached up to a frame late, the
        // frames are never closer than a frame interval.
        let (_, end) = frames[frames.len() - 1];
        assert!(end + frame_interval(MARQUEE_FPS) >= ms(1000), "{end:?}");
        let moving = &frames[1..frames.len() - 1];
        for (_, shown) in moving {
            assert!(*shown >= frame_interval(MARQUEE_FPS), "{shown:?}");
        }
        // One second of scrolling at 15 frames a second.
        assert!((14..=16).contains(&moving.len()), "{}", moving.len());
        // Slow: 10 px/s changes 10 times a second, every pixel shown.
        let slow = Marquee::new(20.0, 10.0, ms(500));
        let frames = marquee_timeline(&slow, MARQUEE_FPS);
        let offsets: Vec<u32> = frames.iter().map(|(offset, _)| *offset).collect();
        assert_eq!(offsets, (0..=20).collect::<Vec<_>>());
        // Nothing to scroll: one frame.
        assert_eq!(
            marquee_timeline(&Marquee::new(0.0, 30.0, ms(500)), 15).len(),
            1
        );
    }

    #[test]
    fn nothing_to_scroll_stands_still() {
        let still = Marquee::new(0.0, 30.0, ms(500));
        for at in [0, 400, 900, 5000] {
            assert_eq!(still.offset_at(ms(at)), 0);
        }
        let no_pause = Marquee::new(10.0, 10.0, Duration::ZERO);
        assert_eq!(no_pause.offset_at(ms(500)), 5);
        assert_eq!(no_pause.cycle(), ms(1000));
    }
}

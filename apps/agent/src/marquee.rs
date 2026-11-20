//! Scrolling (marquee) titles on devices (`docs/key-text.md`).
//!
//! The service decides which keys scroll: those of the page a connected
//! device shows whose title is a marquee title wider than the key
//! ([`deckpilot_render::marquee`] at the device's key size). It hands that
//! list to the device's [`Animator`], one thread per device, which renders
//! the frames at the device's own key size and sends only frames that
//! differ from the last one sent (a pause is one frame), at most
//! [`MARQUEE_FPS`] a second per key. It never touches the service task, so
//! input handling never waits for it.
//!
//! Frames are tagged with an epoch ([`crate::devices::DeviceIo::next_epoch`]):
//! the service starts a new one whenever the list changes (a page switch, a
//! layout or profile change, locking), before it sends the keys' still
//! images, so a frame of the old list that was already on its way is
//! dropped by the device thread rather than drawn over the new page. A
//! device that disconnects takes its animator with it; on reconnecting the
//! list is made again.
//!
//! [`MarqueeScheduler`] is the timing without the thread, driven by an
//! injectable clock in tests.

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deckpilot_core::{DefaultAppearance, KeyFace};
use deckpilot_device::{ControlId, Frame};
use deckpilot_render::{Icons, Marquee};

/// Most frames a second of one scrolling key on a device.
pub const MARQUEE_FPS: u32 = deckpilot_render::MARQUEE_FPS;

/// Monotonic time since some fixed start; injectable for tests.
pub type MarqueeClock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The system's monotonic clock, from now.
pub fn monotonic_clock() -> MarqueeClock {
    let start = Instant::now();
    Arc::new(move || start.elapsed())
}

/// A key whose title scrolls, with the face, key size and model defaults
/// it is drawn with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollingKey {
    pub control: ControlId,
    pub face: KeyFace,
    pub width: u32,
    pub height: u32,
    /// The device model's colours for keys that set none.
    pub look: DefaultAppearance,
}

#[derive(Debug)]
struct Animation {
    key: ScrollingKey,
    marquee: Marquee,
    /// When it started, on the scheduler's clock.
    started: Duration,
    /// When its next frame is due.
    due: Duration,
    /// The offset last sent, if it was sent.
    shown: Option<u32>,
}

/// What the scheduler did, for tests and the cost measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarqueeStats {
    /// Frames rendered and sent.
    pub frames: usize,
    /// Frames due whose offset had not changed: not rendered, not sent.
    pub unchanged: usize,
}

/// The frames of scrolling keys, as a pure function of the time.
#[derive(Debug)]
pub struct MarqueeScheduler {
    animations: BTreeMap<ControlId, Animation>,
    icons: Icons,
    fps: u32,
    stats: MarqueeStats,
}

impl Default for MarqueeScheduler {
    fn default() -> Self {
        Self::new(MARQUEE_FPS)
    }
}

impl MarqueeScheduler {
    pub fn new(fps: u32) -> Self {
        Self {
            animations: BTreeMap::new(),
            icons: Icons::default(),
            fps: fps.max(1),
            stats: MarqueeStats::default(),
        }
    }

    /// Animates exactly `keys` from `now`, keys whose title does not scroll
    /// left out. A key that was animated with the same face and size keeps
    /// its place in the cycle; every key's current frame is sent again
    /// (frames of the list before may have been dropped).
    pub fn set(&mut self, keys: Vec<ScrollingKey>, icons: Icons, now: Duration) {
        let mut previous = std::mem::take(&mut self.animations);
        for key in keys {
            let Some(marquee) = deckpilot_render::marquee(&key.face, key.width, key.height, &icons)
            else {
                continue;
            };
            let started = previous
                .remove(&key.control)
                .filter(|old| old.key == key && old.marquee == marquee)
                .map_or(now, |old| old.started);
            self.animations.insert(
                key.control,
                Animation {
                    key,
                    marquee,
                    started,
                    due: now,
                    shown: None,
                },
            );
        }
        self.icons = icons;
    }

    /// Stops every animation.
    pub fn clear(&mut self) {
        self.animations.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    /// The controls animated, in order.
    pub fn controls(&self) -> Vec<ControlId> {
        self.animations.keys().copied().collect()
    }

    /// When the next frame is due; `None` when nothing scrolls.
    pub fn next_due(&self) -> Option<Duration> {
        self.animations.values().map(|a| a.due).min()
    }

    /// The frames due at `now` that differ from the ones sent: each key at
    /// its offset `now` (rendered at its key size). Keys whose offset did
    /// not change are skipped. The next frame of a key is due when its
    /// offset changes next, at most [`MarqueeScheduler::new`]'s frames a
    /// second.
    pub fn due(&mut self, now: Duration) -> Vec<(ControlId, Frame)> {
        let mut frames = Vec::new();
        for animation in self.animations.values_mut() {
            if animation.due > now {
                continue;
            }
            let elapsed = now.saturating_sub(animation.started);
            let offset = animation.marquee.offset_at(elapsed);
            if animation.shown == Some(offset) {
                self.stats.unchanged += 1;
            } else {
                let key = &animation.key;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "offsets are a few hundred pixels"
                )]
                let frame = deckpilot_render::render_scrolled(
                    &key.face,
                    key.width,
                    key.height,
                    &self.icons,
                    &key.look,
                    offset as f32,
                );
                frames.push((key.control, frame));
                animation.shown = Some(offset);
                self.stats.frames += 1;
            }
            animation.due = animation.started
                + deckpilot_render::next_frame(&animation.marquee, elapsed, self.fps);
        }
        frames
    }

    pub fn stats(&self) -> MarqueeStats {
        self.stats
    }
}

/// Where an animator sends its frames: the device's I/O thread, with the
/// epoch they were made in.
pub type FrameSink = Box<dyn Fn(u64, Vec<(ControlId, Frame)>) + Send>;

enum Message {
    Set {
        keys: Vec<ScrollingKey>,
        icons: Icons,
        epoch: u64,
    },
}

/// The scrolling keys of one device, on a thread of its own: it sleeps
/// until the next frame is due (or a new list arrives), and not at all
/// while nothing scrolls.
pub struct Animator {
    messages: Option<mpsc::Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Animator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Animator").finish_non_exhaustive()
    }
}

impl Animator {
    pub fn spawn(name: &str, sink: FrameSink, clock: MarqueeClock) -> std::io::Result<Self> {
        let (messages, inbox) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(format!("marquee-{name}"))
            .spawn(move || run(&inbox, &sink, &clock))?;
        Ok(Self {
            messages: Some(messages),
            thread: Some(thread),
        })
    }

    /// Animates `keys` from now on, their frames tagged with `epoch`.
    pub fn set(&self, keys: Vec<ScrollingKey>, icons: Icons, epoch: u64) {
        if let Some(messages) = &self.messages {
            let _ = messages.send(Message::Set { keys, icons, epoch });
        }
    }
}

impl Drop for Animator {
    /// Stops the thread: it ends after the frame it may be rendering.
    fn drop(&mut self) {
        self.messages = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(inbox: &mpsc::Receiver<Message>, sink: &FrameSink, clock: &MarqueeClock) {
    let mut scheduler = MarqueeScheduler::default();
    let mut epoch = 0;
    loop {
        let message = match scheduler.next_due() {
            None => inbox
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            Some(due) => inbox.recv_timeout(due.saturating_sub(clock())),
        };
        match message {
            Ok(Message::Set {
                keys,
                icons,
                epoch: next,
            }) => {
                epoch = next;
                scheduler.set(keys, icons, clock());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        let frames = scheduler.due(clock());
        if !frames.is_empty() {
            sink(epoch, frames);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use deckpilot_core::{FaceStyle, KeyAppearance, TextAppearance, TextMode};

    use super::*;

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn key(index: u16) -> ControlId {
        ControlId::Key { index }
    }

    fn scrolling(index: u16, title: &str, side: u32) -> ScrollingKey {
        let text = TextAppearance {
            mode: TextMode::Marquee,
            speed: Some(30),
            pause: Some(1000),
            ..TextAppearance::default()
        };
        ScrollingKey {
            control: key(index),
            face: KeyFace::new(FaceStyle::Action, title).with_appearance(Some(&KeyAppearance {
                text: Some(text),
                ..KeyAppearance::default()
            })),
            width: side,
            height: side,
            look: DefaultAppearance::BUILTIN,
        }
    }

    const LONG: &str = "Deploy the staging environment";

    /// Runs the scheduler from `from` to `to` as its thread would: woken
    /// when the next frame is due. Returns the frames with their times.
    fn play(
        scheduler: &mut MarqueeScheduler,
        from: Duration,
        to: Duration,
    ) -> Vec<(Duration, ControlId, Frame)> {
        let mut sent = Vec::new();
        let mut now = from;
        while now <= to {
            for (control, frame) in scheduler.due(now) {
                sent.push((now, control, frame));
            }
            match scheduler.next_due() {
                Some(due) => now = due.max(now + Duration::from_nanos(1)),
                None => break,
            }
        }
        sent
    }

    #[test]
    fn only_titles_that_scroll_are_animated() {
        let mut scheduler = MarqueeScheduler::default();
        let mut fits = scrolling(1, "Mail", 72);
        let wrapped = ScrollingKey {
            face: KeyFace::new(FaceStyle::Action, LONG),
            ..scrolling(2, LONG, 72)
        };
        scheduler.set(
            vec![scrolling(0, LONG, 72), fits.clone(), wrapped],
            Icons::default(),
            Duration::ZERO,
        );
        assert_eq!(scheduler.controls(), [key(0)]);
        // The first frame right away, at the device's key size.
        let frames = scheduler.due(Duration::ZERO);
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].1.width(), frames[0].1.height()), (72, 72));
        // Nothing scrolls: nothing is due, the thread sleeps.
        fits.face.title = "Ok".into();
        scheduler.set(vec![fits], Icons::default(), ms(10));
        assert!(scheduler.is_empty());
        assert_eq!(scheduler.next_due(), None);
        assert_eq!(scheduler.due(ms(5000)).len(), 0);
    }

    #[test]
    fn frames_are_bounded_and_unchanged_ones_are_not_sent() {
        let mut scheduler = MarqueeScheduler::new(MARQUEE_FPS);
        let key0 = scrolling(0, LONG, 72);
        let marquee = deckpilot_render::marquee(&key0.face, 72, 72, &Icons::default()).unwrap();
        scheduler.set(vec![key0], Icons::default(), Duration::ZERO);
        let cycle = marquee.cycle();
        let sent = play(&mut scheduler, Duration::ZERO, cycle * 3);
        // Never two frames of a key closer than a frame interval.
        let interval = deckpilot_render::frame_interval(MARQUEE_FPS);
        for pair in sent.windows(2) {
            assert!(
                pair[1].0.saturating_sub(pair[0].0) >= interval,
                "{:?}",
                pair[1].0
            );
        }
        // Each frame differs from the one before: none written twice.
        for pair in sent.windows(2) {
            assert_ne!(pair[0].2.rgb(), pair[1].2.rgb(), "{:?}", pair[1].0);
        }
        // The pauses are one frame each: nothing is sent during them.
        let in_pause = sent
            .iter()
            .filter(|(at, ..)| {
                let phase =
                    Duration::from_nanos(u64::try_from(at.as_nanos() % cycle.as_nanos()).unwrap());
                phase > ms(5) && phase < marquee.pause().saturating_sub(ms(5))
            })
            .count();
        assert_eq!(in_pause, 0);
        // At most 15 a second, far fewer than a frame per pixel at most.
        let seconds = (cycle * 3).as_secs_f64();
        #[expect(clippy::cast_precision_loss, reason = "a few hundred frames")]
        let rate = sent.len() as f64 / seconds;
        assert!(rate <= f64::from(MARQUEE_FPS) + 0.5, "{rate}");
        assert!(sent.len() >= 3 * 2, "{}", sent.len());
        // Waking up between changes sends nothing.
        let mut idle = MarqueeScheduler::new(MARQUEE_FPS);
        idle.set(
            vec![scrolling(0, LONG, 72)],
            Icons::default(),
            Duration::ZERO,
        );
        assert_eq!(idle.due(Duration::ZERO).len(), 1);
        // Still in the first pause: the same offset, not rendered.
        let due = idle.next_due().unwrap();
        assert!(due >= marquee.pause(), "sleeps through the pause: {due:?}");
        assert_eq!(idle.due(ms(500)).len(), 0);
    }

    #[test]
    #[expect(clippy::cast_precision_loss, reason = "offsets are small")]
    fn a_new_list_replaces_the_old_and_keeps_unchanged_keys_in_step() {
        let mut scheduler = MarqueeScheduler::default();
        let (a, b) = (
            scrolling(0, LONG, 72),
            scrolling(1, "Synchronise the archive", 72),
        );
        scheduler.set(vec![a.clone(), b.clone()], Icons::default(), Duration::ZERO);
        let sent = play(&mut scheduler, Duration::ZERO, ms(1500));
        assert!(sent.iter().any(|(_, c, _)| *c == key(0)));
        assert!(sent.iter().any(|(_, c, _)| *c == key(1)));
        // Another page: key 1 is gone; key 0 (the same face) continues
        // where it was and its current frame is sent again.
        scheduler.set(vec![a.clone()], Icons::default(), ms(1500));
        let again = scheduler.due(ms(1500));
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].0, key(0));
        let expected = deckpilot_render::render_scrolled(
            &a.face,
            72,
            72,
            &Icons::default(),
            &a.look,
            deckpilot_render::marquee(&a.face, 72, 72, &Icons::default())
                .unwrap()
                .offset_at(ms(1500)) as f32,
        );
        assert_eq!(again[0].1.rgb(), expected.rgb());
        let later = play(&mut scheduler, ms(1501), ms(4000));
        assert!(later.iter().all(|(_, c, _)| *c == key(0)));
        // A changed face starts from the beginning.
        let mut changed = a;
        changed.face.title = "Deploy the production environment".into();
        scheduler.set(vec![changed.clone()], Icons::default(), ms(4000));
        let restarted = scheduler.due(ms(4000));
        let start = deckpilot_render::render_scrolled(
            &changed.face,
            72,
            72,
            &Icons::default(),
            &changed.look,
            0.0,
        );
        assert_eq!(restarted[0].1.rgb(), start.rgb());
        // Locked, another profile: nothing.
        scheduler.set(Vec::new(), Icons::default(), ms(4100));
        assert!(scheduler.is_empty());
    }

    /// Different devices: each key is drawn and timed at its own size.
    #[test]
    fn every_key_size_is_animated_at_its_own_resolution() {
        for side in [72, 80, 96, 120] {
            let mut scheduler = MarqueeScheduler::default();
            scheduler.set(
                vec![scrolling(0, LONG, side)],
                Icons::default(),
                Duration::ZERO,
            );
            let frames = play(&mut scheduler, Duration::ZERO, ms(3000));
            assert!(frames.len() > 2, "{side}");
            assert!(
                frames
                    .iter()
                    .all(|(_, _, f)| (f.width(), f.height()) == (side, side))
            );
        }
    }

    /// The thread: frames arrive tagged with the list's epoch, and none
    /// after it was dropped.
    #[test]
    fn the_animator_thread_sends_tagged_frames_until_dropped() {
        let received: Arc<Mutex<Vec<(u64, ControlId)>>> = Arc::default();
        let sink_received = received.clone();
        let animator = Animator::spawn(
            "test",
            Box::new(move |epoch, frames| {
                let mut list = sink_received.lock().unwrap();
                list.extend(frames.into_iter().map(|(control, _)| (epoch, control)));
            }),
            monotonic_clock(),
        )
        .unwrap();
        animator.set(vec![scrolling(3, LONG, 72)], Icons::default(), 7);
        let deadline = Instant::now() + Duration::from_secs(10);
        while received.lock().unwrap().len() < 3 {
            assert!(Instant::now() < deadline, "no frames");
            std::thread::sleep(ms(20));
        }
        assert!(
            received
                .lock()
                .unwrap()
                .iter()
                .all(|entry| *entry == (7, key(3)))
        );
        animator.set(Vec::new(), Icons::default(), 8);
        drop(animator);
        let count = received.lock().unwrap().len();
        std::thread::sleep(ms(300));
        assert_eq!(received.lock().unwrap().len(), count, "stopped");
    }
}

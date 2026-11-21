//! Token-bucket rate limits with an injectable clock.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Monotonic and wall time, injectable for tests.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Monotonic time since an arbitrary fixed point.
    fn now(&self) -> Duration;
    /// Milliseconds since the Unix epoch (audit timestamps only).
    fn wall_ms(&self) -> u64;
}

#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn wall_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to.
#[derive(Debug, Default, Clone)]
pub struct ManualClock(Arc<Mutex<Duration>>);

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wall_ms(&self) -> u64 {
        self.now().as_millis() as u64
    }
}

/// `capacity` tokens, refilled continuously at `capacity / per`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub capacity: u32,
    pub per: Duration,
}

impl Rate {
    pub const fn new(capacity: u32, per: Duration) -> Self {
        Self { capacity, per }
    }
}

/// Limits per plugin (design §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimits {
    /// Every request, allowed or not.
    pub any: Rate,
    pub http: Rate,
    pub notify: Rate,
    pub log: Rate,
    pub storage: Rate,
    /// Publishing or clearing own variables (10/s).
    pub variables: Rate,
    /// Opening a URL in the browser (design §5.2: 1 per 2 s).
    pub open_url: Rate,
    /// Opening URLs over a longer window (20 per 10 minutes, refilled
    /// continuously): a plugin cannot keep opening tabs at the short rate.
    pub open_url_window: Rate,
    /// `time.now` (20/s).
    pub time: Rate,
    /// `random.bytes`, counted in **bytes**: 4,096 per minute, refilled
    /// continuously, so one run is bounded as well.
    pub random_bytes: Rate,
    /// `dialog.open` (plugin API 1.5): 6 per minute, refilled
    /// continuously.
    pub dialog: Rate,
    /// `tool.run` starts (plugin API 1.9, X3): 20 per second.
    pub tool_run: Rate,
    /// `tool.run` with `tools:docker:modify` (X3c): 6 per minute.
    pub tool_modify: Rate,
    /// `resource.open` (X4): 1 per second, with a burst of 5 (its
    /// per-invocation cap), so `open-project` can open its editor, docs
    /// and pages at once.
    pub resource_open: Rate,
    /// `url.openParam` (X5): 1 per 2 s, with a burst of 10 (its
    /// per-invocation cap); `open_url_window` applies as well.
    pub open_url_param: Rate,
    /// `project.select` and `project.cycle` (X6): 2 per second without a
    /// burst, so a second switch within 500 ms is throttled.
    pub project_switch: Rate,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            any: Rate::new(200, Duration::from_secs(1)),
            http: Rate::new(60, Duration::from_secs(60)),
            notify: Rate::new(5, Duration::from_secs(60)),
            log: Rate::new(50, Duration::from_secs(1)),
            storage: Rate::new(100, Duration::from_secs(1)),
            variables: Rate::new(10, Duration::from_secs(1)),
            open_url: Rate::new(1, Duration::from_secs(2)),
            open_url_window: Rate::new(20, Duration::from_secs(600)),
            time: Rate::new(20, Duration::from_secs(1)),
            random_bytes: Rate::new(4096, Duration::from_secs(60)),
            dialog: Rate::new(6, Duration::from_secs(60)),
            tool_run: Rate::new(20, Duration::from_secs(1)),
            tool_modify: Rate::new(6, Duration::from_secs(60)),
            resource_open: Rate::new(5, Duration::from_secs(5)),
            open_url_param: Rate::new(10, Duration::from_secs(20)),
            project_switch: Rate::new(1, Duration::from_millis(500)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Bucket {
    tokens: f64,
    last: Duration,
}

impl Bucket {
    pub(crate) fn new(rate: Rate, now: Duration) -> Self {
        Self {
            tokens: f64::from(rate.capacity),
            last: now,
        }
    }

    /// Takes one token if available.
    #[cfg(test)]
    pub(crate) fn take(&mut self, rate: Rate, now: Duration) -> bool {
        self.take_n(rate, now, 1)
    }

    /// Takes `n` tokens if all of them are available, none otherwise.
    pub(crate) fn take_n(&mut self, rate: Rate, now: Duration, n: u32) -> bool {
        let elapsed = now.saturating_sub(self.last).as_secs_f64();
        let per = rate.per.as_secs_f64().max(f64::MIN_POSITIVE);
        let cap = f64::from(rate.capacity);
        self.tokens = (self.tokens + elapsed * cap / per).min(cap);
        self.last = now;
        let n = f64::from(n);
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_tokens_are_taken_together_or_not_at_all() {
        let rate = Rate::new(10, Duration::from_secs(10));
        let mut b = Bucket::new(rate, Duration::ZERO);
        assert!(b.take_n(rate, Duration::ZERO, 6));
        // Four are left: six are refused and nothing is taken.
        assert!(!b.take_n(rate, Duration::ZERO, 6));
        assert!(b.take_n(rate, Duration::ZERO, 4));
        assert!(!b.take_n(rate, Duration::ZERO, 1));
        // One token per second comes back.
        assert!(b.take_n(rate, Duration::from_secs(3), 3));
        assert!(!b.take_n(rate, Duration::from_secs(3), 1));
        // More than the capacity is never available.
        assert!(!b.take_n(rate, Duration::from_secs(3600), 11));
    }

    #[test]
    fn bucket_refills() {
        let rate = Rate::new(2, Duration::from_secs(1));
        let mut b = Bucket::new(rate, Duration::ZERO);
        assert!(b.take(rate, Duration::ZERO));
        assert!(b.take(rate, Duration::ZERO));
        assert!(!b.take(rate, Duration::ZERO));
        assert!(!b.take(rate, Duration::from_millis(100)));
        assert!(b.take(rate, Duration::from_millis(600)));
        // Never more than capacity, however long it was idle.
        let later = Duration::from_secs(3600);
        assert!(b.take(rate, later));
        assert!(b.take(rate, later));
        assert!(!b.take(rate, later));
    }
}

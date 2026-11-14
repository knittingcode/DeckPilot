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
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            any: Rate::new(200, Duration::from_secs(1)),
            http: Rate::new(60, Duration::from_secs(60)),
            notify: Rate::new(5, Duration::from_secs(60)),
            log: Rate::new(50, Duration::from_secs(1)),
            storage: Rate::new(100, Duration::from_secs(1)),
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
    pub(crate) fn take(&mut self, rate: Rate, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.last).as_secs_f64();
        let per = rate.per.as_secs_f64().max(f64::MIN_POSITIVE);
        let cap = f64::from(rate.capacity);
        self.tokens = (self.tokens + elapsed * cap / per).min(cap);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
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

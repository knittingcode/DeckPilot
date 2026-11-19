//! System monitor keys: CPU, memory, network and disk use.
//!
//! One [`Sampler`] per module reads the values for every key and action
//! that shows them, with one `sysinfo` instance. A value is read again at
//! most every [`SAMPLE_INTERVAL`]; keys shown at the same time share it.
//! Nothing runs in the background: values are read only when a shown key's
//! state or an action asks for them, so when no such key is shown nothing
//! is read at all.
//!
//! CPU use and network throughput are the difference between two readings.
//! The first reading, and one after a pause longer than [`STALE_AFTER`]
//! (no key was shown), takes a second reading [`PRIME_WAIT`] later, so a
//! value is never an average over the pause.
//!
//! The rest of the module is pure: thresholds, units and the key states,
//! tested with injected values.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use deckpilot_core::{ButtonState, ExecutionContext, Indicator, MAX_STATE_SERIES, StateError};

use crate::codes;

/// How often a shown CPU, memory or network key's state is read at most.
pub const STATE_INTERVAL: Duration = Duration::from_secs(2);
/// How often a shown disk key's state is read at most.
pub const DISK_STATE_INTERVAL: Duration = Duration::from_secs(30);
/// A reading younger than this is shared instead of read again.
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(1500);
/// A disk list younger than this is shared instead of read again.
pub const DISK_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
/// A reading older than this is no base for a difference: the next value
/// takes two readings.
pub const STALE_AFTER: Duration = Duration::from_secs(10);
/// Time between the two readings of a first CPU or network value.
pub const PRIME_WAIT: Duration =
    sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.saturating_add(Duration::from_millis(50));

/// How a value compares with the key's thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warning,
    Critical,
}

impl Level {
    /// The `level` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    pub fn indicator(self) -> Indicator {
        match self {
            Self::Ok => Indicator::Ok,
            Self::Warning => Indicator::Warning,
            Self::Critical => Indicator::Error,
        }
    }
}

/// Thresholds in percent. For use (CPU, memory) a value at or above a
/// threshold reaches it; for free space (disk), at or below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub warning: u8,
    pub critical: u8,
}

impl Thresholds {
    /// The level of a use: critical first, so thresholds in the wrong order
    /// still give the stronger level.
    pub fn of_use(self, percent: u8) -> Level {
        if percent >= self.critical {
            Level::Critical
        } else if percent >= self.warning {
            Level::Warning
        } else {
            Level::Ok
        }
    }

    /// The level of a free share.
    pub fn of_free(self, percent: u8) -> Level {
        if percent <= self.critical {
            Level::Critical
        } else if percent <= self.warning {
            Level::Warning
        } else {
            Level::Ok
        }
    }
}

/// `part` of `total` in whole percent (0–100); `None` without a total.
pub fn percent(part: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let share = (part.min(total) as f64 / total as f64 * 100.0).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(share as u8)
}

/// A CPU use as whole percent; `None` when the system reported none.
pub fn cpu_percent(usage: f32) -> Option<u8> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    usage
        .is_finite()
        .then(|| usage.clamp(0.0, 100.0).round() as u8)
}

const RATE_UNITS: [&str; 5] = ["B", "k", "M", "G", "T"];

/// `value` in units of 1000: at most three digits, one decimal below 10.
fn scaled(value: f64) -> (String, usize) {
    let mut value = if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    };
    let mut unit = 0;
    loop {
        let text = if unit == 0 || value >= 9.95 {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        };
        let shown: f64 = text.parse().unwrap_or(value);
        if shown >= 1000.0 && unit + 1 < RATE_UNITS.len() {
            value /= 1000.0;
            unit += 1;
            continue;
        }
        return (text, unit);
    }
}

/// A throughput for messages, e.g. `1.2 MB/s` or `56 kB/s`.
pub fn rate(bytes_per_second: f64) -> String {
    let (number, unit) = scaled(bytes_per_second);
    let unit = if unit == 0 { "" } else { RATE_UNITS[unit] };
    format!("{number} {unit}B/s")
}

/// A throughput for a key, at most four characters, e.g. `1.2M` or `56K`.
pub fn compact_rate(bytes_per_second: f64) -> String {
    let (number, unit) = scaled(bytes_per_second);
    format!("{number}{}", RATE_UNITS[unit].to_ascii_uppercase())
}

/// A size in GiB without decimals from 10 on, e.g. `32` or `7.5`.
pub fn short_gib(bytes: u64) -> String {
    let text = gib(bytes);
    match text.split_once('.') {
        Some((whole, _)) if whole.len() >= 2 => whole.to_owned(),
        _ => text,
    }
}

/// A size in GiB with one decimal, e.g. `15.8`.
pub fn gib(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64 / f64::from(1u32 << 30);
    format!("{value:.1}")
}

/// What an action reports and its key shows, from one reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub message: String,
    /// The action's outputs.
    pub data: Vec<(&'static str, String)>,
    pub state: ButtonState,
}

/// `system.cpu`: `37%` under the key's title with the use as a bar, the recent values as a
/// chart and the level as the indicator.
pub fn cpu_report(usage: u8, history: &[u8], thresholds: Thresholds) -> Report {
    let level = thresholds.of_use(usage);
    let mut state = ButtonState::new()
        .with_detail(format!("{usage}%"))
        .with_indicator(level.indicator())
        .with_progress(usage);
    if history.len() >= 2 {
        state = state.with_series(history.to_vec());
    }
    Report {
        message: format!("CPU {usage}%"),
        data: vec![
            ("usagePercent", usage.to_string()),
            ("level", level.as_str().into()),
        ],
        state,
    }
}

/// Memory as read: bytes in use and installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub used: u64,
    pub total: u64,
}

/// `system.memory`: used and installed GiB under the key's title, e.g.
/// `9.9/16 GiB`, with the used share as a bar; the message adds the share.
///
/// # Errors
///
/// `unavailable` when the system reported no memory.
pub fn memory_report(memory: Memory, thresholds: Thresholds) -> Result<Report, StateError> {
    let used = percent(memory.used, memory.total).ok_or_else(|| unavailable("memory"))?;
    let level = thresholds.of_use(used);
    Ok(Report {
        message: format!(
            "memory {used}% used ({} of {} GiB)",
            gib(memory.used),
            gib(memory.total)
        ),
        data: vec![
            ("usedPercent", used.to_string()),
            ("usedBytes", memory.used.to_string()),
            ("totalBytes", memory.total.to_string()),
            ("level", level.as_str().into()),
        ],
        state: ButtonState::new()
            .with_detail(format!(
                "{}/{} GiB",
                gib(memory.used),
                short_gib(memory.total)
            ))
            .with_indicator(level.indicator())
            .with_progress(used),
    })
}

/// One network interface's throughput between two readings.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceRate {
    pub name: String,
    pub received: f64,
    pub transmitted: f64,
    pub loopback: bool,
}

/// Whether an interface only reaches this computer: all its addresses are
/// loopback addresses, or, without addresses, its name says so (`lo`,
/// `lo0`, Windows' `Loopback Pseudo-Interface 1`).
pub fn is_loopback(name: &str, addresses: &[IpAddr]) -> bool {
    if !addresses.is_empty() {
        return addresses.iter().all(IpAddr::is_loopback);
    }
    let name = name.trim().to_ascii_lowercase();
    let numbered = name
        .strip_prefix("lo")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()));
    numbered || name.contains("loopback")
}

/// `system.network`: the named interface's throughput (case ignored), or
/// the sum of every interface but loopback.
///
/// # Errors
///
/// `not-found` when no interface has the name.
pub fn network_report(
    rates: &[InterfaceRate],
    interface: Option<&str>,
) -> Result<Report, StateError> {
    let wanted = interface.map(str::trim).filter(|name| !name.is_empty());
    let counted: Vec<&InterfaceRate> = match wanted {
        Some(name) => {
            let found = rates
                .iter()
                .find(|rate| rate.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    StateError::new(
                        codes::NOT_FOUND,
                        format!("no network interface is named “{name}”"),
                    )
                })?;
            vec![found]
        }
        None => rates.iter().filter(|rate| !rate.loopback).collect(),
    };
    let received: f64 = counted.iter().map(|rate| rate.received).sum();
    let transmitted: f64 = counted.iter().map(|rate| rate.transmitted).sum();
    let names: Vec<&str> = counted.iter().map(|rate| rate.name.as_str()).collect();
    let shown = format!("in {}, out {}", rate(received), rate(transmitted));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = |value: f64| (value.max(0.0).round() as u64).to_string();
    Ok(Report {
        message: match wanted {
            Some(name) => format!("{name}: {shown}"),
            None => shown,
        },
        data: vec![
            ("receivedPerSecond", whole(received)),
            ("transmittedPerSecond", whole(transmitted)),
            ("interfaces", names.join(",")),
        ],
        // Received and sent, one line each under the key's title.
        state: ButtonState::new().with_detail(format!(
            "\u{2193} {}\n\u{2191} {}",
            rate(received),
            rate(transmitted)
        )),
    })
}

/// A mounted volume as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub mount_point: PathBuf,
    pub total: u64,
    pub available: u64,
}

/// A path as compared with mount points: without Windows' `\\?\`, with
/// `/` separators, no trailing separator, and on Windows in lower case.
fn comparable(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut text = text.replace('\\', "/");
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    text
}

/// The volume `path` is on: the one with the longest mount point that
/// contains it. Volumes without a size are ignored.
pub fn volume_for<'a>(path: &Path, volumes: &'a [Volume]) -> Option<&'a Volume> {
    let path = comparable(path);
    volumes
        .iter()
        .filter(|volume| volume.total > 0)
        .filter_map(|volume| {
            let mount = comparable(&volume.mount_point);
            let inside = path == mount
                || mount == "/"
                || path
                    .strip_prefix(&mount)
                    .is_some_and(|rest| rest.starts_with('/'));
            inside.then_some((mount.len(), volume))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, volume)| volume)
}

/// `system.disk`: the used share under the key's title, e.g. `55% used`,
/// also as a bar; the level of the free share is the indicator.
///
/// # Errors
///
/// `unavailable` when the volume reported no size.
pub fn disk_report(volume: &Volume, thresholds: Thresholds) -> Result<Report, StateError> {
    let free = percent(volume.available, volume.total).ok_or_else(|| unavailable("disk space"))?;
    let level = thresholds.of_free(free);
    let mount = volume.mount_point.display().to_string();
    Ok(Report {
        message: format!(
            "{free}% free on {mount} ({} of {} GiB)",
            gib(volume.available),
            gib(volume.total)
        ),
        data: vec![
            ("freePercent", free.to_string()),
            ("freeBytes", volume.available.to_string()),
            ("totalBytes", volume.total.to_string()),
            ("mountPoint", mount),
            ("level", level.as_str().into()),
        ],
        state: ButtonState::new()
            .with_detail(format!("{}% used", 100 - free))
            .with_indicator(level.indicator())
            .with_progress(100 - free),
    })
}

fn unavailable(what: &str) -> StateError {
    StateError::new(
        codes::UNAVAILABLE,
        format!("this computer does not report its {what}"),
    )
}

fn cancelled() -> StateError {
    StateError::new("cancelled", "cancelled")
}

#[derive(Debug, Clone)]
struct Timed<T> {
    at: Instant,
    value: T,
}

impl<T> Timed<T> {
    fn younger_than(&self, now: Instant, age: Duration) -> bool {
        now.saturating_duration_since(self.at) < age
    }
}

#[derive(Default)]
struct Inner {
    system: Option<sysinfo::System>,
    cpu: Option<Timed<f32>>,
    cpu_history: VecDeque<u8>,
    networks: Option<sysinfo::Networks>,
    network: Option<Timed<Vec<InterfaceRate>>>,
    memory: Option<Timed<Memory>>,
    volumes: Option<Timed<Vec<Volume>>>,
}

/// The module's shared reader of system values; see the
/// [module documentation](self).
#[derive(Default)]
pub struct Sampler {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler").finish_non_exhaustive()
    }
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Callers wait for a reading in progress and then share it.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The total CPU use in whole percent and the recent values, oldest
    /// first.
    ///
    /// # Errors
    ///
    /// `unavailable` when the system reports no CPU use, `cancelled` when
    /// the context was cancelled while waiting for a second reading.
    pub fn cpu(&self, context: &ExecutionContext<'_>) -> Result<(u8, Vec<u8>), StateError> {
        let mut inner = self.lock();
        let now = Instant::now();
        let last = inner.cpu.clone();
        let usage = match last {
            Some(last) if last.younger_than(now, SAMPLE_INTERVAL) => last.value,
            last => {
                let primed = last.is_some_and(|last| last.younger_than(now, STALE_AFTER));
                if !primed {
                    inner.cpu_history.clear();
                }
                let system = inner.system.get_or_insert_with(sysinfo::System::new);
                if !primed {
                    system.refresh_cpu_usage();
                    if !context.wait(PRIME_WAIT) {
                        return Err(cancelled());
                    }
                }
                system.refresh_cpu_usage();
                let usage = system.global_cpu_usage();
                let value = cpu_percent(usage).ok_or_else(|| unavailable("CPU use"))?;
                inner.cpu = Some(Timed {
                    at: Instant::now(),
                    value: usage,
                });
                inner.cpu_history.push_back(value);
                while inner.cpu_history.len() > MAX_STATE_SERIES {
                    inner.cpu_history.pop_front();
                }
                usage
            }
        };
        let value = cpu_percent(usage).ok_or_else(|| unavailable("CPU use"))?;
        Ok((value, inner.cpu_history.iter().copied().collect()))
    }

    /// Memory in use and installed.
    ///
    /// # Errors
    ///
    /// `unavailable` when the system reports no memory.
    pub fn memory(&self) -> Result<Memory, StateError> {
        let mut inner = self.lock();
        let now = Instant::now();
        if let Some(last) = &inner.memory
            && last.younger_than(now, SAMPLE_INTERVAL)
        {
            return Ok(last.value);
        }
        let system = inner.system.get_or_insert_with(sysinfo::System::new);
        system.refresh_memory();
        let memory = Memory {
            used: system.used_memory(),
            total: system.total_memory(),
        };
        if memory.total == 0 {
            return Err(unavailable("memory"));
        }
        inner.memory = Some(Timed {
            at: now,
            value: memory,
        });
        Ok(memory)
    }

    /// Every interface's throughput since the previous reading.
    ///
    /// # Errors
    ///
    /// `cancelled` when the context was cancelled while waiting for a
    /// second reading.
    pub fn network(
        &self,
        context: &ExecutionContext<'_>,
    ) -> Result<Vec<InterfaceRate>, StateError> {
        let mut inner = self.lock();
        let now = Instant::now();
        let last_at = match &inner.network {
            Some(last) if last.younger_than(now, SAMPLE_INTERVAL) => return Ok(last.value.clone()),
            Some(last) if last.younger_than(now, STALE_AFTER) && inner.networks.is_some() => {
                Some(last.at)
            }
            _ => None,
        };
        let (mut networks, since) =
            if let (Some(networks), Some(at)) = (inner.networks.take(), last_at) {
                (networks, at)
            } else {
                let networks = sysinfo::Networks::new_with_refreshed_list();
                let at = Instant::now();
                if !context.wait(PRIME_WAIT) {
                    return Err(cancelled());
                }
                (networks, at)
            };
        networks.refresh(true);
        let at = Instant::now();
        let seconds = at.saturating_duration_since(since).as_secs_f64().max(0.001);
        #[allow(clippy::cast_precision_loss)]
        let mut rates: Vec<InterfaceRate> = networks
            .list()
            .iter()
            .map(|(name, data)| {
                let addresses: Vec<IpAddr> = data
                    .ip_networks()
                    .iter()
                    .map(|network| network.addr)
                    .collect();
                InterfaceRate {
                    name: name.clone(),
                    received: data.received() as f64 / seconds,
                    transmitted: data.transmitted() as f64 / seconds,
                    loopback: is_loopback(name, &addresses),
                }
            })
            .collect();
        rates.sort_by(|a, b| a.name.cmp(&b.name));
        inner.networks = Some(networks);
        inner.network = Some(Timed {
            at,
            value: rates.clone(),
        });
        Ok(rates)
    }

    /// The mounted volumes.
    pub fn volumes(&self) -> Vec<Volume> {
        let mut inner = self.lock();
        let now = Instant::now();
        if let Some(last) = &inner.volumes
            && last.younger_than(now, DISK_SAMPLE_INTERVAL)
        {
            return last.value.clone();
        }
        let volumes: Vec<Volume> = sysinfo::Disks::new_with_refreshed_list()
            .list()
            .iter()
            .map(|disk| Volume {
                mount_point: disk.mount_point().to_path_buf(),
                total: disk.total_space(),
                available: disk.available_space(),
            })
            .collect();
        inner.volumes = Some(Timed {
            at: now,
            value: volumes.clone(),
        });
        volumes
    }
}

#[cfg(test)]
mod tests;

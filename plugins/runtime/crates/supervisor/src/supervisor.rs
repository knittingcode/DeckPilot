//! Restart policy around [`PluginHost`] (design §3.6): after a crash the
//! host is started again on demand with backoff 1, 2, 4 … 60 s; five
//! crashes within 10 minutes disable the plugin until the user re-enables
//! it. Spawn and load failures count as crashes.

use std::collections::VecDeque;
use std::fmt;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::{ActionOutcome, Params};
use deckpilot_plugin_broker::{Broker, Clock, PluginId};

use crate::host::{CapabilityExecutor, HostConfig, HostError, PluginHost};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub crash_window: Duration,
    pub max_crashes_in_window: usize,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            crash_window: Duration::from_secs(600),
            max_crashes_in_window: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorState {
    /// No host process; the next invocation starts one.
    Idle,
    Running,
    /// No host process; starting one is allowed after `retry_after`.
    BackingOff {
        retry_after: Duration,
    },
    /// Crash loop: disabled until [`Supervisor::reenable`].
    Disabled,
}

#[derive(Debug)]
pub enum SupervisorError {
    Host(HostError),
    BackingOff { retry_after: Duration },
    Disabled,
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(e) => write!(f, "host: {e}"),
            Self::BackingOff { retry_after } => write!(f, "restarting in {retry_after:?}"),
            Self::Disabled => f.write_str("disabled after repeated crashes"),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// Supervises the host of one plugin.
pub struct Supervisor {
    config: HostConfig,
    broker: Broker,
    plugin: PluginId,
    policy: RestartPolicy,
    clock: Arc<dyn Clock>,
    host: Option<PluginHost>,
    crashes: VecDeque<Duration>,
    consecutive: u32,
    next_start: Option<Duration>,
    disabled: bool,
    last_exit: Option<ExitStatus>,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("plugin", &self.plugin)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    pub fn new(
        config: HostConfig,
        broker: Broker,
        plugin: PluginId,
        policy: RestartPolicy,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            broker,
            plugin,
            policy,
            clock,
            host: None,
            crashes: VecDeque::new(),
            consecutive: 0,
            next_start: None,
            disabled: false,
            last_exit: None,
        }
    }

    pub fn state(&self) -> SupervisorState {
        if self.disabled {
            return SupervisorState::Disabled;
        }
        if self.host.as_ref().is_some_and(PluginHost::is_running) {
            return SupervisorState::Running;
        }
        match self.next_start {
            Some(at) if at > self.clock.now() => SupervisorState::BackingOff {
                retry_after: at - self.clock.now(),
            },
            _ => SupervisorState::Idle,
        }
    }

    /// Exit status of the last host that ended.
    pub fn last_exit(&self) -> Option<ExitStatus> {
        self.last_exit
    }

    pub fn host(&self) -> Option<&PluginHost> {
        self.host.as_ref()
    }

    /// Starts the host now if allowed (otherwise on the next invocation).
    pub fn start(&mut self) -> Result<(), SupervisorError> {
        self.ensure_host().map(|_| ())
    }

    /// Runs one action, starting the host first if needed.
    pub fn invoke(
        &mut self,
        action: &str,
        params: &Params,
        timeout: Duration,
        executor: &mut dyn CapabilityExecutor,
    ) -> Result<ActionOutcome, SupervisorError> {
        let host = self.ensure_host()?;
        match host.invoke(action, params, timeout, executor) {
            Ok(outcome) => {
                self.consecutive = 0;
                Ok(outcome)
            }
            Err(e) if e.host_died() => {
                self.last_exit = e.status();
                self.host = None;
                self.record_crash();
                Err(SupervisorError::Host(e))
            }
            Err(e) => Err(SupervisorError::Host(e)),
        }
    }

    /// Clears the crash history after the user re-enabled the plugin.
    pub fn reenable(&mut self) {
        self.disabled = false;
        self.crashes.clear();
        self.consecutive = 0;
        self.next_start = None;
    }

    /// Stops the host (shutdown, then kill after the grace period).
    pub fn shutdown(&mut self) -> Option<ExitStatus> {
        let status = self
            .host
            .take()
            .and_then(|h| h.shutdown(self.config.shutdown_grace));
        if status.is_some() {
            self.last_exit = status;
        }
        status
    }

    fn ensure_host(&mut self) -> Result<&mut PluginHost, SupervisorError> {
        if self.disabled {
            return Err(SupervisorError::Disabled);
        }
        if self.host.as_ref().is_some_and(|h| !h.is_running()) {
            self.host = None;
        }
        if self.host.is_none() {
            let now = self.clock.now();
            if let Some(at) = self.next_start.filter(|at| *at > now) {
                return Err(SupervisorError::BackingOff {
                    retry_after: at - now,
                });
            }
            let session = self.broker.session(self.plugin.clone());
            match PluginHost::spawn(&self.config, session) {
                Ok(h) => self.host = Some(h),
                Err(e) => {
                    self.last_exit = e.status();
                    self.record_crash();
                    return Err(SupervisorError::Host(e));
                }
            }
        }
        Ok(self.host.as_mut().expect("host was just set"))
    }

    fn record_crash(&mut self) {
        let now = self.clock.now();
        self.crashes.push_back(now);
        while self
            .crashes
            .front()
            .is_some_and(|t| now.saturating_sub(*t) > self.policy.crash_window)
        {
            self.crashes.pop_front();
        }
        if self.crashes.len() >= self.policy.max_crashes_in_window {
            self.disabled = true;
            return;
        }
        let factor = 1u32.checked_shl(self.consecutive).unwrap_or(u32::MAX);
        let backoff = self
            .policy
            .initial_backoff
            .saturating_mul(factor)
            .min(self.policy.max_backoff);
        self.consecutive = self.consecutive.saturating_add(1);
        self.next_start = Some(now + backoff);
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

//! Audit records (design §5.7): operation, target and decision, never a
//! value. The target is an origin for network requests and absent for
//! everything else; a URL path or query, a header, a body, a log line, a
//! notification text and a storage key or value are never recorded.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::DenyReason;
use crate::plugin_id::PluginId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub seq: u64,
    pub at_ms: u64,
    pub plugin: PluginId,
    pub op: &'static str,
    /// `https://host[:port]` for network operations; `None` otherwise.
    pub target: Option<String>,
    pub decision: AuditDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDecision {
    Allowed,
    Denied(DenyReason),
}

/// Where audit records go. Implementations must not block for long: the
/// broker calls this on every decision.
pub trait AuditSink: Send + Sync + std::fmt::Debug {
    fn record(&self, record: AuditRecord);
}

/// Bounded in-memory sink (oldest records dropped first).
#[derive(Debug)]
pub struct MemoryAudit {
    capacity: usize,
    records: Mutex<VecDeque<AuditRecord>>,
}

impl MemoryAudit {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: Mutex::new(VecDeque::new()),
        }
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, record: AuditRecord) {
        let mut r = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if r.len() == self.capacity {
            r.pop_front();
        }
        r.push_back(record);
    }
}

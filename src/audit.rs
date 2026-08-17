use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CAPACITY: usize = 1024;

#[derive(Clone, Debug, Serialize)]
pub struct AuditEvent {
    pub timestamp_ms: u128,
    pub sandbox_id: String,
    pub policy_hash: Option<String>,
    pub phase: String,
    pub source: String,
    pub decision: String,
    pub capability: String,
    pub target: String,
    pub rule_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditSnapshot {
    pub events: Vec<AuditEvent>,
    pub dropped_events: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditSummary {
    pub event_count: usize,
    pub dropped_events: u64,
    pub decisions: BTreeMap<String, u64>,
    pub capabilities: BTreeMap<String, u64>,
}

#[derive(Clone)]
pub struct AuditSink(Arc<Mutex<AuditBuffer>>);

struct AuditBuffer {
    sandbox_id: String,
    policy_hash: Option<String>,
    phase: String,
    capacity: usize,
    events: VecDeque<AuditEvent>,
    dropped_events: u64,
}

impl AuditSink {
    pub fn new(sandbox_id: String, policy_hash: Option<String>) -> Self {
        Self::with_capacity(sandbox_id, policy_hash, DEFAULT_CAPACITY)
    }

    fn with_capacity(sandbox_id: String, policy_hash: Option<String>, capacity: usize) -> Self {
        Self(Arc::new(Mutex::new(AuditBuffer {
            sandbox_id,
            policy_hash,
            phase: "static".into(),
            capacity,
            events: VecDeque::with_capacity(capacity),
            dropped_events: 0,
        })))
    }

    pub fn record(
        &self,
        source: &str,
        decision: &str,
        capability: &str,
        target: impl Into<String>,
        rule_id: Option<String>,
        reason: impl Into<String>,
    ) {
        let mut buffer = self.0.lock().unwrap();
        if buffer.events.len() == buffer.capacity {
            buffer.events.pop_front();
            buffer.dropped_events += 1;
        }
        let event = AuditEvent {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            sandbox_id: buffer.sandbox_id.clone(),
            policy_hash: buffer.policy_hash.clone(),
            phase: buffer.phase.clone(),
            source: source.into(),
            decision: decision.into(),
            capability: capability.into(),
            target: target.into(),
            rule_id,
            reason: reason.into(),
        };
        buffer.events.push_back(event);
    }

    pub fn snapshot(&self) -> AuditSnapshot {
        let buffer = self.0.lock().unwrap();
        AuditSnapshot {
            events: buffer.events.iter().cloned().collect(),
            dropped_events: buffer.dropped_events,
        }
    }

    pub fn summary(&self) -> AuditSummary {
        let snapshot = self.snapshot();
        let mut decisions = BTreeMap::new();
        let mut capabilities = BTreeMap::new();
        for event in &snapshot.events {
            *decisions.entry(event.decision.clone()).or_default() += 1;
            *capabilities.entry(event.capability.clone()).or_default() += 1;
        }
        AuditSummary {
            event_count: snapshot.events.len(),
            dropped_events: snapshot.dropped_events,
            decisions,
            capabilities,
        }
    }

    pub fn set_phase(&self, phase: &str) {
        self.0.lock().unwrap().phase = phase.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_counts_evictions() {
        let sink = AuditSink::with_capacity("sb-test".into(), None, 2);
        for target in ["one", "two", "three"] {
            sink.record("test", "allow", "network", target, None, "test");
        }
        let snapshot = sink.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].target, "two");
        assert_eq!(snapshot.dropped_events, 1);
    }
}

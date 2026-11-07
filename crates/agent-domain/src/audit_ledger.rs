use crate::guardrail::{ActionSeverity, GuardrailPolicy};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditIntegrityError {
    #[error("Invalid sequence number at index {index}: expected {expected}, found {found}")]
    InvalidSequence {
        index: usize,
        expected: u64,
        found: u64,
    },

    #[error("Previous hash mismatch at sequence {sequence}: expected {expected}, found {found}")]
    InvalidPrevHash {
        sequence: u64,
        expected: String,
        found: String,
    },

    #[error("Hash mismatch (tampering detected) at sequence {sequence}: computed {computed}, found {found}")]
    TamperedHash {
        sequence: u64,
        computed: String,
        found: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuditEventType {
    CommandAllowed,
    CommandBlocked,
    CommandException,
    ApprovalRequired,
    SecurityAlert,
    ShellExecution,
    PolicyViolation,
    Custom(String),
}

impl AuditEventType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::CommandAllowed => "COMMAND_ALLOWED",
            Self::CommandBlocked => "COMMAND_BLOCKED",
            Self::CommandException => "COMMAND_EXCEPTION",
            Self::ApprovalRequired => "APPROVAL_REQUIRED",
            Self::SecurityAlert => "SECURITY_ALERT",
            Self::ShellExecution => "SHELL_EXECUTION",
            Self::PolicyViolation => "POLICY_VIOLATION",
            Self::Custom(s) => s.as_str(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::CommandAllowed)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::CommandBlocked | Self::PolicyViolation)
    }

    pub fn is_exception(&self) -> bool {
        matches!(self, Self::CommandException)
    }
}

impl Default for AuditEventType {
    fn default() -> Self {
        Self::Custom("UNKNOWN".to_string())
    }
}

impl fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for AuditEventType {
    fn from(s: &str) -> Self {
        match s {
            "COMMAND_ALLOWED" => Self::CommandAllowed,
            "COMMAND_BLOCKED" => Self::CommandBlocked,
            "COMMAND_EXCEPTION" => Self::CommandException,
            "APPROVAL_REQUIRED" => Self::ApprovalRequired,
            "SECURITY_ALERT" => Self::SecurityAlert,
            "SHELL_EXECUTION" => Self::ShellExecution,
            "POLICY_VIOLATION" => Self::PolicyViolation,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl From<String> for AuditEventType {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

impl From<AuditEventType> for String {
    fn from(event_type: AuditEventType) -> Self {
        event_type.to_string()
    }
}

impl AsRef<str> for AuditEventType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Serialize for AuditEventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuditEventType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(AuditEventType::from(s))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEntry {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub event_type: AuditEventType,
    pub action: String,
    pub severity: ActionSeverity,
    pub details: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditEntry {
    pub fn compute_hash(
        sequence: u64,
        timestamp: &DateTime<Utc>,
        action: &str,
        severity: &ActionSeverity,
        details: &str,
        prev_hash: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        let payload = format!(
            "{}:{}:{}:{:?}:{}:{}",
            sequence,
            timestamp.to_rfc3339(),
            action,
            severity,
            details,
            prev_hash
        );
        hasher.update(payload.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn new(
        sequence: u64,
        timestamp: DateTime<Utc>,
        action: impl Into<String>,
        severity: ActionSeverity,
        details: impl Into<String>,
        prev_hash: impl Into<String>,
    ) -> Self {
        let action_str = action.into();
        let event_type = AuditEventType::from(action_str.as_str());
        Self::new_with_actor(
            sequence, timestamp, event_type, action_str, severity, details, None, prev_hash,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_actor(
        sequence: u64,
        timestamp: DateTime<Utc>,
        event_type: AuditEventType,
        action: impl Into<String>,
        severity: ActionSeverity,
        details: impl Into<String>,
        actor: Option<String>,
        prev_hash: impl Into<String>,
    ) -> Self {
        let action = action.into();
        let details = details.into();
        let prev_hash = prev_hash.into();
        let hash = Self::compute_hash(
            sequence, &timestamp, &action, &severity, &details, &prev_hash,
        );
        Self {
            sequence,
            timestamp,
            event_type,
            action,
            severity,
            details,
            actor,
            prev_hash,
            hash,
        }
    }

    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    pub fn with_event_type(mut self, event_type: AuditEventType) -> Self {
        self.event_type = event_type;
        self
    }

    pub fn calculate_hash(&self) -> String {
        Self::compute_hash(
            self.sequence,
            &self.timestamp,
            &self.action,
            &self.severity,
            &self.details,
            &self.prev_hash,
        )
    }

    pub fn verify_hash(&self) -> bool {
        self.hash == self.calculate_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AuditLedger {
    entries: Vec<AuditEntry>,
}

impl AuditLedger {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn from_entries(entries: Vec<AuditEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last_entry(&self) -> Option<&AuditEntry> {
        self.entries.last()
    }

    pub fn last_hash(&self) -> &str {
        self.last_entry()
            .map(|e| e.hash.as_str())
            .unwrap_or(GENESIS_HASH)
    }

    pub fn record(
        &mut self,
        action: impl Into<String>,
        severity: ActionSeverity,
        details: impl Into<String>,
    ) -> &AuditEntry {
        let action_str = action.into();
        let event_type = AuditEventType::from(action_str.as_str());
        self.record_entry(event_type, action_str, severity, details, None)
    }

    pub fn record_event(
        &mut self,
        event_type: AuditEventType,
        severity: ActionSeverity,
        details: impl Into<String>,
    ) -> &AuditEntry {
        let action_str = event_type.to_string();
        self.record_entry(event_type, action_str, severity, details, None)
    }

    pub fn record_with_actor(
        &mut self,
        action: impl Into<String>,
        severity: ActionSeverity,
        details: impl Into<String>,
        actor: impl Into<String>,
    ) -> &AuditEntry {
        let action_str = action.into();
        let event_type = AuditEventType::from(action_str.as_str());
        self.record_entry(
            event_type,
            action_str,
            severity,
            details,
            Some(actor.into()),
        )
    }

    pub fn record_exception(
        &mut self,
        action: impl Into<String>,
        details: impl Into<String>,
    ) -> &AuditEntry {
        self.record(action, ActionSeverity::HighImpact, details)
    }

    fn record_entry(
        &mut self,
        event_type: AuditEventType,
        action: String,
        severity: ActionSeverity,
        details: impl Into<String>,
        actor: Option<String>,
    ) -> &AuditEntry {
        let sequence = self.entries.len() as u64;
        let prev_hash = self.last_hash().to_string();
        let timestamp = Utc::now();
        let entry = AuditEntry::new_with_actor(
            sequence, timestamp, event_type, action, severity, details, actor, prev_hash,
        );
        self.entries.push(entry);
        self.entries.last().expect("entry just pushed")
    }

    pub fn append(&mut self, entry: AuditEntry) -> Result<(), AuditIntegrityError> {
        let expected_sequence = self.entries.len() as u64;
        if entry.sequence != expected_sequence {
            return Err(AuditIntegrityError::InvalidSequence {
                index: self.entries.len(),
                expected: expected_sequence,
                found: entry.sequence,
            });
        }

        let expected_prev_hash = self.last_hash();
        if entry.prev_hash != expected_prev_hash {
            return Err(AuditIntegrityError::InvalidPrevHash {
                sequence: entry.sequence,
                expected: expected_prev_hash.to_string(),
                found: entry.prev_hash,
            });
        }

        let computed = entry.calculate_hash();
        if entry.hash != computed {
            return Err(AuditIntegrityError::TamperedHash {
                sequence: entry.sequence,
                computed,
                found: entry.hash,
            });
        }

        self.entries.push(entry);
        Ok(())
    }

    pub fn record_guardrail_check(
        &mut self,
        policy: &GuardrailPolicy,
        command: &str,
    ) -> Result<(), String> {
        policy.evaluate_and_record(self, command)
    }

    pub fn verify_integrity(&self) -> Result<(), AuditIntegrityError> {
        let mut expected_prev = GENESIS_HASH;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.sequence != i as u64 {
                return Err(AuditIntegrityError::InvalidSequence {
                    index: i,
                    expected: i as u64,
                    found: entry.sequence,
                });
            }

            if entry.prev_hash != expected_prev {
                return Err(AuditIntegrityError::InvalidPrevHash {
                    sequence: entry.sequence,
                    expected: expected_prev.to_string(),
                    found: entry.prev_hash.clone(),
                });
            }

            let computed = entry.calculate_hash();
            if entry.hash != computed {
                return Err(AuditIntegrityError::TamperedHash {
                    sequence: entry.sequence,
                    computed,
                    found: entry.hash.clone(),
                });
            }

            expected_prev = &entry.hash;
        }
        Ok(())
    }

    pub fn is_valid(&self) -> bool {
        self.verify_integrity().is_ok()
    }
}

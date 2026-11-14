mod normalize;

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use normalize::{classify_shell_error, normalize_signature_text};

const MAX_TITLE_LEN: usize = 160;

const SHELL_TOOLS: &[&str] = &[
    "bash",
    "shell",
    "exec",
    "exec_command",
    "local_shell",
    "run_command",
    "terminal",
];

/// Privacy-safe clustered failure fingerprint (only hashed signature + redacted normalized local title).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSignature {
    pub kind: String,
    pub signature: String,
    pub title: String,
}

/// Compute a deterministic SHA-256 signature and canonical title for a failure event.
pub fn failure_signature(kind: &str, tool: &str, text: &str) -> FailureSignature {
    let lower_tool = tool.trim().to_ascii_lowercase();
    let (key_material, title) = if SHELL_TOOLS.contains(&lower_tool.as_str()) {
        let (code, class) = classify_shell_error(text);
        let title = match code {
            Some(c) if class != "failed" => format!("exit {c}: {class}"),
            Some(c) => format!("exit {c}"),
            None => class.to_string(),
        };
        let code_part = code.map(|c| c.to_string()).unwrap_or_default();
        (
            format!("{kind}\n{code_part}\n{class}"),
            title.chars().take(MAX_TITLE_LEN).collect(),
        )
    } else {
        let normalized = normalize_signature_text(text);
        let title = if normalized.is_empty() {
            "failed".to_string()
        } else {
            normalized.chars().take(MAX_TITLE_LEN).collect()
        };
        (format!("{kind}\n{tool}\n{normalized}"), title)
    };

    let digest = Sha256::digest(key_material.as_bytes());
    let mut short_hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        use std::fmt::Write;
        let _ = write!(short_hex, "{byte:02x}");
    }

    FailureSignature {
        kind: kind.to_string(),
        signature: format!("{kind}:{short_hex}"),
        title,
    }
}

/// Result of recording a repeated tool invocation inside an active execution scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatObservation {
    pub count: usize,
    pub threshold_hit: Option<usize>,
}

struct BoundedCounter {
    counts: HashMap<String, usize>,
    order: VecDeque<String>,
}

/// Per-scope bounded LRU counter flagging identical repeated tool calls in a single task turn.
pub struct RepeatDetector {
    scopes: HashMap<String, BoundedCounter>,
    thresholds: Vec<usize>,
    max_keys_per_scope: usize,
}

impl RepeatDetector {
    /// Construct a detector firing at sorted `thresholds` with per-scope key bound `max_keys_per_scope`.
    pub fn new(mut thresholds: Vec<usize>, max_keys_per_scope: usize) -> Self {
        thresholds.sort_unstable();
        thresholds.dedup();
        Self {
            scopes: HashMap::new(),
            thresholds,
            max_keys_per_scope: max_keys_per_scope.max(1),
        }
    }

    /// Normalize `(tool, args_or_input)` into a deduplication key, returning `None` for bare tool names.
    pub fn canonical_tool_key(tool: &str, args_or_input: &str) -> Option<String> {
        let collapsed: String = args_or_input.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty()
            || collapsed == tool
            || collapsed.eq_ignore_ascii_case("MCP: tool")
        {
            return None;
        }
        Some(format!("{}:{}", tool.trim(), collapsed))
    }

    /// Record one invocation key under `scope_id` and return current count + any matched threshold.
    pub fn record(&mut self, scope_id: &str, key: &str) -> RepeatObservation {
        let entry = self
            .scopes
            .entry(scope_id.to_string())
            .or_insert_with(|| BoundedCounter {
                counts: HashMap::new(),
                order: VecDeque::new(),
            });

        if let Some(pos) = entry.order.iter().position(|existing| existing == key) {
            entry.order.remove(pos);
        } else if entry.counts.len() >= self.max_keys_per_scope {
            if let Some(oldest) = entry.order.pop_front() {
                entry.counts.remove(&oldest);
            }
        }
        entry.order.push_back(key.to_string());

        let count = entry.counts.get(key).copied().unwrap_or(0) + 1;
        entry.counts.insert(key.to_string(), count);

        let threshold_hit = if self.thresholds.contains(&count) {
            Some(count)
        } else {
            None
        };

        RepeatObservation {
            count,
            threshold_hit,
        }
    }

    /// Drop tracked tool-call state when a task or thread settles.
    pub fn settle(&mut self, scope_id: &str) {
        self.scopes.remove(scope_id);
    }
}

/// Decision returned by `ExecutionLoopGuard` after observing a turn, tool call, or failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopVerdict {
    Allow { count: usize },
    Warn { count: usize, reason: String },
    Break { count: usize, reason: String },
}

/// Configurable continuation/loop caps inspired by OpenMausBot's runtime contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLoopConfig {
    pub max_total_turns: usize,
    pub repeat_warn_at: usize,
    pub repeat_break_at: usize,
    pub consecutive_error_break_at: usize,
}

impl Default for ExecutionLoopConfig {
    fn default() -> Self {
        Self {
            max_total_turns: 40,
            repeat_warn_at: 3,
            repeat_break_at: 5,
            consecutive_error_break_at: 3,
        }
    }
}

#[derive(Default)]
struct FailureStreak {
    last_signature: String,
    count: usize,
}

/// Stateful execution loop guard combining turn budget, repeated call detection, and identical error streaks.
pub struct ExecutionLoopGuard {
    config: ExecutionLoopConfig,
    turns: usize,
    repeats: RepeatDetector,
    failure_streaks: HashMap<String, FailureStreak>,
}

impl ExecutionLoopGuard {
    pub fn new(config: ExecutionLoopConfig) -> Self {
        let mut thresholds = Vec::new();
        if config.repeat_warn_at > 0 {
            thresholds.push(config.repeat_warn_at);
        }
        if config.repeat_break_at > 0 {
            thresholds.push(config.repeat_break_at);
        }
        Self {
            config,
            turns: 0,
            repeats: RepeatDetector::new(thresholds, 256),
            failure_streaks: HashMap::new(),
        }
    }

    /// Advance turn counter and break if `max_total_turns` is exceeded.
    pub fn step_turn(&mut self) -> LoopVerdict {
        self.turns += 1;
        if self.config.max_total_turns > 0 && self.turns > self.config.max_total_turns {
            return LoopVerdict::Break {
                count: self.turns,
                reason: format!(
                    "turn budget {} exceeded ({})",
                    self.config.max_total_turns, self.turns
                ),
            };
        }
        LoopVerdict::Allow { count: self.turns }
    }

    /// Observe a tool call inside `scope_id`; flags repeated identical calls at warn/break thresholds.
    pub fn observe_tool_call(&mut self, scope_id: &str, tool: &str, args: &str) -> LoopVerdict {
        let Some(key) = RepeatDetector::canonical_tool_key(tool, args) else {
            return LoopVerdict::Allow { count: 0 };
        };
        let obs = self.repeats.record(scope_id, &key);
        if self.config.repeat_break_at > 0 && obs.count >= self.config.repeat_break_at {
            return LoopVerdict::Break {
                count: obs.count,
                reason: format!("repeated identical tool call {} {} times", tool, obs.count),
            };
        }
        if obs.threshold_hit == Some(self.config.repeat_warn_at) {
            return LoopVerdict::Warn {
                count: obs.count,
                reason: format!("repeating identical tool call {} ({}x)", tool, obs.count),
            };
        }
        LoopVerdict::Allow { count: obs.count }
    }

    /// Observe a tool/command failure and break on repeated identical failure signatures.
    pub fn observe_failure(
        &mut self,
        scope_id: &str,
        kind: &str,
        tool: &str,
        error_text: &str,
    ) -> LoopVerdict {
        let sig = failure_signature(kind, tool, error_text);
        let streak = self.failure_streaks.entry(scope_id.to_string()).or_default();
        if streak.last_signature == sig.signature {
            streak.count += 1;
        } else {
            streak.last_signature = sig.signature.clone();
            streak.count = 1;
        }

        if self.config.consecutive_error_break_at > 0
            && streak.count >= self.config.consecutive_error_break_at
        {
            return LoopVerdict::Break {
                count: streak.count,
                reason: format!(
                    "identical failure {} ({}) repeated {} times",
                    sig.signature, sig.title, streak.count
                ),
            };
        }
        if streak.count >= 2 {
            return LoopVerdict::Warn {
                count: streak.count,
                reason: format!(
                    "failure {} ({}) occurred {} times consecutively",
                    sig.signature, sig.title, streak.count
                ),
            };
        }
        LoopVerdict::Allow {
            count: streak.count,
        }
    }

    /// Clear consecutive failure streak and tracked calls once a scope succeeds or settles.
    pub fn settle_scope(&mut self, scope_id: &str) {
        self.repeats.settle(scope_id);
        self.failure_streaks.remove(scope_id);
    }
}

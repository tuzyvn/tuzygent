use agent_domain::{redact_json_value, redact_secrets_in_text};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const SKELETON_STR_PREVIEW_BYTES: usize = 160;
const SKELETON_MAX_KEYS_PREVIEW: usize = 12;
const SKELETON_TEXT_HEAD_LINES: usize = 15;
const SKELETON_TEXT_TAIL_LINES: usize = 10;

/// Progressive-disclosure projection options for MCP tool output.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionOpts {
    /// Maximum JSON nesting depth to retain; deeper nodes collapse into a skeleton summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_depth: Option<usize>,
    /// Optional JSON Pointer (e.g. `"/ui_tree"`) to drill down into a subtree before projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_pointer: Option<String>,
    /// When true, filters array/object entries to interactive/actionable items.
    #[serde(default)]
    pub interactive_only: bool,
}

impl ProjectionOpts {
    /// Extracts optional `_projection` options from tool call arguments.
    pub fn from_arguments(args: &Value) -> Result<Option<Self>, String> {
        let Some(raw) = args.get("_projection") else {
            return Ok(None);
        };
        if raw.is_null() {
            return Ok(None);
        }
        let parsed: Self = serde_json::from_value(raw.clone())
            .map_err(|e| format!("Invalid '_projection' options: {e}"))?;
        Ok(Some(parsed))
    }
}

/// Redacts secrets and optionally projects a JSON payload according to `ProjectionOpts`.
pub fn redact_and_project_text(text: &str, opts: Option<&ProjectionOpts>) -> Result<String, String> {
    if let Ok(json_val) = serde_json::from_str::<Value>(text) {
        let redacted = redact_json_value(&json_val);
        let projected = match opts {
            Some(o) => project_json_value(&redacted, o)?,
            None => redacted,
        };
        return serde_json::to_string(&projected)
            .map_err(|e| format!("Failed to serialize projected JSON: {e}"));
    }

    let redacted = redact_secrets_in_text(text);
    if let Some(o) = opts {
        if o.skeleton_depth.is_some() {
            return Ok(summarize_text_lines(&redacted));
        }
    }
    Ok(redacted)
}

/// Applies `root_pointer`, `interactive_only`, and `skeleton_depth` to a JSON value.
pub fn project_json_value(value: &Value, opts: &ProjectionOpts) -> Result<Value, String> {
    let rooted = match opts.root_pointer.as_deref() {
        Some(ptr) if !ptr.is_empty() => value
            .pointer(ptr)
            .cloned()
            .ok_or_else(|| format!("ROOT_NOT_FOUND: JSON pointer '{}' not found", ptr))?,
        _ => value.clone(),
    };

    let filtered = if opts.interactive_only {
        filter_interactive(&rooted).unwrap_or(Value::Array(Vec::new()))
    } else {
        rooted
    };

    match opts.skeleton_depth {
        Some(max_depth) => Ok(collapse_skeleton(&filtered, 0, max_depth)),
        None => Ok(filtered),
    }
}

fn is_interactive_object(map: &Map<String, Value>) -> bool {
    if map.get("interactive").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    for key in ["actions", "available_actions"] {
        if let Some(Value::Array(arr)) = map.get(key) {
            if !arr.is_empty() {
                return true;
            }
        }
    }
    false
}

fn filter_interactive(value: &Value) -> Option<Value> {
    match value {
        Value::Array(items) => {
            let kept: Vec<Value> = items.iter().filter_map(filter_interactive).collect();
            if kept.is_empty() {
                None
            } else {
                Some(Value::Array(kept))
            }
        }
        Value::Object(map) => {
            if is_interactive_object(map) {
                return Some(Value::Object(map.clone()));
            }
            let mut out = Map::new();
            for (k, v) in map {
                if let Some(filtered_child) = filter_interactive(v) {
                    out.insert(k.clone(), filtered_child);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Object(out))
            }
        }
        _ => None,
    }
}

fn collapse_skeleton(value: &Value, depth: usize, max_depth: usize) -> Value {
    match value {
        Value::Object(map) => {
            if depth >= max_depth {
                let keys: Vec<Value> = map
                    .keys()
                    .take(SKELETON_MAX_KEYS_PREVIEW)
                    .map(|k| Value::String(k.clone()))
                    .collect();
                return serde_json::json!({
                    "__skeleton__": "object",
                    "keys": keys,
                    "total_keys": map.len(),
                });
            }
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), collapse_skeleton(v, depth + 1, max_depth));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            if depth >= max_depth {
                return serde_json::json!({
                    "__skeleton__": "array",
                    "len": items.len(),
                });
            }
            Value::Array(
                items
                    .iter()
                    .map(|item| collapse_skeleton(item, depth + 1, max_depth))
                    .collect(),
            )
        }
        Value::String(s) if depth >= max_depth && s.len() > SKELETON_STR_PREVIEW_BYTES => {
            let mut end = SKELETON_STR_PREVIEW_BYTES;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!("{}... [{} chars]", &s[..end], s.chars().count()))
        }
        other => other.clone(),
    }
}

fn summarize_text_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= SKELETON_TEXT_HEAD_LINES + SKELETON_TEXT_TAIL_LINES {
        return text.to_string();
    }
    let omitted = lines.len() - SKELETON_TEXT_HEAD_LINES - SKELETON_TEXT_TAIL_LINES;
    let mut out = Vec::with_capacity(SKELETON_TEXT_HEAD_LINES + SKELETON_TEXT_TAIL_LINES + 1);
    out.extend_from_slice(&lines[..SKELETON_TEXT_HEAD_LINES]);
    let marker = format!("[... {omitted} lines omitted by skeleton projection ...]");
    let tail_start = lines.len() - SKELETON_TEXT_TAIL_LINES;
    let mut result = out.join("\n");
    result.push('\n');
    result.push_str(&marker);
    result.push('\n');
    result.push_str(&lines[tail_start..].join("\n"));
    result
}

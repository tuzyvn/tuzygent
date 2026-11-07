use agent_domain::SubTask;
use serde_json::Value;

/// Trích xuất và parse Remediation SubTask từ văn bản JSON phản hồi
pub fn parse_remediation_json(
    raw: &str,
    failed_subtask: &SubTask,
    error_context: &str,
    retry_num: u32,
) -> Option<SubTask> {
    let json_text = if let Some(start) = raw.find("```json") {
        let after = raw.get(start + 7..).unwrap_or("");
        after
            .find("```")
            .and_then(|end| after.get(..end))
            .unwrap_or(after)
            .trim()
    } else if let Some(start) = raw.find("```") {
        let after = raw.get(start + 3..).unwrap_or("");
        after
            .find("```")
            .and_then(|end| after.get(..end))
            .unwrap_or(after)
            .trim()
    } else if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
        if end >= start {
            raw.get(start..=end).unwrap_or(raw)
        } else {
            raw
        }
    } else {
        raw
    };

    if let Ok(val) = serde_json::from_str::<Value>(json_text) {
        let default_id = format!("{}-FIX-{}", failed_subtask.id, retry_num);
        let id = val
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&default_id);
        let title = val
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Remediate error");
        let cmd = val
            .get("command_or_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Apply remediation");
        let ver = val
            .get("expected_verification")
            .and_then(|v| v.as_str())
            .unwrap_or("Error resolved");

        let mut rem =
            SubTask::new_remediation(id, title, cmd, &failed_subtask.id, error_context, retry_num);
        rem.expected_verification = ver.to_string();
        return Some(rem);
    }

    None
}

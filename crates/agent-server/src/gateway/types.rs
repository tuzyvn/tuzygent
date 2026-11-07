use agent_engine::AccountEngineKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AddAccountReq {
    pub engine: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extra_env: HashMap<String, String>,
}

pub fn parse_model_spec(model_str: &str) -> Result<(AccountEngineKind, Option<String>), String> {
    let lower = model_str.to_lowercase();
    if lower.starts_with("antigravity") || lower.starts_with("agy") || lower.starts_with("gemini") {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Antigravity, specific_model))
    } else if lower.starts_with("codex")
        || lower.starts_with("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
    {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Codex, specific_model))
    } else if lower.starts_with("claude") {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Claude, specific_model))
    } else {
        Err(format!(
            "Model '{}' is not recognized or permitted. Supported engines: antigravity, codex, claude.",
            model_str
        ))
    }
}

pub fn format_prompt_from_messages(messages: &[ChatMessage]) -> (String, String) {
    let mut system_context = String::new();
    let mut prompt = String::new();

    for msg in messages {
        match msg.role.to_lowercase().as_str() {
            "system" => {
                if !system_context.is_empty() {
                    system_context.push('\n');
                }
                system_context.push_str(&msg.content);
            }
            "user" => {
                if !prompt.is_empty() {
                    prompt.push_str("\n\n[USER]:\n");
                }
                prompt.push_str(&msg.content);
            }
            "assistant" => {
                prompt.push_str("\n\n[ASSISTANT]:\n");
                prompt.push_str(&msg.content);
            }
            _ => {
                prompt.push_str("\n\n");
                prompt.push_str(&msg.content);
            }
        }
    }

    (system_context, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_model_spec_antigravity() -> Result<(), Box<dyn std::error::Error>> {
        let (kind, override_model) = parse_model_spec("antigravity")?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model, None);

        let (kind, override_model) = parse_model_spec("gemini/gemini-1.5-pro")?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model.as_deref(), Some("gemini-1.5-pro"));

        let (kind, override_model) = parse_model_spec("agy/flash")?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model.as_deref(), Some("flash"));

        Ok(())
    }

    #[test]
    fn test_parse_model_spec_codex_and_claude() -> Result<(), Box<dyn std::error::Error>> {
        let (kind, override_model) = parse_model_spec("codex/gpt-4o")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model.as_deref(), Some("gpt-4o"));

        let (kind, override_model) = parse_model_spec("gpt-4")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model, None);

        let (kind, override_model) = parse_model_spec("claude/claude-3-5-sonnet")?;
        assert_eq!(kind, AccountEngineKind::Claude);
        assert_eq!(override_model.as_deref(), Some("claude-3-5-sonnet"));

        Ok(())
    }

    #[test]
    fn test_parse_model_spec_unsupported() {
        let err = parse_model_spec("unsupported-model")
            .expect_err("Expected unsupported model to return an Err");
        assert!(err.contains("not recognized or permitted"));
    }

    #[test]
    fn test_parse_model_spec_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
        // Empty string
        assert!(parse_model_spec("").is_err());

        // Leading whitespace or unsupported model names
        assert!(parse_model_spec(" gpt-4").is_err());
        assert!(parse_model_spec("deepseek-r1").is_err());
        assert!(parse_model_spec("llama-3").is_err());

        // Case insensitivity
        let (kind, override_model) = parse_model_spec("ANTIGRAVITY/gemini-2.5")?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model.as_deref(), Some("gemini-2.5"));

        let (kind, override_model) = parse_model_spec("CLAUDE")?;
        assert_eq!(kind, AccountEngineKind::Claude);
        assert_eq!(override_model, None);

        // O1 and O3 prefixes
        let (kind, override_model) = parse_model_spec("O1-preview")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model, None);

        let (kind, override_model) = parse_model_spec("o3-mini")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model, None);

        let (kind, override_model) = parse_model_spec("o1/reasoning-high")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model.as_deref(), Some("reasoning-high"));

        // Trailing slash and multiple slashes
        let (kind, override_model) = parse_model_spec("agy/")?;
        assert_eq!(kind, AccountEngineKind::Antigravity);
        assert_eq!(override_model.as_deref(), Some(""));

        let (kind, override_model) = parse_model_spec("codex/nested/model/path")?;
        assert_eq!(kind, AccountEngineKind::Codex);
        assert_eq!(override_model.as_deref(), Some("nested/model/path"));

        Ok(())
    }

    #[test]
    fn test_format_prompt_from_messages() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "System prompt line 1".to_string(),
            },
            ChatMessage {
                role: "system".to_string(),
                content: "System prompt line 2".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "How are you?".to_string(),
            },
        ];

        let (sys, prompt) = format_prompt_from_messages(&messages);
        assert_eq!(sys, "System prompt line 1\nSystem prompt line 2");
        assert!(prompt.starts_with("Hello!"));
        assert!(prompt.contains("[ASSISTANT]:\nHi there!"));
        assert!(prompt.contains("[USER]:\nHow are you?"));
    }
}

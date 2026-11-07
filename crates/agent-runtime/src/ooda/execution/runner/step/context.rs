use crate::ooda::execution::SubtaskExecutor;
use tracing::info;

/// Standard characters per token estimation heuristic.
pub const CHARS_PER_TOKEN: usize = 4;

/// Default token budget for working context in OODA execution (~8,000 chars).
pub const DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET: usize = 2000;

/// Estimates the token count of a given string using a standard heuristic.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(CHARS_PER_TOKEN)
    }
}

#[derive(Debug, Clone)]
struct ContextStep {
    subtask_id: Option<String>,
    summary: String,
    full_text: String,
}

fn make_summary(id_opt: &Option<String>, full: &str) -> String {
    let first_line = full.lines().next().unwrap_or("").trim();
    let max_summary_chars = 60;
    let trimmed_line = if first_line.chars().count() > max_summary_chars {
        let truncated: String = first_line.chars().take(max_summary_chars).collect();
        format!("{}...", truncated)
    } else {
        first_line.to_string()
    };

    if let Some(id) = id_opt {
        if trimmed_line.starts_with(id) {
            trimmed_line
        } else {
            format!("{}: {}", id, trimmed_line)
        }
    } else {
        trimmed_line
    }
}

fn parse_steps(ctx: &str) -> Vec<ContextStep> {
    let mut steps = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in ctx.lines() {
        let trimmed = line.trim();
        // Check if line indicates a new subtask or step header
        if let Some(pos) = trimmed.find(':') {
            let prefix = trimmed[..pos].trim();
            let is_id = !prefix.is_empty()
                && !prefix.contains(' ')
                && prefix.len() <= 32
                && (prefix.starts_with("ST-")
                    || prefix.starts_with("step")
                    || prefix.starts_with("Step")
                    || prefix.starts_with("task")
                    || prefix.starts_with("Task")
                    || prefix.starts_with("subtask")
                    || prefix
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-'));

            if is_id {
                // If we had a previous step collected, push it
                if !current_lines.is_empty() || current_id.is_some() {
                    let full = current_lines.join("\n").trim().to_string();
                    if !full.is_empty() {
                        let summary = make_summary(&current_id, &full);
                        steps.push(ContextStep {
                            subtask_id: current_id.take(),
                            summary,
                            full_text: full,
                        });
                    }
                    current_lines.clear();
                }
                current_id = Some(prefix.to_string());
                current_lines.push(line.to_string());
                continue;
            }
        }

        if !current_lines.is_empty() || !trimmed.is_empty() {
            current_lines.push(line.to_string());
        }
    }

    if !current_lines.is_empty() || current_id.is_some() {
        let full = current_lines.join("\n").trim().to_string();
        if !full.is_empty() {
            let summary = make_summary(&current_id, &full);
            steps.push(ContextStep {
                subtask_id: current_id,
                summary,
                full_text: full,
            });
        }
    }

    steps
}

fn window_single_text(text: &str, max_chars: usize) -> String {
    let notice = "[... Earlier context windowed ...]\n";
    let notice_chars = notice.chars().count();
    if max_chars <= notice_chars {
        let total = text.chars().count();
        if total <= max_chars {
            text.to_string()
        } else {
            text.chars().skip(total - max_chars).collect()
        }
    } else {
        let remaining_chars = max_chars - notice_chars;
        let total = text.chars().count();
        let tail: String = if total <= remaining_chars {
            text.to_string()
        } else {
            text.chars().skip(total - remaining_chars).collect()
        };
        format!("{}{}", notice, tail)
    }
}

/// Compacts working context to fit within a specified token budget.
///
/// If the context exceeds the token budget, older subtasks and steps are summarized
/// via rolling compaction, while preserving the full output of the most recent key steps.
pub fn compact_working_context_with_budget(ctx: &str, max_tokens: usize) -> String {
    if max_tokens == 0 || ctx.is_empty() {
        return String::new();
    }

    if estimate_tokens(ctx) <= max_tokens {
        return ctx.to_string();
    }

    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    let steps = parse_steps(ctx);

    if steps.is_empty() {
        return window_single_text(ctx, max_chars);
    }

    if steps.len() == 1 {
        let mut single_res = window_single_text(&steps[0].full_text, max_chars);
        if estimate_tokens(&single_res) > max_tokens {
            let total = single_res.chars().count();
            if total > max_chars {
                single_res = single_res.chars().skip(total - max_chars).collect();
            }
        }
        return single_res;
    }

    let num_steps = steps.len();
    let mut candidate = String::new();
    let mut found = false;

    // Try keeping steps from index k to num_steps-1 in full, summarizing 0 to k-1
    for k in 1..num_steps {
        let older_steps = &steps[..k];
        let recent_steps = &steps[k..];

        let recent_full = recent_steps
            .iter()
            .map(|s| s.full_text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Level 1: Try detailed one-line summaries for older steps if budget allows
        let summaries: Vec<String> = older_steps
            .iter()
            .map(|s| format!("- {}", s.summary))
            .collect();
        let header_with_summaries = format!(
            "[Summarized earlier subtasks ({} steps):\n{}\n]\n\n",
            older_steps.len(),
            summaries.join("\n")
        );
        let test_str_summaries = format!("{}{}", header_with_summaries, recent_full);
        if test_str_summaries.chars().count() <= max_chars {
            candidate = test_str_summaries;
            found = true;
            break;
        }

        // Level 2: Try single-line rolled-up header with IDs
        let ids: Vec<String> = older_steps
            .iter()
            .map(|s| s.subtask_id.clone().unwrap_or_else(|| "step".to_string()))
            .collect();

        let header_with_ids = format!(
            "[Earlier subtasks compacted ({} steps): {}]\n\n",
            older_steps.len(),
            ids.join(", ")
        );

        let test_str_with_ids = format!("{}{}", header_with_ids, recent_full);
        if test_str_with_ids.chars().count() <= max_chars {
            candidate = test_str_with_ids;
            found = true;
            break;
        }

        // Level 3: Try minimal header if IDs make it too long
        let minimal_header = format!(
            "[Earlier subtasks compacted ({} steps)]\n\n",
            older_steps.len()
        );
        let test_str_minimal = format!("{}{}", minimal_header, recent_full);
        if test_str_minimal.chars().count() <= max_chars {
            candidate = test_str_minimal;
            found = true;
            break;
        }
    }

    if !found {
        // Even keeping only the last step in full exceeds budget.
        // Summarize all older steps, and window the latest step from the start to preserve its identifier and key action.
        let older_count = num_steps - 1;
        let header = format!("[Earlier subtasks compacted ({} steps)]\n\n", older_count);
        let header_chars = header.chars().count();
        if max_chars > header_chars {
            let avail = max_chars - header_chars;
            let last_full = &steps[num_steps - 1].full_text;
            let last_chars = last_full.chars().count();
            let step_part: String = if last_chars <= avail {
                last_full.clone()
            } else {
                last_full.chars().take(avail).collect()
            };
            candidate = format!("{}{}", header, step_part);
        } else {
            candidate = header.chars().take(max_chars).collect();
        }
    }

    // Defensive guarantee: ensure candidate strictly respects token budget
    if estimate_tokens(&candidate) > max_tokens {
        let total = candidate.chars().count();
        if total > max_chars {
            candidate = candidate.chars().take(max_chars).collect();
        }
    }

    candidate
}

/// Incorporates in-flight supplements into system context if available,
/// using the default token budget for working context compaction.
pub async fn resolve_active_system_context(
    executor: &SubtaskExecutor,
    long_term_ctx: &str,
    working_context: &str,
) -> String {
    resolve_active_system_context_with_budget(
        executor,
        long_term_ctx,
        working_context,
        DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET,
    )
    .await
}

/// Incorporates in-flight supplements into system context if available,
/// compacting the working context to fit within the specified token budget.
pub async fn resolve_active_system_context_with_budget(
    executor: &SubtaskExecutor,
    long_term_ctx: &str,
    working_context: &str,
    max_tokens: usize,
) -> String {
    let compacted_working_context =
        compact_working_context_with_budget(working_context, max_tokens);
    let mut active_system_context = format!(
        "{}\n# Previous step results (reference data)\n{}",
        long_term_ctx, compacted_working_context
    );
    if let Some(sup) = executor.supplement_ctrl.try_recv().await {
        info!(
            "💡 [OODA] Incorporating In-Flight Supplement from: {}",
            sup.author_name
        );
        active_system_context.push_str("\n\n");
        active_system_context.push_str(&sup.to_instruction_block());
    }
    active_system_context
}

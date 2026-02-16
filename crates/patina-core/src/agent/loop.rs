use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;
use rig::completion::{CompletionModel, CompletionRequest, Message, ToolDefinition};
use rig::message::{
    AssistantContent, Reasoning, Text, ToolCall, ToolResult, ToolResultContent, UserContent,
};
use rig::OneOrMany;
use tracing::{debug, info, warn};

use crate::agent::context::ContextBuilder;
use crate::agent::memory_index::MemoryIndex;
use crate::agent::model_pool::ModelPool;
use crate::session::SessionManager;
use crate::tools::ToolRegistry;

/// Find the largest byte index <= `max` that is a UTF-8 char boundary.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Data needed to run a memory consolidation LLM call.
/// Captured as a snapshot so the call can run without borrowing AgentLoop.
pub struct ConsolidationTask {
    pub session_key: String,
    pub end: usize,
    pub conversation: String,
    pub current_memory: String,
    pub memory_path: PathBuf,
    pub history_path: PathBuf,
}

/// Result of a successful consolidation, used to update session state.
pub struct ConsolidationResult {
    pub session_key: String,
    pub end: usize,
}

/// Per-model parameter overrides keyed by substring pattern.
/// E.g. ("kimi-k2.5", {temperature: 1.0}) forces temperature for Kimi K2.5.
#[derive(Debug, Clone, Default)]
pub struct ModelOverrides {
    pub entries: Vec<(String, OverrideParams)>,
}

#[derive(Debug, Clone, Default)]
pub struct OverrideParams {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
}

impl ModelOverrides {
    /// Return the built-in model overrides (matching Python's registry).
    pub fn defaults() -> Self {
        Self {
            entries: vec![(
                "kimi-k2.5".to_string(),
                OverrideParams {
                    temperature: Some(1.0),
                    max_tokens: None,
                },
            )],
        }
    }

    /// Find overrides matching a model name (case-insensitive substring match).
    pub fn find(&self, model_name: &str) -> Option<&OverrideParams> {
        let lower = model_name.to_lowercase();
        self.entries
            .iter()
            .find(|(pattern, _)| lower.contains(pattern))
            .map(|(_, params)| params)
    }
}

/// Core agent processing loop.
///
/// Uses rig's CompletionModel directly for LLM calls but runs its own
/// tool dispatch loop (like the Python version) for maximum control.
#[allow(deprecated)]
pub struct AgentLoop {
    pub models: ModelPool,
    pub sessions: SessionManager,
    pub context: ContextBuilder,
    pub tools: ToolRegistry,
    pub max_iterations: usize,
    pub temperature: f64,
    pub max_tokens: u64,
    pub memory_window: usize,
    pub model_overrides: ModelOverrides,
    pub memory_index: Option<Arc<MemoryIndex>>,
}

#[allow(deprecated)]
impl AgentLoop {
    fn interrupt_flag_path(session_key: &str) -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let safe = session_key
            .chars()
            .map(|c| match c {
                '/' | '\\' | ':' | ' ' => '_',
                _ => c,
            })
            .collect::<String>();
        home.join(".patina")
            .join("interrupts")
            .join(format!("{safe}.flag"))
    }

    fn consume_interrupt(session_key: &str) -> bool {
        let flag = Self::interrupt_flag_path(session_key);
        if flag.exists() {
            if let Err(e) = std::fs::remove_file(&flag) {
                warn!("Failed to clear interrupt flag '{}': {e}", flag.display());
            }
            true
        } else {
            false
        }
    }

    /// Process a single user message and return the assistant's response
    /// plus a flag indicating whether memory consolidation is needed.
    pub async fn process_message(
        &mut self,
        session_key: &str,
        user_message: &str,
        media: Option<&[String]>,
    ) -> Result<(String, bool)> {
        if Self::consume_interrupt(session_key) {
            return Ok(("Interrupted before processing.".to_string(), false));
        }

        let session = self.sessions.get_or_create_checked(session_key)?;
        let history = session.get_history(self.memory_window);

        // Build messages for context
        let messages_json =
            self.context
                .build_messages(&history, user_message, None, None, media)?;

        // Log context summary
        {
            let system_chars = messages_json
                .first()
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            let msg_summary: Vec<String> = messages_json
                .iter()
                .skip(1)
                .map(|m| {
                    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                    let len = m
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(|s| s.len())
                        .unwrap_or(0);
                    format!("{role}:{len}")
                })
                .collect();
            debug!(
                "Context: system={system_chars} chars, history=[{}]",
                msg_summary.join(", ")
            );
        }

        // Convert to rig Message format
        let system_prompt = messages_json
            .first()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let mut chat_history: Vec<Message> = Vec::new();
        for msg_json in messages_json.iter().skip(1) {
            let role = msg_json.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = msg_json
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            match role {
                "user" => {
                    chat_history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text {
                            text: content.to_string(),
                        })),
                    });
                }
                "assistant" => {
                    let mut parts: Vec<AssistantContent> = Vec::new();
                    // Include reasoning_content if present (for thinking models)
                    if let Some(reasoning) =
                        msg_json.get("reasoning_content").and_then(|r| r.as_str())
                    {
                        if !reasoning.is_empty() {
                            parts.push(AssistantContent::Reasoning(Reasoning::new(reasoning)));
                        }
                    }
                    parts.push(AssistantContent::Text(Text {
                        text: content.to_string(),
                    }));
                    chat_history.push(Message::Assistant {
                        id: None,
                        content: OneOrMany::many(parts).unwrap_or_else(|_| {
                            OneOrMany::one(AssistantContent::Text(Text {
                                text: content.to_string(),
                            }))
                        }),
                    });
                }
                _ => {}
            }
        }

        // The last message in chat_history is the current user prompt
        let prompt = chat_history.pop().unwrap_or_else(|| Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: user_message.to_string(),
            })),
        });

        // Build tool definitions
        let tool_defs: Vec<ToolDefinition> = self
            .tools
            .list()
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect();

        // Run the agent loop with tool calling
        let (response, tools_used, reasoning) = self
            .run_loop(
                session_key,
                &system_prompt,
                chat_history,
                prompt,
                &tool_defs,
                "default",
            )
            .await?;

        // Save to session
        let session = self.sessions.get_or_create_checked(session_key)?;
        session.add_message("user", user_message);
        session.add_message_full("assistant", &response, tools_used, reasoning);
        self.sessions.save(session_key)?;

        // Reindex memory files so new content is searchable immediately.
        // Hash-based, so unchanged files are skipped cheaply.
        if let Some(ref index) = self.memory_index {
            if let Err(e) = index.reindex() {
                warn!("Memory reindex after message failed: {e}");
            }
        }

        // Check if memory consolidation is needed (caller should run it
        // *after* delivering the response so the user isn't blocked).
        let needs_consolidation = {
            let session = match self.sessions.get_or_create_checked(session_key) {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to reload session '{session_key}' for consolidation check: {e}");
                    return Ok((response, false));
                }
            };
            session.messages.len() > self.memory_window
        };

        Ok((response, needs_consolidation))
    }

    /// Snapshot session data for consolidation without borrowing mutably.
    /// Returns `None` if there's nothing to consolidate.
    pub fn prepare_consolidation(
        &self,
        session_key: &str,
        archive_all: bool,
    ) -> Option<ConsolidationTask> {
        let session = self.sessions.sessions.get(session_key)?;

        let keep_count = if archive_all {
            0
        } else {
            self.memory_window / 2
        };

        let total = session.messages.len();
        if total <= keep_count {
            return None;
        }

        let end = total.saturating_sub(keep_count);
        if end <= session.last_consolidated {
            return None;
        }

        let messages_to_process = &session.messages[session.last_consolidated..end];
        if messages_to_process.is_empty() {
            return None;
        }

        let mut conversation = String::new();
        for msg in messages_to_process {
            let ts = msg.timestamp.as_deref().unwrap_or("unknown");
            let role = msg.role.to_uppercase();
            let tools_info = match &msg.tools_used {
                Some(tools) if !tools.is_empty() => {
                    format!(" [tools: {}]", tools.join(", "))
                }
                _ => String::new(),
            };
            conversation.push_str(&format!("[{ts}] {role}{tools_info}: {}\n", msg.content));
        }

        let current_memory = self.context.memory().read_long_term().unwrap_or_default();
        let memory_store = self.context.memory();

        Some(ConsolidationTask {
            session_key: session_key.to_string(),
            end,
            conversation,
            current_memory,
            memory_path: memory_store.memory_path().to_path_buf(),
            history_path: memory_store.history_path().to_path_buf(),
        })
    }

    /// Run the consolidation LLM call and write memory files.
    /// This is a static method that doesn't need `self`.
    pub async fn run_consolidation(
        model: &CompletionModelHandle<'static>,
        task: &ConsolidationTask,
    ) -> Option<ConsolidationResult> {
        let prompt = format!(
            r#"You are a memory consolidation agent. Process this conversation and return a JSON object with exactly two keys:

1. "history_entry": A paragraph (2-5 sentences) summarizing the key events/decisions/topics. Start with a timestamp like [YYYY-MM-DD HH:MM]. Include enough detail to be useful when found by grep search later.

2. "memory_update": The updated long-term memory content. Add any new facts: user location, preferences, personal info, habits, project context, technical decisions, tools/services used. If nothing new, return the existing content unchanged.

## Current Long-term Memory
{}

## Conversation to Process
{}

Respond with ONLY valid JSON, no markdown fences."#,
            task.current_memory, task.conversation
        );

        let request = CompletionRequest {
            preamble: None,
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::Text(Text { text: prompt })),
            }),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: Some(0.3),
            max_tokens: Some(2048),
            tool_choice: None,
            additional_params: None,
        };

        let response = match model.completion(request).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Memory consolidation LLM call failed: {e}");
                return None;
            }
        };

        let response_text: String = response
            .choice
            .iter()
            .filter_map(|c| match c {
                AssistantContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();

        debug!("Memory consolidation LLM response: {}", response_text);

        let json_str = strip_markdown_fences(&response_text);

        debug!(
            "Memory consolidation JSON after fence stripping: {}",
            json_str
        );

        let parsed: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "Memory consolidation: failed to parse JSON response: {e}\n\
                     Raw response (first 500 chars): {}",
                    if response_text.len() > 500 {
                        &response_text[..floor_char_boundary(&response_text, 500)]
                    } else {
                        &response_text
                    }
                );
                return None;
            }
        };

        // Write memory files directly using paths from the task
        if let Some(entry) = parsed.get("history_entry").and_then(|e| e.as_str()) {
            if let Some(parent) = task.history_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&task.history_path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = writeln!(file, "\n{entry}") {
                        warn!("Failed to append history: {e}");
                    } else {
                        info!("Memory consolidation: appended history entry");
                    }
                }
                Err(e) => warn!("Failed to open history file: {e}"),
            }
        }

        if let Some(update) = parsed.get("memory_update").and_then(|u| u.as_str()) {
            if let Some(parent) = task.memory_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&task.memory_path, update) {
                Ok(()) => info!("Memory consolidation: updated long-term memory"),
                Err(e) => warn!("Failed to update memory: {e}"),
            }
        }

        Some(ConsolidationResult {
            session_key: task.session_key.clone(),
            end: task.end,
        })
    }

    /// Apply a completed consolidation result to update session state.
    pub fn apply_consolidation(&mut self, result: &ConsolidationResult) {
        if let Some(session) = self.sessions.sessions.get_mut(&result.session_key) {
            session.last_consolidated = result.end;
            if let Err(e) = self.sessions.save(&result.session_key) {
                warn!(
                    "Failed to persist session '{}' after consolidation: {e}",
                    result.session_key
                );
            }
        } else {
            warn!(
                "Session '{}' no longer exists after consolidation",
                result.session_key
            );
        }

        // Reindex memory after consolidation writes new content
        if let Some(ref index) = self.memory_index {
            if let Err(e) = index.reindex() {
                warn!("Memory reindex after consolidation failed: {e}");
            }
        }
    }

    /// Get the model handle for a given tier (cloned for use in spawned tasks).
    pub fn model_for_tier(&self, tier: &str) -> CompletionModelHandle<'static> {
        let (model, _) = self.models.get(tier);
        model.clone()
    }

    /// Consolidate old messages synchronously (convenience wrapper).
    /// Used by `/new` command and CLI interactive mode where blocking is acceptable.
    pub async fn consolidate_memory(&mut self, session_key: &str, archive_all: bool) {
        let task = match self.prepare_consolidation(session_key, archive_all) {
            Some(t) => t,
            None => return,
        };
        let (model, _) = self.models.get("consolidation");
        if let Some(result) = Self::run_consolidation(model, &task).await {
            self.apply_consolidation(&result);
        }
    }

    /// Run the LLM <> tool loop until the model produces a text response or max iterations.
    ///
    /// Returns (response_text, tools_used, reasoning_content).
    async fn run_loop(
        &self,
        session_key: &str,
        system_prompt: &str,
        mut chat_history: Vec<Message>,
        prompt: Message,
        tool_defs: &[ToolDefinition],
        tier: &str,
    ) -> Result<(String, Vec<String>, Option<String>)> {
        let (model, model_name) = self.models.get(tier);
        let model_name = model_name.to_string();
        let mut tools_used = Vec::new();
        let mut current_prompt = prompt;
        let mut accumulated_reasoning = String::new();
        let mut consecutive_errors: usize = 0;
        const MAX_CONSECUTIVE_ERRORS: usize = 3;

        for iteration in 0..self.max_iterations {
            if Self::consume_interrupt(session_key) {
                return Ok((
                    "Interrupted.".to_string(),
                    tools_used,
                    if accumulated_reasoning.is_empty() {
                        None
                    } else {
                        Some(accumulated_reasoning)
                    },
                ));
            }

            // Build the rig CompletionRequest
            let mut all_messages = chat_history.clone();
            all_messages.push(current_prompt.clone());

            // Apply model-specific overrides (e.g. kimi-k2.5 forces temperature=1.0)
            let (effective_temp, effective_max_tokens) =
                if let Some(overrides) = self.model_overrides.find(&model_name) {
                    (
                        overrides.temperature.unwrap_or(self.temperature),
                        overrides.max_tokens.unwrap_or(self.max_tokens),
                    )
                } else {
                    (self.temperature, self.max_tokens)
                };

            debug!(
                "LLM request [{}/{}]: {} messages, temp={effective_temp}, max_tokens={effective_max_tokens}",
                iteration + 1,
                self.max_iterations,
                all_messages.len()
            );

            let request = CompletionRequest {
                preamble: Some(system_prompt.to_string()),
                chat_history: OneOrMany::many(all_messages.clone())
                    .unwrap_or_else(|_| OneOrMany::one(current_prompt.clone())),
                documents: Vec::new(),
                tools: tool_defs.to_vec(),
                temperature: Some(effective_temp),
                max_tokens: Some(effective_max_tokens),
                tool_choice: None,
                additional_params: None,
            };

            let llm_start = std::time::Instant::now();
            let response = model
                .completion(request)
                .await
                .map_err(|e| anyhow::anyhow!("LLM completion error: {e}"))?;
            let llm_elapsed = llm_start.elapsed();

            // Check what the model returned
            let mut has_tool_calls = false;
            let mut text_content = String::new();
            let mut tool_calls_to_execute: Vec<ToolCall> = Vec::new();

            for content in response.choice.iter() {
                match content {
                    AssistantContent::Text(t) => {
                        text_content.push_str(&t.text);
                    }
                    AssistantContent::ToolCall(tc) => {
                        has_tool_calls = true;
                        tool_calls_to_execute.push(tc.clone());
                    }
                    AssistantContent::Reasoning(r) => {
                        let reasoning_text = r.reasoning.join(" ");
                        info!("Model reasoning: {reasoning_text}");
                        if !accumulated_reasoning.is_empty() {
                            accumulated_reasoning.push('\n');
                        }
                        accumulated_reasoning.push_str(&reasoning_text);
                    }
                    _ => {}
                }
            }

            let reasoning = if accumulated_reasoning.is_empty() {
                None
            } else {
                Some(accumulated_reasoning.clone())
            };

            if !has_tool_calls {
                // Model returned a text response — we're done
                if text_content.is_empty() {
                    text_content = "I've completed processing but have no response to give.".into();
                }
                debug!(
                    "LLM response [{}/{}]: text ({} chars) in {:.1}s",
                    iteration + 1,
                    self.max_iterations,
                    text_content.len(),
                    llm_elapsed.as_secs_f64()
                );
                return Ok((text_content, tools_used, reasoning));
            }

            debug!(
                "LLM response [{}/{}]: {} tool call(s) in {:.1}s",
                iteration + 1,
                self.max_iterations,
                tool_calls_to_execute.len(),
                llm_elapsed.as_secs_f64()
            );

            // Execute tool calls and feed results back
            // First, add the assistant message with tool calls to history
            chat_history.push(current_prompt);
            chat_history.push(Message::Assistant {
                id: None,
                content: response.choice.clone(),
            });

            // Execute each tool call
            let mut tool_results: Vec<UserContent> = Vec::new();
            let mut iteration_has_success = false;
            let mut last_error = String::new();
            for tc in &tool_calls_to_execute {
                if Self::consume_interrupt(session_key) {
                    return Ok((
                        "Interrupted.".to_string(),
                        tools_used,
                        if accumulated_reasoning.is_empty() {
                            None
                        } else {
                            Some(accumulated_reasoning)
                        },
                    ));
                }

                let tool_name = &tc.function.name;
                let tool_args = &tc.function.arguments;
                tools_used.push(tool_name.clone());

                let args_preview = tool_args.to_string();
                let preview = if args_preview.len() > 200 {
                    let end = floor_char_boundary(&args_preview, 200);
                    format!("{}...", &args_preview[..end])
                } else {
                    args_preview
                };
                info!(
                    "Tool call [{}/{}]: {tool_name}({preview})",
                    iteration + 1,
                    self.max_iterations
                );

                let result = match self.tools.execute(tool_name, tool_args.clone()).await {
                    Ok(r) => {
                        if r.starts_with("Error executing ") {
                            last_error.clone_from(&r);
                        } else {
                            iteration_has_success = true;
                        }
                        r
                    }
                    Err(e) => {
                        let err = format!("Error executing {tool_name}: {e}");
                        last_error.clone_from(&err);
                        err
                    }
                };

                let result_preview = if result.len() > 200 {
                    let end = floor_char_boundary(&result, 200);
                    format!("{}... ({} chars)", &result[..end], result.len())
                } else {
                    result.clone()
                };
                debug!("Tool result [{tool_name}]: {result_preview}");

                tool_results.push(UserContent::ToolResult(ToolResult {
                    id: tc.id.clone(),
                    call_id: tc.call_id.clone(),
                    content: OneOrMany::one(ToolResultContent::Text(Text { text: result })),
                }));
            }

            // Circuit breaker: bail if all tool calls have failed for too many
            // consecutive iterations (e.g. model keeps generating malformed params).
            if iteration_has_success {
                consecutive_errors = 0;
            } else {
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    warn!(
                        "Circuit breaker: {consecutive_errors} consecutive iterations with all tool calls failing"
                    );
                    let reasoning = if accumulated_reasoning.is_empty() {
                        None
                    } else {
                        Some(accumulated_reasoning)
                    };
                    return Ok((
                        format!(
                            "I'm having trouble using a tool correctly and had to stop retrying. \
                             Last error: {last_error}. Could you try rephrasing your request?"
                        ),
                        tools_used,
                        reasoning,
                    ));
                }
            }

            // Add tool results as a user message.
            // Keep the continuation prompt minimal to avoid the model over-interpreting results.
            tool_results.push(UserContent::Text(Text {
                text:
                    "If more tool calls are needed, make them. Otherwise, respond with the result."
                        .into(),
            }));
            current_prompt = Message::User {
                content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                    OneOrMany::one(UserContent::Text(Text {
                        text: "If more tool calls are needed, make them. Otherwise, respond with the result.".into(),
                    }))
                }),
            };
        }

        warn!(
            "Agent loop reached max iterations ({}) without final response",
            self.max_iterations
        );
        let reasoning = if accumulated_reasoning.is_empty() {
            None
        } else {
            Some(accumulated_reasoning)
        };
        Ok((
            "I've been working on this but reached the maximum number of iterations. Here's what I've done so far.".to_string(),
            tools_used,
            reasoning,
        ))
    }
}

/// Strip markdown code fences from an LLM response to extract raw content.
/// Handles ```json, ```, and plain text (no fences).
fn strip_markdown_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
    {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_fences_json() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_markdown_fences_plain() {
        let input = "```\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_markdown_fences_none() {
        let input = "{\"key\": \"value\"}";
        assert_eq!(strip_markdown_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_markdown_fences_with_whitespace() {
        let input = "  \n```json\n{\"key\": \"value\"}\n```\n  ";
        assert_eq!(strip_markdown_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_markdown_fences_no_closing() {
        let input = "```json\n{\"key\": \"value\"}";
        assert_eq!(strip_markdown_fences(input), "{\"key\": \"value\"}");
    }
}

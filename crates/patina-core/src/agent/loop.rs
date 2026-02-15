use anyhow::Result;
use rig::completion::{CompletionModel, CompletionRequest, Message, ToolDefinition};
use rig::message::{
    AssistantContent, Reasoning, Text, ToolCall, ToolResult, ToolResultContent, UserContent,
};
use rig::OneOrMany;
use tracing::{debug, info, warn};

use crate::agent::context::ContextBuilder;
use crate::session::SessionManager;
use crate::tools::ToolRegistry;

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
pub struct AgentLoop<M: CompletionModel> {
    pub model: M,
    pub sessions: SessionManager,
    pub context: ContextBuilder,
    pub tools: ToolRegistry,
    pub max_iterations: usize,
    pub temperature: f64,
    pub max_tokens: u64,
    pub memory_window: usize,
    pub model_name: String,
    pub model_overrides: ModelOverrides,
}

impl<M: CompletionModel> AgentLoop<M> {
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
            )
            .await?;

        // Save to session
        let session = self.sessions.get_or_create_checked(session_key)?;
        session.add_message("user", user_message);
        session.add_message_full("assistant", &response, tools_used, reasoning);
        self.sessions.save(session_key)?;

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

    /// Consolidate old messages into MEMORY.md/HISTORY.md via LLM summarization.
    ///
    /// When `archive_all` is true (e.g. on /new), consolidates all messages.
    /// Otherwise, consolidates messages older than the keep window.
    pub async fn consolidate_memory(&mut self, session_key: &str, archive_all: bool) {
        // Extract the data we need from the session before borrowing self for LLM call
        let (conversation, end) = {
            let session = match self.sessions.sessions.get(session_key) {
                Some(s) => s,
                None => return,
            };

            let keep_count = if archive_all {
                0
            } else {
                self.memory_window / 2
            };

            let total = session.messages.len();
            if total <= keep_count {
                return;
            }

            let end = total.saturating_sub(keep_count);
            if end <= session.last_consolidated {
                return;
            }

            let messages_to_process = &session.messages[session.last_consolidated..end];
            if messages_to_process.is_empty() {
                return;
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

            (conversation, end)
        };

        let current_memory = self.context.memory().read_long_term().unwrap_or_default();

        let prompt = format!(
            r#"You are a memory consolidation agent. Process this conversation and return a JSON object with exactly two keys:

1. "history_entry": A paragraph (2-5 sentences) summarizing the key events/decisions/topics. Start with a timestamp like [YYYY-MM-DD HH:MM]. Include enough detail to be useful when found by grep search later.

2. "memory_update": The updated long-term memory content. Add any new facts: user location, preferences, personal info, habits, project context, technical decisions, tools/services used. If nothing new, return the existing content unchanged.

## Current Long-term Memory
{current_memory}

## Conversation to Process
{conversation}

Respond with ONLY valid JSON, no markdown fences."#
        );

        // Call the LLM for consolidation
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

        let response = match self.model.completion(request).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Memory consolidation LLM call failed: {e}");
                return;
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

        // Strip markdown fences if present
        let trimmed = response_text.trim();
        let json_str = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed)
            .strip_suffix("```")
            .unwrap_or(trimmed)
            .trim();

        let parsed: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                warn!("Memory consolidation: failed to parse JSON response: {e}");
                return;
            }
        };

        let memory = self.context.memory();

        // Append history entry
        if let Some(entry) = parsed.get("history_entry").and_then(|e| e.as_str()) {
            if let Err(e) = memory.append_history(entry) {
                warn!("Failed to append history: {e}");
            } else {
                info!("Memory consolidation: appended history entry");
            }
        }

        // Update long-term memory
        if let Some(update) = parsed.get("memory_update").and_then(|u| u.as_str()) {
            if let Err(e) = memory.write_long_term(update) {
                warn!("Failed to update memory: {e}");
            } else {
                info!("Memory consolidation: updated long-term memory");
            }
        }

        // Update last_consolidated counter
        if let Some(session) = self.sessions.sessions.get_mut(session_key) {
            session.last_consolidated = end;
            if let Err(e) = self.sessions.save(session_key) {
                warn!("Failed to persist session '{session_key}' after consolidation: {e}");
            }
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
    ) -> Result<(String, Vec<String>, Option<String>)> {
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
                if let Some(overrides) = self.model_overrides.find(&self.model_name) {
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
            let response = self
                .model
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
                    format!("{}...", &args_preview[..200])
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
                    format!("{}... ({} chars)", &result[..200], result.len())
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

            // Add tool results as a user message, plus a reflection prompt
            // (matches Python: "Reflect on the results and decide next steps.")
            tool_results.push(UserContent::Text(Text {
                text: "Reflect on the results and decide next steps.".into(),
            }));
            current_prompt = Message::User {
                content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                    OneOrMany::one(UserContent::Text(Text {
                        text: "Tool execution completed. Reflect on the results and decide next steps.".into(),
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

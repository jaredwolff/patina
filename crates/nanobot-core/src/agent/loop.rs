use anyhow::Result;
use rig::completion::{CompletionModel, CompletionRequest, Message, ToolDefinition};
use rig::message::{AssistantContent, Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::OneOrMany;
use tracing::{info, warn};

use crate::agent::context::ContextBuilder;
use crate::session::SessionManager;
use crate::tools::ToolRegistry;

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
}

impl<M: CompletionModel> AgentLoop<M> {
    /// Process a single user message and return the assistant's response.
    pub async fn process_message(
        &mut self,
        session_key: &str,
        user_message: &str,
    ) -> Result<String> {
        let session = self.sessions.get_or_create(session_key);
        let history = session.get_history(self.memory_window);

        // Build messages for context
        let messages_json = self
            .context
            .build_messages(&history, user_message, None, None)?;

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
                    chat_history.push(Message::Assistant {
                        id: None,
                        content: OneOrMany::one(AssistantContent::Text(Text {
                            text: content.to_string(),
                        })),
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
        let (response, tools_used) = self
            .run_loop(&system_prompt, chat_history, prompt, &tool_defs)
            .await?;

        // Save to session
        let session = self.sessions.get_or_create(session_key);
        session.add_message("user", user_message);
        session.add_message_with_tools("assistant", &response, tools_used);
        self.sessions.save(session_key)?;

        Ok(response)
    }

    /// Run the LLM ↔ tool loop until the model produces a text response or max iterations.
    async fn run_loop(
        &self,
        system_prompt: &str,
        mut chat_history: Vec<Message>,
        prompt: Message,
        tool_defs: &[ToolDefinition],
    ) -> Result<(String, Vec<String>)> {
        let mut tools_used = Vec::new();
        let mut current_prompt = prompt;

        for iteration in 0..self.max_iterations {
            // Build the rig CompletionRequest
            let mut all_messages = chat_history.clone();
            all_messages.push(current_prompt.clone());

            let request = CompletionRequest {
                preamble: Some(system_prompt.to_string()),
                chat_history: OneOrMany::many(all_messages.clone())
                    .unwrap_or_else(|_| OneOrMany::one(current_prompt.clone())),
                documents: Vec::new(),
                tools: tool_defs.to_vec(),
                temperature: Some(self.temperature),
                max_tokens: Some(self.max_tokens),
                tool_choice: None,
                additional_params: None,
            };

            let response = self
                .model
                .completion(request)
                .await
                .map_err(|e| anyhow::anyhow!("LLM completion error: {e}"))?;

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
                    _ => {}
                }
            }

            if !has_tool_calls {
                // Model returned a text response — we're done
                if text_content.is_empty() {
                    text_content = "I've completed processing but have no response to give.".into();
                }
                return Ok((text_content, tools_used));
            }

            // Execute tool calls and feed results back
            // First, add the assistant message with tool calls to history
            chat_history.push(current_prompt);
            chat_history.push(Message::Assistant {
                id: None,
                content: response.choice.clone(),
            });

            // Execute each tool call
            let mut tool_results: Vec<UserContent> = Vec::new();
            for tc in &tool_calls_to_execute {
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
                    Ok(r) => r,
                    Err(e) => format!("Error executing {tool_name}: {e}"),
                };

                tool_results.push(UserContent::ToolResult(ToolResult {
                    id: tc.id.clone(),
                    call_id: tc.call_id.clone(),
                    content: OneOrMany::one(ToolResultContent::Text(Text { text: result })),
                }));
            }

            // Add tool results as a user message and continue the loop
            current_prompt = Message::User {
                content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
                    OneOrMany::one(UserContent::Text(Text {
                        text: "Tool execution completed.".into(),
                    }))
                }),
            };
        }

        warn!(
            "Agent loop reached max iterations ({}) without final response",
            self.max_iterations
        );
        Ok((
            "I've been working on this but reached the maximum number of iterations. Here's what I've done so far.".to_string(),
            tools_used,
        ))
    }
}

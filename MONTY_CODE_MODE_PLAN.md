# Monty Code Mode Integration Plan

## Executive Summary

This document outlines a plan to integrate Monty's code execution engine into nanobot-rs as an alternative execution mode. Instead of the traditional LLM → tool call → execute → repeat loop, the agent will write Python code that Monty executes with access to nanobot's tools as Python functions.

**Goal:** Reduce LLM round trips, enable complex control flow (loops, conditionals), and provide a more natural programming interface for multi-step tasks.

**Approach:** Implement code mode as a parallel execution path alongside traditional tool calling, switchable via config.

**Estimated effort:** 2-3 weeks for full implementation and testing.

---

## Motivation

### Current Architecture (Tool Calling Mode)

```
User: "Read all .rs files in src/ and count total lines"

Round 1: LLM → list_dir("src/")
Round 2: LLM → read_file("src/main.rs")
Round 3: LLM → read_file("src/lib.rs")
Round 4: LLM → read_file("src/agent.rs")
...
Round N: LLM → "Total: X lines"
```

**Problems:**
- Linear, sequential execution
- N+1 LLM calls for N files
- High latency (each round trip = 1-2 seconds)
- No control flow (LLM can't use loops/conditionals)
- Expensive in API costs

### Proposed Architecture (Code Mode)

```
User: "Read all .rs files in src/ and count total lines"

Round 1: LLM writes Python code:
```python
files = list_dir("src/")
rs_files = [f for f in files if f.endswith(".rs")]
total = 0
for f in rs_files:
    content = read_file(f"src/{f}")
    total += len(content.split("\n"))
f"Total: {total} lines"
```

Round 2: Monty executes code (pausing at each host function call)
Round 3: LLM → response with result
```

**Benefits:**
- Single LLM call generates the plan
- Code executes locally (fast)
- Natural control flow (loops, conditionals, error handling)
- Reduced costs and latency
- Better for complex multi-step tasks

---

## Architecture Design

### Execution Mode Selection

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Traditional tool calling (current behavior)
    Tools,
    /// Code mode via Monty (new)
    Code,
    /// Let the LLM choose per task
    Hybrid,
}
```

**Config:**
```json
{
  "agents": {
    "defaults": {
      "executionMode": "code",
      "codeRuntime": {
        "resourceLimits": {
          "maxMemoryMb": 100,
          "maxExecutionMs": 30000,
          "maxAllocations": 100000
        }
      }
    }
  }
}
```

### System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         AgentLoop                            │
│                                                              │
│  ┌──────────────────────┐         ┌────────────────────┐   │
│  │  Traditional Mode    │         │    Code Mode       │   │
│  │  (Tool Calling)      │         │    (Monty)         │   │
│  │                      │         │                    │   │
│  │  LLM returns         │         │  LLM returns       │   │
│  │  ToolCall objects    │         │  Python code       │   │
│  │         ↓            │         │         ↓          │   │
│  │  ToolRegistry        │         │  MontyExecutor     │   │
│  │  .execute()          │         │  .execute()        │   │
│  └──────────────────────┘         └────────────────────┘   │
│              ↓                              ↓               │
│         ┌────────────────────────────────────────┐          │
│         │         Tool Implementations           │          │
│         │  (filesystem, shell, web, etc.)        │          │
│         └────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────────────┘
```

### Monty Host Function Interface

Monty's external functions are the bridge between sandboxed Python code and nanobot's tools:

```rust
/// Host API exposed to Monty-executed code.
/// Maps nanobot tools to Python-callable functions.
pub struct MontyHostFunctions {
    tools: Arc<ToolRegistry>,
    workspace: PathBuf,
}

impl MontyHostFunctions {
    /// Convert nanobot's Tool trait calls to Monty external functions.
    pub async fn call_tool(&self, name: &str, params: Value) -> Result<MontyObject> {
        let result_str = self.tools.execute(name, params).await?;
        Ok(MontyObject::String(result_str))
    }
}
```

**Available to Python code:**
- `read_file(path: str) -> str`
- `write_file(path: str, content: str) -> str`
- `edit_file(path: str, old: str, new: str) -> str`
- `list_dir(path: str) -> list[str]`
- `exec_command(command: str) -> str`
- `web_search(query: str, count: int) -> str`
- `web_fetch(url: str) -> str`
- `send_message(content: str) -> str`
- `schedule_cron(expression: str, code: str) -> str`

---

## Implementation Phases

### Phase 1: Monty Integration Foundation (3-4 days)

**Goal:** Get Monty running as a standalone executor, separate from AgentLoop.

#### 1.1 Add Dependencies

```toml
# nanobot-core/Cargo.toml
[dependencies]
monty = "0.0.4"
```

#### 1.2 Create MontyExecutor

Create `nanobot-core/src/agent/monty_executor.rs`:

```rust
use anyhow::Result;
use monty::{MontyRun, MontyObject, NoLimitTracker, StdPrint, LimitTracker};
use std::sync::Arc;
use crate::tools::ToolRegistry;

/// Resource limits for Monty execution.
#[derive(Debug, Clone)]
pub struct MontyResourceLimits {
    pub max_memory_mb: usize,
    pub max_execution_ms: u64,
    pub max_allocations: usize,
}

impl Default for MontyResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: 100,
            max_execution_ms: 30000,
            max_allocations: 100000,
        }
    }
}

/// Custom limit tracker for Monty.
pub struct MontyLimitTracker {
    limits: MontyResourceLimits,
    start_time: std::time::Instant,
    allocations: usize,
}

impl LimitTracker for MontyLimitTracker {
    fn on_allocation(&mut self, size: usize) -> Result<(), String> {
        self.allocations += 1;
        if self.allocations > self.limits.max_allocations {
            return Err(format!(
                "Allocation limit exceeded: {} > {}",
                self.allocations, self.limits.max_allocations
            ));
        }
        Ok(())
    }

    fn check_execution_time(&self) -> Result<(), String> {
        let elapsed = self.start_time.elapsed().as_millis() as u64;
        if elapsed > self.limits.max_execution_ms {
            return Err(format!(
                "Execution time limit exceeded: {}ms > {}ms",
                elapsed, self.limits.max_execution_ms
            ));
        }
        Ok(())
    }
}

/// Executes Python code via Monty with access to nanobot tools.
pub struct MontyExecutor {
    tools: Arc<ToolRegistry>,
    limits: MontyResourceLimits,
}

impl MontyExecutor {
    pub fn new(tools: Arc<ToolRegistry>, limits: MontyResourceLimits) -> Self {
        Self { tools, limits }
    }

    /// Execute Python code with tool access.
    pub async fn execute(&self, code: &str, inputs: Vec<(&str, MontyObject)>) -> Result<String> {
        // Extract input names
        let input_names: Vec<String> = inputs.iter().map(|(n, _)| n.to_string()).collect();
        let input_values: Vec<MontyObject> = inputs.into_iter().map(|(_, v)| v).collect();

        // External functions are the tool names
        let external_functions: Vec<String> = self.tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        // Create Monty runner
        let runner = MontyRun::new(
            code.to_string(),
            "agent_code.py",
            input_names,
            external_functions,
        )?;

        // TODO: Implement iterative execution with host function calls
        // For now, use synchronous execution (no external functions)
        let mut limit_tracker = MontyLimitTracker {
            limits: self.limits.clone(),
            start_time: std::time::Instant::now(),
            allocations: 0,
        };

        let result = runner.run(input_values, &mut limit_tracker, &mut StdPrint)?;

        Ok(format!("{:?}", result))
    }
}
```

#### 1.3 Add Config Schema

Update `nanobot-config/src/schema.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefaults {
    // ... existing fields ...

    #[serde(default = "default_execution_mode")]
    pub execution_mode: ExecutionMode,

    #[serde(default)]
    pub code_runtime: CodeRuntimeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Tools,
    Code,
    Hybrid,
}

fn default_execution_mode() -> ExecutionMode {
    ExecutionMode::Tools // Keep current behavior as default
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRuntimeConfig {
    #[serde(default = "default_max_memory_mb")]
    pub max_memory_mb: usize,

    #[serde(default = "default_max_execution_ms")]
    pub max_execution_ms: u64,

    #[serde(default = "default_max_allocations")]
    pub max_allocations: usize,
}

impl Default for CodeRuntimeConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 100,
            max_execution_ms: 30000,
            max_allocations: 100000,
        }
    }
}
```

#### 1.4 Write Unit Tests

Create `nanobot-core/src/agent/monty_executor.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;

    #[tokio::test]
    async fn test_simple_execution() {
        let tools = Arc::new(ToolRegistry::new());
        let executor = MontyExecutor::new(tools, MontyResourceLimits::default());

        let code = "x + y";
        let inputs = vec![
            ("x", MontyObject::Int(10)),
            ("y", MontyObject::Int(32)),
        ];

        let result = executor.execute(code, inputs).await.unwrap();
        assert_eq!(result, "42");
    }

    #[tokio::test]
    async fn test_resource_limits() {
        let tools = Arc::new(ToolRegistry::new());
        let limits = MontyResourceLimits {
            max_execution_ms: 100,
            ..Default::default()
        };
        let executor = MontyExecutor::new(tools, limits);

        // Infinite loop should be caught
        let code = "while True: pass";
        let result = executor.execute(code, vec![]).await;
        assert!(result.is_err());
    }
}
```

**Deliverable:** Monty executes simple Python code with resource limits. No tool access yet.

---

### Phase 2: Host Function Integration (4-5 days)

**Goal:** Enable Monty code to call nanobot tools as Python functions.

#### 2.1 Implement Iterative Execution

Monty's execution model: code runs until it hits an external function call, pauses, returns control to host. Host provides the result, code resumes.

Update `MontyExecutor::execute`:

```rust
pub async fn execute(&self, code: &str, inputs: Vec<(&str, MontyObject)>) -> Result<String> {
    let input_names: Vec<String> = inputs.iter().map(|(n, _)| n.to_string()).collect();
    let input_values: Vec<MontyObject> = inputs.into_iter().map(|(_, v)| v).collect();

    let external_functions: Vec<String> = self.tools
        .list()
        .iter()
        .map(|t| t.name().to_string())
        .collect();

    let runner = MontyRun::new(
        code.to_string(),
        "agent_code.py",
        input_names,
        external_functions,
    )?;

    // Start execution
    let mut progress = runner.start(input_values, &mut MontyLimitTracker::new(self.limits.clone()), &mut StdPrint)?;

    // Iterative execution loop
    loop {
        match progress {
            monty::RunProgress::Snapshot(snapshot) => {
                // Code paused at an external function call
                let func_name = snapshot.function_name();
                let args = snapshot.args();

                tracing::debug!(
                    "Monty calling external function: {}({:?})",
                    func_name,
                    args
                );

                // Convert args to JSON
                let params = self.monty_args_to_json(args)?;

                // Call the tool
                let result_str = self.tools.execute(func_name, params).await?;

                // Convert result back to MontyObject
                let result_obj = MontyObject::String(result_str);

                // Resume execution with result
                progress = snapshot.resume(result_obj, &mut MontyLimitTracker::new(self.limits.clone()), &mut StdPrint)?;
            }
            monty::RunProgress::Complete(result) => {
                // Execution finished
                return Ok(self.monty_object_to_string(result.output()));
            }
        }
    }
}

fn monty_args_to_json(&self, args: &[MontyObject]) -> Result<serde_json::Value> {
    // Convert Monty's arguments to JSON for tool execution
    // Handle: Int, Float, String, Bool, List, Dict, None
    todo!("Convert MontyObject to serde_json::Value")
}

fn monty_object_to_string(&self, obj: &MontyObject) -> String {
    // Format MontyObject as string result
    match obj {
        MontyObject::String(s) => s.clone(),
        MontyObject::Int(i) => i.to_string(),
        MontyObject::Float(f) => f.to_string(),
        MontyObject::Bool(b) => b.to_string(),
        MontyObject::None => "None".to_string(),
        MontyObject::List(items) => format!("{:?}", items),
        MontyObject::Dict(map) => format!("{:?}", map),
        _ => format!("{:?}", obj),
    }
}
```

#### 2.2 Implement Type Conversions

Monty uses `MontyObject` enum, nanobot uses `serde_json::Value`. Need bidirectional conversion:

```rust
/// Convert MontyObject to JSON for tool calls.
fn monty_to_json(obj: &MontyObject) -> Result<serde_json::Value> {
    match obj {
        MontyObject::None => Ok(Value::Null),
        MontyObject::Bool(b) => Ok(Value::Bool(*b)),
        MontyObject::Int(i) => Ok(json!(i)),
        MontyObject::Float(f) => Ok(json!(f)),
        MontyObject::String(s) => Ok(Value::String(s.clone())),
        MontyObject::List(items) => {
            let arr: Result<Vec<_>> = items.iter().map(monty_to_json).collect();
            Ok(Value::Array(arr?))
        }
        MontyObject::Dict(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map.iter() {
                obj.insert(k.clone(), monty_to_json(v)?);
            }
            Ok(Value::Object(obj))
        }
        MontyObject::Tuple(items) => {
            // Treat tuples as arrays
            let arr: Result<Vec<_>> = items.iter().map(monty_to_json).collect();
            Ok(Value::Array(arr?))
        }
        _ => anyhow::bail!("Unsupported MontyObject type: {:?}", obj),
    }
}

/// Convert JSON result from tool to MontyObject.
fn json_to_monty(val: &serde_json::Value) -> MontyObject {
    match val {
        Value::Null => MontyObject::None,
        Value::Bool(b) => MontyObject::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MontyObject::Int(i)
            } else if let Some(f) = n.as_f64() {
                MontyObject::Float(f)
            } else {
                MontyObject::String(n.to_string())
            }
        }
        Value::String(s) => MontyObject::String(s.clone()),
        Value::Array(arr) => {
            MontyObject::List(arr.iter().map(json_to_monty).collect())
        }
        Value::Object(map) => {
            let mut dict = std::collections::HashMap::new();
            for (k, v) in map.iter() {
                dict.insert(k.clone(), json_to_monty(v));
            }
            MontyObject::Dict(dict)
        }
    }
}
```

#### 2.3 Add Type Stubs for Tools

Monty supports type checking with stubs. Create Python type definitions for nanobot tools:

```rust
impl MontyExecutor {
    /// Generate Python type stubs for available tools.
    fn generate_type_stubs(&self) -> String {
        let mut stubs = String::new();

        for tool in self.tools.list() {
            // Parse JSON schema to Python type hints
            let params_schema = tool.parameters_schema();
            let params_str = self.schema_to_python_params(&params_schema);

            stubs.push_str(&format!(
                "def {}({}) -> str:\n    \"\"\"{}\"\"\"\n    ...\n\n",
                tool.name(),
                params_str,
                tool.description()
            ));
        }

        stubs
    }

    fn schema_to_python_params(&self, schema: &serde_json::Value) -> String {
        // Convert JSON schema properties to Python function signature
        // Example: {"path": {"type": "string"}} -> "path: str"
        // This is simplified - full implementation needs to handle all types

        let props = schema.get("properties").and_then(|p| p.as_object());
        let required = schema.get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        if let Some(props) = props {
            props.iter()
                .map(|(name, prop)| {
                    let type_hint = match prop.get("type").and_then(|t| t.as_str()) {
                        Some("string") => "str",
                        Some("integer") => "int",
                        Some("number") => "float",
                        Some("boolean") => "bool",
                        Some("array") => "list",
                        Some("object") => "dict",
                        _ => "Any",
                    };

                    if required.contains(&name.as_str()) {
                        format!("{}: {}", name, type_hint)
                    } else {
                        format!("{}: {} = None", name, type_hint)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        }
    }
}
```

#### 2.4 Integration Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolRegistry, filesystem::*};

    #[tokio::test]
    async fn test_tool_calling() {
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(ListDirTool::new(None)));

        let executor = MontyExecutor::new(Arc::new(tools), Default::default());

        let code = r#"
files = list_dir(".")
len(files)
"#;

        let result = executor.execute(code, vec![]).await.unwrap();
        assert!(result.parse::<i64>().is_ok());
    }

    #[tokio::test]
    async fn test_multi_step_workflow() {
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(ReadFileTool::new(None)));
        tools.register(Box::new(WriteFileTool::new(None)));

        let executor = MontyExecutor::new(Arc::new(tools), Default::default());

        let code = r#"
content = read_file("test.txt")
upper = content.upper()
write_file("test_upper.txt", upper)
"Done"
"#;

        let result = executor.execute(code, vec![]).await.unwrap();
        assert_eq!(result, "Done");
    }
}
```

**Deliverable:** Monty code can call nanobot tools. Type checking works with stubs.

---

### Phase 3: Agent Loop Integration (3-4 days)

**Goal:** Modify `AgentLoop` to support code mode alongside tool calling.

#### 3.1 Detect Execution Mode from LLM Response

The LLM needs to know whether to return tool calls or code. We signal this via system prompt:

**Traditional mode system prompt:**
```
You are an AI assistant with access to these tools:
- read_file(path: str) -> str
- write_file(path: str, content: str) -> str
...

When you need to perform actions, call tools using the tool calling interface.
```

**Code mode system prompt:**
```
You are an AI assistant that writes Python code to accomplish tasks.

Available functions you can call in your code:
```python
def read_file(path: str) -> str:
    """Read contents of a file."""
    ...

def write_file(path: str, content: str) -> str:
    """Write content to a file."""
    ...
```

When the user asks you to do something:
1. Write Python code that accomplishes the task
2. Use the available functions to interact with the system
3. Return the code as a single Python script
4. The code will be executed in a secure sandbox

Example:
User: "Count lines in all .rs files"
Assistant:
```python
files = list_dir("src/")
rs_files = [f for f in files if f.endswith(".rs")]
total = 0
for f in rs_files:
    content = read_file(f"src/{f}")
    total += len(content.split("\n"))
f"Total: {total} lines"
```
```

#### 3.2 Modify ContextBuilder

Update `nanobot-core/src/agent/context.rs`:

```rust
impl ContextBuilder {
    pub fn build_system_prompt(
        &self,
        tools: &ToolRegistry,
        execution_mode: ExecutionMode,
    ) -> String {
        let mut prompt = String::new();

        // Load AGENTS.md, SOUL.md, etc.
        prompt.push_str(&self.load_agent_context());

        // Add tool context based on mode
        match execution_mode {
            ExecutionMode::Tools => {
                prompt.push_str("\n\n# Available Tools\n\n");
                prompt.push_str("When you need to perform actions, call these tools:\n\n");
                for tool in tools.list() {
                    prompt.push_str(&format!(
                        "- {}: {}\n",
                        tool.name(),
                        tool.description()
                    ));
                }
            }
            ExecutionMode::Code => {
                prompt.push_str("\n\n# Code Mode\n\n");
                prompt.push_str("You are an AI assistant that writes Python code to accomplish tasks.\n\n");
                prompt.push_str("Available functions:\n\n```python\n");
                prompt.push_str(&self.generate_python_function_stubs(tools));
                prompt.push_str("```\n\n");
                prompt.push_str("When given a task:\n");
                prompt.push_str("1. Write Python code that accomplishes it\n");
                prompt.push_str("2. Use the available functions\n");
                prompt.push_str("3. Return code in a ```python code block\n");
                prompt.push_str("4. The code will be executed in a secure sandbox\n");
            }
            ExecutionMode::Hybrid => {
                prompt.push_str("\n\n# Hybrid Mode\n\n");
                prompt.push_str("You can either:\n");
                prompt.push_str("- Call individual tools for simple tasks\n");
                prompt.push_str("- Write Python code for complex multi-step tasks\n\n");
                prompt.push_str("Available tools:\n");
                // Include both formats
            }
        }

        prompt
    }

    fn generate_python_function_stubs(&self, tools: &ToolRegistry) -> String {
        // Same as MontyExecutor::generate_type_stubs
        let mut stubs = String::new();
        for tool in tools.list() {
            let params = self.schema_to_params(tool.parameters_schema());
            stubs.push_str(&format!(
                "def {}({}) -> str:\n    \"\"\"{}\"\"\"\n    ...\n\n",
                tool.name(),
                params,
                tool.description()
            ));
        }
        stubs
    }
}
```

#### 3.3 Modify AgentLoop::process_message

Update `nanobot-core/src/agent/loop.rs`:

```rust
impl<M: CompletionModel> AgentLoop<M> {
    pub async fn process_message(
        &mut self,
        session_key: &str,
        user_message: &str,
    ) -> Result<String> {
        // ... existing interrupt check ...

        // Load session
        let mut session = self.sessions.get_or_create(session_key).await?;
        session.add_user_message(user_message);

        // Build context based on execution mode
        let system_prompt = self.context.build_system_prompt(
            &self.tools,
            self.execution_mode,
        );

        let messages = self.context.build_messages(
            &session,
            self.memory_window,
        );

        // Process based on mode
        let response = match self.execution_mode {
            ExecutionMode::Tools => {
                self.process_tool_mode(&mut session, &system_prompt, &messages).await?
            }
            ExecutionMode::Code => {
                self.process_code_mode(&mut session, &system_prompt, &messages).await?
            }
            ExecutionMode::Hybrid => {
                self.process_hybrid_mode(&mut session, &system_prompt, &messages).await?
            }
        };

        // Save session
        session.add_assistant_message(&response);
        self.sessions.save(&session).await?;

        Ok(response)
    }

    async fn process_tool_mode(
        &mut self,
        session: &mut Session,
        system_prompt: &str,
        messages: &[Message],
    ) -> Result<String> {
        // Existing tool calling logic
        // ... (current implementation)
    }

    async fn process_code_mode(
        &mut self,
        session: &mut Session,
        system_prompt: &str,
        messages: &[Message],
    ) -> Result<String> {
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > self.max_iterations {
                return Ok(format!(
                    "Maximum iterations ({}) reached. Task may be too complex.",
                    self.max_iterations
                ));
            }

            info!("[{}/{}] Calling LLM for code generation", iteration, self.max_iterations);

            // Call LLM
            let response = self.model
                .completion_request(&system_prompt)
                .messages(messages.to_vec())
                .send()
                .await?;

            // Extract Python code from response
            if let Some(code) = self.extract_python_code(&response.content) {
                info!("[{}/{}] Executing Python code", iteration, self.max_iterations);

                // Execute via Monty
                let executor = MontyExecutor::new(
                    Arc::new(self.tools.clone()),
                    self.code_runtime_config.clone(),
                );

                match executor.execute(&code, vec![]).await {
                    Ok(result) => {
                        // Execution succeeded
                        return Ok(result);
                    }
                    Err(e) => {
                        // Execution failed - feed error back to LLM
                        let error_msg = format!("Code execution error: {}", e);
                        warn!("{}", error_msg);

                        // Add error to context and retry
                        session.add_assistant_message(&code);
                        session.add_system_message(&error_msg);

                        // Continue loop - LLM will see error and try again
                    }
                }
            } else {
                // No code in response - treat as final answer
                return Ok(response.content);
            }
        }
    }

    async fn process_hybrid_mode(
        &mut self,
        session: &mut Session,
        system_prompt: &str,
        messages: &[Message],
    ) -> Result<String> {
        // Try to detect if response contains code or tool calls
        // If code: use process_code_mode
        // If tool calls: use process_tool_mode
        // This requires inspecting the LLM response format

        // Simplified: try code mode first, fallback to tools
        // A more sophisticated approach would use explicit markers
        todo!("Implement hybrid mode detection")
    }

    fn extract_python_code(&self, content: &str) -> Option<String> {
        // Extract code from ```python ... ``` blocks
        let re = regex::Regex::new(r"```python\n(.*?)\n```").unwrap();
        re.captures(content)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }
}
```

#### 3.4 Wire Up Config

Update `nanobot-cli/src/main.rs`:

```rust
async fn build_agent_loop(config: &Config) -> Result<AgentLoop<impl CompletionModel>> {
    // ... existing setup ...

    let execution_mode = config.agents.defaults.execution_mode;
    let code_runtime_config = config.agents.defaults.code_runtime.clone();

    Ok(AgentLoop {
        model,
        sessions,
        context,
        tools,
        max_iterations: config.agents.defaults.max_iterations,
        temperature,
        max_tokens,
        memory_window,
        model_name,
        model_overrides,
        execution_mode,
        code_runtime_config,
    })
}
```

**Deliverable:** Agent can run in code mode. User configures via `executionMode: "code"`.

---

### Phase 4: Optimization & Polish (2-3 days)

#### 4.1 Add Stdout/Stderr Capture

Monty captures print statements. Surface them in the response:

```rust
pub struct MontyPrintCapture {
    stdout: Arc<Mutex<Vec<String>>>,
    stderr: Arc<Mutex<Vec<String>>>,
}

impl monty::PrintHandler for MontyPrintCapture {
    fn print(&mut self, msg: &str) {
        self.stdout.lock().unwrap().push(msg.to_string());
    }

    fn eprint(&mut self, msg: &str) {
        self.stderr.lock().unwrap().push(msg.to_string());
    }
}

// In MontyExecutor::execute:
let print_handler = MontyPrintCapture::new();
let result = runner.run(input_values, &mut limit_tracker, &mut print_handler)?;

// Include stdout in result
let stdout = print_handler.stdout.lock().unwrap().join("\n");
if !stdout.is_empty() {
    result_str.push_str(&format!("\n\nOutput:\n{}", stdout));
}
```

#### 4.2 Add Code Validation

Before executing, validate Python syntax:

```rust
impl MontyExecutor {
    fn validate_code(&self, code: &str) -> Result<()> {
        // Monty will catch syntax errors during parsing
        // But we can do a quick check for obvious issues

        if code.trim().is_empty() {
            anyhow::bail!("Empty code block");
        }

        // Check for dangerous patterns (even though Monty sandboxes)
        let dangerous = ["__import__", "eval(", "exec(", "compile("];
        for pattern in dangerous {
            if code.contains(pattern) {
                warn!("Code contains potentially dangerous pattern: {}", pattern);
            }
        }

        Ok(())
    }
}
```

#### 4.3 Add Caching for Parsed Code

If the same code runs multiple times (e.g., cron jobs), cache the parsed MontyRun:

```rust
use std::collections::HashMap;

pub struct MontyExecutor {
    tools: Arc<ToolRegistry>,
    limits: MontyResourceLimits,
    cache: Arc<Mutex<HashMap<String, Vec<u8>>>>, // code hash -> serialized MontyRun
}

impl MontyExecutor {
    pub async fn execute(&self, code: &str) -> Result<String> {
        let code_hash = self.hash_code(code);

        // Try to load from cache
        let runner = if let Some(cached) = self.cache.lock().unwrap().get(&code_hash) {
            MontyRun::load(cached)?
        } else {
            let runner = MontyRun::new(/* ... */)?;

            // Cache for next time
            let serialized = runner.dump()?;
            self.cache.lock().unwrap().insert(code_hash.clone(), serialized);

            runner
        };

        // Execute...
    }

    fn hash_code(&self, code: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}
```

#### 4.4 Improve Error Messages

Make Monty errors more helpful to the LLM:

```rust
impl MontyExecutor {
    fn format_monty_error(&self, err: &monty::Error) -> String {
        match err {
            monty::Error::RuntimeError { message, traceback } => {
                format!(
                    "Runtime error: {}\n\nTraceback:\n{}",
                    message,
                    traceback
                )
            }
            monty::Error::SyntaxError { message, line } => {
                format!("Syntax error at line {}: {}", line, message)
            }
            monty::Error::ResourceLimit { message } => {
                format!("Resource limit exceeded: {}", message)
            }
            _ => format!("Execution error: {}", err),
        }
    }
}
```

#### 4.5 Add Telemetry

Track code mode usage:

```rust
#[derive(Debug, Default)]
pub struct MontyMetrics {
    pub executions: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_execution_time_ms: u64,
    pub avg_iterations: f64,
}

impl MontyExecutor {
    pub async fn execute_with_metrics(
        &self,
        code: &str,
        metrics: Arc<Mutex<MontyMetrics>>,
    ) -> Result<String> {
        let start = std::time::Instant::now();

        metrics.lock().unwrap().executions += 1;

        let result = self.execute(code, vec![]).await;

        let elapsed = start.elapsed().as_millis() as u64;

        let mut m = metrics.lock().unwrap();
        match &result {
            Ok(_) => m.successes += 1,
            Err(_) => m.failures += 1,
        }
        m.total_execution_time_ms += elapsed;

        result
    }
}
```

**Deliverable:** Production-ready code mode with validation, caching, error handling.

---

### Phase 5: Testing & Documentation (2-3 days)

#### 5.1 Integration Tests

Create `nanobot-core/tests/code_mode_integration.rs`:

```rust
use nanobot_core::agent::AgentLoop;
use nanobot_config::{Config, ExecutionMode};

#[tokio::test]
async fn test_code_mode_file_operations() {
    let mut config = Config::default();
    config.agents.defaults.execution_mode = ExecutionMode::Code;

    let mut agent = build_test_agent(config).await;

    let response = agent.process_message(
        "test:code_mode",
        "Create a file called test.txt with the content 'Hello from code mode'"
    ).await.unwrap();

    assert!(response.contains("Hello from code mode"));
}

#[tokio::test]
async fn test_code_mode_loops() {
    let mut config = Config::default();
    config.agents.defaults.execution_mode = ExecutionMode::Code;

    let mut agent = build_test_agent(config).await;

    let response = agent.process_message(
        "test:code_mode",
        "Count how many .rs files are in the src/ directory"
    ).await.unwrap();

    assert!(response.contains("files") || response.parse::<i32>().is_ok());
}

#[tokio::test]
async fn test_code_mode_error_recovery() {
    let mut config = Config::default();
    config.agents.defaults.execution_mode = ExecutionMode::Code;
    config.agents.defaults.max_iterations = 3;

    let mut agent = build_test_agent(config).await;

    // Force an error scenario
    let response = agent.process_message(
        "test:code_mode",
        "Read a file that doesn't exist: nonexistent.txt"
    ).await.unwrap();

    // Agent should handle error gracefully
    assert!(response.contains("error") || response.contains("not found"));
}

#[tokio::test]
async fn test_fallback_to_tools_mode() {
    let mut config = Config::default();
    config.agents.defaults.execution_mode = ExecutionMode::Tools;

    let mut agent = build_test_agent(config).await;

    let response = agent.process_message(
        "test:tools_mode",
        "List files in the current directory"
    ).await.unwrap();

    // Should work in traditional mode too
    assert!(!response.is_empty());
}
```

#### 5.2 Performance Benchmarks

Create `nanobot-core/benches/code_mode.rs`:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_tool_mode_vs_code_mode(c: &mut Criterion) {
    let mut group = c.benchmark_group("execution_modes");

    group.bench_function("tool_mode_sequential", |b| {
        b.iter(|| {
            // Benchmark traditional tool calling
            // Read 10 files sequentially via tool calls
        });
    });

    group.bench_function("code_mode_loop", |b| {
        b.iter(|| {
            // Benchmark code mode
            // Read 10 files in a loop via Monty
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_tool_mode_vs_code_mode);
criterion_main!(benches);
```

#### 5.3 Update Documentation

Update `nanobot-rs/README.md`:

```markdown
## Execution Modes

Nanobot supports two execution modes:

### Tool Calling Mode (default)

Traditional LLM tool calling. The model declares tool calls, the agent executes them, results are fed back.

**Config:**
```json
{
  "agents": {
    "defaults": {
      "executionMode": "tools"
    }
  }
}
```

**Best for:** Simple, single-step tasks. Well-defined tool usage patterns.

### Code Mode (experimental)

The LLM writes Python code that is executed in a secure Monty sandbox. Tools are available as Python functions.

**Config:**
```json
{
  "agents": {
    "defaults": {
      "executionMode": "code",
      "codeRuntime": {
        "maxMemoryMb": 100,
        "maxExecutionMs": 30000
      }
    }
  }
}
```

**Best for:** Complex multi-step tasks, loops, conditional logic, batch operations.

**Example task:** "Read all .rs files and count total lines"

**Tool mode:** 10+ LLM round trips (list_dir → read_file × N → count)

**Code mode:** 1 LLM call generates:
```python
files = list_dir("src/")
total = sum(len(read_file(f"src/{f}").split("\n"))
            for f in files if f.endswith(".rs"))
f"Total: {total} lines"
```

**Security:** Code runs in Monty's sandbox with no filesystem, network, or system access unless explicitly granted via allowed functions.
```

Create `nanobot-rs/docs/code-mode.md`:

```markdown
# Code Mode Guide

## Overview

Code mode allows the LLM to write Python code instead of declaring tool calls. The code is executed in a secure Monty sandbox with access to nanobot's tools as Python functions.

## When to Use Code Mode

Use code mode when:
- Task requires loops or conditionals
- Need to process multiple items
- Complex logic that's awkward to express as sequential tool calls
- Want to reduce LLM round trips

Use tool mode when:
- Simple, single-step tasks
- Model doesn't reliably write correct Python
- Need maximum transparency (tool calls are easier to inspect)

## Configuration

... (detailed config examples)

## Python API Available to LLM

... (list all tools with signatures)

## Security

... (explain Monty sandbox, resource limits)

## Troubleshooting

... (common errors, debugging tips)
```

#### 5.4 Add Example Config

Create `nanobot-rs/config.code-mode.example.json`:

```json
{
  "agents": {
    "defaults": {
      "model": "llama3.2",
      "maxTokens": 8192,
      "temperature": 0.7,
      "maxIterations": 10,
      "memoryWindow": 50,
      "executionMode": "code",
      "codeRuntime": {
        "maxMemoryMb": 100,
        "maxExecutionMs": 30000,
        "maxAllocations": 100000
      }
    }
  },
  "providers": {
    "ollama": {
      "baseUrl": "http://localhost:11434"
    }
  },
  "tools": {
    "workspace": "~/workspace"
  }
}
```

**Deliverable:** Full test coverage, benchmarks, documentation.

---

## Migration Strategy

### Backward Compatibility

**Key principle:** Existing users should see no change unless they opt in.

1. **Default to tool mode:** `executionMode: "tools"` is the default
2. **Config validation:** Warn if code mode is enabled but Monty isn't available
3. **Session compatibility:** Code mode sessions use same JSONL format
4. **Gradual rollout:** Users can test code mode on specific sessions via config override

### Rollout Plan

**Phase 1: Experimental (first 2 weeks after merge)**
- Code mode available behind config flag
- Documentation clearly marks it as "experimental"
- Gather feedback from early adopters

**Phase 2: Beta (weeks 3-4)**
- Address issues found in Phase 1
- Add more example workflows
- Performance tuning

**Phase 3: Stable (week 5+)**
- Promote code mode to stable status
- Consider making it the default for models that handle it well
- Add model-specific recommendations (GPT-4 vs Llama 3.2)

### Feature Flags

Support per-session execution mode override:

```json
{
  "agents": {
    "defaults": {
      "executionMode": "tools"
    },
    "overrides": {
      "telegram:123456": {
        "executionMode": "code"
      }
    }
  }
}
```

---

## Testing Strategy

### Unit Tests

- **MontyExecutor:** Code execution, type conversion, error handling
- **ContextBuilder:** System prompt generation for code mode
- **AgentLoop:** Mode selection, code extraction, iteration limits

### Integration Tests

- **End-to-end workflows:** File operations, web search, multi-step tasks
- **Error recovery:** Invalid code, resource limits, tool errors
- **Mode comparison:** Same task in tool mode vs code mode

### Manual Testing Scenarios

1. **Simple task:** "What files are in the current directory?"
2. **Loop task:** "Count lines in all .rs files"
3. **Conditional task:** "Find all TODO comments in .rs files"
4. **Error handling:** "Read a file that doesn't exist"
5. **Resource limits:** "Generate an infinite loop" (should timeout)
6. **Mixed tasks:** Switch between code mode and tool mode in same session

### Performance Testing

- **Latency:** Measure end-to-end response time
- **LLM calls:** Count how many LLM round trips for same task
- **Memory:** Monitor Monty sandbox memory usage
- **Throughput:** Concurrent sessions with code mode

---

## Risk Mitigation

### Risk 1: LLM Code Quality

**Problem:** LLM generates buggy Python code.

**Mitigation:**
- Iteration with error feedback (max N retries)
- Clear error messages help LLM self-correct
- Type stubs improve code quality
- Fall back to tool mode if code mode fails repeatedly

### Risk 2: Monty API Changes

**Problem:** Monty is pre-1.0, API may break.

**Mitigation:**
- Pin exact Monty version in Cargo.toml
- Monitor Monty releases for breaking changes
- Encapsulate all Monty interaction in MontyExecutor
- Easy to swap implementations if needed

### Risk 3: Security Vulnerabilities

**Problem:** Sandbox escape or resource exhaustion.

**Mitigation:**
- Use Monty's built-in resource limits
- Validate all inputs/outputs at boundary
- Regular security audits of host function interface
- Monitor Monty's security advisories

### Risk 4: User Confusion

**Problem:** Users don't understand when to use code mode.

**Mitigation:**
- Clear documentation with examples
- Good defaults (tool mode for now)
- Warning messages if code mode fails repeatedly
- Let LLM choose in hybrid mode

### Risk 5: Debugging Difficulty

**Problem:** Harder to debug code mode vs tool calls.

**Mitigation:**
- Capture and surface all stdout/stderr
- Detailed logging of Monty execution
- Save generated code to session history
- Option to inspect intermediate results

---

## Success Metrics

Track these metrics to evaluate code mode success:

1. **Latency reduction:** Task completion time (tool mode vs code mode)
2. **Cost reduction:** Number of LLM API calls per task
3. **Reliability:** Success rate (did task complete correctly?)
4. **User satisfaction:** Opt-in rate, user feedback
5. **Resource usage:** Memory, CPU, execution time

**Target goals (after Phase 3):**
- 50% reduction in LLM calls for multi-step tasks
- <100ms Monty execution overhead
- 95%+ code execution success rate
- <5% increase in memory footprint

---

## Future Enhancements

### 1. Persistent Code Mode

Allow LLM to maintain a persistent Python environment across messages:

```python
# Message 1
data = load_data()

# Message 2 (reuses same environment)
filtered = [x for x in data if x > 10]

# Message 3
save_results(filtered)
```

Implementation: Serialize Monty heap between messages.

### 2. Mixed Mode

LLM can use both code and tool calls in same response:

```json
{
  "thought": "I'll use code to process files, then call send_message",
  "code": "results = [analyze(f) for f in list_dir('.')]",
  "tool_calls": [
    {"name": "send_message", "args": {"content": "Analysis complete"}}
  ]
}
```

### 3. Skill-Specific Code Environments

Skills can provide custom Python modules to the Monty environment:

```yaml
---
name: data-analysis
pythonModules:
  - pandas_lite  # Minimal pandas subset for Monty
  - numpy_core
---
```

### 4. Code Templates

Pre-written code snippets the LLM can reuse:

```python
# Template: batch_file_operation
def process_files(pattern, operation):
    files = list_dir(".")
    matching = [f for f in files if pattern in f]
    return [operation(f) for f in matching]
```

LLM can call templates like: `process_files("*.rs", lambda f: len(read_file(f)))`

---

## Appendix: Monty Reference

### Key Monty APIs

```rust
// Create runner
let runner = MontyRun::new(
    code: String,
    script_name: &str,
    inputs: Vec<String>,
    external_functions: Vec<String>,
)?;

// Synchronous execution
let result = runner.run(
    input_values: Vec<MontyObject>,
    limit_tracker: impl LimitTracker,
    print_handler: impl PrintHandler,
)?;

// Iterative execution
let progress = runner.start(inputs, limits, print)?;
loop {
    match progress {
        RunProgress::Snapshot(s) => {
            let result = call_host_function(s.function_name(), s.args());
            progress = s.resume(result, limits, print)?;
        }
        RunProgress::Complete(c) => break c.output(),
    }
}

// Serialization
let bytes = runner.dump()?;
let restored = MontyRun::load(&bytes)?;
```

### Supported Python Subset

**Supported:**
- Variables, functions, async functions
- Control flow: if/elif/else, for, while, break, continue
- Data types: int, float, str, bool, list, dict, tuple, set
- Comprehensions: list/dict/set
- String formatting: f-strings
- Operators: arithmetic, comparison, logical, membership
- Standard modules: sys, typing, asyncio, dataclasses (soon), json (soon)

**Not supported:**
- Classes (coming soon)
- Match statements (coming soon)
- Imports (except allowed modules)
- Standard library (except select modules)
- Third-party packages

### Resource Limits

```rust
pub trait LimitTracker {
    fn on_allocation(&mut self, size: usize) -> Result<(), String>;
    fn check_execution_time(&self) -> Result<(), String>;
}
```

Limits can track:
- Memory allocations
- Total memory used
- Execution time
- Stack depth

---

## Questions & Decisions

### Decision Log

| Decision | Rationale | Date |
|----------|-----------|------|
| Default to tool mode | Backward compatibility, code mode still experimental | - |
| Use Monty's type checking | Improves code quality, standard feature | - |
| Iterative execution with pause/resume | Needed for async tool calls, enables snapshotting | - |
| Error feedback loop (max N retries) | LLM can self-correct syntax/logic errors | - |
| Separate MontyExecutor module | Clean separation, easier to test and swap implementations | - |

### Open Questions

1. **Should hybrid mode be the default eventually?**
   - Pro: Best of both worlds, LLM chooses optimal approach
   - Con: More complex, LLM might choose poorly

2. **How to handle long-running code?**
   - Current: Hard timeout via resource limits
   - Alternative: Progress reporting, user can cancel

3. **Should we support persistent environments?**
   - Pro: More natural for multi-turn coding tasks
   - Con: Complexity, state management, memory leaks

4. **What's the right max iteration limit for code mode?**
   - Current: Use same limit as tool mode
   - Alternative: Lower limit for code mode (code should work first try?)

---

## Implementation Checklist

### Phase 1: Foundation
- [ ] Add Monty dependency
- [ ] Create MontyExecutor module
- [ ] Implement basic code execution (no tools)
- [ ] Add resource limits
- [ ] Write unit tests

### Phase 2: Tool Integration
- [ ] Implement iterative execution
- [ ] Add MontyObject ↔ JSON conversion
- [ ] Map nanobot tools to external functions
- [ ] Generate Python type stubs
- [ ] Write integration tests

### Phase 3: Agent Loop
- [ ] Add ExecutionMode enum to config
- [ ] Update ContextBuilder for code mode prompts
- [ ] Implement process_code_mode
- [ ] Add code extraction from LLM response
- [ ] Wire up config
- [ ] Test end-to-end

### Phase 4: Polish
- [ ] Add stdout/stderr capture
- [ ] Implement code validation
- [ ] Add execution caching
- [ ] Improve error messages
- [ ] Add telemetry/metrics

### Phase 5: Testing & Docs
- [ ] Write integration tests
- [ ] Add performance benchmarks
- [ ] Update README.md
- [ ] Create code-mode.md guide
- [ ] Add example config
- [ ] Manual testing across scenarios

### Launch
- [ ] Merge to main
- [ ] Tag release
- [ ] Announce experimental feature
- [ ] Gather user feedback
- [ ] Iterate based on feedback

---

## Conclusion

Monty code mode is a significant architectural addition that aligns with industry trends (Anthropic's programmatic tool calling, Cloudflare's code mode) while maintaining nanobot's local-first philosophy.

**Key advantages:**
- Reduces LLM API calls by 50-90% for multi-step tasks
- Enables natural control flow (loops, conditionals)
- No additional infrastructure (Monty is in-process, fast)
- Maintains security via sandboxing
- Backward compatible with existing tool mode

**Implementation is straightforward:**
- Monty provides the sandbox and runtime
- Integration is ~1000 lines of glue code
- Config-driven, user opt-in
- Existing tools work unchanged

**Recommended approach:**
- Start with tool mode as default
- Roll out code mode as experimental feature
- Let users test and provide feedback
- Evaluate metrics and iterate
- Promote to stable when ready

The 5-phase plan provides a clear path from proof-of-concept to production-ready feature in 2-3 weeks.

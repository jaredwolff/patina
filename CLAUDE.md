# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nanobot is a lightweight AI agent framework written in Rust, designed to run LLM agents with tool-calling capabilities across multiple chat channels. It supports interactive CLI mode and gateway mode for persistent messaging integrations.

**This is a rewrite of the Python nanobot** (see `../RUST_REWRITE_PLAN.md` for full strategy). The Rust version maintains compatibility with Python's config and session formats to allow side-by-side operation during transition.

## Workspace Structure

This is a Cargo workspace with 4 crates:

- **nanobot-core**: Agent loop, session management, tool system, message bus
- **nanobot-config**: Configuration schema and loading
- **nanobot-channels**: Channel adapters (Telegram, etc.) and ChannelManager
- **nanobot-cli**: Main binary with CLI and gateway modes

## Build and Test Commands

```bash
# Build all crates
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Run the CLI agent in interactive mode
cargo run --bin nanobot-cli -- agent

# Run a single message (non-interactive)
cargo run --bin nanobot-cli -- agent -m "your message here"

# Start gateway mode (currently unimplemented)
cargo run --bin nanobot-cli -- serve

# Specify custom config
cargo run --bin nanobot-cli -- -c /path/to/config.json agent
```

## Configuration

Config is stored in JSON format (camelCase). Default locations checked in order:
1. `--config` CLI argument
2. `./config.json`
3. `~/.nanobot/config.json`

See `config.example.json` for full schema. Key sections:
- `agents.defaults`: Model settings, workspace path, iteration limits
- `providers`: API keys and base URLs for Ollama, OpenAI, Anthropic, etc.
- `channels`: Telegram bot config (token, allowlist)
- `tools`: Workspace restrictions, exec timeouts

## Architecture

### Agent Loop (nanobot-core/src/agent/loop.rs)

The `AgentLoop` is the core orchestrator:
1. Loads session history from JSONL files (compatible with Python nanobot)
2. Builds context using `ContextBuilder`
3. Calls LLM with tool definitions via rig-core
4. Executes tool calls via `ToolRegistry`
5. Feeds results back to LLM in a loop (max iterations configurable)
6. Saves final response to session

Tool loop iterations are logged with `[N/max]` prefix.

### Sessions (nanobot-core/src/session.rs)

Sessions are persisted as JSONL files in `~/.nanobot/sessions/`:
- First line is metadata (type="metadata", timestamps, last_consolidated)
- Subsequent lines are messages (role, content, timestamp, tools_used)
- Session keys like `"cli:interactive"` are sanitized to filenames (`cli_interactive.jsonl`)

Sessions track:
- Message history with timestamps
- Tools used per assistant message
- Created/updated timestamps
- Memory window limits (only recent N messages sent to LLM)

### Tool System (nanobot-core/src/tools/)

All tools implement the `Tool` trait:
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<String>;
}
```

Current tools:
- **filesystem**: `read_file`, `write_file`, `edit_file`, `list_dir` (optional workspace restriction)
- **shell**: `exec_command` (runs in workspace dir, configurable timeout)
- **web**, **message**, **spawn**, **cron**: Defined but not yet implemented

Tools are registered in `main.rs` during `build_agent_loop()`.

### Message Bus (nanobot-core/src/bus.rs)

Async pub/sub system connecting channels to the agent:
- `InboundMessage`: From channels → agent (channel, sender_id, chat_id, content, media)
- `OutboundMessage`: From agent → channels (channel, chat_id, content)
- Uses Tokio mpsc for inbound, broadcast for outbound

Session keys are derived as `"{channel}:{chat_id}"`.

### Provider Selection (nanobot-cli/src/main.rs:78-118)

The `create_model()` function prioritizes providers:
1. **OpenAI-compatible** (if `providers.openai.apiBase` or `apiKey` set): Supports llama.cpp, vLLM, OpenAI API
2. **Ollama** (default): Local-first provider at `http://localhost:11434`

Note: The codebase uses `rig-core` 0.30 for LLM abstraction. The `CompletionModelHandle` pattern is used to work around lifetime issues.

### Context Builder (nanobot-core/src/agent/context.rs)

Builds system prompt and message history for the LLM. Injects workspace path and available tools into context. Currently uses a simple message list builder but is extensible for:
- Skills (nanobot-core/src/agent/skills.rs)
- Subagents (nanobot-core/src/agent/subagent.rs)
- Memory consolidation (nanobot-core/src/agent/memory.rs)

## Development Notes

### Session Persistence Format

Sessions use JSONL to match the Python nanobot format for cross-compatibility. When reading sessions:
- Check first line for `_type: "metadata"`
- Parse timestamps as RFC3339 or Python isoformat fallback
- Handle malformed lines gracefully (skip with warning)

### Error Handling

- Tools return `Result<String>` — errors are formatted and sent back to the LLM
- Agent loop catches LLM errors and returns user-facing messages
- Session save failures are logged but don't crash the agent

### Logging

Uses `tracing` with env filter (default: info level). Set `RUST_LOG=debug` for verbose output including:
- Tool call details (name, args preview, iteration count)
- Model reasoning tokens (from providers that support it)
- Session load/save operations

### Gateway Mode (Not Yet Implemented)

The `serve` command is a placeholder. Future implementation will:
1. Initialize `ChannelManager`
2. Register enabled channels from config
3. Start each channel's listener (e.g., Telegram long polling)
4. Route inbound messages through `MessageBus`
5. Run one `AgentLoop` per session concurrently

## Implementation Status

Currently at **Phase 1: Core Foundation** (see ../RUST_REWRITE_PLAN.md). Implemented:
- ✅ Config loading (JSON with camelCase, serde)
- ✅ Session persistence (JSONL, Python-compatible)
- ✅ Message bus (tokio mpsc/broadcast)
- ✅ LLM integration via rig-core (Ollama default, OpenAI-compatible support)
- ✅ Tool system (registry + basic filesystem/shell tools)
- ✅ Agent loop (LLM ↔ tool iteration)
- ✅ Context builder (system prompt + message history)
- ✅ CLI (interactive mode with rustyline)

Not yet implemented:
- ❌ Memory consolidation (MEMORY.md/HISTORY.md summarization)
- ❌ Skills loader (YAML frontmatter, progressive loading)
- ❌ Web tools (Brave search, readability extraction)
- ❌ Subagent system (background task spawning)
- ❌ Cron service
- ❌ Heartbeat
- ❌ Gateway mode (channel integrations)

## Design Principles from RUST_REWRITE_PLAN.md

### Local-First Philosophy

Provider priority order (see main.rs:78-118):
1. **Ollama** (local, no API key needed) — default
2. **OpenAI-compatible** (vLLM, llama.cpp, LocalAI via custom apiBase)
3. Cloud providers (Anthropic, OpenAI, DeepSeek, etc.) — configured fallbacks

The agent loop uses rig's `CompletionModel` trait — provider selection is config-driven, not hardcoded.

### Python Compatibility

These formats are **intentionally compatible** with the Python nanobot:
- **config.json**: Same camelCase fields, same structure
- **Session JSONL**: Same format (metadata line + message lines)
- **Session keys**: Same `"{channel}:{chat_id}"` pattern

This allows running Python and Rust versions side-by-side during transition.

### Skills Architecture (Future)

Three layers (from RUST_REWRITE_PLAN.md):
1. **Markdown skills** (preferred): `SKILL.md` files with YAML frontmatter — LLM interprets instructions, no compilation needed
2. **Bundled scripts**: Python/Bash scripts in `scripts/` dir — executed via `exec` tool
3. **WASM plugins** (deferred): Native tool plugins with sandboxed execution

Current implementation: None yet (Phase 2). When implementing skills loader:
- Parse YAML frontmatter from SKILL.md
- Check requirements (`bins`, `env`)
- Progressive loading: metadata always in context, full body on-demand
- **Must be compatible with existing Python-era skills** — no migration required

### Channel Architecture (Future)

The `Channel` trait (defined in RUST_REWRITE_PLAN.md but not yet implemented):
```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&mut self, bus: mpsc::Sender<InboundMessage>) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
    fn is_allowed(&self, sender_id: &str) -> bool;
}
```

Priority order: Telegram → Discord → Slack → Email.

## Code Quality Guidelines

This is a new project with no legacy code. When making changes:

- **No backwards-compatibility hacks**: Delete unused code completely rather than commenting it out, renaming with underscores, or adding "removed" comments
- **Clean refactoring**: If something is unused after a change, remove it entirely
- **Modern Rust idioms**: Use current best practices, no need to maintain old patterns
- **Direct solutions**: Implement features cleanly without workarounds for historical reasons
- **Respect Python compatibility where specified**: config.json and session JSONL formats must remain compatible

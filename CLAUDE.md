# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nanobot-rs is a lightweight AI agent framework written in Rust, designed to run LLM agents with tool-calling capabilities across multiple chat channels. It supports interactive CLI mode and gateway mode for persistent messaging integrations.

**Inspired by the nanobot concept**, this is a standalone Rust implementation optimized for low memory usage, fast startup, and local-first inference.

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
- Uses standard JSONL format for interoperability

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

Sessions use JSONL for simplicity and interoperability. When reading sessions:
- Check first line for `_type: "metadata"`
- Parse timestamps as RFC3339 with ISO 8601 fallback
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

Currently at **Phase 1: Core Foundation**. Implemented:
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

## Design Principles

### Local-First Philosophy

Provider priority order (see main.rs:78-118):
1. **Ollama** (local, no API key needed) — default
2. **OpenAI-compatible** (vLLM, llama.cpp, LocalAI via custom apiBase)
3. Cloud providers (Anthropic, OpenAI, DeepSeek, etc.) — configured fallbacks

The agent loop uses rig's `CompletionModel` trait — provider selection is config-driven, not hardcoded.

### Configuration and Session Formats

Standard formats for interoperability:
- **config.json**: JSON with camelCase field names
- **Session JSONL**: Metadata line followed by message lines
- **Session keys**: Format `"{channel}:{chat_id}"`

### Skills Architecture (Future)

Three-layer design:
1. **Markdown skills** (preferred): `SKILL.md` files with YAML frontmatter — LLM interprets instructions, no compilation needed
2. **Bundled scripts**: Python/Bash scripts in `scripts/` dir — executed via `exec` tool
3. **WASM plugins** (deferred): Native tool plugins with sandboxed execution

Current implementation: None yet (Phase 2). When implementing skills loader:
- Parse YAML frontmatter from SKILL.md
- Check requirements (`bins`, `env`)
- Progressive loading: metadata always in context, full body on-demand

### Channel Architecture (Future)

The `Channel` trait (not yet implemented):
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
- **Standard formats**: Use standard JSON and JSONL formats for config and sessions
- **Write tests**: Every new feature should include tests (see Testing Guidelines below)

## Testing Guidelines

### Test-Driven Development

Write tests as you develop new features. Tests should be written:
- **During development** — not as an afterthought
- **Before marking a feature complete** — untested code is incomplete code
- **For both happy paths and error cases** — test failure scenarios too

### What to Test

**Always write tests for:**
- ✅ New tools (each tool should have unit tests)
- ✅ Session persistence (loading/saving edge cases)
- ✅ Config loading (validation, defaults, malformed input)
- ✅ Tool execution (parameter parsing, error handling)
- ✅ Message bus (routing, serialization)
- ✅ Provider selection logic
- ✅ Channel implementations (when added)

**Optional (but encouraged):**
- Integration tests for agent loop
- End-to-end tests for CLI commands
- Property-based tests for parsers

### Test Organization

```rust
// In the same file as the implementation (for unit tests)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_execution() {
        // Test implementation
    }

    #[tokio::test]
    async fn test_async_tool() {
        // Async test
    }
}
```

For integration tests, use `tests/` directory in each crate.

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p nanobot-core

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Run tests with logging
RUST_LOG=debug cargo test
```

### Test Requirements Before Committing

Before committing a new feature:
1. ✅ All existing tests must pass (`cargo test`)
2. ✅ New tests added for the feature
3. ✅ Tests cover both success and error cases
4. ✅ No warnings from `cargo test`

**Do NOT commit**:
- ❌ Features without tests
- ❌ Code that breaks existing tests
- ❌ Tests that are commented out or ignored without reason

## Documentation Updates

### README.md Maintenance

The README.md should be kept up-to-date as features are implemented. When completing a feature:

1. **Update implementation status**:
   - Move features from 🚧 "In Progress" to ✅ "Implemented"
   - Update the Phase completion status
   - Remove "Stub" or "Partial" notes when fully working

2. **Update feature tables**:
   - Mark tools/channels as ✅ when implemented
   - Update status badges (alpha → beta → stable)
   - Add new features to appropriate sections

3. **Update examples**:
   - Add working examples for new features
   - Remove "planned" or "not yet implemented" warnings
   - Add configuration examples for new channels/providers

4. **Keep it accurate**:
   - Don't claim features work if they're stubbed
   - Be honest about limitations
   - Update performance metrics if measured

### When to Update README

Update README.md:
- ✅ When a major feature becomes functional (memory consolidation, skills loader, new channel)
- ✅ When moving between phases (Phase 1 → Phase 2)
- ✅ When fixing significant bugs that affect documented behavior
- ✅ When adding new configuration options
- ❌ Not for minor refactorings or internal changes
- ❌ Not for every small commit

## Git Commit Workflow

After completing work on nanobot-rs, follow this workflow to commit changes:

### When to Commit

Commit changes when:
- A feature is implemented and builds successfully (`cargo build`)
- Tests pass (`cargo test`)
- The code is in a working, stable state
- A logical unit of work is complete (e.g., "implement skills loader", "add Telegram channel")
- README.md has been updated if the feature is user-facing

### Commit Process

1. **Update documentation** (if user-facing change):
   - Update README.md implementation status
   - Update feature tables and examples
   - Mark features as complete in roadmap

2. **Build and test**:
   ```bash
   cargo build && cargo test
   ```
   Only proceed if both succeed.

3. **Check git status**:
   ```bash
   git status
   ```
   Review what files have changed.

4. **Stage relevant files**:
   ```bash
   git add nanobot-rs/
   ```
   Or stage specific files if mixing work.

5. **Create commit with descriptive message**:
   ```bash
   git commit -m "feat(nanobot-rs): implement memory consolidation

   - Add MEMORY.md/HISTORY.md summarization
   - Integrate with agent loop on threshold
   - Add tests for consolidation logic

   🤖 Generated with Claude Code

   Co-Authored-By: Claude <noreply@anthropic.com>"
   ```

### Commit Message Guidelines

- **Prefix**: Use conventional commits (feat, fix, docs, refactor, test, chore)
- **Scope**: Use `(nanobot-rs)` to distinguish from Python nanobot commits
- **Subject**: Imperative mood, lowercase, no period
- **Body**: Bullet points explaining what changed and why
- **Footer**: Include Claude Code attribution

Examples:
- `feat(nanobot-rs): add Telegram channel with thread support`
- `fix(nanobot-rs): handle malformed session JSONL gracefully`
- `docs(nanobot-rs): update README with Phase 2 completion`
- `refactor(nanobot-rs): extract provider selection to separate module`
- `test(nanobot-rs): add integration tests for agent loop`

**Good commit that includes README update:**
```bash
git commit -m "feat(nanobot-rs): implement memory consolidation

- Add MEMORY.md/HISTORY.md summarization
- Integrate with agent loop on threshold
- Add tests for consolidation logic
- Update README: mark memory consolidation as ✅

🤖 Generated with Claude Code

Co-Authored-By: Claude <noreply@anthropic.com>"
```

### Important Notes

- **Do NOT commit** if build/tests fail
- **Do NOT mix** unrelated changes in one commit
- **Do NOT push** to remote unless explicitly requested
- **Do commit frequently** — small, logical commits are better than large monolithic ones
- **Do update README** when completing user-facing features
- **Keep documentation in sync** — outdated docs are worse than no docs

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nanobot-rs is a Rust rewrite of the Python nanobot framework (`../nanobot/`). It is a lightweight AI agent framework designed to run LLM agents with tool-calling capabilities across multiple chat channels. It supports interactive CLI mode and gateway mode for persistent messaging integrations.

The Python version at `../nanobot/` is the **reference implementation**. The Rust version aims for feature parity, optimized for low memory usage, fast startup, and local-first inference.

## Python Reference Comparison

**Always compare with the Python implementation** when working on nanobot-rs. The Python codebase at `../nanobot/` is the source of truth for behavior and features.

### How to Compare

Before implementing or modifying any feature:
1. **Read the Python version first** — check `../nanobot/` for the equivalent module
2. **Match behavior exactly** — unless there's a documented Rust-specific improvement
3. **Check the rewrite plan** — see `../RUST_REWRITE_PLAN.md` for architecture decisions and phase status
4. **Verify parity** — after changes, confirm the Rust version handles the same edge cases as Python

### Module Mapping (Python → Rust)

| Python Module | Rust Crate/Module | Parity Status |
|---|---|---|
| `nanobot/agent/loop.py` | `nanobot-core/src/agent/loop.rs` | ✅ Complete |
| `nanobot/agent/context.py` | `nanobot-core/src/agent/context.rs` | ✅ Complete |
| `nanobot/agent/memory.py` | `nanobot-core/src/agent/memory.rs` | ✅ Complete |
| `nanobot/agent/skills.py` | `nanobot-core/src/agent/skills.rs` | ✅ Complete |
| `nanobot/agent/subagent.py` | `nanobot-core/src/agent/subagent.rs` | ✅ Complete |
| `nanobot/agent/tools/` | `nanobot-core/src/tools/` | ✅ Complete (all 12 tools) |
| `nanobot/config/` | `nanobot-config/` | ✅ Complete |
| `nanobot/session/` | `nanobot-core/src/session.rs` | ✅ Complete |
| `nanobot/bus/` | `nanobot-core/src/bus.rs` | ✅ Complete |
| `nanobot/cron/` | `nanobot-core/src/cron/` | ✅ Complete |
| `nanobot/heartbeat/` | `nanobot-core/src/heartbeat.rs` | ✅ Complete |
| `nanobot/providers/` | rig-core (external) | ✅ Complete (19 vs 50+ providers) |
| `nanobot/providers/transcription.py` | `nanobot-transcribe/` | ✅ Improved (local-first Parakeet + Groq fallback) |
| `nanobot/channels/telegram.py` | `nanobot-channels/src/telegram.rs` | ✅ Complete |
| `nanobot/channels/discord.py` | — | ❌ Not started |
| `nanobot/channels/slack.py` | — | ❌ Not started |
| `nanobot/channels/whatsapp.py` | — | ❌ Not started |
| `nanobot/channels/qq.py` | — | ❌ Not started |
| `nanobot/channels/dingtalk.py` | — | ❌ Not started |
| `nanobot/channels/feishu.py` | — | ❌ Not started |
| `nanobot/channels/mochat.py` | — | ❌ Not started |
| `nanobot/channels/email.py` | — | ❌ Not started |
| `nanobot/cli/` | `nanobot-cli/` | ✅ Complete |

### Rewrite Plan Status (from `../RUST_REWRITE_PLAN.md`)

| Phase | Description | Status |
|---|---|---|
| Phase 1 | Core Foundation (config, session, bus, LLM, tools, agent loop, CLI) | ✅ Complete |
| Phase 2 | Full Agent Features (memory, skills, web tools, subagents, cron, heartbeat) | ✅ Complete |
| Phase 3 | Channel Architecture + Telegram | ✅ Complete |
| Phase 4 | Polish & Ship (onboarding, error handling, testing, packaging) | ⚠️ Nearly Complete (error audit + integration test remain) |
| Future | Additional channels (Discord, Slack, Email, etc.) | ❌ Not started |
| Future | Semantic memory with vector databases | ❌ Not started |

### What to Check When Making Changes

When modifying any Rust module, **proactively verify against Python**:

- **Agent loop changes** → Read `../nanobot/agent/loop.py` — check iteration limits, error handling, tool call format, response assembly
- **Tool changes** → Read the corresponding `../nanobot/agent/tools/*.py` — check parameter schemas match, error messages are similar, edge cases handled
- **Session changes** → Read `../nanobot/session/manager.py` — verify JSONL format compatibility (files must be interchangeable)
- **Config changes** → Read `../nanobot/config/schema.py` — field names, defaults, and validation must match
- **Channel changes** → Read `../nanobot/channels/telegram.py` — check message formatting, media handling, command routing
- **Bus/routing changes** → Read `../nanobot/bus/` — verify session key format (`{channel}:{chat_id}`)
- **Memory changes** → Read `../nanobot/agent/memory.py` — check consolidation logic, MEMORY.md/HISTORY.md format
- **Skills changes** → Read `../nanobot/agent/skills.py` — check YAML frontmatter parsing, progressive loading behavior

### Known Differences (Intentional)

These divergences from Python are by design:
- **LLM providers**: Rust uses `rig-core` (19 providers) instead of `litellm` (50+). Covers all major providers. Note: rig-core does not support claude-cli (subprocess-based) - this would require a custom provider implementation. Future: Add claude-cli provider support for local Claude Code integration.
- **Voice transcription**: Rust uses local-first Parakeet TDT with Groq as fallback. Python uses Groq only.
- **Provider priority**: Rust defaults to Ollama (local-first). Python uses litellm's routing.
- **Binary deployment**: Single static binary vs Python venv.

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
- **web**: `web_search` (Brave API), `web_fetch` (readability extraction)
- **message**: `message` (send to chat channels)
- **spawn**: `spawn` (background subagent tasks)
- **cron**: `cron_add`, `cron_remove`, `cron_list` (scheduled jobs)

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

### Gateway Mode (nanobot-cli/src/main.rs — `run_gateway()`)

The `serve` command starts the full gateway:
1. Initializes `ChannelManager` and registers enabled channels
2. Starts Telegram long polling (with Parakeet transcription)
3. Starts cron service and heartbeat (if enabled)
4. Routes inbound messages through `MessageBus` to `AgentLoop`
5. Handles `/new`, `/help`, `/start` slash commands
6. Dispatches outbound messages to appropriate channels
7. Graceful shutdown on Ctrl-C

## Implementation Status

**Phases 1-3 complete. Phase 4 in progress.** See `../RUST_REWRITE_PLAN.md` for full plan.

Core agent (all tools, memory, skills, cron, heartbeat, subagents) and Telegram channel are fully implemented. The main remaining gaps are additional channel integrations and Phase 4 polish items.

Implemented:
- ✅ Config loading (JSON with camelCase, serde)
- ✅ Session persistence (JSONL, Python-compatible)
- ✅ Message bus (tokio mpsc/broadcast)
- ✅ LLM integration via rig-core (Ollama default, OpenAI-compatible support)
- ✅ Tool system (registry + all 12 tools: filesystem, shell, web, message, spawn, cron)
- ✅ Agent loop (LLM ↔ tool iteration)
- ✅ Context builder (system prompt + message history)
- ✅ CLI (interactive mode with rustyline)
- ✅ Memory consolidation (MEMORY.md/HISTORY.md summarization)
- ✅ Skills loader (YAML frontmatter, progressive loading)
- ✅ Web tools (Brave search, readability extraction)
- ✅ Subagent system (background task spawning)
- ✅ Cron service
- ✅ Heartbeat
- ✅ Telegram channel (teloxide, voice transcription, media handling)
- ✅ Voice transcription (local Parakeet TDT + Groq fallback)
- ✅ Gateway mode (`serve` command with Telegram)
- ✅ Onboarding wizard (interactive + `--non-interactive`)
- ✅ Status/interrupt commands (flag-file interrupt mechanism)
- ✅ Binary packaging (release script + checksums)
- ✅ Cross-compilation (CI builds Linux, macOS, Windows)

Phase 4 remaining:
- ⚠️ Error handling audit — some `unwrap()` calls in production paths need review
- ⚠️ Telegram integration test — only unit tests exist, no end-to-end test

Not yet implemented (future):
- ❌ Additional channels (Discord, Slack, WhatsApp, QQ, DingTalk, Feishu, MoChat, Email)
- ❌ Semantic memory with vector databases
- ❌ Claude CLI provider integration — requires custom subprocess-based provider
- ❌ Security improvements from LocalGPT (sandbox, content sanitization, signed policies) — see `LOCALGPT_COMPARISON.md`
- ❌ Monty code execution mode — see `MONTY_CODE_MODE_PLAN.md`

## Future Improvements

See these planning documents for detailed future improvements:

### LOCALGPT_COMPARISON.md
Comprehensive comparison with LocalGPT (~23K LOC) identifying security and memory improvements:
- **P0 Priority**: Kernel sandbox (Landlock + seccomp), content sanitization pipeline
- **P1 Priority**: Memory search with FTS5 + embeddings, signed security policies (LocalGPT.md)
- **P2 Priority**: TurnGate concurrency control, security audit log
- **P3 Priority**: Claude CLI provider, additional provider improvements

Key features to adopt:
- Kernel-enforced sandbox for shell execution (Landlock on Linux, multi-platform)
- SQLite FTS5 + sqlite-vec for semantic memory search with local embeddings (fastembed)
- Content sanitization to prevent prompt injection attacks
- Signed workspace security policies with HMAC verification
- Hash-chained security audit log

### MONTY_CODE_MODE_PLAN.md
Plan to integrate Monty's Python execution engine as alternative to tool calling:
- **Execution modes**: Traditional tool calling vs code mode vs hybrid
- **Benefits**: Reduce LLM round trips by 50-90%, enable natural control flow (loops, conditionals)
- **Implementation**: 5-phase plan (~2-3 weeks), backward compatible
- **Security**: Monty sandbox with resource limits, type stubs for tools
- **Use case**: Complex multi-step tasks that are awkward as sequential tool calls

Example: "Read all .rs files and count total lines"
- Tool mode: 10+ LLM round trips (list_dir → read_file × N → count)
- Code mode: 1 LLM call generates Python loop, executes locally

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

### Skills Architecture (nanobot-core/src/agent/skills.rs)

Three-layer design (layers 1 and 2 implemented):
1. **Markdown skills** (implemented): `SKILL.md` files with YAML frontmatter — LLM interprets instructions. Skills loader parses frontmatter, checks requirements (`bins`, `env` via `which` crate and `std::env`), progressive loading (metadata always in context, full body on-demand via `read_file`). Always-loaded skills injected into system prompt.
2. **Bundled scripts** (implemented): Python/Bash scripts in `scripts/` dir — executed via `exec` tool
3. **WASM plugins** (deferred): Native tool plugins with sandboxed execution

### Channel Architecture (nanobot-channels/)

The `Channel` trait (`base.rs`) and `ChannelManager` (`manager.rs`) are fully implemented:
```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, bus: mpsc::Sender<InboundMessage>) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
    fn is_allowed(&self, sender_id: &str) -> bool;
}
```

Currently implemented: Telegram. Future: Discord → Slack → Email.

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

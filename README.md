<div align="center">
  <h1>patina-bot: Lightweight AI Agent Framework</h1>
  <p>
    <img src="https://img.shields.io/badge/rust-1.80+-orange" alt="Rust">
    <img src="https://img.shields.io/badge/status-beta-blue" alt="Status">
    <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
  </p>
  <p><strong>Ultra-lightweight AI agent framework in Rust — single static binary, no runtime dependencies</strong></p>
</div>

---

## Why Rust?

**patina-bot** is a Rust-based AI agent framework designed for:

- **Minimal footprint**: ~30MB memory (vs Python's ~200-400MB)
- **Fast startup**: ~50-100ms (vs Python's ~2-3s)
- **Single binary**: No Python runtime, pip, or venv needed
- **Type safety**: Catch bugs at compile time
- **Local-first**: Supports Ollama and local inference

---

## Install

### From Source

```bash
git clone <repo-url>
cd patina-bot
cargo build --release
```

Binary will be at `target/release/patina`.

### Using Cargo Install

```bash
cargo install --path crates/patina-cli
```

---

## Quick Start

### 1. Initialize

```bash
patina onboard
```

Creates config and workspace files (default: `~/.patina/config.json`).

### 2. Configure

Config file priority:
1. `--config` CLI argument
2. `./config.json`
3. `~/.patina/config.json`

**Ollama (local):**

```json
{
  "agents": {
    "defaults": {
      "provider": "ollama",
      "model": "llama3.2"
    }
  },
  "providers": {
    "ollama": {
      "apiBase": "http://localhost:11434"
    }
  }
}
```

**Anthropic (Claude):**

```json
{
  "agents": {
    "defaults": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-5-20250514"
    }
  },
  "providers": {
    "anthropic": {
      "apiKey": "sk-ant-..."
    }
  }
}
```

**OpenAI:**

```json
{
  "agents": {
    "defaults": {
      "provider": "openai",
      "model": "gpt-4"
    }
  },
  "providers": {
    "openai": {
      "apiKey": "sk-..."
    }
  }
}
```

### 3. Chat

```bash
# Single message
patina agent -m "What is 2+2?"

# Interactive mode
patina agent

# Start gateway (web UI + channels)
patina serve
```

---

## Architecture

Cargo workspace with 5 crates:

```
patina-bot/
├── crates/
│   ├── patina-core/        # Agent loop, tools, sessions, bus, usage tracking
│   ├── patina-config/      # Configuration schema and loading
│   ├── patina-channels/    # Channel adapters (Web, Telegram, Slack)
│   ├── patina-cli/         # CLI binary (agent + serve commands)
│   └── patina-transcribe/  # Voice transcription (Parakeet TDT + Groq)
└── web/                    # Web UI (Preact + TypeScript, built with Vite/Bun)
```

### Message Flow

```
User Input (CLI / Web UI / Telegram / Slack)
    |
[ Channel Adapter ]
    |
[ Message Bus (tokio mpsc/broadcast) ]
    |
[ Agent Loop ] <-> [ LLM Provider (rig-core) ]
    |                      |
[ Tool Registry ]    [ Streaming text_delta -> WebSocket ]
    |
[ Session Manager (JSONL) ]
    |
[ Response ] -> User
```

---

## Features

### Core

| Feature | Status | Notes |
|---------|--------|-------|
| Config Loading | Done | JSON with camelCase |
| Session Persistence | Done | JSONL format |
| Message Bus | Done | Tokio mpsc/broadcast |
| LLM Integration | Done | Via rig-core 0.30 |
| Agent Loop | Done | LLM + tool iteration with max iterations |
| Context Builder | Done | System prompt + message history |
| Memory Consolidation | Done | MEMORY.md/HISTORY.md summarization |
| Memory Index | Done | FTS5 search with SHA256 change detection |
| Skills Loader | Done | YAML frontmatter, progressive loading |
| Subagent System | Done | Background task spawning |
| Cron Service | Done | Scheduled jobs with CLI management |
| Heartbeat | Done | Background heartbeat loop |
| Prompt Caching | Done | Anthropic cache_control support |
| Usage Tracking | Done | SQLite with cost estimates and web dashboard |

### Channels

| Channel | Status | Notes |
|---------|--------|-------|
| CLI | Done | Interactive REPL (rustyline) |
| Web UI | Done | Multi-chat, personas, streaming, usage dashboard |
| Telegram | Done | Long polling, voice/photo/document, thread support |
| Slack | Done | Socket Mode, thread support, allowlist |
| Discord | Planned | |
| Email | Planned | |

### Web UI Features

- Multi-chat sidebar with session management
- Hash-based routing (`/#/chats`, `/#/tasks`, `/#/usage`) — page survives refresh
- Agent personas (per-chat, UI-managed, model tiers)
- LLM response streaming (real-time text display)
- Cancel/stop generation (button + ESC key)
- Usage dashboard with cost estimates
- Task kanban board with drag-and-drop
- Task detail overlay with chat thread
- Multi-client sync (WebSocket)
- Chat ID display for usage cross-reference
- Scroll-to-bottom button for long threads
- Markdown rendering with syntax highlighting

### Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents |
| `write_file` | Write/overwrite file |
| `edit_file` | Replace text in file |
| `list_dir` | List directory contents |
| `exec_command` | Execute shell command (configurable timeout) |
| `web_search` | Brave Search API |
| `web_fetch` | Fetch URL content (readability extraction) |
| `message` | Send to channel/user |
| `spawn` | Launch background subagent |
| `cron_add/remove/list` | Manage scheduled jobs |

### Providers

Supported: `anthropic`, `openai`, `ollama`, `openrouter`, `deepseek`, `groq`, `gemini`.

Set `agents.defaults.provider` and `agents.defaults.model` in config. API keys are resolved from config (`providers.<name>.apiKey`) then environment variables (e.g. `ANTHROPIC_API_KEY`).

---

## Configuration

<details>
<summary>Full config.json schema</summary>

```json
{
  "agents": {
    "defaults": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-5-20250514",
      "workspace": "~/.patina/workspace",
      "maxTokens": 8192,
      "temperature": 0.7,
      "maxToolIterations": 20,
      "memoryWindow": 30
    }
  },
  "channels": {
    "telegram": {
      "enabled": false,
      "token": "",
      "allowFrom": []
    },
    "slack": {
      "enabled": false,
      "botToken": "",
      "appToken": "",
      "allowFrom": []
    }
  },
  "providers": {
    "ollama": { "apiBase": "http://localhost:11434" },
    "anthropic": { "apiKey": "" },
    "openai": { "apiKey": "" },
    "openrouter": { "apiKey": "" },
    "deepseek": { "apiKey": "" },
    "groq": { "apiKey": "" },
    "gemini": { "apiKey": "" }
  },
  "tools": {
    "restrictToWorkspace": false,
    "exec": { "timeoutSecs": 60 },
    "web": { "search": { "apiKey": "", "maxResults": 5 } }
  },
  "gateway": {
    "host": "0.0.0.0",
    "port": 18790
  },
  "heartbeat": {
    "enabled": false,
    "intervalSecs": 1800
  },
  "transcription": {
    "mode": "auto",
    "modelPath": "~/.patina/models/parakeet-tdt",
    "executionProvider": "cpu",
    "autoDownload": true
  }
}
```

</details>

---

## CLI Reference

```bash
# Initialize config and workspace
patina onboard
patina onboard --non-interactive

# Interactive chat
patina agent

# Single message
patina agent -m "Hello, world!"

# Custom session
patina agent -s "my-session"

# Start gateway (web UI + channels)
patina serve

# Interrupt a running session
patina interrupt --session "cli:interactive"

# Show status
patina status

# Cron management
patina cron list
patina cron add --name morning --message "Daily check-in" --every 3600
patina cron run <job_id>
```

### Build Commands

```bash
cargo build              # Build all crates
cargo build --release    # Build optimized
cargo test               # Run tests
RUST_LOG=debug cargo run --bin patina -- agent  # Run with logging

# Build web UI (required after changing web/src/*)
cd web && bun install && bun run build

# Dev mode (Vite dev server with HMR, proxies API/WS to backend)
cd web && bun run dev
```

---

## Channels

### Web UI

Built with Preact + TypeScript + Vite (Bun as runtime). The build produces a single `index.html` with all JS/CSS inlined, embedded into the Rust binary via `include_str!()`. `web/dist/index.html` is committed to git so `cargo build` works without Bun installed.

Features: multi-chat, personas, streaming responses, cancel generation, usage dashboard, task kanban, hash routing (refresh-safe), multi-client sync.

Start with `patina serve` and open `http://localhost:18790`.

### Telegram

Long polling (no webhook needed), markdown-to-HTML conversion, thread/topic support, voice/photo/document handling, typing indicators, proxy support.

```json
{
  "channels": {
    "telegram": {
      "enabled": true,
      "token": "YOUR_BOT_TOKEN",
      "allowFrom": ["USER_ID"]
    }
  }
}
```

### Slack

Socket Mode (no public URL needed), thread support, markdown conversion, allowlist filtering.

```json
{
  "channels": {
    "slack": {
      "enabled": true,
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "allowFrom": ["USER_ID"]
    }
  }
}
```

---

## Skills

Markdown files with YAML frontmatter that extend agent capabilities.

- **Layer 1: Markdown skills** — `SKILL.md` files with YAML metadata, LLM interprets instructions
- **Layer 2: Bundled scripts** — Python/Bash scripts in `scripts/` directory, executed via `exec_command`
- **Layer 3: WASM plugins** — Planned for future

```
~/.patina/skills/my-skill/
├── SKILL.md          # Instructions + YAML metadata
├── scripts/          # Optional helper scripts
└── references/       # Optional docs (loaded on-demand)
```

---

## Sessions

Stored as JSONL files at `~/.patina/sessions/{session_key}.jsonl`.

Session keys use format `{channel}:{chat_id}` (e.g., `web:abc-123`, `telegram:-100123`).

---

## Performance

| Metric | Python | Rust | Improvement |
|--------|--------|------|-------------|
| Binary Size | ~50MB+ (venv) | ~10-15MB | 3-5x smaller |
| Memory (idle) | ~80-120MB | ~5-15MB | 10-50x less |
| Startup Time | ~2-3s | ~50-100ms | 20-30x faster |
| Runtime Deps | Python 3.11+, pip | None | Static binary |
| Concurrency | GIL-limited | True parallelism | Native async |

---

## Contributing

PRs welcome.

- Delete unused code completely (no backwards-compat hacks)
- Modern Rust idioms
- Write tests for new features (`cargo test`)
- Format with `cargo fmt`

---

## License

MIT

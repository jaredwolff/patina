<div align="center">
  <h1>patina-bot: Lightweight AI Agent Framework</h1>
  <p>
    <img src="https://img.shields.io/badge/rust-1.75+-orange" alt="Rust">
    <img src="https://img.shields.io/badge/status-alpha-yellow" alt="Status">
    <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
  </p>
  <p><strong>Ultra-lightweight AI agent framework in Rust — 10-50x lower memory, single static binary, no runtime dependencies</strong></p>
</div>

---

## 🚀 Why Rust?

**patina-bot** is a Rust-based AI agent framework inspired by the nanobot concept, designed for:

- **🪶 Minimal footprint**: ~30MB memory (vs Python's ~200-400MB)
- **⚡ Fast startup**: ~50-100ms (vs Python's ~2-3s)
- **📦 Single binary**: No Python runtime, pip, or venv needed
- **🔒 Type safety**: Catch bugs at compile time
- **🌍 Local-first**: Prioritizes Ollama and local inference by default

**Current Status**: Core CLI + gateway are working, including Telegram polling, cron/heartbeat services, and markdown skill loading.

---

## 📦 Install

### From Source (Recommended)

```bash
git clone https://github.com/HKUDS/nanobot.git
cd nanobot/patina-bot
cargo build --release
```

Binary will be at `target/release/patina`.

### Using Cargo Install

```bash
cargo install --path patina-bot/crates/patina-cli
```

---

## 🚀 Quick Start

### 1. Initialize

```bash
patina onboard
```

This creates config and workspace files (default path: `~/.patina/config.json`).
If `./config.json` exists in your current directory, that local file is used instead.

### 2. Configure

Edit the config file selected by priority:
1. `--config` CLI argument
2. `./config.json`
3. `~/.patina/config.json`

**For local-first (Ollama, recommended):**

```json
{
  "providers": {
    "ollama": {
      "apiBase": "http://localhost:11434"
    }
  },
  "agents": {
    "defaults": {
      "model": "llama3.2",
      "maxTokens": 4096,
      "temperature": 0.7
    }
  }
}
```

**For cloud providers (OpenAI-compatible):**

```json
{
  "providers": {
    "openai": {
      "apiKey": "sk-...",
      "apiBase": "https://api.openai.com/v1"
    }
  },
  "agents": {
    "defaults": {
      "model": "gpt-4"
    }
  }
}
```

**For Anthropic (Claude):**

```json
{
  "providers": {
    "anthropic": {
      "apiKey": "sk-ant-..."
    }
  },
  "agents": {
    "defaults": {
      "model": "claude-sonnet-4-5-20250514"
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
```

---

## 🏗️ Architecture

patina-bot is a Cargo workspace with 4 crates:

```
patina-bot/
├── crates/
│   ├── patina-core/      # Agent loop, tools, session management
│   ├── patina-config/    # Configuration schema and loading
│   ├── patina-channels/  # Channel adapters (Telegram, etc.)
│   └── patina-cli/       # CLI binary
├── Cargo.toml             # Workspace manifest
└── README.md
```

### Message Flow

```
User Input
    ↓
[ CLI / Channel Listener ]
    ↓
[ Message Bus (tokio mpsc/broadcast) ]
    ↓
[ Agent Loop ] ←→ [ LLM Provider (rig-core) ]
    ↓                      ↓
[ Tool Registry ]    [ Tool Execution ]
    ↓                      ↓
[ Session Manager (JSONL) ]
    ↓
[ Response ] → User
```

---

## ✨ Features

### ✅ Implemented (Phase 1 Complete)

| Feature | Status | Notes |
|---------|--------|-------|
| **Config Loading** | ✅ | JSON with camelCase, Python-compatible |
| **Session Persistence** | ✅ | JSONL format, interchangeable with Python |
| **Message Bus** | ✅ | Tokio channels (mpsc/broadcast) |
| **LLM Integration** | ✅ | Via rig-core 0.30 |
| **Provider Support** | ✅ | Ollama (default), OpenAI, Anthropic, OpenRouter, DeepSeek, Groq, Gemini |
| **Tool System** | ✅ | Registry + dynamic dispatch |
| **File Tools** | ✅ | read_file, write_file, edit_file, list_dir |
| **Shell Tool** | ✅ | exec_command with timeout |
| **Web Tools** | ✅ | web_search (Brave), web_fetch |
| **Agent Loop** | ✅ | LLM ↔ tool iteration with max iterations |
| **Context Builder** | ✅ | System prompt + message history |
| **CLI** | ✅ | Interactive mode (rustyline REPL) |
| **Workspace** | ✅ | AGENTS.md, SOUL.md, USER.md support |

### ✅ Delivered (Phase 2-3)

| Feature | Status | Priority | Notes |
|---------|--------|----------|-------|
| **Memory Consolidation** | ✅ Basic | HIGH | Session-aware consolidation path is wired in |
| **Skills Loader** | ✅ | HIGH | YAML frontmatter, workspace override, builtin fallback |
| **Subagent System** | ✅ | MEDIUM | Background task spawning via `spawn` tool |
| **Cron Service** | ✅ | MEDIUM | Integrated service + CLI management commands |
| **Heartbeat** | ✅ | LOW | Background heartbeat loop in gateway |
| **Telegram Channel** | ✅ | HIGH | Long polling, media handling, markdown conversion |
| **Gateway Mode** | ✅ | HIGH | Multi-channel routing via `patina serve` |
| **Message Tool** | ✅ | MEDIUM | Sends notifications via active channel bus |

### 📋 Planned (Phase 4+)

| Feature | Priority | Notes |
|---------|----------|-------|
| **Discord Channel** | MEDIUM | WebSocket integration via twilight/serenity |
| **Slack Channel** | MEDIUM | Socket Mode via slack-morphism |
| **Email Channel** | LOW | IMAP/SMTP polling |
| **WhatsApp Channel** | LOW | Bridge complexity |
| **Cron CLI Commands** | LOW | list/add/remove/enable/run |
| **Vector Memory** | FUTURE | Semantic search with embedded DB |
| **WASM Plugin System** | FUTURE | Hot-loadable native tools |

---

## 🔧 Configuration

Config file location (checked in order):
1. `--config` CLI argument
2. `./config.json`
3. `~/.patina/config.json`

### Full Config Schema

<details>
<summary>Expand to see full config.json structure</summary>

```json
{
  "agents": {
    "defaults": {
      "workspace": "~/.patina/workspace",
      "model": "gpt-oss-20b-GGUF",
      "maxTokens": 8192,
      "temperature": 0.7,
      "maxToolIterations": 20,
      "memoryWindow": 50,
    }
  },
  "channels": {
    "telegram": {
      "enabled": false,
      "token": "",
      "allowFrom": [],
      "proxy": null
    }
  },
  "providers": {
    "ollama": {
      "apiBase": "http://localhost:11434"
    },
    "anthropic": {
      "apiKey": ""
    },
    "openai": {
      "apiKey": ""
    },
    "openrouter": {
      "apiKey": ""
    },
    "deepseek": {
      "apiKey": ""
    },
    "groq": {
      "apiKey": ""
    },
    "gemini": {
      "apiKey": ""
    }
  },
  "tools": {
    "restrictToWorkspace": false,
    "exec": {
      "timeoutSecs": 60
    },
    "web": {
      "search": {
        "apiKey": "",
        "maxResults": 5
      }
    }
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

### Provider Priority

Provider selection is currently:

1. **OpenAI-compatible** when `providers.openai.apiBase` is set
2. **OpenRouter key prefix** (`sk-or-*`) when present
3. **Auto-detect by model name**:
   - `claude-*` → Anthropic
   - `gpt-*`, `o1-*`, `o3-*`, `o4-*` → OpenAI
   - `deepseek-*` → DeepSeek
   - `gemini-*` → Gemini
   - Models with `/` → OpenRouter
4. **OpenRouter fallback** when its API key is configured
5. **Ollama fallback** (local default)

### Security

| Option | Default | Description |
|--------|---------|-------------|
| `tools.restrictToWorkspace` | `false` | When `true`, restricts file/shell tools to workspace directory |
| `channels.*.allowFrom` | `[]` | Whitelist of user IDs (empty = allow all) |

---

## 🖥️ CLI Reference

```bash
# Initialize config and workspace
patina onboard

# Initialize without prompts
patina onboard --non-interactive

# Interactive chat (REPL)
patina agent

# Single message
patina agent -m "Hello, world!"

# Custom session ID
patina agent -s "my-session"

# Custom config path
patina -c /path/to/config.json agent

# Interrupt a running session
patina interrupt --session "cli:interactive"

# Start gateway (receive messages from channels)
patina serve

# Show status
patina status

# Cron management
patina cron list
patina cron add --name morning --message "Daily check-in" --every 3600
patina cron run <job_id>

# Channel status
patina channels status
```

### Build Commands

```bash
# Build all crates
cargo build

# Build release (optimized)
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run --bin patina -- agent
```

---

## 📡 Channels

### Telegram

**Status**: ✅ Implemented

**Current Features**:
- Long polling (no webhook needed)
- Markdown-to-HTML conversion with table support
- Thread/topic support (separate contexts per topic)
- Voice/photo/document handling
- Typing indicators
- Proxy support
- `/new`, `/help` slash commands

**Configuration**:

```json
{
  "channels": {
    "telegram": {
      "enabled": true,
      "token": "YOUR_BOT_TOKEN",
      "allowFrom": ["USER_ID"],
      "proxy": "socks5://127.0.0.1:1080"
    }
  }
}
```

**Setup**:
1. Create bot via @BotFather
2. Copy token to config
3. Run `patina serve`

### Other Channels

| Channel | Status | Crate | Priority |
|---------|--------|-------|----------|
| Discord | 📋 Planned | twilight/serenity | Medium |
| Slack | 📋 Planned | slack-morphism | Medium |
| WhatsApp | 📋 Planned | TBD | Low |
| Email | 📋 Planned | async-imap + lettre | Low |

All channels implement the `Channel` trait for drop-in compatibility.

---

## 🛠️ Tools

Tools are registered at runtime and exposed to the LLM for execution.

### Built-in Tools

| Tool | Status | Description |
|------|--------|-------------|
| `read_file` | ✅ | Read file contents |
| `write_file` | ✅ | Write/overwrite file |
| `edit_file` | ✅ | Replace text in file |
| `list_dir` | ✅ | List directory contents |
| `exec_command` | ✅ | Execute shell command |
| `web_search` | ✅ | Brave Search API |
| `web_fetch` | ✅ | Fetch URL content |
| `message` | ✅ | Send to channel/user |
| `spawn` | ✅ | Launch background subagent |
| `cron` | ✅ | Schedule/list/remove/trigger jobs |

### Tool Trait

All tools implement:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<String>;
}
```

### Workspace Restriction

When `tools.restrictToWorkspace: true`:
- All file/shell operations confined to workspace directory
- Prevents path traversal attacks
- Recommended for production deployments

---

## 📚 Skills

**Status**: ✅ Implemented (markdown skills)

Skills are markdown files with YAML frontmatter that extend agent capabilities.

### Current Architecture

**Layer 1: Markdown Skills** (preferred)
- `SKILL.md` with YAML frontmatter
- LLM interprets instructions
- No compilation needed
- **Python-compatible** — existing skills work unchanged

**Layer 2: Bundled Scripts**
- Python/Bash scripts in `scripts/` directory
- Executed via `exec_command` tool
- Language-agnostic subprocess execution

**Layer 3: WASM Plugins** (future)
- Native tool plugins
- Sandboxed execution
- Hot-loadable at runtime
- Language-agnostic (Rust, Go, C, AssemblyScript)

### Example Skill Structure

```
~/.patina/skills/my-skill/
├── SKILL.md          # Instructions + YAML metadata
├── scripts/          # Optional helper scripts
│   └── helper.py
├── references/       # Optional docs (loaded on-demand)
└── assets/           # Optional files
```

---

## 💾 Sessions

Sessions are stored as JSONL files at `~/.patina/sessions/{session_key}.jsonl`.

### Format

**Line 1** (metadata):
```json
{"_type":"metadata","created_at":"2025-02-14T10:00:00Z","updated_at":"2025-02-14T10:30:00Z","metadata":{},"last_consolidated":0}
```

**Subsequent lines** (messages):
```json
{"role":"user","content":"Hello","timestamp":"2025-02-14T10:01:00Z"}
{"role":"assistant","content":"Hi!","timestamp":"2025-02-14T10:01:05Z","tools_used":["read_file"]}
```

### Session Format

Sessions use a simple JSONL format:
- Line 1: Metadata (created_at, updated_at, etc.)
- Subsequent lines: Messages with role, content, timestamp
- Session keys use format: `{channel}:{chat_id}`

---

## 📈 Performance

Expected improvements over Python:

| Metric | Python | Rust | Improvement |
|--------|--------|------|-------------|
| **Binary Size** | ~50MB+ (venv) | ~10-15MB | 3-5x smaller |
| **Memory (idle)** | ~80-120MB | ~5-15MB | 10-50x less |
| **Startup Time** | ~2-3s | ~50-100ms | 20-30x faster |
| **Runtime Dep** | Python 3.11+, pip | None | Static binary |
| **Concurrency** | GIL-limited | True parallelism | Native async |

---

## 🗺️ Roadmap

### Phase 1: Core Foundation ✅ (Complete)
- Config, session, message bus
- LLM integration (rig-core)
- Tool system + basic tools
- Agent loop, context builder
- CLI (interactive mode)

### Phase 2: Full Agent Features ✅ (Delivered)
- Memory consolidation
- Skills loader
- Web tools (search, fetch)
- Subagent system
- Cron service + CLI commands
- Heartbeat

### Phase 3: Telegram + Gateway ✅ (Delivered)
- Channel trait + manager
- Telegram integration (teloxide)
- Gateway mode
- Slash commands

### Phase 4: Polish & Ship 📋 (Planned)
- Onboarding wizard
- Error handling audit
- Testing (unit + integration)
- Binary packaging
- Cross-compilation

### Future: Additional Channels 🔮
- Discord, Slack, Email
- Each channel is self-contained
- No core agent changes needed

---

## 🤝 Contributing

PRs welcome! The codebase is designed to be clean and approachable.

### Development Guidelines

- **No backwards-compatibility hacks** — Delete unused code completely
- **Python compatibility where specified** — config.json and session JSONL must remain compatible
- **Modern Rust idioms** — Use current best practices
- **Clean refactoring** — Remove, don't comment out

### Useful Commands

```bash
# Format code
cargo fmt

# Lint
cargo clippy

# Check without building
cargo check

# Watch mode (install cargo-watch)
cargo watch -x check -x test -x run

# Documentation
cargo doc --open
```

Release and packaging notes live in `RELEASE.md`.

---

## 📜 License

MIT License — same as Python nanobot.

---

## 🔗 Links

- **Inspiration**: Inspired by the lightweight AI agent concept
- **GitHub Issues**: [Report issues and feature requests](https://github.com/HKUDS/nanobot/issues)

---

<p align="center">
  <sub>Built with Rust 🦀 for performance, safety, and developer experience</sub>
</p>

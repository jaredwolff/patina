---
name: memory
description: Two-layer memory system with search-based recall.
always: true
---

# Memory

## Structure

- `memory/MEMORY.md` — Long-term facts (preferences, project context, relationships). Searchable via `memory_search`.
- `memory/HISTORY.md` — Append-only event log. Searchable via `memory_search`.

## Recall

Use the `memory_search` tool to find past context, user preferences, and event history before answering questions about previous conversations.

## When to Update MEMORY.md

Write important facts immediately using `edit_file` or `write_file`:
- User preferences ("I prefer dark mode")
- Project context ("The API uses OAuth2")
- Relationships ("Alice is the project lead")

## Auto-consolidation

Old conversations are automatically summarized and appended to HISTORY.md when the session grows large. Long-term facts are extracted to MEMORY.md. You don't need to manage this.

# AGENTS.md - HalluScribe

## Stack
Tauri v2 + Svelte 5 + Rust. Windows/Linux/macOS.

---

## What This Project Is
HalluScribe is a standalone app that reads AI coding session JSONL files (Claude Code, Codex, Continue, Forge),
preprocesses them with a local Gemma 4 model (via llama.cpp or Ollama), and saves human-readable
session logs to `~/.halluscribe/`. It runs a nightly sweep - not real-time hooks.

The full plan is in `HALLUSCRIBE_PLAN.md`. That is the single source of truth for all design decisions.

---

## Hard Stops - Never Do These
- No hardcoded secrets, API keys, or OS paths
- No destructive commands (`rm -rf`, `DROP TABLE`, `git reset --hard`) without explicit user confirmation
- No duplicate implementations - grep before creating anything new
- No `unsafe` Rust without explicit approval
- No direct frontend-to-Rust calls - all Rust logic exposed via `invoke_handler` commands or emitted events
- No coupling back to HalluMeter code - HalluScribe is fully standalone

---

## File Headers
Every created or modified source file must begin with the repo's required file header comment.
Applies to `.rs`, `.ts`, `.svelte`, `.css`, `.js`. Does NOT apply to `.json`, `.md`, `.toml`, or generated files.

---

## Investigation Hard Limit
- Max 5 investigation steps before stopping
- Stop and ask the user at step 3 if direction is unclear
- Never silently pivot to a different approach mid-investigation

---

## File Size Limit
- 350 LOC soft target per source file; 500 LOC hard ceiling
- Between 350 and 500 is allowed when splitting would scatter one cohesive unit; above 500, split into modules
- Tests go in sibling `*_tests.rs` files and do not count toward the limit
- Does NOT apply to `.md`, `.json`, `.toml`, config files, or generated files

---

## Single Point of Truth
- `HALLUSCRIBE_PLAN.md` -> single source for all design and architecture decisions
- `src-tauri/assets/curves.json` -> single source for model degradation data (copied from HalluMeter, do not diverge)
- `tauri.conf.json` -> single source for app config
- `src-tauri/src/core.rs` -> all Rust pure logic (parsing, interpolation, fill_pct)
- Grep before adding any new constant, type, or function

---

## Architecture Rules
- HalluScribe reads JSONL files independently - it does NOT import or call any HalluMeter code
- Archive lives at `~/.halluscribe/` - never hardcode this path, always use Tauri path API
- Sweep only - no real-time triggers, no compact_boundary event hooks, no SummaryJob queue
- Gemma inference: llama.cpp is primary backend (GBNF grammar for structured JSON); Ollama is secondary
- Process jobs sequentially - never concurrent Gemma inference
- Unload model immediately after each response (llama.cpp: kill subprocess; Ollama: keep_alive=0)

---

## Reference Files
`reference/` contains read-only copies of HalluMeter source files for reference during implementation.
Do NOT modify them. Do NOT import them. They exist so you can see how HalluMeter solved the same problems.

| File | What to reference it for |
|---|---|
| `reference/claude.rs` | fill_pct computation and JSONL path discovery for Claude Code |
| `reference/codex.rs` | Same for Codex |
| `reference/continue_reader.rs` | Same for Continue |
| `reference/settings.rs` | Settings struct pattern and serde defaults approach |

## Sibling Projects on Disk (Read-Only Resources)
Do NOT import or couple to either project. Translate relevant logic into HalluScribe's own modules.
Full file-level index in `HALLUSCRIBE_PLAN.md` -> "Available Implementation Resources".

| Project | Path | Primary use |
|---|---|---|
| HalluMeter | `<path-to-hallumeter-on-your-machine>` | Tauri v2 scaffold: Cargo.toml dep versions, package.json versions, vite.config.ts, capabilities format, lib.rs builder pattern, settings.rs pattern |
| AdvanLLM | `<path-to-advanllm-on-your-machine>` | llama-server subprocess lifecycle (`llamaserver/manager.py`), Gemma 4 thinking token handling + SSE client (`gui/services/chat_service_llamaserver.py`), Ollama `/api/generate` payload (`ollama/client/ollama_engine_generation.py`) |

---

## Code Quality Gates (before every commit AND push)
```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npx vitest run
```
All must pass. No exceptions.

---

## CI vs Local - Why Remote Can Fail When Local Passes

CI (`.github/workflows/ci.yml`) runs on **ubuntu-latest, windows-latest, macos-latest** with **Rust 1.95.0** pinned.
Local development is Windows-only. These gaps cause most push failures:

1. **`cargo fmt --check`** - CI uses `--check` (fails if any file is unformatted). Local `cargo fmt` silently reformats in place. Always run `cargo fmt` before pushing, not just before committing.
2. **Case-sensitive paths (Linux)** - Linux filesystems are case-sensitive; Windows is not. A mismatched module name or `use` path that compiles locally will fail on `ubuntu-latest`.
3. **Cross-platform `#[cfg]` branches** - code inside `#[cfg(target_os = "windows")]` is never compiled locally. The `else` branch is compiled for the first time on Linux/macOS runners and may have unseen errors.
4. **Clippy warnings are hard errors** - CI passes `-D warnings`. A warning you see and ignore locally is a build failure on the runner.
5. **Rust version** - CI pins 1.95.0. If your local toolchain is newer, a stabilised API you use may not exist in 1.95.0. Verify with `rustc --version`.

---

## Rust Rules
- All paths via Tauri path API (`app.path()`) - never hardcode OS paths
- Platform differences via `#[cfg(target_os = "...")]` - no platform assumptions
- Tauri commands must be registered in `invoke_handler` before use
- `which` crate for locating external binaries (llama-server, ollama) - never assume PATH

## Frontend Rules
- Svelte 5 runes syntax (`$state`, `$derived`, `$effect`) - no legacy Options API
- Pure logic functions extracted to `src/lib/` for testability
- No direct DOM manipulation - use Svelte reactivity
- Same fonts and colour palette as HalluMeter - one product family

---

## No Fallbacks Unless Requested
- No silent error swallowing
- No hardcoded fallback values for user-configurable params
- `todo!()` stubs are acceptable in scaffold phase; remove before shipping

---

## Ask vs Proceed

| Situation | Action |
|---|---|
| Deleting any file | Ask |
| Changing `curves.json` data values | Ask |
| Adding a new dependency to Cargo.toml or package.json | Ask |
| Changing window dimensions | Ask |
| Adding a new Tauri capability/permission | Ask |
| Implementing work beyond current phase scope | Ask |
| Renaming a public Rust function or Tauri event name | Ask |
| Anything that touches CI/CD config | Ask |
| Modifying anything in `reference/` | Ask - these are read-only |
| Bug fix within current phase scope | Proceed |
| Formatting / clippy fixes | Proceed |
| Adding tests for existing functions | Proceed |

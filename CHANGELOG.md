# Changelog

All notable changes to HalluScribe will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.3.33] - 2026-09-18

### Fixed
- Sweep summary requests force the tool call with `tool_choice: "required"`, which llama-server accepts, instead of the object form some builds reject
- Large-session chunk budget now scales with the configured context instead of stopping at 45K tokens, so a single oversized turn fits when the context is raised

### Changed
- Every llama-server HalluScribe starts exposes `--metrics` for local monitoring
- TOKENS column is centred in the session table

## [0.3.31] - 2026-09-18

### Changed
- The app now opens on the Session Summary tab instead of Briefing
- Qwen 3.8 llama.cpp tuning template matches the retuned Forge setup: batch 16384, micro-batch 1024, 3 draft tokens, vision projector on the GPU

## [0.3.30] - 2026-09-12

### Fixed
- Forge session context fill is now measured from compaction records instead of a character estimate, so Forge sessions show a real fill, respect the minimum-fill gate, and are no longer always flagged as estimated

## [0.3.29] - 2026-09-10

### Added
- Grok chat-export import support, with discovery for common extracted-export layouts
- Configurable llama.cpp GPU placement, model-specific MTP drafter selection, and sampling controls
- Archive analysis, provider session counts, token counts, and active-agent status in the UI

### Changed
- Import-path ownership is now explicit per workspace; host-level machine settings stay with the host
- Local-agent archive handling and project labels are more accurate; Continue import support was removed

### Fixed
- Archive, settings, workspace, and embedding persistence now report corrupt or incomplete state rather than silently treating it as empty
- Llama-server lifecycle, diagnostic logs, raw-transcript versioning, search cache invalidation, and profile refresh cancellation
- Sweep and capture failures remain visible instead of being reported as successful completion

## [0.3.19] - 2026-08-14

### Fixed
- Prevent corrupt archive, settings, workspace, and embedding state from being silently treated as empty or overwritten
- Report incomplete sweeps and raw-capture persistence failures instead of showing false success
- Surface llama-server process, PID-registry, and diagnostic-log failures for recovery

## [0.3.18] - 2026-08-08

### Added
- Multi-workspace archive support, including verified workspace relocation and per-workspace import ownership
- Per-session raw transcript slicing for multi-chat exports and repair of older whole-export raw copies
- Multi-token-prediction drafter discovery for compatible llama.cpp models
- Context-window usage reporting in interactive chat

### Changed
- Reuse a compatible multimodal llama-server for later text-only turns
- Strengthen archive-tool grounding rules after retrieval failures or user challenges
- Keep app settings synchronized after save and report repaired raw transcripts during backfill
- Ship the standalone `halluscribe-mcp` binary beside every platform release

### Fixed
- Profile refresh progress and dropped profile evidence
- Model and llama.cpp binary changes not taking effect until restart
- Image attachment validation, preview, and window-size persistence
- Raw transcript reads for imported multi-session chat exports

### Security
- Raw transcripts are stored unredacted. The local `halluscribe-mcp` server can expose their
  verbatim contents through `search_raw_transcripts` and `read_raw_session`, including secrets
  or tool output removed from summaries. Persona Pack exports still require an explicit raw-data
  opt-in and never include raw transcripts implicitly.

## [0.1.0] - 2026-04-23

### Added
- JSONL reader for Claude Code, Codex, and Continue session files
- Local Gemma 4 inference via llama.cpp (primary) and Ollama (secondary)
- Nightly sweep scheduler with configurable time window
- Session archive written to `~/.halluscribe/` in human-readable Markdown
- Full-text and semantic search across archived sessions
- Briefing mode: multi-turn chat over archived session context
- Web search integration via Tavily (optional)
- Settings UI: backend selection, model paths, schedule time, generation limits
- System tray icon with sweep status and manual trigger
- Cross-platform support: Windows, macOS, Linux

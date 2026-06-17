# Changelog

All notable changes to HalluScribe will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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

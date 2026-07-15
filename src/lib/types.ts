// HalluScribe — shared TypeScript types (mirrors Rust structs).

export interface IndexEntry {
  id: string;
  project: string;
  date: string;
  title: string;
  tool: string;
  fill_pct: number;
  session_timestamp?: string;
  updated_at?: string;
  session_type: string;
  error_tags: string[];
  topic_tags: string[];
  archive_path: string;
  source_jsonl: string;
  provider?: string;
  fill_estimated?: boolean;
  transcript_hash?: string;
  secret_flags?: string[];
}

export interface SessionStats {
  /** Sessions HalluScribe has fully summarised and archived. */
  total: number;
  /** Raw JSONL sessions discovered on disk before any filtering. */
  raw_total?: number;
  by_tool: Record<string, number>;
  by_type: Record<string, number>;
  top_error_tags: string[];
  top_topic_tags: string[];
}

export interface HalluScribeSettings {
  backend: "llamacpp" | "ollama";
  llama_server_bin: string;
  gemma_model_path: string;
  embedding_model_path: string;
  gpu_layers: number;
  llama_server_port: number;
  ollama_host: string;
  ollama_port: number;
  ollama_model: string;
  ollama_api_key: string;
  tavily_api_key: string;
  summary_min_fill_pct: number;
  lookback_hours: number;
  chatgpt_import_path: string;
  claudeai_import_path: string;
  gemini_import_path: string;
  ollama_chat_db_path: string;
  continue_data_path: string;
  scheduled_processing_enabled: boolean;
  schedule_time: string;
  idle_threshold_mins: number;
  first_run: boolean;
  ctx_size: number;
  max_tokens: number;
  briefing_window_hours: number;
  tts_piper_bin: string;
  tts_voice: string;
}

/** Filters passed to run_briefing. Empty strings mean "no restriction". */
export interface BriefingFilters {
  dateFrom: string;
  dateTo: string;
  fillMin: string;
  fillMax: string;
  keyword: string;
}

export type BriefingScope =
  | { kind: "archive-wide" }
  | { kind: "filter-based"; filters: BriefingFilters }
  | { kind: "selected-session-ids"; sessionIds: string[]; label: string };

export type ChatScope =
  | { kind: "briefing-scope" }
  | { kind: "archive-wide" };

export type ChatSearchMode = "archive" | "semantic";

export interface ChatAttachment {
  name: string;
  mimeType: string;
  base64: string;
}

export interface ChatImage {
  mime_type: string;
  data: string;
}

/** One rendered turn in the chat UI (not the same as ChatMessage sent to Rust). */
export interface Turn {
  /** Stable per-turn id (monotonic counter), used to key the shared SpeakController. */
  id: string;
  role: "user" | "assistant";
  thinkingText: string;
  answerText: string;
  toolActivity: string | null;
  streaming: boolean;
  attachmentName?: string;
  searchMode?: ChatSearchMode;
  webSearchEnabled?: boolean;
  thinkingEnabled?: boolean;
  backendName?: string;
  modelName?: string;
}

export interface RecordedTurnState {
  search_mode: ChatSearchMode;
  web_search_enabled: boolean;
  thinking_enabled: boolean;
  backend_name: string;
  model_name: string;
}

export interface RecordedChatTurn {
  role: "user" | "assistant";
  content: string;
  tool_activity?: string | null;
  attachment_name?: string | null;
  had_thinking_output: boolean;
  state?: RecordedTurnState;
}

export interface RecordedSessionSaveResult {
  session_id: string;
  relative_path: string;
  absolute_path: string;
}

/** One turn in the chat conversation. */
export interface ChatMessage {
  role: "user" | "assistant" | "tool";
  content: string;
  tool_call_id?: string;
  name?: string;
  images?: ChatImage[];
}

/** Emitted by sweep-progress events during a sweep pass. */
export interface SweepProgress {
  current: number;
  total: number;
  session_id: string;
  status: string;
}

/** Emitted during a semantic embedding rebuild. */
export interface EmbeddingRebuildProgress {
  current: number;
  total: number;
  indexed: number;
  skipped: number;
  failed: number;
}

/** Result of a raw-transcript backfill pass (Persona Parity Phase A). */
export interface BackfillResult {
  recovered: number;
  already_had: number;
  source_missing: number;
  total: number;
}

/** Emitted by `raw-capture-progress` events during the startup raw capture
 *  pass and returned by `get_capture_status`. Mirrors the Rust
 *  `archive::CaptureStatus` enum's internally-tagged JSON shape. */
export type CaptureStatus =
  | { state: "idle" }
  | { state: "running"; done: number; total: number; captured: number }
  | { state: "done"; done: number; total: number; captured: number }
  | { state: "cancelled" };

export interface RawExcerpt {
  line_no: number;
  excerpt: string;
}

export interface RawSessionMatches {
  session_id: string;
  total_hits: number;
  excerpts: RawExcerpt[];
  excerpts_truncated: boolean;
  /** True for sessions in the archive index. False for sessions the startup
   *  raw capture pass preserved but no sweep has summarised yet - `title`/
   *  `date` stand in for the metadata a summary would otherwise supply. */
  summarised: boolean;
  /** Source filename, only set when `summarised` is false. */
  title?: string;
  /** Source file mtime (`YYYY-MM-DD`), only set when `summarised` is false. */
  date?: string;
}

export interface RawSearchResult {
  sessions: RawSessionMatches[];
  sessions_scanned: number;
  sessions_without_raw: number;
  sessions_failed: string[];
  total_hits: number;
  results_truncated: boolean;
}

/** Payload from briefing-token / chat-token Tauri events. */
export interface TokenPayload {
  text: string;
  is_thinking: boolean;
}

/** Payload from briefing-tool-call / chat-tool-call Tauri events. */
export interface ToolCallPayload {
  tool: string;
  args: Record<string, unknown>;
}

/** Payload from chat-usage Tauri event — token counts after each chat turn. */
export interface ChatUsagePayload {
  prompt_tokens: number;
  completion_tokens: number;
  ctx_size: number;
}

export type WebSearchStatus = "ready" | "missing-api-key" | "unsupported-backend" | "loading";

/** Result of previewing a redaction before applying it. */
export interface RedactionPreview {
  occurrences: number;
  excerpts: string[];
}

/** Result of applying a redaction to an archived session body. */
export interface RedactionOutcome {
  replacements: number;
  backup_path: string;
}

/** Which of the two profiles (Phase 2c) an operation targets. */
export type ProfileScope = "work" | "personal";

/** Emitted by `profile-progress` events during a profile refresh. */
export interface ProfileProgressPayload {
  current: number;
  total: number;
  stage: "mapping" | "merging" | "writing";
  scope: ProfileScope;
}

/** Emitted by the `profile-done` event when a profile refresh finishes. */
export interface ProfileDonePayload {
  busy: boolean;
  session_count: number;
  facts_count: number;
  errors: string[];
  scope: ProfileScope;
}

/** One registered person/workspace (Persona Parity Phase E). */
export interface WorkspaceInfo {
  name: string;
  path: string;
  import_only: boolean;
}

/** Snapshot returned by list_workspaces. `active` is null when the default root is active. */
export interface WorkspaceListDto {
  default_root: string;
  default_name: string | null;
  active: string | null;
  workspaces: WorkspaceInfo[];
}

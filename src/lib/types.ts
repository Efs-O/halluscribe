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
  /** Tokens the model generated in this session. 0 on entries archived before the column existed. */
  output_tokens?: number;
  /** True when output_tokens was inferred from character counts rather than reported by the server. */
  tokens_estimated?: boolean;
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
  window_width: number | null;
  window_height: number | null;
  embedding_model_path: string;
  gpu_layers: number;
  /** `--device`: comma-separated llama.cpp device names. Empty = all devices. */
  gpu_devices: string;
  /** `--split-mode`: none | layer | row | tensor. Empty = llama.cpp default. */
  gpu_split_mode: string;
  /** `--tensor-split`: per-device fractions, e.g. "0.7,0.3". Empty = even. */
  gpu_tensor_split: string;
  /** `--main-gpu`: device holding the KV cache. -1 = unset. */
  gpu_main_index: number;
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
  grok_import_path: string;
  ollama_chat_db_path: string;
  forge_sessions_path: string;
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
  /** Data URI of the attached image, for the in-bubble thumbnail. Held in
   *  memory only - deliberately not part of the saved session payload, which
   *  keeps `attachment_name` alone so the archive does not carry megabytes of
   *  base64 per turn. */
  attachmentDataUrl?: string;
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
  /** Stored raws that were whole-export copies and got replaced with the
   *  per-session slice. Zero on archives that never had the old raws. */
  repaired: number;
  source_missing: number;
  total: number;
}

/** Emitted by `raw-capture-progress` events during the startup raw capture
 *  pass and returned by `get_capture_status`. Mirrors the Rust
 *  `archive::CaptureStatus` enum's internally-tagged JSON shape. */
export type CaptureStatus =
  | { state: "idle" }
  | { state: "running"; done: number; total: number; captured: number; failed: number }
  | { state: "done"; done: number; total: number; captured: number }
  | { state: "failed"; done: number; total: number; captured: number; errors: string[] }
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
  /**
   * Hard failures AND non-fatal warnings (skipped facts, dropped citations,
   * merge fallbacks), mixed. A non-empty list does NOT mean the run failed —
   * check `failed_batches` for that.
   */
  errors: string[];
  /** Map batches that failed outright. 0 = the profile was written. */
  failed_batches: number;
  scope: ProfileScope;
  /**
   * The user stopped the run at a safe boundary. Not an error: nothing was
   * written, the watermark did not move, and the partial mapping work was
   * saved so the next run resumes from it.
   */
  cancelled: boolean;
}

/** One registered person/workspace (Persona Parity Phase E). */
export interface WorkspaceInfo {
  name: string;
  path: string;
  import_only: boolean;
}

/** What move_workspace copied. The old folder is kept and reported back. */
export interface MoveWorkspaceResult {
  files: number;
  bytes: number;
  old_path: string;
  new_path: string;
}

/** Snapshot returned by list_workspaces. `active` is null when the default root is active. */
export interface WorkspaceListDto {
  default_root: string;
  default_name: string | null;
  active: string | null;
  workspaces: WorkspaceInfo[];
}

/** Whether a configured chat-import path actually has an export in it.
 *  Mirrors `scanner::import_status::ImportPathState`. `no_export_found` is the
 *  case the diagnostic exists for: a path that is set, real, and still empty. */
export type ImportPathState =
  | "unset"
  | "missing_folder"
  | "no_export_found"
  | "found";

/** One provider's import-path health, as reported by `chat_import_status`. */
export interface ImportPathStatus {
  provider: string;
  label: string;
  path: string;
  state: ImportPathState;
  file_count: number;
  total_bytes: number;
  files: string[];
}

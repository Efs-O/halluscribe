// HalluScribe - durable archive chat state, persistence, and event orchestration.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appendAssistantToken, formatToolActivity } from "./chatTurns";
import type {
  BriefingScope,
  ChatAttachment,
  ChatMessage,
  ChatScope,
  ChatSearchMode,
  ChatUsagePayload,
  HalluScribeSettings,
  ProfileScope,
  RecordedChatTurn,
  RecordedSessionSaveResult,
  ToolCallPayload,
  TokenPayload,
  Turn,
  WebSearchStatus,
} from "./types";

export type ChatClearReason = "clear" | "scope-change" | "search-mode-change";
type SnapshotReason = "toggle_on" | "user_message" | "tool_call" | "assistant_done" | ChatClearReason;

export class ChatController {
  scope = $state<ChatScope>({ kind: "archive-wide" });
  searchMode = $state<ChatSearchMode>("archive");
  profileScope = $state<ProfileScope>("work");
  turns = $state<Turn[]>([]);
  ctxUsedPct = $state(0);
  streaming = $state(false);
  webSearchEnabled = $state(false);
  saveSessionEnabled = $state(false);
  saveSessionNote = $state<string | null>(null);
  saveSessionNoteIsError = $state(false);
  thinkingEnabled = $state(false);
  thinkingChangePending = $state(false);

  private activeRecordedSessionPath = $state<string | null>(null);
  private turnIdCounter = 0;

  constructor(
    private readonly settings: () => HalluScribeSettings | null,
    private readonly briefingScope: () => BriefingScope,
  ) {}

  async registerListeners(): Promise<UnlistenFn[]> {
    return Promise.all([
      listen<TokenPayload>("chat-token", (event) => {
        const last = this.turns[this.turns.length - 1];
        const next = appendAssistantToken(last, event.payload);
        if (last && next) Object.assign(last, next);
      }),
      listen("chat-done", () => {
        const last = this.turns[this.turns.length - 1];
        if (last) last.streaming = false;
        this.streaming = false;
        this.thinkingChangePending = false;
        void this.persistRecordedSnapshot("assistant_done");
      }),
      listen<ChatUsagePayload>("chat-usage", (event) => {
        const { prompt_tokens, completion_tokens, ctx_size } = event.payload;
        const used = prompt_tokens + completion_tokens;
        this.ctxUsedPct = Math.min(100, Math.round((used / ctx_size) * 10) * 10);
      }),
      listen<ToolCallPayload>("chat-tool-call", (event) => {
        const last = this.turns[this.turns.length - 1];
        if (last) last.toolActivity = formatToolActivity(event.payload.tool, event.payload.args);
        void this.persistRecordedSnapshot("tool_call");
      }),
    ]);
  }

  webSearchStatus(): WebSearchStatus {
    const settings = this.settings();
    if (!settings) return "loading";
    if (!settings.ollama_api_key.trim() && !settings.tavily_api_key.trim()) {
      return "missing-api-key";
    }
    return "ready";
  }

  imageAttachEnabled(): boolean {
    const settings = this.settings();
    return Boolean(settings && (settings.backend === "ollama" || settings.backend === "llamacpp"));
  }

  async send(text: string, attachment: ChatAttachment | null): Promise<void> {
    if (this.streaming) return;
    this.streaming = true;
    this.thinkingChangePending = false;
    this.saveSessionNote = null;
    const visibleText = text.trim();
    const backendText = visibleText || (attachment ? "Describe the attached image." : "");
    const turnState = {
      searchMode: this.searchMode,
      webSearchEnabled: this.webSearchEnabled && this.webSearchStatus() === "ready",
      thinkingEnabled: this.thinkingEnabled,
      backendName: this.currentBackendName(),
      modelName: this.currentModelName(),
    };
    this.turns.push({
      id: this.nextTurnId(),
      role: "user",
      thinkingText: "",
      answerText: visibleText,
      toolActivity: null,
      streaming: false,
      attachmentName: attachment?.name,
      attachmentDataUrl: attachment
        ? `data:${attachment.mimeType};base64,${attachment.base64}`
        : undefined,
      ...turnState,
    });
    this.turns.push({
      id: this.nextTurnId(),
      role: "assistant",
      thinkingText: "",
      answerText: "",
      toolActivity: null,
      streaming: true,
      ...turnState,
    });
    try {
      await this.persistRecordedSnapshot("user_message");
      const messages: ChatMessage[] = this.turns
        .filter((turn) => turn.role === "user" || (turn.role === "assistant" && turn.answerText))
        .map((turn) => ({ role: turn.role, content: turn.answerText }));
      const lastUserMessage = [...messages].reverse().find((message) => message.role === "user");
      if (lastUserMessage) lastUserMessage.content = backendText;
      if (attachment && lastUserMessage) {
        lastUserMessage.images = [{ mime_type: attachment.mimeType, data: attachment.base64 }];
      }
      const scope = this.briefingScope();
      const allowedSessionIds = this.scope.kind === "briefing-scope" && scope.kind === "selected-session-ids"
        ? scope.sessionIds
        : null;
      await invoke("send_chat_message", {
        messages,
        allowedSessionIds,
        webSearchEnabled: this.webSearchEnabled && this.webSearchStatus() === "ready",
        thinkingEnabled: this.thinkingEnabled,
        searchMode: this.searchMode,
        profileScope: this.profileScope,
      });
    } catch (error) {
      const last = this.turns[this.turns.length - 1];
      if (last) {
        last.answerText = `Error: ${error}`;
        last.streaming = false;
      }
      this.streaming = false;
    }
  }

  async stop(): Promise<void> {
    await invoke("cancel_chat");
  }

  async clear(reason: ChatClearReason = "clear"): Promise<boolean> {
    const canClear = await this.persistRecordedSnapshot(reason);
    if (!canClear) return false;
    this.turns = [];
    this.ctxUsedPct = 0;
    this.webSearchEnabled = false;
    this.thinkingChangePending = false;
    this.activeRecordedSessionPath = null;
    return true;
  }

  resetForBriefingScope(selectedScope: boolean): void {
    this.scope = selectedScope ? { kind: "briefing-scope" } : { kind: "archive-wide" };
    this.turns = [];
  }

  toggleWebSearch(): void {
    if (this.webSearchStatus() === "ready") this.webSearchEnabled = !this.webSearchEnabled;
  }

  toggleSaveSession(): void {
    const next = !this.saveSessionEnabled;
    this.saveSessionEnabled = next;
    this.saveSessionNote = next
      ? "Chat recording enabled for this session."
      : "Chat recording disabled.";
    this.saveSessionNoteIsError = false;
    if (next) void this.persistRecordedSnapshot("toggle_on");
  }

  toggleThinking(): void {
    this.thinkingEnabled = !this.thinkingEnabled;
    this.thinkingChangePending = this.streaming;
  }

  async setScope(kind: ChatScope["kind"]): Promise<void> {
    if (this.scope.kind === kind) return;
    const cleared = await this.clear("scope-change");
    if (!cleared) return;
    this.scope = kind === "briefing-scope" ? { kind } : { kind: "archive-wide" };
  }

  async setSearchMode(mode: ChatSearchMode): Promise<void> {
    if (this.searchMode === mode) return;
    const cleared = await this.clear("search-mode-change");
    if (!cleared) return;
    this.searchMode = mode;
  }

  setProfileScope(scope: ProfileScope): void {
    this.profileScope = scope;
  }

  private nextTurnId(): string {
    this.turnIdCounter += 1;
    return `t${this.turnIdCounter}`;
  }

  private currentBackendName(): string {
    return this.settings()?.backend === "ollama" ? "ollama" : "llama.cpp";
  }

  private currentModelName(): string {
    const settings = this.settings();
    if (!settings) return "Gemma 4";
    return settings.backend === "llamacpp"
      ? settings.gemma_model_path.trim() || "Gemma 4"
      : settings.ollama_model.trim() || "gemma4:26b";
  }

  private selectedSourceSessionIds(): string[] {
    const scope = this.briefingScope();
    return this.scope.kind === "briefing-scope" && scope.kind === "selected-session-ids"
      ? [...scope.sessionIds]
      : [];
  }

  private buildRecordedTurns(): RecordedChatTurn[] {
    return this.turns.map((turn) => ({
      role: turn.role,
      content: turn.answerText,
      tool_activity: turn.toolActivity ?? null,
      attachment_name: turn.attachmentName ?? null,
      had_thinking_output: Boolean(turn.thinkingText.trim()),
      state: turn.searchMode && turn.backendName && turn.modelName
        ? {
          search_mode: turn.searchMode,
          web_search_enabled: Boolean(turn.webSearchEnabled),
          thinking_enabled: Boolean(turn.thinkingEnabled),
          backend_name: turn.backendName,
          model_name: turn.modelName,
        }
        : undefined,
    }));
  }

  private async persistRecordedSnapshot(reason: SnapshotReason): Promise<boolean> {
    if (!this.saveSessionEnabled || this.turns.length === 0) return true;
    if (!this.settings()) {
      this.saveSessionNote = "Chat recording skipped because settings are not loaded yet.";
      this.saveSessionNoteIsError = true;
      return false;
    }
    try {
      const result = await invoke<RecordedSessionSaveResult>("save_recorded_chat_session", {
        request: {
          existing_relative_path: this.activeRecordedSessionPath,
          chat_search_mode: this.searchMode,
          source_session_ids: this.selectedSourceSessionIds(),
          web_search_enabled: this.webSearchEnabled && this.webSearchStatus() === "ready",
          thinking_enabled: this.thinkingEnabled,
          had_thinking_output: this.turns.some((turn) => turn.thinkingText.trim().length > 0),
          backend_name: this.currentBackendName(),
          model_name: this.currentModelName(),
          boundary_reason: reason,
          turns: this.buildRecordedTurns(),
        },
      });
      this.activeRecordedSessionPath = result.relative_path;
      this.saveSessionNote = `Recorded chat saved: ${result.absolute_path}`;
      this.saveSessionNoteIsError = false;
      return true;
    } catch (error) {
      this.saveSessionNote = `Failed to save recorded chat: ${String(error)}`;
      this.saveSessionNoteIsError = true;
      return false;
    }
  }

}

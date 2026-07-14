<!-- HalluScribe - root component. Nav routing + top bar. -->
<script lang="ts">
  import { exit } from "@tauri-apps/plugin-process";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open } from "@tauri-apps/plugin-shell";
  import { onMount } from "svelte";
  import NavBar from "./components/NavBar.svelte";
  import WorkspaceBadge from "./components/WorkspaceBadge.svelte";
  import RunNowButton from "./components/RunNowButton.svelte";
  import BriefingPanel from "./components/briefing/BriefingPanel.svelte";
  import SessionList from "./components/sessions/SessionList.svelte";
  import ProfilePanel from "./components/profile/ProfilePanel.svelte";
  import SettingsForm from "./components/settings/SettingsForm.svelte";
  import { appendAssistantToken } from "./lib/chatTurns";
  import { SpeakController } from "./lib/tts.svelte.ts";
  import type {
    ChatAttachment,
    ChatUsagePayload,
    TokenPayload,
    ToolCallPayload,
    ChatMessage,
    RecordedChatTurn,
    RecordedSessionSaveResult,
    Turn,
    BriefingFilters,
    BriefingScope,
    ChatScope,
    ChatSearchMode,
    ProfileScope,
    RawSearchResult,
    SweepProgress,
    HalluScribeSettings,
    WebSearchStatus,
  } from "./lib/types";

  type Tab = "briefing" | "sessions" | "profile" | "settings";
  let activeTab = $state<Tab>("briefing");

  // One shared SpeakController owned here (App is never destroyed) so TTS
  // playback keeps running when the user switches tabs — BriefingPanel used to
  // own it and cancelled playback in its onDestroy on every tab change.
  const speakController = new SpeakController();
  let ctxVisible = $state(false);
  let ctxX = $state(0);
  let ctxY = $state(0);
  let briefingAnswer = $state("");
  let briefingHeader = $state("");
  let briefingStreaming = $state(false);
  let briefingError = $state<string | null>(null);
  let briefingWarning = $state<string | null>(null);
  let briefingScope = $state<BriefingScope>({ kind: "archive-wide" });
  let chatScope = $state<ChatScope>({ kind: "archive-wide" });
  let chatSearchMode = $state<ChatSearchMode>("archive");
  let chatProfileScope = $state<ProfileScope>("work");
  // Session-list search state lifted here so the raw/summaries scope, the query,
  // and a completed raw scan survive tab switches (App is never destroyed —
  // SessionList is re-created on every tab change). Matches how chat scope/turns
  // persist above.
  let sessionScope = $state<"summaries" | "raw">("summaries");
  let sessionQuery = $state("");
  let sessionRawResult = $state<RawSearchResult | null>(null);
  let sessionRawError = $state<string | null>(null);
  let turns = $state<Turn[]>([]);
  let ctxUsedPct = $state(0);
  let chatStreaming = $state(false);
  let webSearchEnabled = $state(false);
  let saveSessionEnabled = $state(false);
  let activeRecordedSessionPath = $state<string | null>(null);
  let saveSessionNote = $state<string | null>(null);
  let saveSessionNoteIsError = $state(false);
  let thinkingEnabled = $state(false);
  let thinkingChangePending = $state(false);
  let settingsSnapshot = $state<HalluScribeSettings | null>(null);
  let sweepRunning = $state(false);
  let sweepProgress = $state<SweepProgress | null>(null);
  let sweepToast = $state<{ msg: string; ok: boolean } | null>(null);
  let sweepToastTimer: ReturnType<typeof setTimeout> | undefined;
  let isMaximized = $state(false);
  const appWindow = getCurrentWindow();
  let turnIdCounter = 0;
  function nextTurnId(): string {
    turnIdCounter += 1;
    return `t${turnIdCounter}`;
  }

  function formatToolActivity(tool: string, args: Record<string, unknown>): string {
    if (tool === "web_search") {
      const query = typeof args.query === "string" ? args.query : "";
      return query
        ? `searching the web: ${query}`
        : "searching the web...";
    }
    if (tool === "web_fetch") {
      const url = typeof args.url === "string" ? args.url : "";
      return url
        ? `fetching page: ${url}`
        : "fetching page...";
    }
    if (tool === "search_sessions") {
      const query = typeof args.query === "string" ? args.query : "";
      return query
        ? `searching archives: ${query}`
        : "searching archives...";
    }
    if (tool === "search_sessions_semantic") {
      const query = typeof args.query === "string" ? args.query : "";
      return query
        ? `semantic search: ${query}`
        : "semantic search...";
    }
    if (tool === "read_session") {
      const sessionId = typeof args.session_id === "string" ? args.session_id : "";
      return sessionId
        ? `reading session: ${sessionId}`
        : "reading session...";
    }
    return `running tool: ${tool}`;
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    ctxX = e.clientX;
    ctxY = e.clientY;
    ctxVisible = true;
  }

  function closeCtx() { ctxVisible = false; }

  onMount(async () => {
    isMaximized = await appWindow.isMaximized();
    const unlistenResize = await appWindow.onResized(async () => {
      isMaximized = await appWindow.isMaximized();
    });

    void refreshSettingsSnapshot();

    await listen<string>("briefing-header", (ev) => {
      briefingHeader = ev.payload;
    });

    await listen<string | null>("briefing-warning", (ev) => {
      briefingWarning = ev.payload;
    });

    await listen<TokenPayload>("briefing-token", (ev) => {
      briefingAnswer += ev.payload.text;
    });

    await listen("briefing-done", () => {
      briefingStreaming = false;
    });

    await listen<TokenPayload>("chat-token", (ev) => {
      const last = turns[turns.length - 1];
      const next = appendAssistantToken(last, ev.payload);
      if (last && next) Object.assign(last, next);
    });

    await listen("chat-done", () => {
      const last = turns[turns.length - 1];
      if (last) last.streaming = false;
      chatStreaming = false;
      thinkingChangePending = false;
      void persistRecordedChatSnapshot("assistant_done");
    });

    await listen<ChatUsagePayload>("chat-usage", (ev) => {
      const { prompt_tokens, completion_tokens, ctx_size } = ev.payload;
      const used = prompt_tokens + completion_tokens;
      ctxUsedPct = Math.min(100, Math.round((used / ctx_size) * 10) * 10);
    });

    await listen<ToolCallPayload>("chat-tool-call", (ev) => {
      const last = turns[turns.length - 1];
      if (last) last.toolActivity = formatToolActivity(ev.payload.tool, ev.payload.args);
      void persistRecordedChatSnapshot("tool_call");
    });

    await listen<SweepProgress>("sweep-progress", (ev) => {
      sweepRunning = true;
      sweepProgress = ev.payload;
      if (!sweepToast) {
        sweepToast = { msg: "Sweep running...", ok: true };
      }
    });

    await listen<string>("sweep-done", (ev) => {
      sweepProgress = null;
      sweepToast = { msg: ev.payload, ok: true };
      sweepRunning = false;
      clearTimeout(sweepToastTimer);
      sweepToastTimer = setTimeout(() => { sweepToast = null; }, 5000);
    });

    return () => {
      unlistenResize();
    };
  });

  async function hideToTray() {
    await appWindow.hide();
  }

  async function toggleMaximizeWindow() {
    await appWindow.toggleMaximize();
    isMaximized = await appWindow.isMaximized();
  }

  async function exitApp() {
    await exit(0);
  }

  async function refreshSettingsSnapshot() {
    settingsSnapshot = await invoke<HalluScribeSettings>("get_settings");
  }

  function webSearchStatus(): WebSearchStatus {
    if (!settingsSnapshot) return "loading";
    if (!settingsSnapshot.ollama_api_key.trim() && !settingsSnapshot.tavily_api_key.trim()) {
      return "missing-api-key";
    }
    return "ready";
  }

  function currentBackendName(): string {
    return settingsSnapshot?.backend === "ollama" ? "ollama" : "llama.cpp";
  }

  function currentModelName(): string {
    if (!settingsSnapshot) return "Gemma 4";
    return settingsSnapshot.backend === "llamacpp"
      ? settingsSnapshot.gemma_model_path.trim() || "Gemma 4"
      : settingsSnapshot.ollama_model.trim() || "gemma4:26b";
  }

  function imageAttachEnabled(): boolean {
    return Boolean(
      settingsSnapshot
      && (settingsSnapshot.backend === "ollama" || settingsSnapshot.backend === "llamacpp")
    );
  }

  async function cancelBriefing() {
    await invoke("cancel_briefing");
  }

  function clearBriefingOutput() {
    briefingAnswer = "";
    briefingHeader = "";
    briefingError = null;
    briefingWarning = null;
  }

  function resetForScopeChange(nextScope: BriefingScope) {
    void cancelBriefing();
    briefingScope = nextScope;
    chatScope = nextScope.kind === "selected-session-ids"
      ? { kind: "briefing-scope" }
      : { kind: "archive-wide" };
    briefingStreaming = false;
    turns = [];
    clearBriefingOutput();
  }

  function hasActiveFilters(filters: BriefingFilters): boolean {
    return Boolean(
      filters.dateFrom ||
      filters.dateTo ||
      filters.fillMin ||
      filters.fillMax ||
      filters.keyword.trim()
    );
  }

  function scopeLabel(scope: BriefingScope): string {
    if (scope.kind === "selected-session-ids") return scope.label;
    if (scope.kind === "filter-based") return "Scope: filtered briefing";
    return "Scope: all archive";
  }

  async function runNow() {
    if (sweepRunning) return;
    sweepRunning = true;
    sweepProgress = null;
    clearTimeout(sweepToastTimer);
    sweepToast = null;
    try {
      await invoke("trigger_sweep");
      sweepToast = { msg: "Sweep running...", ok: true };
    } catch (e) {
      sweepToast = { msg: String(e), ok: false };
      sweepRunning = false;
      sweepToastTimer = setTimeout(() => { sweepToast = null; }, 5000);
    }
  }

  async function cancelSweep() {
    await invoke("cancel_sweep");
    sweepToast = { msg: "Stopping - waiting for current session to finish...", ok: true };
  }

  async function runBriefing(filters: BriefingFilters) {
    const nextScope: BriefingScope = briefingScope.kind === "selected-session-ids"
      ? briefingScope
      : hasActiveFilters(filters)
        ? { kind: "filter-based", filters }
        : { kind: "archive-wide" };

    briefingScope = nextScope;
    clearBriefingOutput();
    briefingStreaming = true;
    try {
      await invoke("run_briefing", {
        dateFrom: filters.dateFrom || null,
        dateTo: filters.dateTo || null,
        fillMin: filters.fillMin ? parseFloat(filters.fillMin) : null,
        fillMax: filters.fillMax ? parseFloat(filters.fillMax) : null,
        keyword: filters.keyword || null,
        sessionIds: nextScope.kind === "selected-session-ids" ? nextScope.sessionIds : null,
      });
    } catch (e) {
      briefingError = String(e);
      briefingStreaming = false;
    }
  }

  async function sendChat(text: string, attachment: ChatAttachment | null) {
    if (chatStreaming) return;
    chatStreaming = true;
    thinkingChangePending = false;
    saveSessionNote = null;
    const visibleText = text.trim();
    const backendText = visibleText || (attachment ? "Describe the attached image." : "");
    const turnState = {
      searchMode: chatSearchMode,
      webSearchEnabled: webSearchEnabled && webSearchStatus() === "ready",
      thinkingEnabled,
      backendName: currentBackendName(),
      modelName: currentModelName(),
    };
    turns.push({
      id: nextTurnId(),
      role: "user",
      thinkingText: "",
      answerText: visibleText,
      toolActivity: null,
      streaming: false,
      attachmentName: attachment?.name,
      ...turnState,
    });
    turns.push({
      id: nextTurnId(),
      role: "assistant",
      thinkingText: "",
      answerText: "",
      toolActivity: null,
      streaming: true,
      ...turnState,
    });
    try {
      await persistRecordedChatSnapshot("user_message");
      const messages: ChatMessage[] = turns
        .filter((turn) => turn.role === "user" || (turn.role === "assistant" && turn.answerText))
        .map((turn) => ({ role: turn.role, content: turn.answerText }));
      const lastUserMessage = [...messages].reverse().find((message) => message.role === "user");
      if (lastUserMessage) {
        lastUserMessage.content = backendText;
      }
      if (attachment) {
        if (lastUserMessage) {
          lastUserMessage.images = [{
            mime_type: attachment.mimeType,
            data: attachment.base64,
          }];
        }
      }
      const allowedSessionIds = chatScope.kind === "briefing-scope" && briefingScope.kind === "selected-session-ids"
        ? briefingScope.sessionIds
        : null;
      await invoke("send_chat_message", {
        messages,
        allowedSessionIds,
        webSearchEnabled: webSearchEnabled && webSearchStatus() === "ready",
        thinkingEnabled,
        searchMode: chatSearchMode,
        profileScope: chatProfileScope,
      });
    } catch (e) {
      const last = turns[turns.length - 1];
      if (last) {
        last.answerText = `Error: ${e}`;
        last.streaming = false;
      }
      chatStreaming = false;
    }
  }

  async function stopChat() {
    await invoke("cancel_chat");
  }

  function selectedSourceSessionIds(): string[] {
    return chatScope.kind === "briefing-scope" && briefingScope.kind === "selected-session-ids"
      ? [...briefingScope.sessionIds]
      : [];
  }

  function buildRecordedTurns(): RecordedChatTurn[] {
    return turns.map((turn) => ({
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

  async function persistRecordedChatSnapshot(
    reason: "toggle_on" | "user_message" | "tool_call" | "assistant_done" | "clear" | "scope-change" | "search-mode-change",
  ): Promise<boolean> {
    if (!saveSessionEnabled || turns.length === 0) return true;
    if (!settingsSnapshot) {
      saveSessionNote = "Chat recording skipped because settings are not loaded yet.";
      saveSessionNoteIsError = true;
      return false;
    }
    try {
      const result = await invoke<RecordedSessionSaveResult>("save_recorded_chat_session", {
        request: {
          existing_relative_path: activeRecordedSessionPath,
          chat_search_mode: chatSearchMode,
          source_session_ids: selectedSourceSessionIds(),
          web_search_enabled: webSearchEnabled && webSearchStatus() === "ready",
          thinking_enabled: thinkingEnabled,
          had_thinking_output: turns.some((turn) => turn.thinkingText.trim().length > 0),
          backend_name: currentBackendName(),
          model_name: currentModelName(),
          boundary_reason: reason,
          turns: buildRecordedTurns(),
        },
      });
      activeRecordedSessionPath = result.relative_path;
      saveSessionNote = `Recorded chat saved: ${result.absolute_path}`;
      saveSessionNoteIsError = false;
      return true;
    } catch (error) {
      saveSessionNote = `Failed to save recorded chat: ${String(error)}`;
      saveSessionNoteIsError = true;
      return false;
    }
  }

  async function clearChat(
    reason: "clear" | "scope-change" | "search-mode-change" = "clear",
  ): Promise<boolean> {
    const canClear = await persistRecordedChatSnapshot(reason);
    if (!canClear) return false;
    turns = [];
    ctxUsedPct = 0;
    webSearchEnabled = false;
    thinkingChangePending = false;
    activeRecordedSessionPath = null;
    return true;
  }

  async function sendSelectedSessionsToBriefing(sessionIds: string[]) {
    const cleared = await clearChat("scope-change");
    if (!cleared) return;
    const count = sessionIds.length;
    resetForScopeChange({
      kind: "selected-session-ids",
      sessionIds,
      label: `Scoped to ${count} selected session${count === 1 ? "" : "s"}`,
    });
    activeTab = "briefing";
    void runBriefing({ dateFrom: "", dateTo: "", fillMin: "", fillMax: "", keyword: "" });
  }

  async function clearSelectedScope() {
    const cleared = await clearChat("scope-change");
    if (!cleared) return;
    resetForScopeChange({ kind: "archive-wide" });
    briefingStreaming = false;
  }
</script>

<!-- data-tauri-drag-region makes the top bar drag the frameless window -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="layout" oncontextmenu={onContextMenu}>
  <div class="top-bar" data-tauri-drag-region>
    <button class="brand" onclick={() => open("https://x.com/amandoulou")} title="@amandoulou on X">HALLUSCRIBE</button>
    <NavBar active={activeTab} onchange={(tab) => { activeTab = tab; }} />
    <div class="top-bar-actions">
      <WorkspaceBadge />
      {#if activeTab === "sessions"}
        <RunNowButton
          running={sweepRunning}
          progress={sweepProgress}
          toast={sweepToast}
          onRunNow={runNow}
          onStop={cancelSweep}
        />
      {/if}
      <div class="window-controls">
        <button class="window-btn" onclick={hideToTray} title="Hide to tray">−</button>
        <button class="window-btn" onclick={toggleMaximizeWindow} title={isMaximized ? "Restore" : "Maximize"}>
          {#if isMaximized}❐{:else}□{/if}
        </button>
        <button class="window-btn window-btn-close" onclick={exitApp} title="Exit">×</button>
      </div>
    </div>
  </div>

  <main class="view">
    {#if activeTab === "briefing"}
      <BriefingPanel
        {briefingAnswer}
        {briefingHeader}
        {briefingStreaming}
        {briefingError}
        {briefingWarning}
        briefingScopeLabel={scopeLabel(briefingScope)}
        selectedScopeActive={briefingScope.kind === "selected-session-ids"}
        chatScopeKind={chatScope.kind}
        {chatSearchMode}
        {chatProfileScope}
        {turns}
        {chatStreaming}
        {webSearchEnabled}
        {saveSessionEnabled}
        {saveSessionNote}
        {saveSessionNoteIsError}
        {thinkingEnabled}
        {thinkingChangePending}
        webSearchStatus={webSearchStatus()}
        imageAttachEnabled={imageAttachEnabled()}
        {speakController}
        onRefreshBriefing={(filters) => runBriefing(filters)}
        onStopBriefing={cancelBriefing}
        onSendChat={sendChat}
        onStopChat={stopChat}
        {ctxUsedPct}
        onClearChat={() => clearChat("clear")}
        onToggleWebSearch={() => {
          if (webSearchStatus() === "ready") {
            webSearchEnabled = !webSearchEnabled;
          }
        }}
        onToggleSaveSession={() => {
          const next = !saveSessionEnabled;
          saveSessionEnabled = next;
          saveSessionNote = next
            ? "Chat recording enabled for this session."
            : "Chat recording disabled.";
          saveSessionNoteIsError = false;
          if (next) {
            void persistRecordedChatSnapshot("toggle_on");
          }
        }}
        onToggleThinking={() => {
          thinkingEnabled = !thinkingEnabled;
          thinkingChangePending = chatStreaming;
        }}
        onSetChatScope={async (kind) => {
          if (chatScope.kind === kind) return;
          const cleared = await clearChat("scope-change");
          if (!cleared) return;
          chatScope = kind === "briefing-scope" ? { kind } : { kind: "archive-wide" };
        }}
        onSetChatSearchMode={async (mode) => {
          if (chatSearchMode === mode) return;
          const cleared = await clearChat("search-mode-change");
          if (!cleared) return;
          chatSearchMode = mode;
        }}
        onSelectProfileScope={(scope) => {
          chatProfileScope = scope;
        }}
        onClearScope={clearSelectedScope}
      />
    {:else if activeTab === "sessions"}
      <SessionList
        onSendToBriefing={sendSelectedSessionsToBriefing}
        bind:searchScope={sessionScope}
        bind:query={sessionQuery}
        bind:rawResult={sessionRawResult}
        bind:rawError={sessionRawError}
      />
    {:else if activeTab === "profile"}
      <ProfilePanel />
    {:else}
      <SettingsForm
        initialSettings={settingsSnapshot}
        onSaved={(settings) => { settingsSnapshot = settings; }}
      />
    {/if}
  </main>
</div>

{#if ctxVisible}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="ctx-backdrop" onclick={closeCtx} role="presentation"></div>
  <div class="ctx-menu" style="left:{ctxX}px; top:{ctxY}px;">
    <button onclick={() => { closeCtx(); void hideToTray(); }}>Hide to tray</button>
    <button onclick={() => { closeCtx(); void toggleMaximizeWindow(); }}>{isMaximized ? "Restore" : "Maximize"}</button>
    <button onclick={() => { closeCtx(); void exitApp(); }}>Exit</button>
  </div>
{/if}

<style>
  .layout {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }

  .top-bar-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    -webkit-app-region: no-drag;
  }

  .window-controls {
    display: flex;
    align-items: stretch;
    margin-right: -18px;
  }

  .window-btn {
    width: 46px;
    height: 54px;
    background: transparent;
    border: none;
    color: var(--dim);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .window-btn:hover {
    background: var(--border);
    color: var(--text);
  }

  .window-btn-close:hover {
    background: var(--red);
    color: var(--text);
  }

  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
  }

  .ctx-menu {
    position: fixed;
    z-index: 1000;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px 0;
    min-width: 130px;
    box-shadow: 0 4px 16px rgba(0,0,0,0.4);
  }

  .ctx-menu button {
    display: block;
    width: 100%;
    background: none;
    border: none;
    color: var(--text);
    font-family: inherit;
    font-size: 16px;
    text-align: left;
    padding: 10px 18px;
    cursor: pointer;
  }

  .ctx-menu button:hover { background: var(--border); }
</style>

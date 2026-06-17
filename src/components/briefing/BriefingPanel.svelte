<!-- HalluScribe - BRIEFING view. -->
<!-- All state is owned by App.svelte and passed as props so tab switches preserve it. -->
<script lang="ts">
  import "./BriefingPanel.css";
  import type {
    ChatAttachment,
    Turn,
    BriefingFilters,
    ChatScope,
    ChatSearchMode,
    WebSearchStatus,
  } from "../../lib/types";
  import BriefingFiltersBar from "./BriefingFiltersBar.svelte";
  import ChatScopeToggle from "./ChatScopeToggle.svelte";
  import ChatMessageComp from "./ChatMessage.svelte";
  import ChatInput from "./ChatInput.svelte";

  interface Props {
    briefingAnswer: string;
    briefingHeader: string;
    briefingStreaming: boolean;
    briefingError: string | null;
    briefingWarning: string | null;
    briefingScopeLabel: string;
    selectedScopeActive: boolean;
    chatScopeKind: ChatScope["kind"];
    chatSearchMode: ChatSearchMode;
    turns: Turn[];
    ctxUsedPct: number;
    chatStreaming: boolean;
    webSearchEnabled: boolean;
    saveSessionEnabled: boolean;
    saveSessionNote: string | null;
    saveSessionNoteIsError: boolean;
    thinkingEnabled: boolean;
    thinkingChangePending: boolean;
    webSearchStatus: WebSearchStatus;
    onRefreshBriefing: (filters: BriefingFilters) => void;
    onStopBriefing: () => void;
    onSendChat: (text: string, attachment: ChatAttachment | null) => void | Promise<unknown>;
    onStopChat: () => void | Promise<unknown>;
    onClearChat: () => void | Promise<unknown>;
    onToggleWebSearch: () => void;
    onToggleSaveSession: () => void;
    onToggleThinking: () => void;
    onSetChatScope: (kind: ChatScope["kind"]) => void | Promise<unknown>;
    onSetChatSearchMode: (mode: ChatSearchMode) => void | Promise<unknown>;
    onClearScope: () => void | Promise<unknown>;
    imageAttachEnabled: boolean;
  }

  let {
    briefingAnswer,
    briefingHeader,
    briefingStreaming,
    briefingError,
    briefingWarning,
    briefingScopeLabel,
    selectedScopeActive,
    chatScopeKind,
    chatSearchMode,
    turns,
    ctxUsedPct,
    chatStreaming,
    webSearchEnabled,
    saveSessionEnabled,
    saveSessionNote,
    saveSessionNoteIsError,
    thinkingEnabled,
    thinkingChangePending,
    webSearchStatus,
    onRefreshBriefing,
    onStopBriefing,
    onSendChat,
    onStopChat,
    onClearChat,
    onToggleWebSearch,
    onToggleSaveSession,
    onToggleThinking,
    onSetChatScope,
    onSetChatSearchMode,
    onClearScope,
    imageAttachEnabled,
  }: Props = $props();

  let dateFrom = $state("");
  let dateTo = $state("");
  let fillMin = $state("");
  let fillMax = $state("");
  let keyword = $state("");

  function currentFilters(): BriefingFilters {
    return { dateFrom, dateTo, fillMin, fillMax, keyword };
  }

  function stripMarkdown(text: string): string {
    return text
      .replace(/^#{1,6}\s+/gm, "")
      .replace(/\*\*([^*]+)\*\*/g, "$1");
  }

  let panelEl: HTMLDivElement | undefined;
  let briefingHeightPct = $state(45);
  let resizing = $state(false);

  function onDividerPointerDown(e: PointerEvent) {
    e.preventDefault();
    resizing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onDividerPointerMove(e: PointerEvent) {
    if (!resizing || !panelEl) return;
    const rect = panelEl.getBoundingClientRect();
    const pct = ((e.clientY - rect.top) / rect.height) * 100;
    briefingHeightPct = Math.min(80, Math.max(15, pct));
  }

  function onDividerPointerUp() {
    resizing = false;
  }

  let scrollEl: HTMLDivElement | undefined;

  $effect(() => {
    const _briefing = briefingAnswer;
    const _last = turns.at(-1)?.answerText;
    if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
  });
</script>

<div class="panel" class:resizing bind:this={panelEl}>
  <div class="briefing-zone" style="flex: 0 0 {briefingHeightPct}%">
    <BriefingFiltersBar bind:dateFrom bind:dateTo bind:fillMin bind:fillMax bind:keyword />

    <div class="briefing-header-row">
      <span class="zone-label">BRIEFING</span>
      <div class="briefing-actions">
        <button
          class="refresh-btn"
          onclick={() => onRefreshBriefing(currentFilters())}
          title="Refresh the current briefing"
          disabled={briefingStreaming}
        >
          refresh
        </button>
        {#if selectedScopeActive}
          <button class="scope-btn" onclick={onClearScope} title="Return to normal briefing">use normal briefing</button>
        {/if}
        {#if briefingStreaming}
          <button class="stop-btn" onclick={onStopBriefing} title="Stop briefing">stop</button>
        {/if}
      </div>
    </div>

    <div class="briefing-scroll">
      {#if briefingError}
        <div class="briefing-scope">{briefingScopeLabel}</div>
        <p class="error">{briefingError}</p>
      {:else if !briefingAnswer && !briefingStreaming}
        <div class="briefing-scope">{briefingScopeLabel}</div>
        {#if briefingWarning}
          <p class="warning">{briefingWarning}</p>
        {/if}
        <p class="empty">press refresh to generate briefing</p>
      {:else if briefingStreaming && !briefingAnswer}
        <div class="briefing-scope">{briefingScopeLabel}</div>
        {#if briefingWarning}
          <p class="warning">{briefingWarning}</p>
        {/if}
        <p class="loading">loading model, generating briefing<span class="dot-anim">...</span></p>
      {:else}
        <div class="briefing-scope">
          {briefingScopeLabel}{#if briefingHeader} | {briefingHeader}{/if}
        </div>
        <pre class="briefing-text selectable">{stripMarkdown(briefingAnswer)}{#if briefingStreaming}<span class="cursor">|</span>{/if}</pre>
      {/if}
    </div>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="zone-divider"
    onpointerdown={onDividerPointerDown}
    onpointermove={onDividerPointerMove}
    onpointerup={onDividerPointerUp}
    role="separator"
  ></div>

  <div class="chat-zone">
    <div class="chat-header">
      <span class="zone-label">CHAT</span>
      <div class="chat-controls">
        <div class="search-mode-group">
          <button
            class="scope-btn"
            class:active={chatSearchMode === "archive"}
            onclick={() => onSetChatSearchMode("archive")}
            title="Searches your archived Gemma-generated session summaries using the current archive search flow."
            disabled={chatStreaming}
          >
            archive chat search
          </button>
          <button
            class="scope-btn"
            class:active={chatSearchMode === "semantic"}
            onclick={() => onSetChatSearchMode("semantic")}
            title="Uses EmbeddingGemma to find the most semantically relevant archived session summaries before answering."
            disabled={chatStreaming}
          >
            semantic chat search
          </button>
        </div>
        <div class="chat-actions">
          <ChatScopeToggle {selectedScopeActive} {chatScopeKind} {chatStreaming} {onSetChatScope} />
          {#if turns.length > 0}
            <button class="clear-btn" onclick={onClearChat} title="Clear chat" disabled={chatStreaming}>clear</button>
          {/if}
        </div>
      </div>
    </div>

    <div class="chat-scroll" bind:this={scrollEl}>
      {#each turns as turn (turn)}
        <div class="turn-wrap">
          <ChatMessageComp
            role={turn.role}
            thinkingText={turn.thinkingText}
            answerText={turn.answerText}
            toolActivity={turn.toolActivity}
            streaming={turn.streaming}
            attachmentName={turn.attachmentName}
          />
        </div>
      {/each}
    </div>
    <ChatInput
      disabled={chatStreaming}
      streaming={chatStreaming}
      onsubmit={onSendChat}
      onstop={onStopChat}
      {ctxUsedPct}
      onclearchat={onClearChat}
      {webSearchEnabled}
      {saveSessionEnabled}
      {saveSessionNote}
      {saveSessionNoteIsError}
      {thinkingEnabled}
      {thinkingChangePending}
      {webSearchStatus}
      ontogglewebsearch={onToggleWebSearch}
      ontogglesavesession={onToggleSaveSession}
      ontogglethinking={onToggleThinking}
      {imageAttachEnabled}
    />
  </div>
</div>

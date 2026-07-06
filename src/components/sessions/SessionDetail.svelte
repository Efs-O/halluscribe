<!-- HalluScribe — full markdown content panel (right side of session list). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { displayFillPct, shortDate, timeFromSession } from "../../lib/format";
  import type { IndexEntry } from "../../lib/types";
  import SessionRedactPanel from "./SessionRedactPanel.svelte";

  interface Props {
    session: IndexEntry;
    onclose: () => void;
  }

  function formatTimestamp(value?: string): string {
    if (!value) return "";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return "";
    const day = date.toLocaleDateString("en-GB", {
      day: "2-digit",
      month: "short",
      year: "numeric",
      timeZone: "UTC",
    });
    const time = date.toLocaleTimeString("en-GB", {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
      timeZone: "UTC",
    });
    return `${day} ${time} UTC`;
  }

  let { session, onclose }: Props = $props();

  let content = $state<string | null>(null);
  let error = $state<string | null>(null);
  let fillText = $derived(displayFillPct(session.fill_pct, session.fill_estimated ?? false));
  let providerLabel = $derived((session.provider ?? "").replaceAll("_", " "));
  let sessionTime = $derived(timeFromSession(session.session_timestamp, session.archive_path));
  let createdLabel = $derived(`${shortDate(session.date)} ${sessionTime}`.trim());
  let updatedLabel = $derived(formatTimestamp(session.updated_at) || "—");

  let redactOpen = $state(false);

  function loadContent() {
    content = null;
    error = null;
    invoke<string>("read_session", { sessionId: session.id })
      .then((c) => { content = c; })
      .catch((e) => { error = String(e); });
  }

  $effect(() => {
    loadContent();
  });
</script>

<div class="panel">
  <div class="panel-header">
    <div class="panel-heading">
      <span class="panel-title">{session.title}</span>
      <div class="meta-row">
        <span class="meta-pill">{session.tool}</span>
        <span class="meta-pill">{session.project}</span>
        <span class="meta-pill">{fillText}</span>
        {#if providerLabel}
          <span class="meta-pill subtle">{providerLabel}</span>
        {/if}
      </div>
      <div class="meta-grid">
        <span class="meta-label">Date created</span>
        <span class="meta-value">{createdLabel}</span>
        <span class="meta-label">Last modified</span>
        <span class="meta-value">{updatedLabel}</span>
      </div>
    </div>
    <div class="header-actions">
      {#if session.secret_flags?.length}
        <span class="secret-notice">
          Possible secrets detected: {session.secret_flags.join(", ")}
        </span>
      {/if}
      <button class="redact-toggle-btn btn" onclick={() => (redactOpen = !redactOpen)}>Redact…</button>
      <button class="close-btn btn" onclick={onclose}>× close</button>
    </div>
  </div>
  <SessionRedactPanel
    sessionId={session.id}
    bind:open={redactOpen}
    onApplied={loadContent}
  />
  <div class="panel-body selectable">
    {#if error}
      <p class="err">{error}</p>
    {:else if content === null}
      <p class="loading">loading…</p>
    {:else}
      <pre class="md-content">{content}</pre>
    {/if}
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    border-left: 1px solid var(--border);
    background: var(--bg);
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    gap: 12px;
  }

  .panel-heading {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 8px;
  }

  .panel-title {
    font-size: 14px;
    font-weight: 700;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .meta-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    font-size: 12px;
    color: var(--muted);
  }

  .meta-label {
    color: var(--dim);
    white-space: nowrap;
  }

  .meta-value {
    color: var(--muted);
  }

  .meta-pill {
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--muted);
    font-size: 12px;
    line-height: 1;
    padding: 5px 8px;
    white-space: nowrap;
  }

  .meta-pill.subtle {
    color: var(--dim);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .close-btn, .redact-toggle-btn { font-size: 13px; }

  .secret-notice {
    color: var(--amber);
    font-size: 12px;
    white-space: nowrap;
  }

  .panel-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
  }

  .md-content {
    font-family: 'Courier New', Courier, monospace;
    font-size: 13px;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.65;
  }

  .loading, .err { font-size: 14px; color: var(--muted); }
  .err { color: var(--red); }
</style>

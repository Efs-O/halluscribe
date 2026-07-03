<!-- HalluScribe — PROFILE view: distilled profile.md + latest weekly digest. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import type { ProfileDonePayload, ProfileProgressPayload } from "../../lib/types";

  let profile = $state<string | null>(null);
  let digest = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);

  let running = $state(false);
  let progress = $state<ProfileProgressPayload | null>(null);
  let resultNote = $state<string | null>(null);
  let resultIsError = $state(false);
  let confirmingFullRebuild = $state(false);
  let digestOpen = $state(false);

  let unlistenFns: UnlistenFn[] = [];

  function stageLabel(p: ProfileProgressPayload): string {
    if (p.stage === "merging") return "Merging…";
    if (p.stage === "writing") return "Writing…";
    return `Mapping batch ${p.current} of ${p.total}…`;
  }

  function firstLine(text: string): string {
    return text.split("\n", 1)[0] ?? "";
  }

  function restOfFile(text: string): string {
    const idx = text.indexOf("\n");
    return idx === -1 ? "" : text.slice(idx + 1).replace(/^\n+/, "");
  }

  async function loadProfile() {
    loading = true;
    loadError = null;
    try {
      profile = await invoke<string | null>("get_profile");
      digest = await invoke<string | null>("get_latest_digest");
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  }

  async function startRefresh(full: boolean) {
    if (running) return;
    confirmingFullRebuild = false;
    running = true;
    progress = null;
    resultNote = null;
    resultIsError = false;
    try {
      await invoke("run_profile_refresh", { full });
    } catch (e) {
      running = false;
      resultNote = String(e);
      resultIsError = true;
    }
  }

  function requestFullRebuild() {
    if (running) return;
    confirmingFullRebuild = true;
  }

  onMount(async () => {
    await loadProfile();

    unlistenFns.push(
      await listen<ProfileProgressPayload>("profile-progress", (ev) => {
        running = true;
        progress = ev.payload;
      }),
    );

    unlistenFns.push(
      await listen<ProfileDonePayload>("profile-done", (ev) => {
        running = false;
        progress = null;
        const payload = ev.payload;
        if (payload.busy) {
          resultNote = "Another model job is running — try again later.";
          resultIsError = true;
          return;
        }
        if (payload.errors.length > 0) {
          resultNote = `Distilled ${payload.session_count} sessions → ${payload.facts_count} facts, with errors: ${payload.errors.join("; ")}`;
          resultIsError = true;
        } else {
          resultNote = `Distilled ${payload.session_count} sessions → ${payload.facts_count} facts.`;
          resultIsError = false;
        }
        void loadProfile();
      }),
    );
  });

  onDestroy(() => {
    for (const unlisten of unlistenFns) unlisten();
  });
</script>

<div class="panel">
  <div class="panel-header">
    <span class="zone-label">PROFILE</span>
    <div class="panel-actions">
      {#if profile}
        <button class="btn" onclick={() => startRefresh(false)} disabled={running} title="Distill only sessions newer than the last run">
          Refresh
        </button>
      {/if}
      {#if !confirmingFullRebuild}
        <button class="btn" onclick={requestFullRebuild} disabled={running} title="Re-read the whole archive — can take a long time on large archives">
          Full rebuild
        </button>
      {:else}
        <span class="confirm-row">
          <span class="confirm-text">Re-read the whole archive? This can take a long time.</span>
          <button class="btn-primary" onclick={() => startRefresh(true)}>Confirm</button>
          <button class="btn" onclick={() => (confirmingFullRebuild = false)}>Cancel</button>
        </span>
      {/if}
    </div>
  </div>

  {#if running}
    <div class="progress-row">
      <div class="progress-bar-wrap">
        <div
          class="progress-bar-fill"
          style="width: {progress && progress.total > 0 ? (progress.current / progress.total) * 100 : 0}%"
        ></div>
      </div>
      <span class="progress-label">{progress ? stageLabel(progress) : "Starting…"}</span>
    </div>
  {/if}

  {#if resultNote}
    <p class="note" class:err={resultIsError}>{resultNote}</p>
  {/if}

  <div class="panel-body selectable">
    {#if loading}
      <p class="loading">loading…</p>
    {:else if loadError}
      <p class="err">{loadError}</p>
    {:else if !profile}
      <div class="empty-state">
        <p>No profile has been built yet — run a full build to distill facts from your archived sessions into a persistent profile.</p>
        <button class="btn-primary" onclick={requestFullRebuild} disabled={running}>Build profile</button>
        {#if confirmingFullRebuild}
          <div class="confirm-row">
            <span class="confirm-text">Re-read the whole archive? This can take a long time.</span>
            <button class="btn-primary" onclick={() => startRefresh(true)}>Confirm</button>
            <button class="btn" onclick={() => (confirmingFullRebuild = false)}>Cancel</button>
          </div>
        {/if}
      </div>
    {:else}
      <div class="generated-line">{firstLine(profile)}</div>
      <pre class="md-content">{restOfFile(profile)}</pre>

      <button class="digest-toggle" onclick={() => (digestOpen = !digestOpen)}>
        {digestOpen ? "▾" : "▸"} Latest digest
      </button>
      {#if digestOpen}
        {#if digest}
          <pre class="md-content digest-content">{digest}</pre>
        {:else}
          <p class="empty">No weekly digest has been generated yet.</p>
        {/if}
      {/if}
    {/if}
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg);
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }

  .zone-label {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--dim);
  }

  .panel-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .confirm-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .confirm-text {
    font-size: 12px;
    color: var(--amber);
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .progress-bar-wrap {
    position: relative;
    width: 200px;
    height: 16px;
    background: var(--border);
    border-radius: 4px;
    overflow: hidden;
    flex-shrink: 0;
  }

  .progress-bar-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--green);
    border-radius: 4px;
    transition: width 0.3s ease;
  }

  .progress-label {
    font-size: 12px;
    color: var(--muted);
  }

  .note {
    font-size: 12px;
    color: var(--muted);
    padding: 6px 16px;
    margin: 0;
    border-bottom: 1px solid var(--border);
  }

  .note.err { color: var(--red); }

  .panel-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    max-width: 520px;
    color: var(--muted);
    font-size: 14px;
  }

  .generated-line {
    font-size: 12px;
    color: var(--dim);
    margin-bottom: 10px;
  }

  .md-content {
    font-family: 'Courier New', Courier, monospace;
    font-size: 13px;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.65;
  }

  .digest-toggle {
    background: none;
    border: none;
    border-top: 1px solid var(--border);
    color: var(--muted);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    margin-top: 16px;
    padding: 10px 0;
    text-align: left;
    width: 100%;
  }

  .digest-toggle:hover { color: var(--text); }

  .digest-content {
    padding-top: 4px;
  }

  .loading, .err { font-size: 14px; color: var(--muted); }
  .err { color: var(--red); }
  .empty { font-size: 13px; color: var(--muted); }
</style>

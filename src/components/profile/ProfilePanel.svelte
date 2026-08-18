<!-- HalluScribe — PROFILE view: distilled profile.md + latest weekly digest,
     per scope (Work / Personal — Persona Protocol Phase 2c). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { firstLine, restOfFile, stageLabel } from "../../lib/profile";
  import type { ProfileDonePayload, ProfileProgressPayload, ProfileScope } from "../../lib/types";
  import ProfileRunNote from "./ProfileRunNote.svelte";
  import ProfileScopeTabs from "./ProfileScopeTabs.svelte";

  let scope = $state<ProfileScope>("work");
  let profile = $state<string | null>(null);
  let digest = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);

  // The scope currently holding the shared inference lock, or null when idle.
  // Buttons disable off this (the lock is global — only one job runs at a
  // time across both scopes), independent of which scope is being viewed.
  let busyScope = $state<ProfileScope | null>(null);
  let running = $derived(busyScope !== null);
  // Progress only renders for the scope currently being viewed.
  let progress = $state<ProfileProgressPayload | null>(null);
  // Label shown in the progress area before the first profile-progress event
  // arrives (fresh start vs. remounting into an already-running refresh).
  let progressPlaceholder = $state("Starting…");
  let resultNote = $state<string | null>(null);
  // Per-item warnings from the last run, shown collapsed under the headline.
  let resultDetail = $state<string[]>([]);
  // True only when the run actually failed (`failed_batches > 0`) — warnings
  // alone must never render as a failure.
  let resultIsError = $state(false);
  let confirmingFullRebuild = $state(false);
  // A stop has been signalled and the run has not reported back yet — keeps
  // the stop button from being pressed twice.
  let stopping = $state(false);
  let digestOpen = $state(false);
  // Profile bodies (Work and Personal both) stay behind an explicit reveal so
  // they never just sit on screen. Re-hidden every time the user enters a scope
  // or a build finishes.
  let profileRevealed = $state(false);

  // Persona Pack export — per scope (Persona Parity Phase B): both Work and
  // Personal panels export their own pack; the user chooses which scope's
  // profile + consented archive leaves the machine on each export.
  let includeRaw = $state(false);
  // How many of this scope's sessions have a preserved raw transcript on disk —
  // raw copies are only captured on new sweeps, so this is 0 until the first
  // post-Phase-1 sweep runs. Drives the "incl. raw (N available)" hint and
  // disables the box.
  let rawAvailable = $state(0);
  let exporting = $state(false);
  let exportNote = $state<string | null>(null);
  let exportIsError = $state(false);
  // The export options live in a popover anchored to the Export Pack button, so
  // the "incl. raw" choice only appears when the user is actually exporting.
  let exportOpen = $state(false);

  let unlistenFns: UnlistenFn[] = [];

  async function loadProfile() {
    loading = true;
    loadError = null;
    try {
      profile = await invoke<string | null>("get_profile", { scope });
      digest = await invoke<string | null>("get_latest_digest", { scope });
      rawAvailable = await invoke<number>("count_available_raw", { scope });
      // Never ship raw the export can't actually find (count went to 0).
      if (rawAvailable === 0) includeRaw = false;
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  }

  function selectScope(next: ProfileScope) {
    if (scope === next) return;
    scope = next;
    progress = null;
    resultNote = null;
    resultDetail = [];
    resultIsError = false;
    confirmingFullRebuild = false;
    digestOpen = false;
    profileRevealed = false;
    exportOpen = false;
    exportNote = null;
    exportIsError = false;
    void loadProfile();
  }

  async function exportPack() {
    if (exporting || running) return;
    exporting = true;
    exportNote = null;
    exportIsError = false;
    try {
      const r = await invoke<{
        path: string;
        session_count: number;
        digest_count: number;
        raw_count: number;
        includes_raw: boolean;
      }>("export_persona_pack", { includeRaw, scope });
      exportNote = `Exported ${r.session_count} sessions${r.includes_raw ? ` + ${r.raw_count} raw` : ""} → ${r.path}`;
      exportIsError = false;
    } catch (e) {
      exportNote = String(e);
      exportIsError = true;
    } finally {
      exporting = false;
      exportOpen = false;
    }
  }

  async function startRefresh(full: boolean) {
    if (running) return;
    confirmingFullRebuild = false;
    busyScope = scope;
    progress = null;
    progressPlaceholder = "Starting…";
    resultNote = null;
    resultDetail = [];
    resultIsError = false;
    stopping = false;
    try {
      await invoke("run_profile_refresh", { full, scope });
    } catch (e) {
      busyScope = null;
      resultNote = String(e);
      resultDetail = [];
      resultIsError = true;
    }
  }

  // Stop at the next safe boundary — the end of the current map batch or
  // reduce call. Mirrors the sweep's stop button; never kills a model call.
  async function stopRefresh() {
    if (!running) return;
    stopping = true;
    resultNote = "Stopping — finishing the current step…";
    resultDetail = [];
    resultIsError = false;
    try {
      await invoke("cancel_profile_refresh");
    } catch (e) {
      resultNote = String(e);
      resultIsError = true;
      stopping = false;
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
        busyScope = ev.payload.scope;
        if (ev.payload.scope === scope) {
          progress = ev.payload;
        }
      }),
    );

    unlistenFns.push(
      await listen<ProfileDonePayload>("profile-done", (ev) => {
        const payload = ev.payload;
        // The lock is released regardless of which scope is being viewed.
        busyScope = null;
        stopping = false;
        if (payload.scope !== scope) return;
        progress = null;
        resultDetail = payload.errors;
        if (payload.busy) {
          resultNote = "Another model job is running — try again later.";
          resultDetail = [];
          resultIsError = true;
          return;
        }
        if (payload.cancelled) {
          // Stopping is a user action, not a failure: partial mapping work is
          // on disk and the next run picks up from it.
          resultIsError = false;
          resultNote =
            "Stopped — partial work was saved, and the next run resumes from it.";
          profileRevealed = false;
          void loadProfile();
          return;
        }
        resultIsError = payload.failed_batches > 0;
        if (resultIsError) {
          const batches = payload.failed_batches === 1 ? "batch" : "batches";
          resultNote = `Build did not complete — ${payload.failed_batches} ${batches} failed, so the profile was not updated.`;
        } else if (payload.session_count === 0) {
          // Not a failure and not a success: nothing in the archive matched
          // this scope's consented sources, so there was nothing to distill.
          resultNote =
            "No sessions matched this profile's sources, so nothing was distilled.";
        } else {
          const skipped = payload.errors.length;
          resultNote =
            `Distilled ${payload.session_count} sessions → ${payload.facts_count} facts.` +
            (skipped > 0 ? ` ${skipped} ${skipped === 1 ? "item" : "items"} skipped.` : "");
        }
        // A finished build must not auto-expose the refreshed profile.
        profileRevealed = false;
        void loadProfile();
      }),
    );

    // The refresh runs in a detached thread and this component is destroyed
    // on tab switch: ask the backend whether a run is still in flight so the
    // remounted panel shows the busy state instead of looking idle.
    try {
      const refreshing = await invoke<ProfileScope | null>("get_profile_refresh_status");
      if (refreshing !== null) {
        busyScope = refreshing;
        progressPlaceholder = "Refresh in progress…";
      }
    } catch {
      // Status probe is best-effort; the profile-progress events still arrive.
    }
  });

  onDestroy(() => {
    for (const unlisten of unlistenFns) unlisten();
  });
</script>

<div class="panel">
  <ProfileScopeTabs active={scope} onselect={selectScope} />

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
      {#if profile}
        <span class="action-divider" aria-hidden="true"></span>
        <div class="export-wrap">
          <button
            class="btn"
            onclick={() => (exportOpen = !exportOpen)}
            disabled={exporting || running}
            title="Export this profile + archive as a shareable Persona Pack zip"
          >
            {exporting ? "Exporting…" : "Export Pack ▾"}
          </button>
          {#if exportOpen}
            <div class="export-popover">
              <label
                class="raw-toggle"
                class:disabled={rawAvailable === 0}
                title={rawAvailable === 0
                  ? "No raw transcripts preserved yet — raw copies are captured only on new sweeps."
                  : "Raw transcripts are the un-redacted source — only include when you trust the recipient."}
              >
                <input type="checkbox" bind:checked={includeRaw} disabled={exporting || rawAvailable === 0} />
                <span>incl. raw ({rawAvailable} available)</span>
              </label>
              <button class="btn-primary" onclick={exportPack} disabled={exporting}>
                {exporting ? "Exporting…" : "Export"}
              </button>
            </div>
          {/if}
        </div>
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
      <span class="progress-label">{progress ? stageLabel(progress) : progressPlaceholder}</span>
      <button
        class="btn-stop"
        onclick={stopRefresh}
        disabled={stopping}
        title="Stop after the current step — partial work is kept and the next run resumes from it"
      >
        {stopping ? "stopping…" : "stop"}
      </button>
    </div>
  {/if}

  {#if resultNote}
    <ProfileRunNote note={resultNote} detail={resultDetail} isError={resultIsError} />
  {/if}

  {#if exportNote}
    <p class="note" class:err={exportIsError}>{exportNote}</p>
  {/if}

  <div class="panel-body selectable">
    {#if loading}
      <p class="loading">loading…</p>
    {:else if loadError}
      <p class="err">{loadError}</p>
    {:else if !profile}
      <div class="empty-state">
        {#if resultIsError}
          <p>The last build did not complete, so nothing was written — see the message above.</p>
        {:else}
          <p>No profile has been built yet — run a full build to distill facts from your archived sessions into a persistent profile.</p>
        {/if}
        {#if scope === "personal"}
          <p class="hint">Personal includes chat exports (ChatGPT/Claude.ai/Gemini/Grok), in addition to your coding tools.</p>
        {/if}
        <button class="btn-primary" onclick={requestFullRebuild} disabled={running}>Build profile</button>
        {#if confirmingFullRebuild}
          <div class="confirm-row">
            <span class="confirm-text">Re-read the whole archive? This can take a long time.</span>
            <button class="btn-primary" onclick={() => startRefresh(true)}>Confirm</button>
            <button class="btn" onclick={() => (confirmingFullRebuild = false)}>Cancel</button>
          </div>
        {/if}
      </div>
    {:else if !profileRevealed}
      <div class="reveal-gate">
        <p class="reveal-text">
          {scope === "personal"
            ? "Personal context is hidden — this profile carries private life details."
            : "Profile is hidden."}
        </p>
        <button class="btn-primary" onclick={() => (profileRevealed = true)}>
          ▸ Show {scope === "personal" ? "personal" : "work"} profile
        </button>
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

  .action-divider {
    width: 1px;
    height: 20px;
    background: var(--border);
    margin: 0 4px;
    flex-shrink: 0;
  }

  .export-wrap {
    position: relative;
    display: inline-flex;
  }

  .export-popover {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    padding: 12px;
    min-width: 200px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  }

  .export-popover .btn-primary {
    align-self: flex-end;
  }

  .raw-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--dim);
    cursor: pointer;
  }

  .raw-toggle.disabled {
    opacity: 0.5;
    cursor: default;
  }

  .confirm-text {
    font-size: 12px;
    color: var(--amber);
  }

  /* Copied verbatim from RunNowButton.svelte so the sweep's stop button and
     this one are visually identical. Promote to a shared component if a third
     stop button ever appears. */
  .btn-stop {
    white-space: nowrap;
    background: transparent;
    border: 1px solid var(--amber, #f59e0b);
    border-radius: 4px;
    color: var(--amber, #f59e0b);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    padding: 6px 14px;
    transition: background 0.1s, color 0.1s;
    -webkit-app-region: no-drag;
  }
  .btn-stop:hover { background: var(--amber, #f59e0b); color: #000; }
  .btn-stop:disabled { cursor: default; opacity: 0.6; }
  .btn-stop:disabled:hover { background: transparent; color: var(--amber, #f59e0b); }

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

  .empty-state .hint {
    font-size: 12px;
    color: var(--dim);
    margin: -6px 0 0;
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

  .reveal-gate {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    max-width: 520px;
    padding: 20px;
    border: 1px dashed var(--border);
    border-radius: 6px;
  }

  .reveal-text {
    font-size: 13px;
    color: var(--dim);
    margin: 0;
  }

  .loading, .err { font-size: 14px; color: var(--muted); }
  .err { color: var(--red); }
  .empty { font-size: 13px; color: var(--muted); }
</style>

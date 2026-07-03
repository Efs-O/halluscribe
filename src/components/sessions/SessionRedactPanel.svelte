<!-- HalluScribe - deterministic redaction UI for one archived session (Persona Phase 0). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { RedactionOutcome, RedactionPreview } from "../../lib/types";

  interface Props {
    sessionId: string;
    open: boolean;
    onApplied: () => void;
  }

  let { sessionId, open = $bindable(false), onApplied }: Props = $props();

  let find = $state("");
  let replace = $state("");
  let preview = $state<RedactionPreview | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let resultMessage = $state<string | null>(null);

  function resetFields() {
    find = "";
    replace = "";
    preview = null;
    error = null;
  }

  // Clear the working fields whenever the panel is closed, and clear any
  // stale success banner whenever it is reopened.
  $effect(() => {
    if (!open) {
      resetFields();
    } else {
      resultMessage = null;
    }
  });

  async function previewRedaction() {
    error = null;
    resultMessage = null;
    preview = null;
    if (!find.trim()) {
      error = "Enter the text to redact.";
      return;
    }
    busy = true;
    try {
      preview = await invoke<RedactionPreview>("preview_redaction", {
        sessionId,
        find,
      });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function applyRedaction() {
    error = null;
    busy = true;
    try {
      const outcome = await invoke<RedactionOutcome>("apply_redaction", {
        sessionId,
        find,
        replace,
      });
      resultMessage = `${outcome.replacements} replacement(s) made, backup saved to ${outcome.backup_path}`;
      resetFields();
      open = false;
      onApplied();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if open}
  <div class="redact-panel">
    <label class="redact-label" for="redact-find">Text to redact</label>
    <input
      id="redact-find"
      class="redact-input"
      type="text"
      placeholder="Text to find (min. 4 characters)"
      bind:value={find}
    />
    <label class="redact-label" for="redact-replace">Replacement (optional)</label>
    <input
      id="redact-replace"
      class="redact-input"
      type="text"
      placeholder="[REDACTED]"
      bind:value={replace}
    />
    <div class="redact-actions">
      <button class="btn" onclick={previewRedaction} disabled={busy}>Preview</button>
      {#if preview && preview.occurrences > 0}
        <button class="redact-apply-btn" onclick={applyRedaction} disabled={busy}>
          Apply redaction
        </button>
      {/if}
    </div>
    {#if error}
      <p class="err">{error}</p>
    {/if}
    {#if preview}
      <p class="redact-count">{preview.occurrences} occurrence(s) found</p>
      {#if preview.excerpts.length > 0}
        <pre class="redact-excerpts">{preview.excerpts.join("\n---\n")}</pre>
      {/if}
    {/if}
  </div>
{/if}
{#if resultMessage}
  <p class="redact-result">{resultMessage}</p>
{/if}

<style>
  .redact-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }

  .redact-label {
    font-size: 12px;
    color: var(--dim);
  }

  .redact-input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
    padding: 7px 10px;
  }

  .redact-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .redact-apply-btn {
    background: transparent;
    border: 1px solid #ef4444;
    border-radius: 4px;
    color: #ef4444;
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    padding: 7px 12px;
    transition: background 0.1s, color 0.1s;
  }

  .redact-apply-btn:hover {
    background: #ef4444;
    color: #fff;
  }

  .redact-count {
    font-size: 12px;
    color: var(--muted);
  }

  .redact-excerpts {
    font-family: 'Courier New', Courier, monospace;
    font-size: 12px;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.5;
    max-height: 200px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 8px 10px;
    background: var(--bg);
  }

  .redact-result {
    font-size: 12px;
    color: var(--muted);
    padding: 6px 16px;
    margin: 0;
    border-bottom: 1px solid var(--border);
  }

  .err { font-size: 14px; color: var(--red); }
</style>

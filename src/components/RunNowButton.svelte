<!-- HalluScribe - "run now" / "stop" button with progress bar and toast. -->
<!-- State is owned by App.svelte and passed as props so it survives tab switches. -->
<script lang="ts">
  import type { SweepProgress } from "../lib/types";

  interface Props {
    running: boolean;
    progress: SweepProgress | null;
    toast: { msg: string; ok: boolean } | null;
    onRunNow: () => void;
    onStop: () => void;
    onDismissToast: () => void;
  }
  let { running, progress, toast, onRunNow, onStop, onDismissToast }: Props = $props();
</script>

{#if running}
  <button class="btn-stop" onclick={onStop} title="Stop sweep after current session">stop</button>
{:else}
  <button class="btn-primary run-btn" onclick={onRunNow}>run now</button>
{/if}

{#if progress && running}
  <div class="progress-bar-wrap">
    <div
      class="progress-bar-fill"
      style="width: {progress.total > 0 ? (progress.current / progress.total) * 100 : 0}%"
    ></div>
    <span class="progress-label">
      {progress.current} / {progress.total}
      {#if progress.status === "processing"}· generating...{/if}
      {#if progress.status.startsWith("chunk ")}· {progress.status}{/if}
      {#if progress.status === "skipped"}· skip{/if}
      {#if progress.status === "error"}· err{/if}
    </span>
  </div>
{/if}

{#if toast}
  <div class="toast {toast.ok ? 'success' : 'error'}">
    <span>{toast.msg}</span>
    <button class="toast-dismiss" onclick={onDismissToast} aria-label="Dismiss sweep result">×</button>
  </div>
{/if}

<style>
  .run-btn {
    white-space: nowrap;
  }

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

  .toast-dismiss {
    align-self: flex-start;
    background: transparent;
    border: 0;
    color: currentColor;
    cursor: pointer;
    font: inherit;
    font-size: 18px;
    line-height: 1;
    padding: 0 0 0 10px;
  }

  .progress-bar-wrap {
    position: relative;
    width: 180px;
    height: 18px;
    background: var(--border);
    border-radius: 4px;
    overflow: hidden;
    flex-shrink: 0;
  }

  .progress-bar-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent, #1d4ed8);
    border-radius: 4px;
    transition: width 0.3s ease;
  }

  .progress-label {
    position: relative;
    z-index: 1;
    font-size: 11px;
    color: var(--text);
    padding: 0 6px;
    line-height: 18px;
    white-space: nowrap;
  }
</style>

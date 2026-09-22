<!-- HalluScribe - sweep progress bar (current / total + per-session status). -->
<!-- Shared by the header's run-now button and the business import section, so a -->
<!-- business import shows the same bar as the sweep it runs. -->
<script lang="ts">
  import type { SweepProgress } from "../lib/types";

  interface Props {
    progress: SweepProgress;
  }
  let { progress }: Props = $props();
</script>

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

<style>
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

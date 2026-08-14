<!-- HalluScribe - small, non-blocking status line for the startup raw capture pass. -->
<!-- Polls get_capture_status on mount (so a late mount can rediscover an in-flight -->
<!-- or already-finished pass) and listens for raw-capture-progress events. Silent -->
<!-- when idle or when a finished pass captured nothing new. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { CaptureStatus } from "../lib/types";

  let status = $state<CaptureStatus>({ state: "idle" });

  function visible(current: CaptureStatus): boolean {
    if (current.state === "running") return true;
    if (current.state === "done") return current.captured > 0;
    if (current.state === "failed") return true;
    return false;
  }

  function label(current: CaptureStatus): string {
    if (current.state === "running") {
      const failures = current.failed > 0 ? `, ${current.failed} failed` : "";
      return `preserving raw sessions - ${current.done} / ${current.total} (${current.captured} captured${failures})`;
    }
    if (current.state === "done") {
      return `raw capture done - ${current.captured} session${current.captured === 1 ? "" : "s"} preserved`;
    }
    if (current.state === "failed") {
      return `raw capture incomplete - ${current.captured} preserved, ${current.errors.length} failure${current.errors.length === 1 ? "" : "s"}`;
    }
    return "";
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        status = await invoke<CaptureStatus>("get_capture_status");
      } catch {
        // Non-critical status line - leave it hidden if the command fails.
      }
      unlisten = await listen<CaptureStatus>("raw-capture-progress", (ev) => {
        status = ev.payload;
      });
    })();

    return () => {
      unlisten?.();
    };
  });
</script>

{#if visible(status)}
  <span
    class="capture-status"
    title={status.state === "failed" ? status.errors.join("\n") : "Startup raw capture: preserves coding-tool session files before they get pruned"}
  >
    {label(status)}
  </span>
{/if}

<style>
  .capture-status {
    font-size: 11px;
    color: var(--text-dim, #9ca3af);
    white-space: nowrap;
  }
</style>

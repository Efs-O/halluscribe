<!-- HalluScribe - one-line aggregate stats header for SESSION SUMMARY view. -->
<script lang="ts">
  import type { SessionStats } from "../../lib/types";

  interface Props {
    stats: SessionStats | null;
  }
  let { stats }: Props = $props();
</script>

<div class="strip">
  {#if stats}
    <span class="stat">{stats.total} archived</span>
    <span class="sep">/</span>
    <span class="stat muted">{stats.raw_total ?? "counting..."} on disk</span>
    {#each Object.entries(stats.by_tool).sort((a, b) => b[1] - a[1]) as [tool, count]}
      <span class="sep">·</span>
      <span class="stat">{tool}: {count}</span>
    {/each}
  {:else}
    <span class="stat muted">loading stats...</span>
  {/if}
</div>

<style>
  .strip {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 10px 16px;
    font-size: 15px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }

  .stat { color: var(--text); }
  .stat.muted { color: var(--muted); }
  .sep { color: var(--dim); }
</style>

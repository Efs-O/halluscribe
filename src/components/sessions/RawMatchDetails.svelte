<!-- HalluScribe - inline excerpt list for one session's raw transcript matches. -->
<script lang="ts">
  import type { RawSessionMatches } from "../../lib/types";

  interface Props {
    matches: RawSessionMatches;
  }

  let { matches }: Props = $props();
</script>

<div class="raw-details" role="region" aria-label="Raw transcript matches">
  {#each matches.excerpts as hit (hit.line_no)}
    <div class="excerpt-row">
      <span class="line-no">L{hit.line_no}</span>
      <span class="excerpt">{hit.excerpt}</span>
    </div>
  {/each}
  {#if matches.excerpts_truncated}
    <p class="truncation">showing first {matches.excerpts.length} of {matches.total_hits} hits</p>
  {/if}
</div>

<style>
  .raw-details {
    padding: 6px 16px 10px 54px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .excerpt-row {
    display: flex;
    gap: 10px;
    align-items: baseline;
    padding: 2px 0;
  }

  .line-no {
    flex-shrink: 0;
    min-width: 52px;
    color: var(--dim);
    font-family: monospace;
    font-size: 11px;
    text-align: right;
  }

  .excerpt {
    font-family: monospace;
    font-size: 12px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .truncation {
    margin: 4px 0 0;
    color: var(--dim);
    font-size: 12px;
    font-style: italic;
  }
</style>

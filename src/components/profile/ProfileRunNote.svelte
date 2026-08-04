<!-- HalluScribe — PROFILE view: the result line for one refresh run. Warnings
     stay collapsed behind a toggle so a successful run that merely skipped a
     few facts never reads as a failure (it did: a 374-session run on
     2026-08-04 painted a wall of red over a profile that wrote fine). Kept as
     its own component to keep ProfilePanel.svelte closer to the 350-LOC cap. -->
<script lang="ts">
  interface Props {
    /** One-line headline: what the run did, or how it failed. */
    note: string;
    /** Per-item warnings/errors; empty when the run was entirely clean. */
    detail?: string[];
    /** True only when the run actually failed — never for warnings alone. */
    isError?: boolean;
  }
  let { note, detail = [], isError = false }: Props = $props();

  let open = $state(false);
</script>

<div class="run-note" class:err={isError}>
  <p class="note-text">{note}</p>
  {#if detail.length > 0}
    <button class="detail-toggle" onclick={() => (open = !open)}>
      {open ? "▾" : "▸"}
      {detail.length}
      {detail.length === 1 ? "detail" : "details"}
    </button>
    {#if open}
      <ul class="detail-list">
        {#each detail as line}
          <li>{line}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .run-note {
    padding: 6px 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .note-text {
    font-size: 12px;
    color: var(--muted);
    margin: 0;
  }

  .run-note.err .note-text {
    color: var(--red);
  }

  .detail-toggle {
    background: none;
    border: none;
    color: var(--dim);
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    padding: 4px 0 0;
    text-align: left;
  }

  .detail-toggle:hover {
    color: var(--text);
  }

  .detail-list {
    margin: 4px 0 2px;
    padding-left: 18px;
    max-height: 180px;
    overflow-y: auto;
  }

  .detail-list li {
    font-size: 11px;
    color: var(--dim);
    line-height: 1.5;
    word-break: break-word;
  }
</style>

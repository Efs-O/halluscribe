<!-- HalluScribe - bulk actions for selected sessions. -->
<script lang="ts">
  interface Props {
    count: number;
    onSendToBriefing: () => void;
    onClearSelection: () => void;
    onDelete: () => void;
  }

  let { count, onSendToBriefing, onClearSelection, onDelete }: Props = $props();
</script>

<div class="bulk-bar">
  <span class="bulk-count">{count} selected</span>
  <button class="btn-send" onclick={onSendToBriefing}>Send To Briefing</button>
  <button class="btn-clear" onclick={onClearSelection}>Clear Selection</button>
  <button class="btn-delete" onclick={onDelete}>Delete</button>
  {#if count > 30}
    <span class="bulk-warning">Large selection: current context size may not fit all selected sessions cleanly.</span>
  {:else if count > 15}
    <span class="bulk-warning">Large selection: briefing detail will be compressed to stay readable.</span>
  {/if}
</div>

<style>
  .bulk-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 70%, transparent);
    flex-shrink: 0;
    flex-wrap: wrap;
  }

  .bulk-count {
    font-size: 13px;
    color: var(--amber);
    flex-shrink: 0;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .btn-send,
  .btn-clear,
  .btn-delete {
    background: transparent;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    padding: 7px 12px;
    flex-shrink: 0;
    transition: background 0.1s, color 0.1s, border-color 0.1s;
  }

  .btn-send {
    border: 1px solid var(--amber);
    color: var(--amber);
  }

  .btn-send:hover {
    background: color-mix(in srgb, var(--amber) 14%, transparent);
  }

  .btn-clear {
    border: 1px solid var(--border);
    color: var(--dim);
  }

  .btn-clear:hover {
    border-color: var(--text);
    color: var(--text);
  }

  .btn-delete {
    border: 1px solid #ef4444;
    color: #ef4444;
  }

  .btn-delete:hover {
    background: #ef4444;
    color: #fff;
  }

  .bulk-warning {
    font-size: 13px;
    color: var(--amber);
  }
</style>

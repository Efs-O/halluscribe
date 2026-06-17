<!-- HalluScribe — collapsible reasoning bubble (AdvanLLM pattern). -->
<!-- Collapsed by default while streaming; user can expand/collapse at any time. -->
<script lang="ts">
  interface Props {
    text: string;
    streaming: boolean;
  }
  let { text, streaming }: Props = $props();

  let expanded = $state(false);
  let bodyEl = $state<HTMLDivElement | undefined>(undefined);

  $effect(() => {
    const _text = text;
    if (!expanded || !streaming || !bodyEl) return;
    bodyEl.scrollTop = bodyEl.scrollHeight;
  });
</script>

{#if text}
  <div class="bubble">
    <button class="bubble-header" onclick={() => { expanded = !expanded; }}>
      <span class="arrow">{expanded ? "▼" : "▶"}</span>
      <span class="label">
        {streaming ? "reasoning…" : "reasoning"}
      </span>
      <span class="chars">{text.length} chars</span>
    </button>
    {#if expanded}
      <div class="bubble-body selectable" bind:this={bodyEl}>
        <pre class="think-text">{text}</pre>
      </div>
    {/if}
  </div>
{/if}

<style>
  .bubble {
    display: inline-block;
    width: fit-content;
    max-width: min(82%, 960px);
    border: 1px solid var(--border);
    border-radius: 5px;
    margin-bottom: 10px;
    overflow: hidden;
  }

  .bubble-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: var(--surface);
    border: none;
    color: var(--muted);
    font-family: inherit;
    font-size: 13px;
    padding: 5px 10px;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }

  .bubble-header:hover { background: var(--border); }

  .arrow { font-size: 11px; }
  .label { flex: 1; }
  .chars { color: var(--dim); font-size: 12px; }

  .bubble-body { background: var(--bg); padding: 8px 10px; max-height: 240px; overflow-y: auto; }

  .think-text {
    font-family: 'Courier New', Courier, monospace;
    font-size: 16px;
    color: var(--text);
    white-space: pre-wrap;
    word-break: normal;
    overflow-wrap: break-word;
    line-height: 1.7;
  }
</style>

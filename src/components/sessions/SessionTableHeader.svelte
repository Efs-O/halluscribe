<!-- HalluScribe - sortable session table header with resizable dividers. -->
<script lang="ts">
  import type { SessionSortKey } from "../../lib/search";

  interface Props {
    sortLabel: (key: SessionSortKey, label: string) => string;
    toggleSort: (key: SessionSortKey) => void;
    onDividerMouseDown: (
      e: MouseEvent,
      col: "date" | "title" | "project" | "tool" | "fill" | "tokens",
    ) => void;
  }

  let { sortLabel, toggleSort, onDividerMouseDown }: Props = $props();
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="header-row">
  <span class="check-header"></span>
  <button class="header-btn" onclick={() => toggleSort("date")}>{sortLabel("date", "DATE CREATED")}</button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "date")}></span>
  <button class="header-btn" onclick={() => toggleSort("title")}>{sortLabel("title", "TITLE")}</button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "title")}></span>
  <button class="header-btn" onclick={() => toggleSort("project")}>{sortLabel("project", "PROJECT")}</button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "project")}></span>
  <button class="header-btn" onclick={() => toggleSort("tool")}>{sortLabel("tool", "TOOL")}</button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "tool")}></span>
  <button class="header-btn" onclick={() => toggleSort("fill")}>{sortLabel("fill", "FILL")}</button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "fill")}></span>
  <button class="header-btn tokens-header" onclick={() => toggleSort("tokens")}>
    {sortLabel("tokens", "TOKENS")}
  </button>
  <span class="divider" role="separator" onmousedown={(e) => onDividerMouseDown(e, "tokens")}
  ></span>
  <button class="header-btn" onclick={() => toggleSort("tags")}>{sortLabel("tags", "TAGS")}</button>
</div>

<style>
  .header-row {
    display: grid;
    grid-template-columns: 28px var(--full-cols);
    align-items: center;
    padding: 8px 16px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: var(--amber);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    position: sticky;
    top: 0;
    z-index: 1;
  }

  .check-header {
    width: 28px;
  }

  .header-btn {
    appearance: none;
    background: transparent;
    border: 0;
    color: inherit;
    cursor: pointer;
    font: inherit;
    min-width: 0;
    padding: 0;
    text-align: left;
  }

  .header-btn:hover {
    color: var(--text);
  }

  .tokens-header {
    text-align: center;
  }

  .divider {
    align-self: stretch;
    cursor: col-resize;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
  }

  .divider::after {
    content: "";
    width: 2px;
    height: 60%;
    background: var(--amber);
    border-radius: 2px;
    transition: background 0.12s, width 0.12s;
    opacity: 0.4;
  }

  .divider:hover::after {
    opacity: 1;
    width: 3px;
  }

  :global(.dragging) .divider::after {
    opacity: 1;
  }
</style>

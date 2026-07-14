<!-- HalluScribe - one session row in the session list. -->
<script lang="ts">
  import type { IndexEntry } from "../../lib/types";
  import { displayFillPct, fillClass, shortDate, timeFromSession } from "../../lib/format";

  interface Props {
    session: IndexEntry;
    selected: boolean;
    checked: boolean;
    onclick: (e: MouseEvent | KeyboardEvent) => void;
    ontogglecheck: (checked: boolean) => void;
    /** Raw-scope match count; null/undefined hides the badge (summaries scope). */
    rawHits?: number | null;
    onToggleRaw?: () => void;
  }

  let { session, selected, checked, onclick, ontogglecheck, rawHits = null, onToggleRaw }: Props = $props();

  let fillCls = $derived(fillClass(session.fill_pct));
  let fillText = $derived(displayFillPct(session.fill_pct, session.fill_estimated ?? false));
  let allTags = $derived([...session.error_tags, ...session.topic_tags].slice(0, 5));
  let time = $derived(timeFromSession(session.session_timestamp, session.archive_path));
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="row"
  class:selected
  class:checked
  role="row"
  tabindex="0"
  {onclick}
  onkeydown={(e) => { if (e.key === "Enter") onclick(e); }}
>
  <span class="check-cell">
    <input
      type="checkbox"
      checked={checked}
      aria-label={`Select session ${session.title}`}
      onclick={(e) => e.stopPropagation()}
      onchange={(e) => ontogglecheck((e.currentTarget as HTMLInputElement).checked)}
    />
  </span>
  <span class="date">{shortDate(session.date)}<span class="time">{time}</span></span>
  <span class="gap"></span>
  <span class="title">
    {#if session.secret_flags?.length}
      <span class="secret-badge" title={"Possible secrets: " + session.secret_flags.join(", ")}>⚠</span>
    {/if}
    {#if rawHits != null}
      <button
        class="raw-badge"
        title="Show matching raw transcript excerpts"
        onclick={(e) => { e.stopPropagation(); onToggleRaw?.(); }}
      >
        {rawHits} {rawHits === 1 ? "hit" : "hits"}
      </button>
    {/if}
    {session.title}
  </span>
  <span class="gap"></span>
  <span class="project muted">{session.project}</span>
  <span class="gap"></span>
  <span class="tool muted">{session.tool}</span>
  <span class="gap"></span>
  <span class="fill {fillCls}">{fillText}</span>
  <span class="gap"></span>
  <span class="tags muted">
    {#each allTags as tag, i}
      {tag}{#if i < allTags.length - 1} | {/if}
    {/each}
  </span>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 28px var(--full-cols, 110px 10px 1fr 10px 100px 10px 90px 10px 44px 10px 1fr);
    align-items: center;
    padding: 9px 16px;
    font-size: 13px;
    border-bottom: 1px solid transparent;
    cursor: pointer;
    transition: background 0.1s;
  }

  .gap {}

  .row:hover {
    background: var(--surface);
  }

  .row.selected {
    background: var(--surface);
    border-bottom-color: var(--border);
  }

  .row.checked {
    background: rgba(239, 68, 68, 0.08);
    border-bottom-color: rgba(239, 68, 68, 0.25);
    box-shadow: inset 3px 0 0 #ef4444;
  }

  .check-cell {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .check-cell input {
    width: 15px;
    height: 15px;
    accent-color: var(--amber);
    cursor: pointer;
  }

  .date {
    color: var(--muted);
    white-space: nowrap;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .time {
    font-size: 12px;
    color: var(--dim);
  }

  .title {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }

  .secret-badge {
    display: inline-block;
    color: var(--amber);
    font-size: 11px;
    margin-right: 6px;
    line-height: 1;
  }

  .raw-badge {
    background: none;
    border: 1px solid var(--amber);
    border-radius: 9px;
    color: var(--amber);
    font-family: inherit;
    font-size: 11px;
    line-height: 1;
    padding: 2px 7px;
    margin-right: 6px;
    cursor: pointer;
  }

  .raw-badge:hover {
    background: color-mix(in srgb, var(--amber) 15%, transparent);
  }

  .project,
  .tool {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fill {
    white-space: nowrap;
    font-weight: 700;
  }

  .tags {
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted {
    color: var(--muted);
  }
</style>

<!-- HalluScribe - paginated session list with explicit multi-select and briefing send flow. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import "./SessionList.css";
  import type { IndexEntry, SessionStats } from "../../lib/types";
  import { filterSessions, sortSessions, type SessionSortDirection, type SessionSortKey } from "../../lib/search";
  import StatsStrip from "./StatsStrip.svelte";
  import SessionBulkBar from "./SessionBulkBar.svelte";
  import SessionRow from "./SessionRow.svelte";
  import SessionTableHeader from "./SessionTableHeader.svelte";
  import SessionDetail from "./SessionDetail.svelte";

  interface Props {
    onSendToBriefing: (sessionIds: string[]) => void | Promise<unknown>;
  }

  let { onSendToBriefing }: Props = $props();

  const SEARCH_DEBOUNCE_MS = 300;
  const PAGE_SIZE = 25;
  const STORAGE_KEY = "hs-col-widths";
  const DIV_W = 10;

  const DEFAULTS = { date: 110, title: -1, project: 100, tool: 90, fill: 44, tags: -1 };
  type ColKey = keyof typeof DEFAULTS;
  const COL_KEYS: ColKey[] = ["date", "title", "project", "tool", "fill", "tags"];

  function loadWidths(): typeof DEFAULTS {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
    } catch {}
    return { ...DEFAULTS };
  }

  function saveWidths(widths: typeof DEFAULTS) {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(widths));
    } catch {}
  }

  let cols = $state(loadWidths());

  function fullTemplate(widths: typeof DEFAULTS): string {
    const tracks = COL_KEYS.map((key) => (widths[key] < 0 ? "1fr" : `${widths[key]}px`));
    return [
      tracks[0], `${DIV_W}px`,
      tracks[1], `${DIV_W}px`,
      tracks[2], `${DIV_W}px`,
      tracks[3], `${DIV_W}px`,
      tracks[4], `${DIV_W}px`,
      tracks[5],
    ].join(" ");
  }

  let fullCols = $derived(fullTemplate(cols));
  let dragCol = $state<ColKey | null>(null);
  let dragStartX = 0;
  let dragStartWidth = 0;

  function onDividerMouseDown(e: MouseEvent, col: ColKey) {
    e.preventDefault();
    dragCol = col;
    dragStartX = e.clientX;
    dragStartWidth = cols[col] < 0 ? 120 : cols[col];
  }

  function onMouseMove(e: MouseEvent) {
    if (!dragCol) return;
    const next = Math.max(40, dragStartWidth + (e.clientX - dragStartX));
    cols = { ...cols, [dragCol]: next };
  }

  function onMouseUp() {
    if (!dragCol) return;
    saveWidths(cols);
    dragCol = null;
  }

  let all = $state<IndexEntry[]>([]);
  let searchResults = $state<IndexEntry[]>([]);
  let stats = $state<SessionStats | null>(null);
  let query = $state("");
  let fillMin = $state("");
  let fillMax = $state("");
  let sortKey = $state<SessionSortKey>("date");
  let sortDirection = $state<SessionSortDirection>("desc");
  let page = $state(0);
  let selectedId = $state<string | null>(null);
  let checkedIds = $state<Set<string>>(new Set());

  let filtered = $derived(filterSessions(
    query.trim() ? searchResults : all,
    "",
    fillMin ? parseFloat(fillMin) : undefined,
    fillMax ? parseFloat(fillMax) : undefined,
  ));
  let sorted = $derived(sortSessions(filtered, sortKey, sortDirection));
  let pageCount = $derived(Math.max(1, Math.ceil(sorted.length / PAGE_SIZE)));
  let pageRows = $derived(sorted.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE));
  let selected = $derived(all.find((session) => session.id === selectedId) ?? null);

  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const q = query;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      invoke<IndexEntry[]>("search_sessions_fulltext", { query: q })
        .then((results) => { searchResults = results; });
    }, SEARCH_DEBOUNCE_MS);
  });

  function reload() {
    invoke<IndexEntry[]>("get_recent_sessions").then((sessions) => { all = sessions; });
    invoke<SessionStats>("get_stats").then((nextStats) => {
      stats = nextStats;
      invoke<number>("get_raw_session_total")
        .then((rawTotal) => {
          if (stats) stats = { ...stats, raw_total: rawTotal };
        })
        .catch(() => {});
    });
  }

  onMount(() => {
    reload();
    listen("sweep-done", reload);
  });

  function clearSelection() {
    checkedIds = new Set();
  }

  function setChecked(id: string, checked: boolean) {
    const next = new Set(checkedIds);
    if (checked) next.add(id);
    else next.delete(id);
    checkedIds = next;
  }

  function selectRow(id: string, ctrl: boolean) {
    if (ctrl) {
      setChecked(id, !checkedIds.has(id));
      return;
    }
    selectedId = selectedId === id ? null : id;
  }

  async function confirmDelete() {
    const n = checkedIds.size;
    const label = n === 1 ? "1 session" : `${n} sessions`;
    if (!confirm(`Permanently delete ${label}? This cannot be undone.`)) return;
    await invoke<string[]>("delete_sessions", { ids: [...checkedIds] });
    clearSelection();
    reload();
  }

  function sendSelectionToBriefing() {
    const ids = [...checkedIds];
    if (ids.length === 0) return;
    onSendToBriefing(ids);
    clearSelection();
  }

  function onKeyDown(e: KeyboardEvent) {
    const idx = pageRows.findIndex((row) => row.id === selectedId);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedId = pageRows[Math.min(idx + 1, pageRows.length - 1)]?.id ?? null;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedId = pageRows[Math.max(idx - 1, 0)]?.id ?? null;
    } else if (e.key === "PageDown") {
      e.preventDefault();
      page = Math.min(page + 1, pageCount - 1);
    } else if (e.key === "PageUp") {
      e.preventDefault();
      page = Math.max(page - 1, 0);
    }
  }

  function toggleSort(key: SessionSortKey) {
    sortDirection = sortKey === key
      ? (sortDirection === "asc" ? "desc" : "asc")
      : (key === "date" || key === "fill" ? "desc" : "asc");
    sortKey = key;
    page = 0;
  }

  function sortLabel(key: SessionSortKey, label: string): string {
    if (sortKey !== key) return label;
    return `${label} ${sortDirection === "asc" ? "↑" : "↓"}`;
  }
</script>

<svelte:window onkeydown={onKeyDown} onmousemove={onMouseMove} onmouseup={onMouseUp} />

<div class="container" style="--full-cols: {fullCols};" class:dragging={dragCol !== null}>
  <div class="left" class:narrow={selected !== null}>
    <StatsStrip {stats} />

    <div class="search-bar">
      <input
        class="search-input"
        type="text"
        name="session-search"
        placeholder="search sessions..."
        bind:value={query}
      />
      <span class="fill-label">fill%</span>
      <input class="fill-input" type="number" min="0" max="100" placeholder="min" bind:value={fillMin} title="Minimum fill%" />
      <span class="fill-sep">-</span>
      <input class="fill-input" type="number" min="0" max="100" placeholder="max" bind:value={fillMax} title="Maximum fill%" />
    </div>

    {#if checkedIds.size > 0}
      <SessionBulkBar
        count={checkedIds.size}
        onSendToBriefing={sendSelectionToBriefing}
        onClearSelection={clearSelection}
        onDelete={confirmDelete}
      />
    {/if}

    <div class="list" role="grid" aria-label="Session list">
      <SessionTableHeader {sortLabel} {toggleSort} {onDividerMouseDown} />

      {#each pageRows as session (session.id)}
        <SessionRow
          {session}
          selected={session.id === selectedId}
          checked={checkedIds.has(session.id)}
          onclick={(e) => selectRow(session.id, "ctrlKey" in e && e.ctrlKey)}
          ontogglecheck={(checked) => setChecked(session.id, checked)}
        />
      {/each}

      {#if pageRows.length === 0}
        <p class="empty">
          {query ? "no sessions match that search" : "no sessions archived yet"}
        </p>
      {/if}
    </div>

    {#if pageCount > 1}
      <div class="pagination">
        <button class="btn-nav" onclick={() => { page = Math.max(page - 1, 0); }} disabled={page === 0}>prev</button>
        <span class="page-info">{page + 1} / {pageCount}</span>
        <button class="btn-nav" onclick={() => { page = Math.min(page + 1, pageCount - 1); }} disabled={page >= pageCount - 1}>next</button>
      </div>
    {/if}
  </div>

  {#if selected}
    <div class="right">
      <SessionDetail session={selected} onclose={() => { selectedId = null; }} />
    </div>
  {/if}
</div>

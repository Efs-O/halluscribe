<!-- HalluScribe - paginated session list with explicit multi-select and briefing send flow. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import "./SessionList.css";
  import type { IndexEntry, RawSearchResult, SessionStats } from "../../lib/types";
  import {
    filterSessions, rawCoverageLine, rawMatchesById, rawMatchIds, sortSessions,
    type SessionSortDirection, type SessionSortKey,
  } from "../../lib/search";
  import StatsStrip from "./StatsStrip.svelte";
  import SessionBulkBar from "./SessionBulkBar.svelte";
  import SessionRow from "./SessionRow.svelte";
  import SessionTableHeader from "./SessionTableHeader.svelte";
  import SessionDetail from "./SessionDetail.svelte";
  import RawMatchDetails from "./RawMatchDetails.svelte";

  interface Props {
    onSendToBriefing: (sessionIds: string[]) => void | Promise<unknown>;
    // Lifted to App.svelte so search scope/query and a completed raw scan
    // survive tab switches (this component is destroyed on every tab change).
    searchScope?: "summaries" | "raw";
    query?: string;
    rawResult?: RawSearchResult | null;
    rawError?: string | null;
  }

  let {
    onSendToBriefing,
    searchScope = $bindable("summaries"),
    query = $bindable(""),
    rawResult = $bindable(null),
    rawError = $bindable(null),
  }: Props = $props();

  const SEARCH_DEBOUNCE_MS = 300;
  const RAW_MIN_QUERY_CHARS = 3;
  const PAGE_SIZE = 25;
  const STORAGE_KEY = "hs-col-widths";
  const DIV_W = 10;

  // A width stored before TOKENS existed simply lacks the key; loadWidths
  // spreads over DEFAULTS, so it picks up the default rather than collapsing.
  const DEFAULTS = { date: 110, title: -1, project: 100, tool: 90, fill: 44, tokens: 60, tags: -1 };
  type ColKey = keyof typeof DEFAULTS;
  const COL_KEYS: ColKey[] = ["date", "title", "project", "tool", "fill", "tokens", "tags"];

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
      tracks[5], `${DIV_W}px`,
      tracks[6],
    ].join(" ");
  }

  let fullCols = $derived(fullTemplate(cols));
  let deleteWarning = $state<string | null>(null);
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
  let fillMin = $state("");
  let fillMax = $state("");
  let sortKey = $state<SessionSortKey>("date");
  let sortDirection = $state<SessionSortDirection>("desc");
  let page = $state(0);
  let selectedId = $state<string | null>(null);
  let checkedIds = $state<Set<string>>(new Set());

  let rawLoading = $state(false);
  let expandedRawIds = $state<Set<string>>(new Set());

  let rawIds = $derived(rawResult ? rawMatchIds(rawResult) : null);
  let rawById = $derived(rawResult ? rawMatchesById(rawResult) : null);
  // Raw scope with no completed scan shows the plain unfiltered list; a scan
  // filters the same table by matching ids (no separate results table).
  let baseRows = $derived(
    searchScope === "raw"
      ? (rawIds ? all.filter((session) => rawIds.has(session.id)) : all)
      : (query.trim() ? searchResults : all),
  );
  let filtered = $derived(filterSessions(
    baseRows,
    "",
    fillMin ? parseFloat(fillMin) : undefined,
    fillMax ? parseFloat(fillMax) : undefined,
  ));
  let sorted = $derived(sortSessions(filtered, sortKey, sortDirection));
  let pageCount = $derived(Math.max(1, Math.ceil(sorted.length / PAGE_SIZE)));
  let pageRows = $derived(sorted.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE));
  let selected = $derived(all.find((session) => session.id === selectedId) ?? null);

  // Generation counters so a slow response never overwrites a newer one
  // (raw scans can take seconds once the backfilled corpus grows).
  let summaryGen = 0;
  let rawGen = 0;

  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const q = query;
    const scope = searchScope;
    clearTimeout(debounceTimer);
    if (scope !== "summaries") return;
    const gen = ++summaryGen;
    debounceTimer = setTimeout(() => {
      invoke<IndexEntry[]>("search_sessions_fulltext", { query: q })
        .then((results) => { if (gen === summaryGen) searchResults = results; });
    }, SEARCH_DEBOUNCE_MS);
  });

  async function runRawSearch() {
    const q = query.trim();
    const gen = ++rawGen;
    expandedRawIds = new Set();
    if (q.length < RAW_MIN_QUERY_CHARS) {
      // Empty query = back to the unfiltered list; a too-short one gets a hint.
      rawResult = null;
      rawError = q ? `raw search needs at least ${RAW_MIN_QUERY_CHARS} characters` : null;
      return;
    }
    rawLoading = true;
    rawError = null;
    try {
      const result = await invoke<RawSearchResult>("search_raw_transcripts", { query: q });
      if (gen !== rawGen) return;
      rawResult = result;
      page = 0;
    } catch (error) {
      if (gen !== rawGen) return;
      rawError = String(error);
      rawResult = null;
    } finally {
      if (gen === rawGen) rawLoading = false;
    }
  }

  function setScope(scope: "summaries" | "raw") {
    if (searchScope === scope) return;
    searchScope = scope;
    rawError = null;
    expandedRawIds = new Set();
    page = 0;
  }

  function toggleRawDetails(id: string) {
    const next = new Set(expandedRawIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expandedRawIds = next;
  }

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
    if (!confirm(`Permanently delete ${label}? Its summary and private copies are removed, and future sweeps will not archive it again. This cannot be undone.`)) return;
    let result: { deleted_ids: string[]; failures: { id: string; error: string }[] };
    try {
      result = await invoke("delete_sessions", { ids: [...checkedIds] });
    } catch (error) {
      deleteWarning = `Could not delete: ${error}`;
      return;
    }
    checkedIds = new Set(result.failures.map((failure) => failure.id));
    deleteWarning = result.failures.length
      ? `Could not delete ${result.failures.map((failure) => failure.id).join(", ")}. Close any program using the session and retry.`
      : null;
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
      : (key === "date" || key === "fill" || key === "tokens" ? "desc" : "asc");
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
        placeholder={searchScope === "raw" ? "exact search in raw transcripts..." : "search sessions..."}
        bind:value={query}
        onkeydown={(e) => { if (searchScope === "raw" && e.key === "Enter") runRawSearch(); }}
      />
      <div class="scope-group" role="group" aria-label="Search scope">
        <button
          class="scope-btn"
          class:active={searchScope === "summaries"}
          onclick={() => setScope("summaries")}
          title="Search the Gemma-generated session summaries (as you type)."
        >
          summaries
        </button>
        <button
          class="scope-btn"
          class:active={searchScope === "raw"}
          onclick={() => setScope("raw")}
          title="Exact substring scan of preserved raw transcripts. Press Enter to search."
        >
          raw
        </button>
      </div>
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

    {#if deleteWarning}
      <p class="raw-coverage error" role="alert">{deleteWarning}</p>
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
          rawHits={searchScope === "raw" ? (rawById?.get(session.id)?.total_hits ?? null) : null}
          onToggleRaw={() => toggleRawDetails(session.id)}
        />
        {#if searchScope === "raw" && expandedRawIds.has(session.id) && rawById?.get(session.id)}
          <RawMatchDetails matches={rawById.get(session.id)!} />
        {/if}
      {/each}

      {#if pageRows.length === 0}
        <p class="empty">
          {#if searchScope === "raw"}
            {rawResult ? `no matches in ${rawResult.sessions_scanned} raw transcripts searched` : "no sessions archived yet"}
          {:else}
            {query ? "no sessions match that search" : "no sessions archived yet"}
          {/if}
        </p>
      {/if}
    </div>

    {#if searchScope === "raw"}
      <p class="raw-coverage" class:error={rawError !== null}>
        {#if rawLoading}
          scanning raw transcripts...
        {:else if rawError}
          {rawError}
        {:else if rawResult}
          {rawCoverageLine(rawResult)}{rawResult.results_truncated ? " — results truncated to the newest 200 matching sessions" : ""}
        {:else}
          exact match only — press Enter to scan raw transcripts
        {/if}
      </p>
    {/if}

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

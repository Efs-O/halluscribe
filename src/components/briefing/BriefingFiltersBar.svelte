<!-- HalluScribe - briefing filter controls. -->
<script lang="ts">
  interface Props {
    dateFrom: string;
    dateTo: string;
    fillMin: string;
    fillMax: string;
    keyword: string;
  }

  let { dateFrom = $bindable(), dateTo = $bindable(), fillMin = $bindable(), fillMax = $bindable(), keyword = $bindable() }: Props = $props();
</script>

<div class="filter-bar">
  <input class="filter-date" type="date" bind:value={dateFrom} title="Session date from" />
  <span class="filter-sep">to</span>
  <input class="filter-date" type="date" bind:value={dateTo} title="Session date to" />
  <input class="filter-num" type="number" bind:value={fillMin} min="0" max="100" placeholder="fill% min" title="Minimum fill%" />
  <span class="filter-sep">-</span>
  <input class="filter-num" type="number" bind:value={fillMax} min="0" max="100" placeholder="max" title="Maximum fill%" />
  <input class="filter-kw" type="text" bind:value={keyword} placeholder="keyword..." title="Filter by keyword" />
</div>

<style>
  .filter-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    flex-shrink: 0;
    flex-wrap: wrap;
  }

  .filter-date, .filter-num, .filter-kw {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text);
    font-family: inherit;
    font-size: 14px;
    padding: 6px 10px;
    outline: none;
    transition: border-color 0.15s;
  }

  .filter-date:focus, .filter-num:focus, .filter-kw:focus {
    border-color: var(--amber);
  }

  .filter-date { width: 152px; }
  .filter-num { width: 92px; }
  .filter-kw { flex: 1; min-width: 180px; }

  .filter-sep {
    color: var(--dim);
    font-size: 13px;
    flex-shrink: 0;
  }
</style>

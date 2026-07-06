<!-- HalluScribe — PROFILE view: Work / Personal scope selector (Persona
     Protocol Phase 2c). Kept as its own component to keep ProfilePanel.svelte
     under the repo's 350-LOC file cap. -->
<script lang="ts">
  import type { ProfileScope } from "../../lib/types";

  interface Props {
    active: ProfileScope;
    onselect: (scope: ProfileScope) => void;
  }
  let { active, onselect }: Props = $props();

  const scopes: { id: ProfileScope; label: string }[] = [
    { id: "work", label: "Work" },
    { id: "personal", label: "Personal" },
  ];
</script>

<div class="scope-row">
  {#each scopes as s}
    <button
      class="scope-tab"
      class:active={active === s.id}
      onclick={() => onselect(s.id)}
    >
      {s.label}
    </button>
  {/each}
</div>

<style>
  .scope-row {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }

  .scope-tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.04em;
    padding: 8px 16px;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s;
  }

  .scope-tab:hover {
    color: var(--text);
  }

  .scope-tab.active {
    color: var(--text);
    border-bottom-color: var(--green);
  }
</style>

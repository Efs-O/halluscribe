<!-- HalluScribe — tab navigation: BRIEFING / SESSION SUMMARY / SETTINGS. -->
<script lang="ts">
  interface Props {
    active: "briefing" | "sessions" | "settings";
    onchange: (tab: "briefing" | "sessions" | "settings") => void;
  }
  let { active, onchange }: Props = $props();

  const tabs: { id: "briefing" | "sessions" | "settings"; label: string }[] = [
    { id: "briefing",  label: "BRIEFING" },
    { id: "sessions",  label: "SESSION SUMMARY" },
    { id: "settings",  label: "SETTINGS" },
  ];
</script>

<nav class="navbar">
  {#each tabs as tab}
    <button
      class="tab"
      class:active={active === tab.id}
      onclick={() => onchange(tab.id)}
    >
      {tab.label}
    </button>
  {/each}
</nav>

<style>
  .navbar {
    display: flex;
    gap: 0;
  }

  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    font-family: inherit;
    font-size: 17px;
    font-weight: 600;
    letter-spacing: 0.08em;
    padding: 0 22px;
    height: 54px;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s;
  }

  .tab:hover {
    color: var(--text);
  }

  .tab.active {
    color: var(--text);
    border-bottom-color: var(--green);
  }
</style>

<!-- HalluScribe — always-visible active-workspace label in the top bar, so the
     user can tell which person's archive is loaded from any view. Read-only;
     switching happens in Settings → WORKSPACES. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { WorkspaceListDto } from "../lib/types";

  let name = $state<string | null>(null);
  // Guest workspaces are visually distinguished from the host/default.
  let isGuest = $state(false);

  function activeName(list: WorkspaceListDto): { name: string; guest: boolean } {
    if (list.active === null) {
      return { name: list.default_name?.trim() || "Default", guest: false };
    }
    const match = list.workspaces.find((w) => w.path === list.active);
    return match ? { name: match.name, guest: true } : { name: "Default", guest: false };
  }

  onMount(async () => {
    try {
      const list = await invoke<WorkspaceListDto>("list_workspaces");
      const active = activeName(list);
      name = active.name;
      isGuest = active.guest;
    } catch {
      // Best-effort: if the registry can't be read, show nothing rather than error.
    }
  });
</script>

{#if name}
  <span class="ws-badge" class:guest={isGuest} title="Active workspace — switch in Settings → Workspaces">
    <span class="ws-dot" aria-hidden="true"></span>
    {name}
  </span>
{/if}

<style>
  .ws-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 4px 12px;
    white-space: nowrap;
    -webkit-app-region: no-drag;
  }

  .ws-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--green);
    flex-shrink: 0;
  }

  .ws-badge.guest {
    color: var(--amber);
    border-color: var(--amber);
  }
  .ws-badge.guest .ws-dot { background: var(--amber); }
</style>

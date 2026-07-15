<!-- HalluScribe - workspace registry orchestration and host workspace controls. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { WorkspaceInfo, WorkspaceListDto } from "../../lib/types";
  import "./WorkspaceSwitcher.css";
  import WorkspaceCreateForm from "./WorkspaceCreateForm.svelte";
  import WorkspaceRow from "./WorkspaceRow.svelte";

  let list = $state<WorkspaceListDto | null>(null);
  let busy = $state(false);
  let message = $state("");
  let messageIsError = $state(false);
  let renameInputs = $state<Record<string, string>>({});
  let defaultNameInput = $state("");
  let confirmingDelete = $state<string | null>(null);
  let expanded = $state<Record<string, boolean>>({});
  let confirmingHostAccess = $state<string | null>(null);

  onMount(() => { void loadList(); });

  async function loadList() {
    try {
      list = await invoke<WorkspaceListDto>("list_workspaces");
      const seeded: Record<string, string> = {};
      for (const workspace of list.workspaces) seeded[workspace.path] = workspace.name;
      renameInputs = seeded;
      defaultNameInput = list.default_name ?? "";
    } catch (error) {
      setError(String(error));
    }
  }

  function setError(text: string) {
    message = text;
    messageIsError = true;
  }

  function setInfo(text: string) {
    message = text;
    messageIsError = false;
  }

  function toggleExpanded(path: string) {
    const next = !expanded[path];
    expanded = { ...expanded, [path]: next };
    if (!next) {
      if (confirmingDelete === path) confirmingDelete = null;
      if (confirmingHostAccess === path) confirmingHostAccess = null;
    }
  }

  function isActive(path: string | null): boolean {
    return Boolean(list && (list.active ?? null) === path);
  }

  async function switchTo(path: string | null) {
    if (busy) return;
    busy = true;
    setInfo("Switching workspace...");
    try {
      await invoke("switch_workspace", { path });
      window.location.reload();
    } catch (error) {
      setError(String(error));
      busy = false;
    }
  }

  async function saveRename(workspace: WorkspaceInfo) {
    if (busy) return;
    const name = (renameInputs[workspace.path] ?? "").trim();
    if (!name) {
      setError("Workspace name cannot be empty.");
      return;
    }
    busy = true;
    try {
      await invoke("rename_workspace", { path: workspace.path, name });
      setInfo("Workspace renamed.");
      await loadList();
    } catch (error) {
      setError(String(error));
    } finally {
      busy = false;
    }
  }

  async function saveDefaultName() {
    if (busy) return;
    const name = defaultNameInput.trim();
    if (!name) {
      setError("Default workspace name cannot be empty.");
      return;
    }
    busy = true;
    try {
      await invoke("rename_default_workspace", { name });
      setInfo("Default workspace renamed.");
      await loadList();
    } catch (error) {
      setError(String(error));
    } finally {
      busy = false;
    }
  }

  async function deleteWorkspace(workspace: WorkspaceInfo) {
    if (busy) return;
    busy = true;
    const wasActive = isActive(workspace.path);
    try {
      await invoke("delete_workspace", { path: workspace.path });
      confirmingDelete = null;
      if (wasActive) {
        window.location.reload();
        return;
      }
      setInfo("Workspace removed from the list. Its folder on disk was kept.");
      await loadList();
    } catch (error) {
      setError(String(error));
    } finally {
      busy = false;
    }
  }

  function onToggleImportOnly(workspace: WorkspaceInfo, importOnly: boolean) {
    if (busy) return;
    if (importOnly) {
      confirmingHostAccess = null;
      void applyImportOnly(workspace, true);
    } else {
      confirmingHostAccess = workspace.path;
    }
  }

  async function applyImportOnly(workspace: WorkspaceInfo, importOnly: boolean) {
    if (busy) return;
    busy = true;
    try {
      await invoke("set_workspace_import_only", { path: workspace.path, importOnly });
      confirmingHostAccess = null;
      setInfo("Import-only updated.");
      await loadList();
    } catch (error) {
      setError(String(error));
    } finally {
      busy = false;
    }
  }

  async function cancelHostAccess() {
    confirmingHostAccess = null;
    await loadList();
  }

  async function createWorkspace(
    nameInput: string,
    pathInput: string,
    importOnly: boolean,
  ): Promise<boolean> {
    if (busy) return false;
    const name = nameInput.trim();
    const path = pathInput.trim();
    if (!name) {
      setError("Workspace name cannot be empty.");
      return false;
    }
    if (!path) {
      setError("Workspace path cannot be empty.");
      return false;
    }
    busy = true;
    try {
      await invoke("create_workspace", { name, path, importOnly });
      setInfo("Workspace created. Switch to it above when ready.");
      await loadList();
      return true;
    } catch (error) {
      setError(String(error));
      return false;
    } finally {
      busy = false;
    }
  }
</script>

<section class="workspace-switcher">
  <h2 class="section-title">WORKSPACES</h2>

  {#if !list}
    <p class="field-note">Loading workspaces...</p>
  {:else}
    <div class="ws-list">
      <div class="ws-row">
        <div class="ws-head">
          <span class="ws-name">
            {list.default_name?.trim() || "Default"} <span class="ws-tag">host</span>
          </span>
          {#if isActive(null)}
            <span class="ws-active">● active</span>
          {:else}
            <button class="action-btn" type="button" onclick={() => switchTo(null)} disabled={busy}>
              Switch
            </button>
          {/if}
        </div>
        <p class="field-note ws-path">{list.default_root}</p>

        <div class="ws-inline">
          <input
            type="text"
            bind:value={defaultNameInput}
            placeholder="Name this workspace (e.g. EFSO)"
            aria-label="Rename default workspace"
          />
          <button class="action-btn" type="button" onclick={saveDefaultName} disabled={busy}>Save</button>
        </div>
        <p class="field-note">
          Display label only — renaming never touches the archive or forces a rebuild. This is the
          host workspace and cannot be deleted.
        </p>
      </div>

      {#each list.workspaces as workspace (workspace.path)}
        <WorkspaceRow
          {workspace}
          active={isActive(workspace.path)}
          {busy}
          expanded={!!expanded[workspace.path]}
          renameValue={renameInputs[workspace.path] ?? ""}
          confirmingHostAccess={confirmingHostAccess === workspace.path}
          confirmingDelete={confirmingDelete === workspace.path}
          onToggle={() => toggleExpanded(workspace.path)}
          onSwitch={() => { void switchTo(workspace.path); }}
          onRenameInput={(value) => {
            renameInputs = { ...renameInputs, [workspace.path]: value };
          }}
          onRename={() => { void saveRename(workspace); }}
          onToggleImportOnly={(importOnly) => onToggleImportOnly(workspace, importOnly)}
          onApplyHostAccess={() => { void applyImportOnly(workspace, false); }}
          onCancelHostAccess={() => { void cancelHostAccess(); }}
          onRequestDelete={() => { confirmingDelete = workspace.path; }}
          onCancelDelete={() => { confirmingDelete = null; }}
          onDelete={() => { void deleteWorkspace(workspace); }}
        />
      {/each}
    </div>

    <WorkspaceCreateForm {busy} onCreate={createWorkspace} />

    {#if message}
      <p class="field-note" class:field-note-warn={messageIsError}>{message}</p>
    {/if}
  {/if}
</section>

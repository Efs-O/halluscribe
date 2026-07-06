<!-- HalluScribe - workspace switcher (multi-person workspaces, Persona Parity Phase E). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { WorkspaceInfo, WorkspaceListDto } from "../../lib/types";

  let list = $state<WorkspaceListDto | null>(null);
  let busy = $state(false);
  let message = $state("");
  let messageIsError = $state(false);

  // Rename inputs, keyed by workspace path.
  let renameInputs = $state<Record<string, string>>({});
  // Editable display label for the default (host) root.
  let defaultNameInput = $state("");
  // Path of the guest row currently awaiting delete confirmation, or null.
  let confirmingDelete = $state<string | null>(null);

  // Create-form fields. Import-only defaults ON: a new workspace is almost always
  // a guest (another person's imports), and leaving it off would sweep THIS
  // machine's coding logs into their profile.
  let newName = $state("");
  let newPath = $state("");
  let newImportOnly = $state(true);
  // Path of the guest row awaiting confirmation to turn import-only OFF, or null.
  let confirmingHostAccess = $state<string | null>(null);

  onMount(() => {
    void loadList();
  });

  async function loadList() {
    try {
      list = await invoke<WorkspaceListDto>("list_workspaces");
      const seeded: Record<string, string> = {};
      for (const w of list.workspaces) seeded[w.path] = w.name;
      renameInputs = seeded;
      defaultNameInput = list.default_name ?? "";
    } catch (e) {
      setError(String(e));
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

  function isActive(path: string | null): boolean {
    if (!list) return false;
    return (list.active ?? null) === path;
  }

  // Switching re-points every panel at the new archive_dir. A full page reload
  // cleanly re-mounts all panels against the freshly-active workspace — that IS
  // the intended "reload all panels" behavior for this phase.
  async function switchTo(path: string | null) {
    if (busy) return;
    busy = true;
    setInfo("Switching workspace...");
    try {
      await invoke("switch_workspace", { path });
      window.location.reload();
    } catch (e) {
      setError(String(e));
      busy = false;
    }
  }

  async function saveRename(w: WorkspaceInfo) {
    if (busy) return;
    const name = (renameInputs[w.path] ?? "").trim();
    if (!name) {
      setError("Workspace name cannot be empty.");
      return;
    }
    busy = true;
    try {
      await invoke("rename_workspace", { path: w.path, name });
      setInfo("Workspace renamed.");
      await loadList();
    } catch (e) {
      setError(String(e));
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
    } catch (e) {
      setError(String(e));
    } finally {
      busy = false;
    }
  }

  // Un-registers a guest workspace; its archive folder on disk is left intact.
  // If the deleted workspace was active, the backend falls back to the default
  // root, so we reload the page to re-point every panel cleanly.
  async function deleteWorkspace(w: WorkspaceInfo) {
    if (busy) return;
    busy = true;
    const wasActive = isActive(w.path);
    try {
      await invoke("delete_workspace", { path: w.path });
      confirmingDelete = null;
      if (wasActive) {
        window.location.reload();
        return;
      }
      setInfo("Workspace removed from the list. Its folder on disk was kept.");
      await loadList();
    } catch (e) {
      setError(String(e));
    } finally {
      busy = false;
    }
  }

  // Checkbox handler. Turning import-only ON is safe and applies immediately;
  // turning it OFF exposes this machine's coding logs to the workspace, so we
  // ask for confirmation first instead of applying the change.
  function onToggleImportOnly(w: WorkspaceInfo, importOnly: boolean) {
    if (busy) return;
    if (importOnly) {
      confirmingHostAccess = null;
      void applyImportOnly(w, true);
    } else {
      confirmingHostAccess = w.path;
    }
  }

  async function applyImportOnly(w: WorkspaceInfo, importOnly: boolean) {
    if (busy) return;
    busy = true;
    try {
      await invoke("set_workspace_import_only", { path: w.path, importOnly });
      confirmingHostAccess = null;
      setInfo("Import-only updated.");
      await loadList();
    } catch (e) {
      setError(String(e));
    } finally {
      busy = false;
    }
  }

  // Abandon a pending "turn off" — reload so the checkbox snaps back to its
  // real (still import-only) state.
  async function cancelHostAccess() {
    confirmingHostAccess = null;
    await loadList();
  }

  async function createWorkspace() {
    if (busy) return;
    const name = newName.trim();
    const path = newPath.trim();
    if (!name) {
      setError("Workspace name cannot be empty.");
      return;
    }
    if (!path) {
      setError("Workspace path cannot be empty.");
      return;
    }
    busy = true;
    try {
      await invoke("create_workspace", { name, path, importOnly: newImportOnly });
      newName = "";
      newPath = "";
      newImportOnly = true;
      setInfo("Workspace created. Switch to it above when ready.");
      await loadList();
    } catch (e) {
      setError(String(e));
    } finally {
      busy = false;
    }
  }

</script>

<section>
  <h2 class="section-title">WORKSPACES</h2>

  {#if !list}
    <p class="field-note">Loading workspaces...</p>
  {:else}
    <div class="ws-list">
      <!-- Default root row -->
      <div class="ws-row">
        <div class="ws-head">
          <span class="ws-name">{list.default_name?.trim() || "Default"} <span class="ws-tag">host</span></span>
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
          <button class="action-btn" type="button" onclick={saveDefaultName} disabled={busy}>
            Save
          </button>
        </div>
        <p class="field-note">
          Display label only — renaming never touches the archive or forces a rebuild. This is the
          host workspace and cannot be deleted.
        </p>
      </div>

      {#each list.workspaces as w (w.path)}
        <div class="ws-row">
          <div class="ws-head">
            <span class="ws-name">{w.name}</span>
            {#if isActive(w.path)}
              <span class="ws-active">● active</span>
            {:else}
              <button
                class="action-btn"
                type="button"
                onclick={() => switchTo(w.path)}
                disabled={busy}
              >
                Switch
              </button>
            {/if}
          </div>
          <p class="field-note ws-path">{w.path}</p>

          <div class="ws-inline">
            <input
              type="text"
              bind:value={renameInputs[w.path]}
              placeholder="Workspace name"
              aria-label="Rename workspace"
            />
            <button class="action-btn" type="button" onclick={() => saveRename(w)} disabled={busy}>
              Save
            </button>
          </div>

          <label class="row-label ws-check">
            <span>Import-only (guest workspace)</span>
            <input
              type="checkbox"
              checked={w.import_only}
              onchange={(e) => onToggleImportOnly(w, (e.currentTarget as HTMLInputElement).checked)}
              disabled={busy || confirmingHostAccess === w.path}
            />
          </label>
          <p class="field-note">
            Guest workspace — the sweep ingests only this workspace's chat imports, never the host
            machine's local coding-tool logs.
          </p>

          {#if confirmingHostAccess === w.path}
            <div class="ws-warn">
              <p class="ws-warn-text">
                Turning this off lets the next sweep read <strong>this machine's</strong> Claude
                Code / Codex / Continue / Forge logs and file them into "{w.name}"'s profile. Only do
                this if this workspace is meant to track your own coding activity.
              </p>
              <div class="ws-warn-actions">
                <button
                  class="action-btn action-btn-danger"
                  type="button"
                  onclick={() => applyImportOnly(w, false)}
                  disabled={busy}
                >
                  Turn off anyway
                </button>
                <button
                  class="action-btn"
                  type="button"
                  onclick={cancelHostAccess}
                  disabled={busy}
                >
                  Keep import-only
                </button>
              </div>
            </div>
          {/if}

          <div class="ws-delete">
            {#if confirmingDelete === w.path}
              <span class="ws-confirm-text">
                Remove "{w.name}" from the list? Its folder on disk is kept.
              </span>
              <button
                class="action-btn action-btn-danger"
                type="button"
                onclick={() => deleteWorkspace(w)}
                disabled={busy}
              >
                Remove
              </button>
              <button
                class="action-btn"
                type="button"
                onclick={() => (confirmingDelete = null)}
                disabled={busy}
              >
                Cancel
              </button>
            {:else}
              <button
                class="action-btn action-btn-danger"
                type="button"
                onclick={() => (confirmingDelete = w.path)}
                disabled={busy}
              >
                Delete workspace
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <div class="ws-create">
      <h3 class="section-title">ADD WORKSPACE</h3>
      <p class="field-note">
        Type or paste the absolute folder path for this person's archive. Host-level model settings
        are copied from your default workspace; import paths and history start fresh.
      </p>
      <p class="field-note">A native folder picker is coming in a later update.</p>

      <label class="row-label">
        <span>Name</span>
        <input type="text" bind:value={newName} placeholder="Alex" aria-label="New workspace name" />
      </label>

      <label class="row-label">
        <span>Folder path</span>
        <input
          type="text"
          bind:value={newPath}
          placeholder="D:\personas\alex"
          aria-label="New workspace path"
        />
      </label>

      <label class="row-label">
        <span>Import-only (guest workspace)</span>
        <input type="checkbox" bind:checked={newImportOnly} />
      </label>
      {#if newImportOnly}
        <p class="field-note">
          Recommended for another person: sweeps only this workspace's imports, never this machine's
          coding logs.
        </p>
      {:else}
        <p class="field-note field-note-warn">
          Import-only off: sweeps will read <strong>this machine's</strong> Claude Code / Codex /
          Continue / Forge logs into this workspace. Only leave this off if the workspace is meant to
          track your own coding activity.
        </p>
      {/if}

      <div class="ws-actions">
        <button class="action-btn" type="button" onclick={createWorkspace} disabled={busy}>
          Create workspace
        </button>
      </div>
    </div>

    {#if message}
      <p class="field-note" class:field-note-warn={messageIsError}>{message}</p>
    {/if}
  {/if}
</section>

<style>
  .field-note { color: var(--dim); font-size: 12px; margin: 0; }
  .field-note.field-note-warn { color: #f0c07a; }

  .section-title {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--dim);
    margin-bottom: 4px;
  }

  .ws-list { display: flex; flex-direction: column; gap: 14px; }

  .ws-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.03);
  }

  .ws-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .ws-name { font-size: 14px; color: var(--text); }
  .ws-tag {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 1px 5px;
    margin-left: 4px;
    vertical-align: middle;
  }
  .ws-active { font-size: 12px; color: var(--green); letter-spacing: 0.06em; }
  .ws-path { word-break: break-all; }

  .ws-delete {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ws-confirm-text { font-size: 12px; color: var(--amber); }

  .ws-warn {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--amber);
    border-radius: 5px;
    background: rgba(240, 192, 122, 0.08);
  }
  .ws-warn-text { font-size: 12px; color: var(--amber); margin: 0; line-height: 1.5; }
  .ws-warn-actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .action-btn-danger:hover:not(:disabled) {
    border-color: var(--red);
    color: var(--red);
  }

  .ws-inline { display: flex; gap: 8px; align-items: center; }
  .ws-inline input[type="text"] {
    flex: 1;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 14px;
    padding: 8px 12px;
    outline: none;
  }
  .ws-inline input:focus { border-color: var(--muted); }

  .row-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    font-size: 14px;
    color: var(--text);
  }

  .row-label span { flex: 1; }

  .row-label input[type="text"] {
    flex: 0 0 300px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 14px;
    padding: 8px 12px;
    outline: none;
  }
  .row-label input[type="text"]:focus { border-color: var(--muted); }

  .row-label input[type="checkbox"] {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    accent-color: var(--green);
    cursor: pointer;
  }

  .ws-check { font-size: 13px; }

  .ws-create { display: flex; flex-direction: column; gap: 12px; margin-top: 18px; }
  .ws-actions { display: flex; align-items: flex-start; }

  .action-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
    letter-spacing: 0.06em;
    padding: 8px 12px;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s, opacity 0.15s;
  }

  .action-btn:hover:not(:disabled) {
    border-color: var(--amber);
    color: var(--amber);
  }

  .action-btn:disabled { cursor: default; opacity: 0.6; }
</style>

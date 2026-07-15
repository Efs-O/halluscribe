<!-- HalluScribe - one expandable guest workspace row and its confirmations. -->
<script lang="ts">
  import type { WorkspaceInfo } from "../../lib/types";

  interface Props {
    workspace: WorkspaceInfo;
    active: boolean;
    busy: boolean;
    expanded: boolean;
    renameValue: string;
    confirmingHostAccess: boolean;
    confirmingDelete: boolean;
    onToggle: () => void;
    onSwitch: () => void;
    onRenameInput: (value: string) => void;
    onRename: () => void;
    onToggleImportOnly: (importOnly: boolean) => void;
    onApplyHostAccess: () => void;
    onCancelHostAccess: () => void;
    onRequestDelete: () => void;
    onCancelDelete: () => void;
    onDelete: () => void;
  }

  let {
    workspace,
    active,
    busy,
    expanded,
    renameValue,
    confirmingHostAccess,
    confirmingDelete,
    onToggle,
    onSwitch,
    onRenameInput,
    onRename,
    onToggleImportOnly,
    onApplyHostAccess,
    onCancelHostAccess,
    onRequestDelete,
    onCancelDelete,
    onDelete,
  }: Props = $props();
</script>

<div class="ws-row">
  <div
    class="ws-head ws-head-toggle"
    role="button"
    tabindex="0"
    aria-expanded={expanded}
    onclick={onToggle}
    onkeydown={(event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        onToggle();
      }
    }}
  >
    <span class="ws-chevron">{expanded ? "▾" : "▸"}</span>
    <span class="ws-name">{workspace.name}</span>
    {#if active}<span class="ws-active">● active</span>{/if}
    <span class="ws-head-spacer"></span>
    {#if !active}
      <button
        class="action-btn"
        type="button"
        onclick={(event) => { event.stopPropagation(); onSwitch(); }}
        disabled={busy}
      >
        Switch
      </button>
    {/if}
  </div>

  {#if expanded}
    <div class="ws-body">
      <p class="field-note ws-path">{workspace.path}</p>

      <div class="ws-inline">
        <input
          type="text"
          value={renameValue}
          oninput={(event) => onRenameInput((event.currentTarget as HTMLInputElement).value)}
          placeholder="Workspace name"
          aria-label="Rename workspace"
        />
        <button class="action-btn" type="button" onclick={onRename} disabled={busy}>Save</button>
      </div>

      <label class="row-label ws-check">
        <span>Import-only (guest workspace)</span>
        <input
          type="checkbox"
          checked={workspace.import_only}
          onchange={(event) =>
            onToggleImportOnly((event.currentTarget as HTMLInputElement).checked)}
          disabled={busy || confirmingHostAccess}
        />
      </label>
      <p class="field-note">
        Guest workspace — the sweep ingests only this workspace's chat imports, never the host
        machine's local coding-tool logs.
      </p>

      {#if confirmingHostAccess}
        <div class="ws-warn">
          <p class="ws-warn-text">
            Turning this off lets the next sweep read <strong>this machine's</strong> Claude Code /
            Codex / Continue / Forge logs and file them into "{workspace.name}"'s profile. Only do
            this if this workspace is meant to track your own coding activity.
          </p>
          <div class="ws-warn-actions">
            <button
              class="action-btn action-btn-danger"
              type="button"
              onclick={onApplyHostAccess}
              disabled={busy}
            >
              Turn off anyway
            </button>
            <button class="action-btn" type="button" onclick={onCancelHostAccess} disabled={busy}>
              Keep import-only
            </button>
          </div>
        </div>
      {/if}

      <div class="ws-delete">
        {#if confirmingDelete}
          <span class="ws-confirm-text">
            Remove "{workspace.name}" from the list? Its folder on disk is kept.
          </span>
          <button class="action-btn action-btn-danger" type="button" onclick={onDelete} disabled={busy}>
            Remove
          </button>
          <button class="action-btn" type="button" onclick={onCancelDelete} disabled={busy}>
            Cancel
          </button>
        {:else}
          <button
            class="action-btn action-btn-danger"
            type="button"
            onclick={onRequestDelete}
            disabled={busy}
          >
            Delete workspace
          </button>
        {/if}
      </div>
    </div>
  {/if}
</div>

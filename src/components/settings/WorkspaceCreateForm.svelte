<!-- HalluScribe - new guest workspace form with safe import-only default. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import PathPickerField from "./PathPickerField.svelte";

  interface Props {
    busy: boolean;
    onCreate: (name: string, path: string, importOnly: boolean) => Promise<boolean>;
  }

  let { busy, onCreate }: Props = $props();
  let name = $state("");
  let path = $state("");
  let importOnly = $state(true);
  // Once the user types or browses their own folder, the name stops steering it.
  let pathEdited = $state(false);
  // Placeholder showing where an unnamed workspace would land, so the empty
  // field agrees with the note above it instead of naming an unrelated drive.
  let examplePath = $state("");

  onMount(() => {
    void (async () => {
      try {
        examplePath = await invoke<string>("suggest_workspace_path", { name: "Alex" });
      } catch {
        // Placeholder only - the field still works without it.
      }
    })();
  });

  async function onNameInput(event: Event) {
    name = (event.currentTarget as HTMLInputElement).value;
    if (pathEdited) return;
    try {
      path = await invoke<string>("suggest_workspace_path", { name });
    } catch {
      // Suggestion only - leave the field for the user to fill in.
    }
  }

  // Fired on blur and after Browse. An empty field is not an edit, so the name
  // keeps steering the suggestion until the user actually puts something there.
  function onPathChange() {
    pathEdited = path.trim().length > 0;
  }

  async function create() {
    if (await onCreate(name, path, importOnly)) {
      name = "";
      path = "";
      importOnly = true;
      pathEdited = false;
    }
  }
</script>

<div class="ws-create">
  <h3 class="section-title">ADD WORKSPACE</h3>
  <p class="field-note">
    The folder is suggested from the name, inside your archive's <code>users</code> folder — type or
    paste another absolute path, or use Browse, to put it elsewhere. Host-level model settings are
    copied from your default workspace; this workspace gets its own import folders and history.
  </p>

  <label class="row-label">
    <span>Name</span>
    <input
      type="text"
      value={name}
      oninput={onNameInput}
      placeholder="Alex"
      aria-label="New workspace name"
    />
  </label>

  <PathPickerField
    label="Folder path"
    bind:value={path}
    mode="folder"
    placeholder={examplePath}
    onchange={onPathChange}
  />

  <label class="row-label">
    <span>Import-only (guest workspace)</span>
    <input type="checkbox" bind:checked={importOnly} />
  </label>
  {#if importOnly}
    <p class="field-note">
      Recommended for another person: sweeps only this workspace's imports, never this machine's
      coding logs.
    </p>
  {:else}
    <p class="field-note field-note-warn">
      Import-only off: sweeps will read <strong>this machine's</strong> Claude Code / Codex / Forge
      logs into this workspace. Only leave this off if the workspace is meant to track your own
      coding activity.
    </p>
  {/if}

  <div class="ws-actions">
    <button class="action-btn" type="button" onclick={create} disabled={busy}>Create workspace</button>
  </div>
</div>

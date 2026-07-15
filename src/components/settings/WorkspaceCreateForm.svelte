<!-- HalluScribe - new guest workspace form with safe import-only default. -->
<script lang="ts">
  import PathPickerField from "./PathPickerField.svelte";

  interface Props {
    busy: boolean;
    onCreate: (name: string, path: string, importOnly: boolean) => Promise<boolean>;
  }

  let { busy, onCreate }: Props = $props();
  let name = $state("");
  let path = $state("");
  let importOnly = $state(true);

  async function create() {
    if (await onCreate(name, path, importOnly)) {
      name = "";
      path = "";
      importOnly = true;
    }
  }
</script>

<div class="ws-create">
  <h3 class="section-title">ADD WORKSPACE</h3>
  <p class="field-note">
    Type or paste the absolute folder path for this person's archive, or use Browse. Host-level
    model settings are copied from your default workspace; import paths and history start fresh.
  </p>

  <label class="row-label">
    <span>Name</span>
    <input type="text" bind:value={name} placeholder="Alex" aria-label="New workspace name" />
  </label>

  <PathPickerField
    label="Folder path"
    bind:value={path}
    mode="folder"
    placeholder="D:\personas\alex"
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
      Import-only off: sweeps will read <strong>this machine's</strong> Claude Code / Codex /
      Continue / Forge logs into this workspace. Only leave this off if the workspace is meant to
      track your own coding activity.
    </p>
  {/if}

  <div class="ws-actions">
    <button class="action-btn" type="button" onclick={create} disabled={busy}>Create workspace</button>
  </div>
</div>

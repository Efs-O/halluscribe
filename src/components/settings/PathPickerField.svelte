<!-- HalluScribe - reusable settings path field: text input + native OS folder/file picker. -->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  interface DialogFilter {
    name: string;
    extensions: string[];
  }

  interface Props {
    label: string;
    value: string;
    mode: "folder" | "file";
    placeholder?: string;
    note?: string;
    filters?: DialogFilter[];
    onchange?: () => void | Promise<void>;
  }

  let {
    label,
    value = $bindable(),
    mode,
    placeholder = "",
    note = "",
    filters,
    onchange,
  }: Props = $props();

  async function browse() {
    try {
      const selected = await open({
        directory: mode === "folder",
        multiple: false,
        filters: mode === "file" ? filters : undefined,
      });
      if (selected === null) return;
      value = Array.isArray(selected) ? (selected[0] ?? value) : selected;
      await onchange?.();
    } catch (e) {
      console.error("[path-picker] browse failed:", e);
    }
  }

  async function onBlur() {
    await onchange?.();
  }
</script>

<label class="row-label">
  <span>{label}</span>
  <div class="path-input-row">
    <input type="text" bind:value onblur={onBlur} {placeholder} />
    <button class="browse-btn" type="button" onclick={browse}>Browse...</button>
  </div>
</label>
{#if note}
  <p class="field-note">{note}</p>
{/if}

<style>
  .field-note { color: var(--dim); font-size: 12px; margin: 0; }

  .row-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    font-size: 14px;
    color: var(--text);
  }

  .row-label span { flex: 1; }

  .path-input-row {
    flex: 0 0 300px;
    display: flex;
    gap: 8px;
  }

  .path-input-row input[type="text"] {
    flex: 1;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 14px;
    padding: 8px 12px;
    outline: none;
  }

  .path-input-row input:focus { border-color: var(--muted); }

  .browse-btn {
    flex: 0 0 auto;
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
    letter-spacing: 0.02em;
    padding: 8px 10px;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s;
  }

  .browse-btn:hover {
    border-color: var(--amber);
    color: var(--amber);
  }
</style>

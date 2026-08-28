<!-- HalluScribe - external chat import path settings, with the per-path
     "is there actually an export in there?" diagnostic (GROK_IMPORT_PLAN
     § 12.4.4). Before this, a path pointing at an empty or too-deeply-nested
     folder was completely silent: the sweep imported nothing and said nothing,
     which cost a whole sweep cycle to notice. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { HalluScribeSettings, ImportPathStatus } from "../../lib/types";
  import {
    importStatusTone,
    importStatusMessage,
    importStatusDetail,
    importStatusSummary,
  } from "../../lib/importStatus";
  import PathPickerField from "./PathPickerField.svelte";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
  }

  let { settings = $bindable(), onSave }: Props = $props();

  let statuses = $state<ImportPathStatus[]>([]);
  let checking = $state(false);

  // Generation counter so a slow check never overwrites a newer one - the
  // walk is bounded but four providers on a cold disk still take a moment.
  let checkGen = 0;

  async function refreshStatus() {
    const gen = ++checkGen;
    checking = true;
    try {
      const result = await invoke<ImportPathStatus[]>("chat_import_status");
      if (gen === checkGen) statuses = result;
    } catch {
      // A failed check must not masquerade as "no export found" - showing
      // nothing is honest, showing a warning we did not earn is not.
      if (gen === checkGen) statuses = [];
    } finally {
      if (gen === checkGen) checking = false;
    }
  }

  // Check once when Settings opens, then again after each path edit is saved.
  $effect(() => {
    void refreshStatus();
  });

  async function onBlur() {
    await onSave();
    await refreshStatus();
  }

  function statusFor(provider: string): ImportPathStatus | undefined {
    return statuses.find((status) => status.provider === provider);
  }

  let summary = $derived(importStatusSummary(statuses));
</script>

      <section class="settings-section">
        <h2 class="section-title">CHAT IMPORTS</h2>
        <p class="field-note">
          Point each of these at the folder you unpack that provider's export into. The extracted
          folder can be dropped in exactly as it comes, under any name — the provider's own
          nesting (Google Takeout, Grok's ttl/30d/export_data/…) plus one folder of your own is
          searched. When the same export file exists at two depths, only the shallower one is
          imported.
        </p>
        {#if summary}
          <p class="field-note field-note-warn">{summary}</p>
        {/if}

        {#snippet importStatus(provider: string)}
          {@const status = statusFor(provider)}
          {#if status && !checking}
            <p class="import-status import-status-{importStatusTone(status)}">
              {importStatusMessage(status)}
            </p>
            {#if importStatusDetail(status)}
              <p class="import-status-detail">{importStatusDetail(status)}</p>
            {/if}
          {/if}
        {/snippet}

        <PathPickerField
          label="ChatGPT import path"
          bind:value={settings.chatgpt_import_path}
          mode="folder"
          placeholder=".../chat_sessions/chatgpt"
          onchange={onBlur}
        />
        {@render importStatus("chatgpt")}

        <PathPickerField
          label="Claude.ai import path"
          bind:value={settings.claudeai_import_path}
          mode="folder"
          placeholder=".../chat_sessions/claude"
          onchange={onBlur}
        />
        {@render importStatus("claude_ai")}

        <PathPickerField
          label="Gemini import path"
          bind:value={settings.gemini_import_path}
          mode="folder"
          placeholder=".../chat_sessions/gemini"
          onchange={onBlur}
        />
        {@render importStatus("gemini")}

        <PathPickerField
          label="Grok import path"
          bind:value={settings.grok_import_path}
          mode="folder"
          placeholder=".../chat_sessions/grok"
          note="Folder holding prod-grok-backend.json."
          onchange={onBlur}
        />
        {@render importStatus("grok")}

        <PathPickerField
          label="Ollama Chat database path"
          bind:value={settings.ollama_chat_db_path}
          mode="file"
          placeholder="Auto-detected on Windows — leave blank to use default"
          note="Path to the Ollama desktop app's local chat database (db.sqlite). Leave blank to auto-detect on Windows (%LOCALAPPDATA%\Ollama\db.sqlite). Set manually on macOS/Linux."
          filters={[{ name: "SQLite database", extensions: ["sqlite", "db"] }]}
          onchange={onBlur}
        />

        <PathPickerField
          label="Forge sessions directory"
          bind:value={settings.forge_sessions_path}
          mode="folder"
          placeholder="Auto-detected — leave blank to use default"
          note="Path to the Forge VS Code extension sessions folder. Leave blank to auto-detect (~/.forge/sessions)."
          onchange={onBlur}
        />
      </section>

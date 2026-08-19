<!-- HalluScribe - external chat import path settings. -->
<script lang="ts">
  import type { HalluScribeSettings } from "../../lib/types";
  import PathPickerField from "./PathPickerField.svelte";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
  }

  let { settings = $bindable(), onSave }: Props = $props();
  function onBlur() { void onSave(); }
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

        <PathPickerField
          label="ChatGPT import path"
          bind:value={settings.chatgpt_import_path}
          mode="folder"
          placeholder=".../chat_sessions/chatgpt"
          onchange={onBlur}
        />

        <PathPickerField
          label="Claude.ai import path"
          bind:value={settings.claudeai_import_path}
          mode="folder"
          placeholder=".../chat_sessions/claude"
          onchange={onBlur}
        />

        <PathPickerField
          label="Gemini import path"
          bind:value={settings.gemini_import_path}
          mode="folder"
          placeholder=".../chat_sessions/gemini"
          onchange={onBlur}
        />

        <PathPickerField
          label="Grok import path"
          bind:value={settings.grok_import_path}
          mode="folder"
          placeholder=".../chat_sessions/grok"
          note="Folder holding prod-grok-backend.json."
          onchange={onBlur}
        />

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
          label="Continue data directory"
          bind:value={settings.continue_data_path}
          mode="folder"
          placeholder="Auto-detected — leave blank to use default"
          note="Root .continue directory. Leave blank to auto-detect (Windows: %APPDATA%\.continue; Linux/macOS: ~/.continue)."
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

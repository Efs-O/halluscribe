<!-- HalluScribe - "Business messages" settings section (Phase 9b-UI). The
     opt-in to reading customer conversations from an iPhone backup: the toggle
     (the consent gate), the backup folder, the default country code, the last
     import outcome, and "Run import now". -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { HalluScribeSettings } from "../../lib/types";
  import {
    sensitiveDataWarning,
    runImportEnabled,
    businessImportStatus,
  } from "../../lib/businessMessages";
  import PathPickerField from "./PathPickerField.svelte";
  import type { SettingsNotify } from "./settingsSectionTypes";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
    onNotify: SettingsNotify;
  }

  let { settings = $bindable(), onSave, onNotify }: Props = $props();

  let importing = $state(false);

  let status = $derived(businessImportStatus(settings.business_last_import, settings.business_last_error));
  let canRun = $derived(runImportEnabled(settings.business_ingestion_enabled));

  function onToggle() {
    void onSave();
  }

  async function onFieldChange() {
    await onSave();
  }

  async function runImportNow() {
    if (importing || !canRun) return;
    importing = true;
    try {
      await invoke("trigger_business_import");
      onNotify("Business import started — it runs in the background.");
    } catch (error) {
      onNotify(`Business import failed: ${String(error)}`, "warn", 3200);
    } finally {
      importing = false;
    }
  }
</script>

      <section class="settings-section">
        <h2 class="section-title">BUSINESS MESSAGES</h2>
        <p class="field-note field-note-warn">{sensitiveDataWarning()}</p>

        <label class="row-label">
          <span>Enable business ingestion</span>
          <input
            type="checkbox"
            bind:checked={settings.business_ingestion_enabled}
            onchange={onToggle}
          />
        </label>
        <p class="field-note">
          Off by default. When on, the sweep reads customer conversations (Messages, WhatsApp,
          WhatsApp Business, Viber) from the backup below and archives them to <em>this</em>
          workspace only — never the personal archive, and never the MCP.
        </p>

        <PathPickerField
          label="iPhone backup folder"
          bind:value={settings.apple_backup_path}
          mode="folder"
          placeholder=".../Backups/00008130-0012792A..."
          note="The backup folder that holds Manifest.db."
          onchange={onFieldChange}
        />

        <label class="row-label">
          <span>Default country code</span>
          <input
            type="text"
            bind:value={settings.business_default_country_code}
            onblur={onFieldChange}
            placeholder="e.g. 30"
            inputmode="numeric"
            maxlength="3"
          />
        </label>
        <p class="field-note">
          Used to normalise local numbers to E.164. Leave blank to match only already-international
          numbers (+… / 00…).
        </p>

        <div class="semantic-actions">
          <button
            class="action-btn"
            type="button"
            onclick={runImportNow}
            disabled={importing || !canRun}
          >
            {#if importing}importing...{:else}run import now{/if}
          </button>
          <p class="field-note" class:field-note-warn={status.tone === "warn"}>
            {status.text}
          </p>
        </div>
      </section>

<!-- HalluScribe - "Business messages" settings section (Phase 9b-UI). The
     opt-in to reading customer conversations from an iPhone backup: the toggle
     (the consent gate), the backup folder, the default country code, the last
     import outcome, and "Run import now". -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type {
    HalluScribeSettings,
    OwnerProfileStatusDto,
    SweepProgress,
    WorkspaceListDto,
  } from "../../lib/types";
  import {
    sensitiveDataWarning,
    runImportEnabled,
    businessImportStatus,
    profileTogglesVisible,
    profileToggleNote,
    ownerProfileMissingNote,
  } from "../../lib/businessMessages";
  import PathPickerField from "./PathPickerField.svelte";
  import SweepProgressBar from "../SweepProgressBar.svelte";
  import type { SettingsNotify } from "./settingsSectionTypes";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
    onNotify: SettingsNotify;
    /** The shared sweep state (App's SweepController): a business import is a sweep. */
    sweepRunning: boolean;
    sweepProgress: SweepProgress | null;
    /** The last sweep message (the same toast the header shows on the Sessions tab). */
    sweepToast: { msg: string; ok: boolean } | null;
    onRunImport: () => Promise<string | null>;
    /** Stop the running sweep after its current session (the header's stop). */
    onStopImport: () => Promise<void>;
  }

  let {
    settings = $bindable(),
    onSave,
    onNotify,
    sweepRunning,
    sweepProgress,
    sweepToast,
    onRunImport,
    onStopImport,
  }: Props = $props();
  // Set by the stop button; only read while a sweep runs, so it needs no reset
  // beyond the next stop/start (see `stopImport` and `runImportNow`).
  let stopRequested = $state(false);
  // The owner-profile toggles (D13) are only meaningful in an import-only
  // (non-host) workspace: the host already owns the profile. `active` is null
  // when the default root (the host) is active, so that is never import-only.
  let activeImportOnly = $state(false);
  // A failed list_workspaces call must not be silently swallowed: surface it
  // as a note while still defaulting the toggles to hidden (DEFECT 3).
  let workspaceError = $state("");
  // Per-scope owner-profile status from the business_profile_status command;
  // the note under a toggle that is on but whose owner profile is missing
  // (DEFECT 2) is derived from this.
  let ownerStatus = $state<OwnerProfileStatusDto[]>([]);

  async function loadProfileUi() {
    try {
      const list = await invoke<WorkspaceListDto>("list_workspaces");
      if (list.active == null) return;
      const active = list.workspaces.find((w) => w.path === list.active);
      activeImportOnly = active?.import_only ?? false;
    } catch (error) {
      activeImportOnly = false;
      workspaceError = `Could not detect the active workspace (${String(error)}); profile options are hidden.`;
      return;
    }
    if (!activeImportOnly) return;
    try {
      ownerStatus = await invoke<OwnerProfileStatusDto[]>("business_profile_status");
    } catch (error) {
      workspaceError = `Could not check the owner profile status (${String(error)}).`;
    }
  }

  onMount(() => {
    void loadProfileUi();
  });

  let status = $derived(businessImportStatus(settings.business_last_import, settings.business_last_error));
  let canRun = $derived(runImportEnabled(settings.business_ingestion_enabled));
  let showProfileToggles = $derived(profileTogglesVisible(activeImportOnly));
  let workMissingNote = $derived(
    ownerProfileMissingNote(ownerStatus.find((s) => s.scope === "work")?.state ?? "off", "work"),
  );
  let personalMissingNote = $derived(
    ownerProfileMissingNote(ownerStatus.find((s) => s.scope === "personal")?.state ?? "off", "personal"),
  );

  async function onToggle() {
    await onSave();
    // A toggle change can flip a scope into "on but owner profile missing",
    // so refresh the per-scope status that drives the missing-profile note.
    if (activeImportOnly) {
      try {
        ownerStatus = await invoke<OwnerProfileStatusDto[]>("business_profile_status");
      } catch (error) {
        // The save already succeeded, but the note may now be stale — say so
        // rather than silently keeping a possibly-wrong status (CODEX_EVAL #3).
        workspaceError = `Could not refresh the owner profile status (${String(error)}); the note below may be out of date.`;
      }
    }
  }

  async function onFieldChange() {
    await onSave();
  }

  // A stop request only applies to the sweep that was running; clear it when
  // that sweep ends so the next one starts with a working stop button.
  $effect(() => {
    if (!sweepRunning) stopRequested = false;
  });

  async function runImportNow() {
    if (sweepRunning || !canRun) return;
    stopRequested = false;
    const error = await onRunImport();
    if (error) onNotify(`Business import failed: ${error}`, "warn", 3200);
    else onNotify("Business import started — it runs in the background.");
  }

  async function stopImport() {
    if (!sweepRunning || stopRequested) return;
    try {
      await onStopImport();
      stopRequested = true;
    } catch (error) {
      onNotify(`Could not stop the import: ${String(error)}`, "warn", 3200);
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
          workspace only. The MCP serves the default workspace, so chats imported there are
          readable by connected agents.
        </p>

        <PathPickerField
          label="iPhone backup folder"
          bind:value={settings.apple_backup_path}
          mode="folder"
          placeholder=".../Backups/00008130-0012792A..."
          note="The backup folder that holds Manifest.db."
          onchange={onFieldChange}
        />

        {#if workspaceError}
          <p class="field-note field-note-warn">{workspaceError}</p>
        {/if}

        {#if showProfileToggles}
          <label class="row-label">
            <span>Read your Work profile</span>
            <input
              type="checkbox"
              bind:checked={settings.business_use_work_profile}
              onchange={onToggle}
            />
          </label>
          <p class="field-note">{profileToggleNote("work")}</p>
          {#if workMissingNote}
            <p class="field-note field-note-warn">{workMissingNote}</p>
          {/if}
          <label class="row-label">
            <span>Read your Personal profile</span>
            <input
              type="checkbox"
              bind:checked={settings.business_use_personal_profile}
              onchange={onToggle}
            />
          </label>
          <p class="field-note">{profileToggleNote("personal")}</p>
          {#if personalMissingNote}
            <p class="field-note field-note-warn">{personalMissingNote}</p>
          {/if}
        {/if}

        <label class="row-label">
          <span>Default country code</span>
          <input
            type="text"
            bind:value={settings.business_default_country_code}
            onblur={onFieldChange}
            placeholder="e.g. 30"
            inputmode="numeric"
            maxlength="4"
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
            disabled={sweepRunning || !canRun}
          >
            {#if sweepRunning}sweep running...{:else}run import now{/if}
          </button>
          {#if sweepRunning}
            <button
              class="action-btn"
              type="button"
              onclick={stopImport}
              disabled={stopRequested}
              title="Stop after the current chat finishes"
            >
              {#if stopRequested}stopping after this chat...{:else}stop{/if}
            </button>
          {/if}
          {#if sweepRunning && sweepProgress}
            <SweepProgressBar progress={sweepProgress} />
          {/if}
          {#if !sweepRunning && sweepToast}
            <!-- The sweep-done message; otherwise it only shows on the Sessions tab. -->
            <p class="field-note" class:field-note-warn={!sweepToast.ok}>{sweepToast.msg}</p>
          {/if}
          <p class="field-note" class:field-note-warn={status.tone === "warn"}>
            {status.text}
          </p>
        </div>
      </section>

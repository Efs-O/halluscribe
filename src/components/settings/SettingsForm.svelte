<!-- HalluScribe - settings form composition, persistence, and save feedback. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { HalluScribeSettings, SweepProgress } from "../../lib/types";
  import "./SettingsForm.css";
  import BriefingSettings from "./BriefingSettings.svelte";
  import BusinessMessagesSettings from "./BusinessMessagesSettings.svelte";
  import ChatImportSettings from "./ChatImportSettings.svelte";
  import ModelBackendSettings from "./ModelBackendSettings.svelte";
  import SemanticSearchSettings from "./SemanticSearchSettings.svelte";
  import SweepSettings from "./SweepSettings.svelte";
  import TtsVoiceSettings from "./TtsVoiceSettings.svelte";
  import WorkspaceSwitcher from "./WorkspaceSwitcher.svelte";

  interface Props {
    initialSettings?: HalluScribeSettings | null;
    onSaved?: (settings: HalluScribeSettings) => void;
    sweepRunning: boolean;
    sweepProgress: SweepProgress | null;
    sweepToast: { msg: string; ok: boolean } | null;
    onRunBusinessImport: () => Promise<string | null>;
    onStopBusinessImport: () => Promise<void>;
  }

  let {
    initialSettings = null,
    onSaved,
    sweepRunning,
    sweepProgress,
    sweepToast,
    onRunBusinessImport,
    onStopBusinessImport,
  }: Props = $props();
  let settings = $state<HalluScribeSettings | null>(null);
  let savedFlash = $state(false);
  let savedMessage = $state("saved");
  let savedTone = $state<"ok" | "warn">("ok");
  let flashTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    if (!initialSettings) {
      invoke<HalluScribeSettings>("get_settings").then((value) => { settings = value; });
    }
  });

  $effect(() => {
    if (initialSettings) settings = initialSettings;
  });

  function showFlash(message: string, tone: "ok" | "warn" = "ok", duration = 2200) {
    clearTimeout(flashTimer);
    savedMessage = message;
    savedTone = tone;
    savedFlash = true;
    flashTimer = setTimeout(() => { savedFlash = false; }, duration);
  }

  function toPlainSettings(value: HalluScribeSettings): HalluScribeSettings {
    return JSON.parse(JSON.stringify(value)) as HalluScribeSettings;
  }

  async function save(message = "saved"): Promise<boolean> {
    if (!settings) return false;
    try {
      await invoke("save_settings", { newSettings: settings });
      onSaved?.(toPlainSettings(settings));
      showFlash(message);
      return true;
    } catch (error) {
      console.error("[settings] save failed:", error);
      showFlash(`save failed: ${String(error)}`, "warn", 3200);
      return false;
    }
  }
</script>

<div class="form-wrap">
  {#if !settings}
    <p class="loading">loading settings...</p>
  {:else}
    <form class="settings-form" onsubmit={(event) => event.preventDefault()}>
      <WorkspaceSwitcher />
      <ModelBackendSettings bind:settings onSave={save} onNotify={showFlash} />
      <SemanticSearchSettings bind:settings onSave={save} onNotify={showFlash} />
      <SweepSettings bind:settings onSave={save} onNotify={showFlash} />
      <ChatImportSettings bind:settings onSave={save} />
      <BusinessMessagesSettings
        bind:settings
        onSave={save}
        onNotify={showFlash}
        {sweepRunning}
        {sweepProgress}
        {sweepToast}
        onRunImport={onRunBusinessImport}
        onStopImport={onStopBusinessImport}
      />
      <BriefingSettings bind:settings onSave={save} />
      <TtsVoiceSettings
        bind:piperBin={settings.tts_piper_bin}
        bind:voice={settings.tts_voice}
        onSave={() => { void save(); }}
      />
    </form>

    {#if savedFlash}
      <div class:saved-warn={savedTone === "warn"} class="saved-flash">{savedMessage}</div>
    {/if}
  {/if}
</div>

<style>
  .form-wrap {
    flex: 1;
    overflow-y: auto;
    padding: 24px 28px;
    position: relative;
  }

  .loading { color: var(--muted); font-size: 14px; }

  .saved-flash {
    position: fixed;
    bottom: 16px;
    right: 16px;
    background: rgba(56, 150, 89, 0.14);
    border: 1px solid hsla(141, 45.60%, 40.40%, 0.45);
    border-radius: 5px;
    color: #9ee3b2;
    font-size: 13px;
    padding: 8px 16px;
    max-width: min(520px, calc(100vw - 32px));
  }

  .saved-flash.saved-warn {
    background: rgba(214, 143, 57, 0.12);
    border-color: rgba(214, 143, 57, 0.45);
    color: #f0c07a;
  }
</style>

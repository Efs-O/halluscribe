<!-- HalluScribe - voice (text-to-speech) settings: piper status, voice picker, binary path. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import PathPickerField from "./PathPickerField.svelte";

  interface VoiceInfo {
    name: string;
    sample_rate: number;
  }

  interface TtsStatus {
    piper_installed: boolean;
    voice_count: number;
    selected_voice: string;
  }

  interface Props {
    piperBin: string;
    voice: string;
    onSave: () => void | Promise<void>;
  }

  let { piperBin = $bindable(), voice = $bindable(), onSave }: Props = $props();

  let voices = $state<VoiceInfo[]>([]);
  let status = $state<TtsStatus | null>(null);
  let probeFailed = $state(false);

  async function refresh() {
    try {
      const [voiceList, ttsStatus] = await Promise.all([
        invoke<VoiceInfo[]>("tts_list_voices"),
        invoke<TtsStatus>("tts_status"),
      ]);
      voices = voiceList;
      status = ttsStatus;
      probeFailed = false;
    } catch (e) {
      console.error("[tts] status probe failed:", e);
      voices = [];
      status = null;
      probeFailed = true;
    }
  }

  onMount(() => {
    void refresh();
  });

  async function onVoiceChange() {
    await onSave();
    await refresh();
  }

  async function onPathBlur() {
    await onSave();
    await refresh();
  }
</script>

<section>
  <h2 class="section-title">VOICE (TEXT-TO-SPEECH)</h2>

  <p class="field-note">
    {#if probeFailed}
      Piper: unknown (status check failed)
    {:else if status}
      Piper: {status.piper_installed ? "installed ✓" : "not found"} — Voices: {status.voice_count} installed
    {:else}
      Checking piper status...
    {/if}
  </p>

  <label class="row-label">
    <span>Voice</span>
    {#if voices.length > 0}
      <select bind:value={voice} onchange={onVoiceChange}>
        {#if !voice}
          <option value="" disabled>— select a voice —</option>
        {/if}
        {#each voices as v (v.name)}
          <option value={v.name}>{v.name}</option>
        {/each}
      </select>
    {:else}
      <span class="voice-empty-hint">No voices installed — see setup below</span>
    {/if}
  </label>

  <PathPickerField
    label="Piper binary path"
    bind:value={piperBin}
    mode="file"
    placeholder="auto-detect under ~/.halluscribe/tts/piper"
    note="Leave blank to auto-search the piper folder under the HalluScribe archive."
    onchange={onPathBlur}
  />

  <details class="setup-details">
    <summary>Setup instructions</summary>
    <p class="field-note">
      Text-to-speech runs fully offline via <strong>Piper</strong>. Download a piper binary for your
      platform and one or more voice packs (for example Greek <code>el_GR-joy-medium</code> or English
      <code>en_US-amy-medium</code>) from the Piper voices library on Hugging Face
      (<code>rhasspy/piper-voices</code>). Place the piper binary (and its runtime files) under
      <code>~/.halluscribe/tts/piper/</code>, and voice files (<code>.onnx</code> + <code>.onnx.json</code>)
      under <code>~/.halluscribe/tts/voices/</code>. Both folders are shared across workspaces on this
      machine.
    </p>
  </details>
</section>

<style>
  .field-note { color: var(--dim); font-size: 12px; margin: 0; }

  .section-title {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--dim);
    margin-bottom: 4px;
  }

  .row-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    font-size: 14px;
    color: var(--text);
  }

  .row-label span { flex: 1; }

  .row-label select {
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

  .row-label select:focus { border-color: var(--muted); }

  .voice-empty-hint {
    flex: 0 0 300px;
    color: var(--dim);
    font-size: 13px;
  }

  .setup-details {
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.03);
  }

  .setup-details summary {
    cursor: pointer;
    font-size: 13px;
    color: var(--text);
    letter-spacing: 0.02em;
  }

  .setup-details .field-note {
    margin-top: 8px;
    line-height: 1.5;
  }
</style>

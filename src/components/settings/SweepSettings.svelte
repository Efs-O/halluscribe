<!-- HalluScribe - scheduled sweep and raw transcript recovery settings. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { BackfillResult, HalluScribeSettings } from "../../lib/types";
  import { normalizeScheduleTime } from "../../lib/scheduleTime";
  import type { SettingsNotify } from "./settingsSectionTypes";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
    onNotify: SettingsNotify;
  }

  let { settings = $bindable(), onSave, onNotify }: Props = $props();
  let backfillRunning = $state(false);
  let backfillMessage = $state("");

  function onToggle() { void onSave(); }
  function onBlur() { void onSave(); }

  function scheduleHour(): string {
    return settings.schedule_time.split(":")[0] ?? "";
  }

  function scheduleMinute(): string {
    return settings.schedule_time.split(":")[1] ?? "";
  }

  function setSchedulePart(part: "hour" | "minute", value: string) {
    const digits = value.replace(/\D/g, "").slice(0, 2);
    const nextHour = part === "hour" ? digits : scheduleHour();
    const nextMinute = part === "minute" ? digits : scheduleMinute();
    settings.schedule_time = `${nextHour}:${nextMinute}`;
  }

  async function onScheduleTimeBlur() {
    const normalized = normalizeScheduleTime(settings.schedule_time);
    if (!normalized) {
      onNotify("Sweep time must use 24-hour HH:MM, for example 18:30.", "warn", 3200);
      return;
    }
    settings.schedule_time = normalized;
    await onSave("sweep time saved");
  }

  async function recoverRawTranscripts() {
    if (backfillRunning) return;
    backfillRunning = true;
    backfillMessage = "";
    try {
      const result = await invoke<BackfillResult>("backfill_raw");
      const repaired = result.repaired > 0 ? `, repaired ${result.repaired}` : "";
      backfillMessage =
        `Recovered ${result.recovered} raw transcripts (already had ${result.already_had}${repaired}, source gone ${result.source_missing}, of ${result.total}).`;
      onNotify("raw transcript recovery complete");
    } catch (error) {
      backfillMessage = `Raw transcript recovery failed: ${String(error)}`;
      onNotify("raw transcript recovery failed", "warn", 3200);
    } finally {
      backfillRunning = false;
    }
  }
</script>

      <section class="settings-section">
        <h2 class="section-title">SWEEP</h2>

        <label class="row-label">
          <span>Scheduled nightly sweep</span>
          <input type="checkbox" bind:checked={settings.scheduled_processing_enabled} onchange={onToggle} />
        </label>
        <p class="field-note">
          Off by default — sweeps run only when you click "Run Now". When on, an unattended sweep
          runs daily against the <strong>active</strong> workspace, so enable it per workspace only
          when you want that person's archive kept up to date automatically.
        </p>

        <p class="field-note">
          A compressed copy of each session's original transcript is kept in <code>~/.halluscribe/raw/</code>
          so raw detail survives after the source tool prunes its logs. Raw copies are the untouched
          source — they are never redacted, and Persona Pack exports exclude them unless you opt in per-export.
        </p>

        <div class="semantic-actions">
          <button
            class="action-btn"
            type="button"
            onclick={recoverRawTranscripts}
            disabled={backfillRunning}
          >
            {#if backfillRunning}recovering raw transcripts...{:else}recover raw transcripts{/if}
          </button>
          {#if backfillMessage}
            <p class="field-note" class:field-note-warn={backfillMessage.includes("failed")}>
              {backfillMessage}
            </p>
          {/if}
        </div>

        <label class="row-label">
          <span>Sweep time (24-hour)</span>
          <div class="time-input" onfocusout={onScheduleTimeBlur}>
            <input
              class="time-part"
              type="text"
              value={scheduleHour()}
              oninput={(e) => setSchedulePart("hour", (e.currentTarget as HTMLInputElement).value)}
              inputmode="numeric"
              maxlength="2"
              placeholder="18"
              aria-label="Sweep hour"
            />
            <span class="time-separator">:</span>
            <input
              class="time-part"
              type="text"
              value={scheduleMinute()}
              oninput={(e) => setSchedulePart("minute", (e.currentTarget as HTMLInputElement).value)}
              inputmode="numeric"
              maxlength="2"
              placeholder="30"
              aria-label="Sweep minute"
            />
          </div>
        </label>

        <p class="field-note">Use 24-hour time in <code>HH:MM</code> format, for example <code>02:00</code> or <code>18:30</code>.</p>

        <label class="row-label">
          <span>Min fill % to archive</span>
          <input type="number" bind:value={settings.summary_min_fill_pct} onblur={onBlur} min="0" max="100" step="1" />
        </label>

        <label class="row-label">
          <span>Lookback hours</span>
          <input type="number" bind:value={settings.lookback_hours} onblur={onBlur} min="1" />
        </label>
      </section>

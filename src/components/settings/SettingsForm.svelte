<!-- HalluScribe - settings form. Save-on-change (blur for text/number). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { BackfillResult, EmbeddingRebuildProgress, HalluScribeSettings } from "../../lib/types";
  import WorkspaceSwitcher from "./WorkspaceSwitcher.svelte";
  import TtsVoiceSettings from "./TtsVoiceSettings.svelte";

  interface ApiKeyValidationResult {
    status: "valid" | "invalid" | "unreachable" | "empty";
    message: string;
  }

  interface Props {
    initialSettings?: HalluScribeSettings | null;
    onSaved?: (settings: HalluScribeSettings) => void;
  }

  let { initialSettings = null, onSaved }: Props = $props();
  let s = $state<HalluScribeSettings | null>(null);
  let savedFlash = $state(false);
  let savedMessage = $state("saved");
  let savedTone = $state<"ok" | "warn">("ok");
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  let rebuildRunning = $state(false);
  let rebuildProgress = $state<EmbeddingRebuildProgress | null>(null);
  let rebuildMessage = $state("");
  let backfillRunning = $state(false);
  let backfillMessage = $state("");
  const SCHEDULE_TIME_PATTERN = /^([01]\d|2[0-3]):([0-5]\d)$/;

  onMount(() => {
    if (!initialSettings) {
      invoke<HalluScribeSettings>("get_settings").then((value) => { s = value; });
    }

    const unlistenPromise = listen<EmbeddingRebuildProgress>("embedding-rebuild-progress", (event) => {
      rebuildProgress = event.payload;
    });

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  });

  $effect(() => {
    if (initialSettings) {
      s = initialSettings;
    }
  });

  function showFlash(message: string, tone: "ok" | "warn" = "ok", duration = 2200) {
    clearTimeout(flashTimer);
    savedMessage = message;
    savedTone = tone;
    savedFlash = true;
    flashTimer = setTimeout(() => { savedFlash = false; }, duration);
  }

  function toPlainSettings(settings: HalluScribeSettings): HalluScribeSettings {
    return JSON.parse(JSON.stringify(settings)) as HalluScribeSettings;
  }

  async function save(message = "saved") {
    if (!s) return;
    try {
      await invoke("save_settings", { newSettings: s });
      onSaved?.(toPlainSettings(s));
      showFlash(message);
      return true;
    } catch (e) {
      console.error("[settings] save failed:", e);
      showFlash(`save failed: ${String(e)}`, "warn", 3200);
      return false;
    }
  }

  function onToggle() { void save(); }
  function onBlur() { void save(); }

  function scheduleHour(): string {
    if (!s) return "";
    return s.schedule_time.split(":")[0] ?? "";
  }

  function scheduleMinute(): string {
    if (!s) return "";
    return s.schedule_time.split(":")[1] ?? "";
  }

  function setSchedulePart(part: "hour" | "minute", value: string) {
    if (!s) return;
    const digits = value.replace(/\D/g, "").slice(0, 2);
    const nextHour = part === "hour" ? digits : scheduleHour();
    const nextMinute = part === "minute" ? digits : scheduleMinute();
    s.schedule_time = `${nextHour}:${nextMinute}`;
  }

  function normalizeScheduleTime(value: string): string | null {
    const trimmed = value.trim();
    if (SCHEDULE_TIME_PATTERN.test(trimmed)) return trimmed;
    const compact = /^(\d{1,2}):(\d{1,2})$/.exec(trimmed);
    if (!compact) return null;
    const hour = Number(compact[1]);
    const minute = Number(compact[2]);
    if (!Number.isInteger(hour) || !Number.isInteger(minute)) return null;
    if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
    return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
  }

  async function onScheduleTimeBlur() {
    if (!s) return;
    const normalized = normalizeScheduleTime(s.schedule_time);
    if (!normalized) {
      showFlash("Sweep time must use 24-hour HH:MM, for example 18:30.", "warn", 3200);
      return;
    }
    s.schedule_time = normalized;
    await save("sweep time saved");
  }

  async function onApiKeyBlur() {
    if (!(await save("API key stored"))) return;
    if (!s) return;
    const result = await invoke<ApiKeyValidationResult>("validate_ollama_api_key", {
      apiKey: s.ollama_api_key,
    });
    showFlash(
      result.message,
      result.status === "valid" || result.status === "empty" ? "ok" : "warn",
      4200,
    );
  }

  function onApiKeyKeyDown(e: KeyboardEvent) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    (e.currentTarget as HTMLInputElement).blur();
  }

  function rebuildSummary(progress: EmbeddingRebuildProgress): string {
    return `${progress.current}/${progress.total} sessions processed | updated ${progress.indexed}, already current ${progress.skipped}, failed ${progress.failed}`;
  }

  async function rebuildEmbeddings() {
    if (rebuildRunning) return;
    rebuildRunning = true;
    rebuildProgress = null;
    rebuildMessage = "Starting embedding rebuild...";
    try {
      const result = await invoke<{ indexed: number; skipped: number; failed: number }>("rebuild_session_embeddings");
      rebuildMessage =
        `Embedding rebuild complete. Updated ${result.indexed}, already current ${result.skipped}, failed ${result.failed}.`;
      showFlash("embedding rebuild complete");
    } catch (e) {
      rebuildMessage = `Embedding rebuild failed: ${String(e)}`;
      showFlash("embedding rebuild failed", "warn", 3200);
    } finally {
      rebuildRunning = false;
    }
  }

  async function recoverRawTranscripts() {
    if (backfillRunning || !s?.preserve_raw_transcripts) return;
    backfillRunning = true;
    backfillMessage = "";
    try {
      const r = await invoke<BackfillResult>("backfill_raw");
      backfillMessage =
        `Recovered ${r.recovered} raw transcripts (already had ${r.already_had}, source gone ${r.source_missing}, of ${r.total}).`;
      showFlash("raw transcript recovery complete");
    } catch (e) {
      backfillMessage = `Raw transcript recovery failed: ${String(e)}`;
      showFlash("raw transcript recovery failed", "warn", 3200);
    } finally {
      backfillRunning = false;
    }
  }
</script>

<div class="form-wrap">
  {#if !s}
    <p class="loading">loading settings...</p>
  {:else}
    <form class="form" onsubmit={(e) => e.preventDefault()}>
      <WorkspaceSwitcher />

      <section>
        <h2 class="section-title">GENERAL</h2>

        <label class="row-label">
          <span>Backend</span>
          <select bind:value={s.backend} onchange={onToggle}>
            <option value="llamacpp">llama.cpp</option>
            <option value="ollama">Ollama</option>
          </select>
        </label>

        <div class="backend-note">
          {#if s.backend === "llamacpp"}
            llama.cpp is currently used for sweep session summaries, briefing generation, archive chat, and semantic chat.
          {:else}
            Ollama is currently used for sweep session summaries, briefing generation, archive chat, and semantic chat.
          {/if}
        </div>
      </section>

      <section>
        <h2 class="section-title">LLAMA.CPP</h2>

        <label class="row-label">
          <span>llama-server binary path</span>
          <input
            type="text"
            bind:value={s.llama_server_bin}
            onblur={onBlur}
            placeholder="/path/to/llama-server"
          />
        </label>

        <p class="field-note">Used for local Gemma generation when backend is llama.cpp, and always used for EmbeddingGemma semantic retrieval.</p>

        {#if s.backend === "llamacpp"}
          <label class="row-label">
            <span>Gemma GGUF model path</span>
            <input
              type="text"
              bind:value={s.gemma_model_path}
              onblur={onBlur}
              placeholder="/path/to/model.gguf"
            />
          </label>

          <label class="row-label">
            <span>GPU layers (-1 = all)</span>
            <input type="number" bind:value={s.gpu_layers} onblur={onBlur} min="-1" />
          </label>

          <label class="row-label">
            <span>llama-server port</span>
            <input
              type="number"
              bind:value={s.llama_server_port}
              onblur={onBlur}
              min="1024"
              max="65535"
            />
          </label>
        {/if}
      </section>

      {#if s.backend === "ollama"}
        <section>
          <h2 class="section-title">OLLAMA</h2>

          <label class="row-label">
            <span>Host</span>
            <input type="text" bind:value={s.ollama_host} onblur={onBlur} />
          </label>

          <label class="row-label">
            <span>Port</span>
            <input type="number" bind:value={s.ollama_port} onblur={onBlur} min="1" max="65535" />
          </label>

          <label class="row-label">
            <span>Model tag</span>
            <input type="text" bind:value={s.ollama_model} onblur={onBlur} placeholder="gemma4:26b" />
          </label>

          <label class="row-label">
            <span>Ollama API key</span>
            <input
              type="password"
              bind:value={s.ollama_api_key}
              onblur={onApiKeyBlur}
              onkeydown={onApiKeyKeyDown}
              placeholder="ollama_..."
            />
          </label>

          <p class="field-note">Stored locally on this machine. Required only for optional cloud-backed web search.</p>

          <label class="row-label">
            <span>Tavily API key</span>
            <input
              type="password"
              bind:value={s.tavily_api_key}
              onblur={() => save("Tavily API key stored")}
              onkeydown={onApiKeyKeyDown}
              placeholder="tvly-..."
            />
          </label>

          <p class="field-note">Optional fallback for web search when Ollama web search is unavailable. Stored locally on this machine.</p>
        </section>
      {/if}

      <section>
        <h2 class="section-title">SEMANTIC SEARCH</h2>

        <label class="row-label">
          <span>EmbeddingGemma GGUF model path</span>
          <input
            type="text"
            bind:value={s.embedding_model_path}
            onblur={onBlur}
            placeholder=".../embeddinggemma-300m-q4_0.gguf"
          />
        </label>

        <p class="field-note">Semantic search uses llama.cpp locally with this EmbeddingGemma GGUF model. Session embeddings are updated during sweeps and rebuilds, not on every chat query.</p>

        <div class="semantic-actions">
          <button
            class="action-btn"
            type="button"
            onclick={rebuildEmbeddings}
            disabled={rebuildRunning}
          >
            {#if rebuildRunning}rebuilding embeddings...{:else}rebuild embeddings{/if}
          </button>
          {#if rebuildProgress}
            <p class="field-note">{rebuildSummary(rebuildProgress)}</p>
          {/if}
          {#if rebuildMessage}
            <p class="field-note" class:field-note-warn={rebuildMessage.includes("failed")}>
              {rebuildMessage}
            </p>
          {/if}
        </div>
      </section>

      <section>
        <h2 class="section-title">SWEEP</h2>

        <label class="row-label">
          <span>Scheduled nightly sweep</span>
          <input type="checkbox" bind:checked={s.scheduled_processing_enabled} onchange={onToggle} />
        </label>
        <p class="field-note">
          Off by default — sweeps run only when you click "Run Now". When on, an unattended sweep
          runs daily against the <strong>active</strong> workspace, so enable it per workspace only
          when you want that person's archive kept up to date automatically.
        </p>

        <label class="row-label">
          <span>Preserve raw transcripts</span>
          <input type="checkbox" bind:checked={s.preserve_raw_transcripts} onchange={onToggle} />
        </label>

        <p class="field-note">
          Keeps a compressed copy of each session's original transcript in <code>~/.halluscribe/raw/</code>
          so raw detail survives after the source tool prunes its logs. Raw copies are the untouched
          source — they are never redacted, and Persona Pack exports exclude them unless you opt in per-export.
        </p>

        <div class="semantic-actions">
          <button
            class="action-btn"
            type="button"
            onclick={recoverRawTranscripts}
            disabled={backfillRunning || !s.preserve_raw_transcripts}
          >
            {#if backfillRunning}recovering raw transcripts...{:else}recover raw transcripts{/if}
          </button>
          {#if !s.preserve_raw_transcripts}
            <p class="field-note">Enable Preserve raw transcripts above to recover history for existing sessions.</p>
          {/if}
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
          <input type="number" bind:value={s.summary_min_fill_pct} onblur={onBlur} min="0" max="100" step="1" />
        </label>

        <label class="row-label">
          <span>Lookback hours</span>
          <input type="number" bind:value={s.lookback_hours} onblur={onBlur} min="1" />
        </label>
      </section>

      <section>
        <h2 class="section-title">CHAT IMPORTS</h2>

        <label class="row-label">
          <span>ChatGPT import path</span>
          <input
            type="text"
            bind:value={s.chatgpt_import_path}
            onblur={onBlur}
            placeholder=".../chat_sessions/chatgpt"
          />
        </label>

        <label class="row-label">
          <span>Claude.ai import path</span>
          <input
            type="text"
            bind:value={s.claudeai_import_path}
            onblur={onBlur}
            placeholder=".../chat_sessions/claude"
          />
        </label>

        <label class="row-label">
          <span>Gemini import path</span>
          <input
            type="text"
            bind:value={s.gemini_import_path}
            onblur={onBlur}
            placeholder=".../chat_sessions/gemini"
          />
        </label>

        <label class="row-label">
          <span>Ollama Chat database path</span>
          <input
            type="text"
            bind:value={s.ollama_chat_db_path}
            onblur={onBlur}
            placeholder="Auto-detected on Windows — leave blank to use default"
          />
        </label>
        <p class="field-note">Path to the Ollama desktop app's local chat database (db.sqlite). Leave blank to auto-detect on Windows (%LOCALAPPDATA%\Ollama\db.sqlite). Set manually on macOS/Linux.</p>

        <label class="row-label">
          <span>Continue data directory</span>
          <input
            type="text"
            bind:value={s.continue_data_path}
            onblur={onBlur}
            placeholder="Auto-detected — leave blank to use default"
          />
        </label>
        <p class="field-note">Root .continue directory. Leave blank to auto-detect (Windows: %APPDATA%\.continue; Linux/macOS: ~/.continue).</p>

        <label class="row-label">
          <span>Forge sessions directory</span>
          <input
            type="text"
            bind:value={s.forge_sessions_path}
            onblur={onBlur}
            placeholder="Auto-detected — leave blank to use default"
          />
        </label>
        <p class="field-note">Path to the Forge VS Code extension sessions folder. Leave blank to auto-detect (~/.forge/sessions).</p>
      </section>

      <section>
        <h2 class="section-title">BRIEFING</h2>

        <label class="row-label">
          <span>Context window (tokens)</span>
          <input type="number" bind:value={s.ctx_size} onblur={onBlur} min="8192" max="131072" step="1024" />
        </label>

        <label class="row-label">
          <span>Max output tokens</span>
          <input type="number" bind:value={s.max_tokens} onblur={onBlur} min="512" max="65536" step="512" />
        </label>

        <p class="field-note">Required for sweep session summaries, briefing generation, archive chat, and semantic chat. Generation will not run until both values are set.</p>
      </section>

      <TtsVoiceSettings bind:piperBin={s.tts_piper_bin} bind:voice={s.tts_voice} onSave={onBlur} />
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
  .field-note { color: var(--dim); font-size: 12px; margin: 0; }
  .field-note.field-note-warn { color: #f0c07a; }
  .form { display: flex; flex-direction: column; gap: 32px; max-width: 780px; }
  section { display: flex; flex-direction: column; gap: 14px; }
  .semantic-actions { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .backend-note {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.03);
    color: var(--amber);
    font-size: 12px;
    line-height: 1.5;
  }

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

  .row-label input[type="text"],
  .row-label input[type="password"],
  .row-label input[type="number"],
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

  .row-label input:focus,
  .row-label select:focus { border-color: var(--muted); }

  .time-input {
    flex: 0 0 140px;
    width: 140px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px 12px;
  }

  .time-input:focus-within {
    border-color: var(--muted);
  }

  .time-input .time-part {
    flex: 0 0 44px;
    width: 44px;
    min-width: 44px;
    max-width: 44px;
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--text);
    font-family: inherit;
    font-size: 14px;
    padding: 0 4px;
    box-sizing: border-box;
    text-align: center;
    outline: none;
  }

  .time-separator {
    color: var(--text);
    font-size: 16px;
    line-height: 1;
  }

  .row-label input[type="checkbox"] {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    accent-color: var(--green);
    cursor: pointer;
  }

  .action-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
    letter-spacing: 0.06em;
    padding: 8px 12px;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s, opacity 0.15s;
  }

  .action-btn:hover:not(:disabled) {
    border-color: var(--amber);
    color: var(--amber);
  }

  .action-btn:disabled {
    cursor: default;
    opacity: 0.6;
  }

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

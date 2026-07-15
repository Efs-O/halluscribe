<!-- HalluScribe - semantic search model and embedding rebuild settings. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import type { EmbeddingRebuildProgress, HalluScribeSettings } from "../../lib/types";
  import type { SettingsNotify } from "./settingsSectionTypes";
  import PathPickerField from "./PathPickerField.svelte";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
    onNotify: SettingsNotify;
  }

  let { settings = $bindable(), onSave, onNotify }: Props = $props();
  let rebuildRunning = $state(false);
  let rebuildProgress = $state<EmbeddingRebuildProgress | null>(null);
  let rebuildMessage = $state("");

  onMount(() => {
    const unlistenPromise = listen<EmbeddingRebuildProgress>("embedding-rebuild-progress", (event) => {
      rebuildProgress = event.payload;
    });
    return () => { void unlistenPromise.then((unlisten) => unlisten()); };
  });

  function onBlur() { void onSave(); }

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
      onNotify("embedding rebuild complete");
    } catch (error) {
      rebuildMessage = `Embedding rebuild failed: ${String(error)}`;
      onNotify("embedding rebuild failed", "warn", 3200);
    } finally {
      rebuildRunning = false;
    }
  }
</script>

      <section class="settings-section">
        <h2 class="section-title">SEMANTIC SEARCH</h2>

        <PathPickerField
          label="EmbeddingGemma GGUF model path"
          bind:value={settings.embedding_model_path}
          mode="file"
          placeholder=".../embeddinggemma-300m-q4_0.gguf"
          note="Semantic search uses llama.cpp locally with this EmbeddingGemma GGUF model. Session embeddings are updated during sweeps and rebuilds, not on every chat query."
          filters={[{ name: "GGUF model", extensions: ["gguf"] }]}
          onchange={onBlur}
        />

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

<!-- HalluScribe - local inference backend and API credential settings. -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { HalluScribeSettings } from "../../lib/types";
  import type { SettingsNotify } from "./settingsSectionTypes";
  import PathPickerField from "./PathPickerField.svelte";

  interface ApiKeyValidationResult {
    status: "valid" | "invalid" | "unreachable" | "empty";
    message: string;
  }

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
    onNotify: SettingsNotify;
  }

  let { settings = $bindable(), onSave, onNotify }: Props = $props();

  function onToggle() { void onSave(); }
  function onBlur() { void onSave(); }

  async function onApiKeyBlur() {
    if (!(await onSave("API key stored"))) return;
    const result = await invoke<ApiKeyValidationResult>("validate_ollama_api_key", {
      apiKey: settings.ollama_api_key,
    });
    onNotify(
      result.message,
      result.status === "valid" || result.status === "empty" ? "ok" : "warn",
      4200,
    );
  }

  function onApiKeyKeyDown(event: KeyboardEvent) {
    if (event.key !== "Enter") return;
    event.preventDefault();
    (event.currentTarget as HTMLInputElement).blur();
  }
</script>

      <section class="settings-section">
        <h2 class="section-title">GENERAL</h2>

        <label class="row-label">
          <span>Backend</span>
          <select bind:value={settings.backend} onchange={onToggle}>
            <option value="llamacpp">llama.cpp</option>
            <option value="ollama">Ollama</option>
          </select>
        </label>

        <div class="backend-note">
          {#if settings.backend === "llamacpp"}
            llama.cpp is currently used for sweep session summaries, briefing generation, archive chat, and semantic chat.
          {:else}
            Ollama is currently used for sweep session summaries, briefing generation, archive chat, and semantic chat.
          {/if}
        </div>
      </section>

      <section class="settings-section">
        <h2 class="section-title">LLAMA.CPP</h2>

        <PathPickerField
          label="llama-server binary path"
          bind:value={settings.llama_server_bin}
          mode="file"
          placeholder="/path/to/llama-server"
          note="Used for local Gemma generation when backend is llama.cpp, and always used for EmbeddingGemma semantic retrieval."
          onchange={onBlur}
        />

        {#if settings.backend === "llamacpp"}
          <PathPickerField
            label="Gemma GGUF model path"
            bind:value={settings.gemma_model_path}
            mode="file"
            placeholder="/path/to/model.gguf"
            filters={[{ name: "GGUF model", extensions: ["gguf"] }]}
            onchange={onBlur}
          />

          <label class="row-label">
            <span>GPU layers (-1 = all)</span>
            <input type="number" bind:value={settings.gpu_layers} onblur={onBlur} min="-1" />
          </label>

          <label class="row-label">
            <span>llama-server port</span>
            <input
              type="number"
              bind:value={settings.llama_server_port}
              onblur={onBlur}
              min="1024"
              max="65535"
            />
          </label>
        {/if}
      </section>

      {#if settings.backend === "ollama"}
        <section class="settings-section">
          <h2 class="section-title">OLLAMA</h2>

          <label class="row-label">
            <span>Host</span>
            <input type="text" bind:value={settings.ollama_host} onblur={onBlur} />
          </label>

          <label class="row-label">
            <span>Port</span>
            <input type="number" bind:value={settings.ollama_port} onblur={onBlur} min="1" max="65535" />
          </label>

          <label class="row-label">
            <span>Model tag</span>
            <input type="text" bind:value={settings.ollama_model} onblur={onBlur} placeholder="gemma4:26b" />
          </label>

          <label class="row-label">
            <span>Ollama API key</span>
            <input
              type="password"
              bind:value={settings.ollama_api_key}
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
              bind:value={settings.tavily_api_key}
              onblur={() => onSave("Tavily API key stored")}
              onkeydown={onApiKeyKeyDown}
              placeholder="tvly-..."
            />
          </label>

          <p class="field-note">Optional fallback for web search when Ollama web search is unavailable. Stored locally on this machine.</p>
        </section>
      {/if}

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
            <span>GPU devices</span>
            <input
              type="text"
              bind:value={settings.gpu_devices}
              onblur={onBlur}
              placeholder="empty = all devices"
            />
          </label>
          <p class="field-note">
            Comma-separated names from <code>llama-server --list-devices</code>, e.g.
            <code>CUDA1</code>. Pin the model to one card when the machine has GPUs of
            different speeds — llama.cpp otherwise offloads to every device it can see.
          </p>

          <label class="row-label">
            <span>Split mode</span>
            <select bind:value={settings.gpu_split_mode} onchange={onBlur}>
              <option value="">Default (layer)</option>
              <option value="none">none — single GPU</option>
              <option value="layer">layer — split by layer</option>
              <option value="row">row — split by row</option>
              <option value="tensor">tensor — split by tensor</option>
            </select>
          </label>

          <label class="row-label">
            <span>Tensor split</span>
            <input
              type="text"
              bind:value={settings.gpu_tensor_split}
              onblur={onBlur}
              placeholder="empty = even split"
            />
          </label>
          <p class="field-note">
            Fraction of the model per device, in device order, e.g. <code>0.8,0.2</code>.
            Only applies when more than one device is in use.
          </p>

          <label class="row-label">
            <span>Main GPU (-1 = unset)</span>
            <input type="number" bind:value={settings.gpu_main_index} onblur={onBlur} min="-1" />
          </label>
          <p class="field-note">
            Device index holding the KV cache and intermediate results. Leave unset unless
            device 0 is the card you want to avoid.
          </p>

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

<!-- HalluScribe - briefing context and output limit settings. -->
<script lang="ts">
  import type { HalluScribeSettings } from "../../lib/types";

  interface Props {
    settings: HalluScribeSettings;
    onSave: (message?: string) => Promise<boolean>;
  }

  let { settings = $bindable(), onSave }: Props = $props();
  function onBlur() { void onSave(); }
</script>

      <section class="settings-section">
        <h2 class="section-title">BRIEFING</h2>

        <label class="row-label">
          <span>Context window (tokens)</span>
          <input type="number" bind:value={settings.ctx_size} onblur={onBlur} min="8192" max="131072" step="1024" />
        </label>

        <label class="row-label">
          <span>Max output tokens</span>
          <input type="number" bind:value={settings.max_tokens} onblur={onBlur} min="512" max="65536" step="512" />
        </label>

        <p class="field-note">Required for sweep session summaries, briefing generation, archive chat, and semantic chat. Generation will not run until both values are set.</p>
      </section>

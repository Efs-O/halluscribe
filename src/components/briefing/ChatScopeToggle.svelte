<!-- HalluScribe - chat scope toggle for briefing. -->
<script lang="ts">
  import type { ChatScope } from "../../lib/types";

  interface Props {
    selectedScopeActive: boolean;
    chatScopeKind: ChatScope["kind"];
    chatStreaming: boolean;
    onSetChatScope: (kind: ChatScope["kind"]) => void | Promise<unknown>;
  }

  let { selectedScopeActive, chatScopeKind, chatStreaming, onSetChatScope }: Props = $props();
</script>

{#if selectedScopeActive}
  <div class="chat-scope-toggle">
    <button
      class:active={chatScopeKind === "briefing-scope"}
      onclick={() => onSetChatScope("briefing-scope")}
      disabled={chatStreaming}
    >selected sessions</button>
    <button
      class:active={chatScopeKind === "archive-wide"}
      onclick={() => onSetChatScope("archive-wide")}
      disabled={chatStreaming}
    >all archive</button>
  </div>
{/if}

<style>
  .chat-scope-toggle {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }

  .chat-scope-toggle button {
    background: transparent;
    border: 0;
    color: var(--dim);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    letter-spacing: 0.04em;
    padding: 6px 10px;
    transition: background 0.15s, color 0.15s;
  }

  .chat-scope-toggle button.active {
    background: color-mix(in srgb, var(--amber) 14%, transparent);
    color: var(--amber);
  }

  .chat-scope-toggle button:disabled {
    cursor: default;
    opacity: 0.6;
  }
</style>

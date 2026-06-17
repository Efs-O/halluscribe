<!-- HalluScribe — one chat turn (user or assistant). -->
<!-- Assistant turns show a ThinkingBubble + answer text. -->
<script lang="ts">
  import ThinkingBubble from "./ThinkingBubble.svelte";

  interface Props {
    role: "user" | "assistant";
    thinkingText: string;
    answerText: string;
    toolActivity: string | null;
    streaming: boolean;
    attachmentName?: string;
  }
  let { role, thinkingText, answerText, toolActivity, streaming, attachmentName }: Props = $props();

  function stripMarkdown(text: string): string {
    return text
      .replace(/^#{1,6}\s+/gm, '')
      .replace(/\*\*([^*]+)\*\*/g, '$1');
  }
</script>

<div class="message {role}">
  {#if role === "user"}
    <div class="user-bubble">
      {#if answerText.trim()}
        <div class="user-content selectable">{answerText}</div>
      {/if}
      {#if attachmentName}
        <div class="attachment-note">image: {attachmentName}</div>
      {/if}
    </div>
  {:else}
    {#if thinkingText}
      <ThinkingBubble text={thinkingText} {streaming} />
    {/if}
    {#if toolActivity}
      <div class="tool-activity">{toolActivity}</div>
    {/if}
    <div class="answer-content selectable">
      <pre class="answer-text">{stripMarkdown(answerText)}</pre>
      {#if streaming && !toolActivity}
        <span class="cursor">▋</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .message { margin-bottom: 18px; }

  .user-bubble {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    width: min(80%, 42rem);
  }

  .user-content {
    background: var(--surface);
    border-radius: 5px;
    color: var(--text);
    font-size: 16px;
    padding: 12px 16px;
    display: block;
    width: 100%;
    text-align: left;
    white-space: pre-wrap;
    word-break: normal;
    overflow-wrap: break-word;
  }

  .attachment-note {
    color: #9ad1ff;
    font-size: 12px;
  }

  .message.user { display: flex; justify-content: flex-end; }

  .answer-content {
    font-size: 16px;
    color: var(--text);
    line-height: 1.7;
    display: flex;
    align-items: flex-end;
    gap: 2px;
  }

  .answer-text {
    font-family: 'Courier New', Courier, monospace;
    font-size: 16px;
    white-space: pre-wrap;
    word-break: normal;
    overflow-wrap: break-word;
    flex: 1;
  }

  .cursor {
    color: var(--green);
    animation: blink 1s step-end infinite;
    line-height: 1;
    font-size: 19px;
  }

  @keyframes blink {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0; }
  }

  .tool-activity {
    color: var(--muted);
    font-size: 15px;
    font-style: italic;
    margin-bottom: 6px;
    padding: 4px 0;
  }
</style>

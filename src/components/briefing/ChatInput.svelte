<!-- HalluScribe - chat input bar (textarea + tool toggles + optional image attach). -->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import type { ChatAttachment, WebSearchStatus } from "../../lib/types";

  // No webp: llama.cpp's decoder cannot read it and drops the image without
  // erroring, so the model answers as if nothing were attached. Rust rejects it
  // with an explanation too, for paths typed rather than picked.
  const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "jfif", "gif"];

  interface Props {
    disabled: boolean;
    streaming: boolean;
    onsubmit: (text: string, attachment: ChatAttachment | null) => void | Promise<unknown>;
    onstop: () => void | Promise<unknown>;
    ctxUsedPct?: number;
    onclearchat?: () => void | Promise<unknown>;
    webSearchEnabled: boolean;
    saveSessionEnabled: boolean;
    thinkingEnabled: boolean;
    thinkingChangePending: boolean;
    webSearchStatus: WebSearchStatus;
    ontogglewebsearch: () => void;
    ontogglesavesession: () => void;
    ontogglethinking: () => void;
    imageAttachEnabled: boolean;
    saveSessionNote: string | null;
    saveSessionNoteIsError: boolean;
  }

  let {
    disabled,
    streaming,
    onsubmit,
    onstop,
    ctxUsedPct = 0,
    onclearchat,
    webSearchEnabled,
    saveSessionEnabled,
    thinkingEnabled,
    thinkingChangePending,
    webSearchStatus,
    ontogglewebsearch,
    ontogglesavesession,
    ontogglethinking,
    imageAttachEnabled,
    saveSessionNote,
    saveSessionNoteIsError,
  }: Props = $props();
  let draft = $state("");
  let webNote = $state("");
  let attachment = $state<ChatAttachment | null>(null);

  function submit() {
    const text = draft.trim();
    if ((!text && !attachment) || disabled) return;
    webNote = "";
    const nextText = text;
    const nextAttachment = attachment;
    draft = "";
    attachment = null;
    onsubmit(nextText, nextAttachment);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  function toggleWebSearch() {
    if (disabled) return;
    if (webSearchStatus === "missing-api-key") {
      webNote = "Add an Ollama API key in Settings to enable web search for chat.";
      return;
    }
    if (webSearchStatus === "loading") {
      webNote = "Loading chat settings...";
      return;
    }
    webNote = "";
    ontogglewebsearch();
  }

  // Uses the Tauri dialog plugin rather than <input type="file">. A browser
  // file dialog is owned by the webview, so with "always on top" set it loses
  // the z-order fight against our own window and hides behind it, leaving the
  // app looking frozen while an invisible modal holds input focus.
  async function openImagePicker() {
    if (disabled) return;
    if (!imageAttachEnabled) {
      webNote = "Image analysis is unavailable until a chat backend is configured.";
      return;
    }
    webNote = "";
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: "Images", extensions: IMAGE_EXTENSIONS }],
      });
      if (selected === null) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      attachment = await invoke<ChatAttachment>("read_image_attachment", { path });
      webNote = "";
    } catch (e) {
      webNote = typeof e === "string" ? e : "Failed to read the selected image.";
    }
  }

  function clearAttachment() {
    attachment = null;
  }

  function toggleThinking() {
    ontogglethinking();
  }
</script>

<div class="chat-input-wrap">
  <div class="bar">
    <div class="tool-stack">
      <button
        class:web-active={webSearchEnabled && webSearchStatus === "ready"}
        class="web-btn"
        onclick={toggleWebSearch}
        title="Toggle web search for this chat session"
        aria-label="Toggle web search for this chat session"
        disabled={disabled}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="8.5"></circle>
          <path d="M3.5 12h17"></path>
          <path d="M12 3.5c2.6 2.5 4 5.4 4 8.5s-1.4 6-4 8.5c-2.6-2.5-4-5.4-4-8.5s1.4-6 4-8.5z"></path>
        </svg>
      </button>
      <button
        class:image-active={Boolean(attachment)}
        class="image-btn"
        onclick={openImagePicker}
        title="Attach one image for chat vision"
        aria-label="Attach one image for chat vision"
        disabled={disabled}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="4" y="5" width="16" height="14" rx="2"></rect>
          <circle cx="9" cy="10" r="1.5"></circle>
          <path d="M6.5 17l4.5-4.5 2.5 2.5 3-3 2 2"></path>
        </svg>
      </button>
      <button
        class:thinking-active={thinkingEnabled}
        class:thinking-pending={thinkingChangePending}
        class="thinking-btn"
        onclick={toggleThinking}
        title="Toggle reasoning for chat replies"
        aria-label="Toggle reasoning for chat replies"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 4a6 6 0 0 0-4.8 9.6c.7.9 1.2 1.7 1.4 2.4h6.8c.2-.7.7-1.5 1.4-2.4A6 6 0 0 0 12 4z"></path>
          <path d="M9.5 18h5"></path>
          <path d="M10.5 20h3"></path>
        </svg>
      </button>
    </div>
    <textarea
      class="input selectable"
      rows="5"
      placeholder="ask anything about your sessions..."
      bind:value={draft}
      {disabled}
      onkeydown={onKeyDown}
    ></textarea>
    <div class="send-stack">
      {#if onclearchat}
        {@const remaining = 100 - ctxUsedPct}
        {@const color = remaining > 50 ? '#22c55e' : remaining > 20 ? '#ea580c' : '#dc2626'}
        {@const fill  = remaining > 50 ? '#14532d' : remaining > 20 ? '#fed7aa' : '#fecaca'}
        {@const label = remaining > 50 ? 'new chat' : remaining > 20 ? 'getting long' : 'clear now'}
        <button
          class="ctx-meter"
          class:ctx-pulse={remaining <= 20}
          style="background: linear-gradient(to right, {color} {ctxUsedPct}%, {fill} {ctxUsedPct}%)"
          title="Context used: {ctxUsedPct}% — click to start a fresh chat"
          onclick={onclearchat}
          aria-label="Context used: {ctxUsedPct}% — click to start a fresh chat"
        ><span class="ctx-label">{label}</span></button>
      {/if}
      <button
        class:save-active={saveSessionEnabled}
        class="save-btn"
        onclick={ontogglesavesession}
        title="Toggle session recording for this chat"
        aria-label="Toggle session recording for this chat"
      >
        save
      </button>
      <button
        class:stop-btn={streaming}
        class="btn-primary send-btn"
        onclick={streaming ? onstop : submit}
        title={streaming ? "Stop chat" : "Send chat"}
        aria-label={streaming ? "Stop chat" : "Send chat"}
      >
        {#if streaming}
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="7" y="7" width="10" height="10" rx="1.5"></rect>
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M6 12h9"></path>
            <path d="M12 7l5 5-5 5"></path>
          </svg>
        {/if}
      </button>
    </div>
  </div>
  {#if attachment}
    <div class="attachment-row">
      <button class="attachment-chip" onclick={clearAttachment} title="Remove attached image">
        <img
          class="attachment-thumb"
          src="data:{attachment.mimeType};base64,{attachment.base64}"
          alt="Attached: {attachment.name}"
        />
        <span class="attachment-name">{attachment.name}</span>
        <span class="attachment-remove" aria-hidden="true">x</span>
      </button>
    </div>
  {/if}
  {#if thinkingChangePending}
    <div class="thinking-note">Reasoning change applies next message.</div>
  {/if}
  {#if saveSessionNote}
    <div class:save-note-error={saveSessionNoteIsError} class="save-note">{saveSessionNote}</div>
  {/if}
  {#if webNote}
    <div class="web-note">{webNote}</div>
  {/if}
</div>

<style>

  .chat-input-wrap {
    border-top: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }

  .bar {
    display: flex;
    align-items: stretch;
    gap: 12px;
    padding: 12px 16px;
  }

  .tool-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    justify-content: center;
    align-self: center;
  }

  .send-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    justify-content: center;
    align-self: center;
  }

  .input {
    flex: 1;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    color: var(--text);
    font-family: "Courier New", Courier, monospace;
    font-size: 17px;
    padding: 10px 14px;
    resize: none;
    outline: none;
    line-height: 1.65;
    min-height: 88px;
  }

  .input::placeholder {
    color: var(--amber);
    opacity: 0.6;
  }

  .input:focus { border-color: var(--muted); }
  .input:disabled { opacity: 0.5; cursor: default; }

  .save-btn,
  .web-btn,
  .image-btn,
  .thinking-btn,
  .send-btn {
    flex-shrink: 0;
    width: 38px;
    min-width: 38px;
    height: 38px;
  }

  .save-btn,
  .web-btn,
  .image-btn,
  .thinking-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.03);
    color: #8ea0bb;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s, background 0.15s, transform 0.15s;
  }

  .save-btn {
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .web-btn svg,
  .image-btn svg,
  .thinking-btn svg {
    width: 20px;
    height: 20px;
    stroke: currentColor;
    stroke-width: 1.7;
    fill: none;
  }

  .save-btn:hover:not(:disabled),
  .web-btn:hover:not(:disabled),
  .image-btn:hover:not(:disabled),
  .thinking-btn:hover {
    border-color: var(--amber);
    color: var(--amber);
    background: rgba(214, 143, 57, 0.08);
  }

  .save-btn.save-active {
    border-color: #d14d41;
    color: #ffd5cf;
    background: rgba(209, 77, 65, 0.2);
  }

  .web-btn.web-active {
    border-color: var(--green);
    color: #87d59e;
    background: rgba(80, 180, 120, 0.12);
  }

  .image-btn.image-active {
    border-color: #77c8ff;
    color: #77c8ff;
    background: rgba(76, 157, 255, 0.12);
  }

  .thinking-btn.thinking-active {
    border-color: #d9b36c;
    color: #f4d38c;
    background: rgba(214, 143, 57, 0.16);
  }

  .thinking-btn.thinking-pending {
    border-style: dashed;
  }

  .send-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    align-self: center;
  }

  .send-btn svg {
    width: 20px;
    height: 20px;
    stroke: currentColor;
    stroke-width: 1.9;
    fill: none;
  }

  .send-btn.stop-btn {
    border-color: #d06a6a;
    color: #ffd0d0;
    background: rgba(160, 60, 60, 0.18);
  }

  .save-btn:disabled,
  .web-btn:disabled,
  .image-btn:disabled { opacity: 0.4; cursor: default; }

  .thinking-note {
    padding: 0 16px 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .attachment-row {
    padding: 0 16px 8px;
  }

  .attachment-chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid #4c9dff55;
    background: rgba(76, 157, 255, 0.12);
    color: #9ad1ff;
    border-radius: 8px;
    padding: 5px 10px 5px 5px;
    font-size: 12px;
    cursor: pointer;
  }

  .attachment-thumb {
    width: 34px;
    height: 34px;
    /* Crop to a square rather than distorting whatever aspect ratio the user
       picked; the chip is a reminder of what is attached, not a viewer. */
    object-fit: cover;
    border-radius: 5px;
    display: block;
  }

  .attachment-name {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .attachment-remove {
    opacity: 0.7;
  }

  .save-note,
  .web-note {
    padding: 0 16px 12px;
    font-size: 12px;
  }

  .save-note {
    color: #ffb3a9;
  }

  .save-note.save-note-error {
    color: var(--amber);
  }

  .web-note {
    color: var(--amber);
  }

  .ctx-meter {
    position: relative;
    width: 38px;
    min-width: 38px;
    height: 38px;
    padding: 0;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    flex-shrink: 0;
    transition: opacity 0.15s, transform 0.1s;
    overflow: visible;
  }
  .ctx-meter:hover { opacity: 0.85; transform: scale(1.04); }
  .ctx-label {
    position: absolute;
    right: calc(100% + 6px);
    top: 50%;
    transform: translateY(-50%);
    background: rgba(20, 24, 36, 0.92);
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    white-space: nowrap;
    padding: 3px 7px;
    border-radius: 5px;
    pointer-events: none;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .ctx-meter:hover .ctx-label { opacity: 1; }
  .ctx-pulse { animation: ctx-pulse 1.2s ease-in-out infinite; }
  @keyframes ctx-pulse {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0.65; }
  }
</style>

<!-- HalluScribe - root component. Nav routing + top bar. -->
<script lang="ts">
  import { exit } from "@tauri-apps/plugin-process";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open } from "@tauri-apps/plugin-shell";
  import { onMount } from "svelte";
  import NavBar from "./components/NavBar.svelte";
  import WorkspaceBadge from "./components/WorkspaceBadge.svelte";
  import RunNowButton from "./components/RunNowButton.svelte";
  import CaptureStatusLine from "./components/CaptureStatusLine.svelte";
  import BriefingPanel from "./components/briefing/BriefingPanel.svelte";
  import SessionList from "./components/sessions/SessionList.svelte";
  import ProfilePanel from "./components/profile/ProfilePanel.svelte";
  import SettingsForm from "./components/settings/SettingsForm.svelte";
  import { BriefingController } from "./lib/briefing.svelte";
  import { ChatController } from "./lib/chat.svelte";
  import { SweepController } from "./lib/sweep.svelte";
  import { SpeakController } from "./lib/tts.svelte.ts";
  import type {
    BriefingScope,
    RawSearchResult,
    HalluScribeSettings,
  } from "./lib/types";

  type Tab = "briefing" | "sessions" | "profile" | "settings";
  let activeTab = $state<Tab>("briefing");

  // One shared SpeakController owned here (App is never destroyed) so TTS
  // playback keeps running when the user switches tabs — BriefingPanel used to
  // own it and cancelled playback in its onDestroy on every tab change.
  const speakController = new SpeakController();
  let ctxVisible = $state(false);
  let ctxX = $state(0);
  let ctxY = $state(0);
  // Session-list search state lifted here so the raw/summaries scope, the query,
  // and a completed raw scan survive tab switches (App is never destroyed —
  // SessionList is re-created on every tab change). Matches how chat scope/turns
  // persist above.
  let sessionScope = $state<"summaries" | "raw">("summaries");
  let sessionQuery = $state("");
  let sessionRawResult = $state<RawSearchResult | null>(null);
  let sessionRawError = $state<string | null>(null);
  let settingsSnapshot = $state<HalluScribeSettings | null>(null);
  const briefing = new BriefingController();
  const chat = new ChatController(() => settingsSnapshot, () => briefing.scope);
  const sweep = new SweepController();
  let isMaximized = $state(false);
  const appWindow = getCurrentWindow();

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    ctxX = e.clientX;
    ctxY = e.clientY;
    ctxVisible = true;
  }

  function closeCtx() { ctxVisible = false; }

  onMount(async () => {
    isMaximized = await appWindow.isMaximized();
    const unlistenResize = await appWindow.onResized(async () => {
      isMaximized = await appWindow.isMaximized();
    });

    void refreshSettingsSnapshot();
    const listeners = (await Promise.all([
      briefing.registerListeners(),
      chat.registerListeners(),
      sweep.registerListeners(),
    ])).flat();

    return () => {
      unlistenResize();
      listeners.forEach((unlisten) => unlisten());
      sweep.dispose();
    };
  });

  async function hideToTray() {
    await appWindow.hide();
  }

  async function toggleMaximizeWindow() {
    await appWindow.toggleMaximize();
    isMaximized = await appWindow.isMaximized();
  }

  async function exitApp() {
    await exit(0);
  }

  async function refreshSettingsSnapshot() {
    settingsSnapshot = await invoke<HalluScribeSettings>("get_settings");
  }

  async function sendSelectedSessionsToBriefing(sessionIds: string[]) {
    const cleared = await chat.clear("scope-change");
    if (!cleared) return;
    const count = sessionIds.length;
    const nextScope: BriefingScope = {
      kind: "selected-session-ids",
      sessionIds,
      label: `Scoped to ${count} selected session${count === 1 ? "" : "s"}`,
    };
    briefing.resetScope(nextScope);
    chat.resetForBriefingScope(true);
    activeTab = "briefing";
    void briefing.run({ dateFrom: "", dateTo: "", fillMin: "", fillMax: "", keyword: "" });
  }

  async function clearSelectedScope() {
    const cleared = await chat.clear("scope-change");
    if (!cleared) return;
    briefing.resetScope({ kind: "archive-wide" });
    chat.resetForBriefingScope(false);
  }
</script>

<!-- data-tauri-drag-region makes the top bar drag the frameless window -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="layout" oncontextmenu={onContextMenu}>
  <div class="top-bar" data-tauri-drag-region>
    <button class="brand" onclick={() => open("https://x.com/amandoulou")} title="@amandoulou on X">HALLUSCRIBE</button>
    <NavBar active={activeTab} onchange={(tab) => { activeTab = tab; }} />
    <div class="top-bar-actions">
      <WorkspaceBadge />
      <CaptureStatusLine />
      {#if activeTab === "sessions"}
        <RunNowButton
          running={sweep.running}
          progress={sweep.progress}
          toast={sweep.toast}
          onRunNow={() => sweep.runNow()}
          onStop={() => sweep.cancel()}
        />
      {/if}
      <div class="window-controls">
        <button class="window-btn" onclick={hideToTray} title="Hide to tray">−</button>
        <button class="window-btn" onclick={toggleMaximizeWindow} title={isMaximized ? "Restore" : "Maximize"}>
          {#if isMaximized}❐{:else}□{/if}
        </button>
        <button class="window-btn window-btn-close" onclick={exitApp} title="Exit">×</button>
      </div>
    </div>
  </div>

  <main class="view">
    {#if activeTab === "briefing"}
      <BriefingPanel
        briefingAnswer={briefing.answer}
        briefingHeader={briefing.header}
        briefingStreaming={briefing.streaming}
        briefingError={briefing.error}
        briefingWarning={briefing.warning}
        briefingScopeLabel={briefing.scopeLabel()}
        selectedScopeActive={briefing.selectedScopeActive()}
        chatScopeKind={chat.scope.kind}
        chatSearchMode={chat.searchMode}
        chatProfileScope={chat.profileScope}
        turns={chat.turns}
        chatStreaming={chat.streaming}
        webSearchEnabled={chat.webSearchEnabled}
        saveSessionEnabled={chat.saveSessionEnabled}
        saveSessionNote={chat.saveSessionNote}
        saveSessionNoteIsError={chat.saveSessionNoteIsError}
        thinkingEnabled={chat.thinkingEnabled}
        thinkingChangePending={chat.thinkingChangePending}
        webSearchStatus={chat.webSearchStatus()}
        imageAttachEnabled={chat.imageAttachEnabled()}
        {speakController}
        onRefreshBriefing={(filters) => briefing.run(filters)}
        onStopBriefing={() => briefing.cancel()}
        onSendChat={(text, attachment) => chat.send(text, attachment)}
        onStopChat={() => chat.stop()}
        ctxUsedPct={chat.ctxUsedPct}
        onClearChat={() => chat.clear("clear")}
        onToggleWebSearch={() => chat.toggleWebSearch()}
        onToggleSaveSession={() => chat.toggleSaveSession()}
        onToggleThinking={() => chat.toggleThinking()}
        onSetChatScope={(kind) => chat.setScope(kind)}
        onSetChatSearchMode={(mode) => chat.setSearchMode(mode)}
        onSelectProfileScope={(scope) => chat.setProfileScope(scope)}
        onClearScope={clearSelectedScope}
      />
    {:else if activeTab === "sessions"}
      <SessionList
        onSendToBriefing={sendSelectedSessionsToBriefing}
        bind:searchScope={sessionScope}
        bind:query={sessionQuery}
        bind:rawResult={sessionRawResult}
        bind:rawError={sessionRawError}
      />
    {:else if activeTab === "profile"}
      <ProfilePanel />
    {:else}
      <SettingsForm
        initialSettings={settingsSnapshot}
        onSaved={() => { void refreshSettingsSnapshot(); }}
      />
    {/if}
  </main>
</div>

{#if ctxVisible}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="ctx-backdrop" onclick={closeCtx} role="presentation"></div>
  <div class="ctx-menu" style="left:{ctxX}px; top:{ctxY}px;">
    <button onclick={() => { closeCtx(); void hideToTray(); }}>Hide to tray</button>
    <button onclick={() => { closeCtx(); void toggleMaximizeWindow(); }}>{isMaximized ? "Restore" : "Maximize"}</button>
    <button onclick={() => { closeCtx(); void exitApp(); }}>Exit</button>
  </div>
{/if}

<style>
  .layout {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }

  .top-bar-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    -webkit-app-region: no-drag;
  }

  .window-controls {
    display: flex;
    align-items: stretch;
    margin-right: -18px;
  }

  .window-btn {
    width: 46px;
    height: 54px;
    background: transparent;
    border: none;
    color: var(--dim);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .window-btn:hover {
    background: var(--border);
    color: var(--text);
  }

  .window-btn-close:hover {
    background: var(--red);
    color: var(--text);
  }

  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
  }

  .ctx-menu {
    position: fixed;
    z-index: 1000;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px 0;
    min-width: 130px;
    box-shadow: 0 4px 16px rgba(0,0,0,0.4);
  }

  .ctx-menu button {
    display: block;
    width: 100%;
    background: none;
    border: none;
    color: var(--text);
    font-family: inherit;
    font-size: 16px;
    text-align: left;
    padding: 10px 18px;
    cursor: pointer;
  }

  .ctx-menu button:hover { background: var(--border); }
</style>
